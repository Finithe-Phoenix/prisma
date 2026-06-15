//! Loop-invariant code motion.
//!
//! Mirrors C++ `loop_invariant_motion`. For each natural loop, hoist pure
//! statements whose operands are all defined outside the loop body into the
//! loop's preheader (the unique non-loop predecessor of the header). Re-scans
//! to a fixed point per loop, since a hoist can unlock further hoists.
//!
//! Conservative: skips a loop unless its header has exactly one outside
//! predecessor (no synthetic preheader insertion). Terminators are never
//! hoisted. Today's translator emits single-block functions (zero loops), so
//! this is plumbing — the gain unlocks once multi-block loops are emitted.

use std::collections::{HashMap, HashSet};

use prisma_ir::{Function, Op, Ref};

use crate::cfg;
use crate::Pass;

/// Purity set for hoisting (matches C++ `stmt_is_pure` in licm.cpp — narrower
/// than DCE's: only side-effect-free, position-independent value computations).
fn stmt_is_pure(op: &Op) -> bool {
    matches!(
        op,
        Op::Constant(_)
            | Op::LoadReg(_)
            | Op::LoadSegBase(_)
            | Op::BinOp(_)
            | Op::Compare(_)
            | Op::Extend(_)
            | Op::Truncate(_)
    )
}

fn operand_refs(op: &Op, into: &mut Vec<Ref>) {
    match op {
        Op::BinOp(x) => {
            into.push(x.lhs);
            into.push(x.rhs);
        }
        Op::Compare(x) => {
            into.push(x.lhs);
            into.push(x.rhs);
        }
        Op::Extend(x) => into.push(x.value),
        Op::Truncate(x) => into.push(x.value),
        // Constant / LoadReg / LoadSegBase have no operand refs.
        _ => {}
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Licm;

impl Licm {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Pass for Licm {
    fn name(&self) -> &'static str {
        "loop_invariant_motion"
    }

    fn run(&self, func: Function) -> Function {
        loop_invariant_motion(func)
    }
}

/// Hoist loop-invariant statements to loop preheaders.
pub fn loop_invariant_motion(mut func: Function) -> Function {
    if func.blocks.is_empty() {
        return func;
    }
    let loops = cfg::natural_loops(&func);
    if loops.is_empty() {
        return func;
    }

    let idx: HashMap<u32, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.id, i))
        .collect();

    // Predecessor map for preheader detection.
    let mut preds: HashMap<u32, Vec<u32>> = HashMap::new();
    for b in &func.blocks {
        for s in cfg::successors(&func, b.id) {
            preds.entry(s).or_default().push(b.id);
        }
    }

    for loop_ in &loops {
        let body_ids: HashSet<u32> = loop_.body.iter().copied().collect();

        // Preheader = unique non-loop predecessor of the header.
        let Some(hpreds) = preds.get(&loop_.header) else {
            continue;
        };
        let outside: Vec<u32> = hpreds
            .iter()
            .copied()
            .filter(|p| !body_ids.contains(p))
            .collect();
        if outside.len() != 1 {
            continue;
        }
        let preheader_id = outside[0];
        let Some(&preheader_pos) = idx.get(&preheader_id) else {
            continue;
        };
        if func.blocks[preheader_pos].stmts.is_empty() {
            continue; // no terminator slot
        }

        // Refs defined in the loop body; shrinks as we hoist.
        let mut body_refs: HashSet<Ref> = HashSet::new();
        for &bid in &loop_.body {
            for st in &func.blocks[idx[&bid]].stmts {
                if let Some(r) = st.result {
                    body_refs.insert(r);
                }
            }
        }

        let mut changed = true;
        while changed {
            changed = false;
            for &bid in &loop_.body {
                let bpos = idx[&bid];
                loop {
                    let term_pos = {
                        let n = func.blocks[bpos].stmts.len();
                        if n == 0 {
                            break;
                        }
                        n - 1
                    };
                    // Find the first hoistable statement before the terminator.
                    let mut hoist_at: Option<usize> = None;
                    for i in 0..term_pos {
                        let stmt = &func.blocks[bpos].stmts[i];
                        if stmt.result.is_none() || !stmt_is_pure(&stmt.op) {
                            continue;
                        }
                        let mut ops = Vec::new();
                        operand_refs(&stmt.op, &mut ops);
                        if ops.iter().all(|r| !body_refs.contains(r)) {
                            hoist_at = Some(i);
                            break;
                        }
                    }
                    let Some(i) = hoist_at else { break };

                    // Remove from body, insert before the preheader terminator.
                    let hoisted = func.blocks[bpos].stmts.remove(i);
                    let hoisted_result = hoisted.result;
                    let ph = &mut func.blocks[preheader_pos].stmts;
                    let insert_pos = ph.len() - 1;
                    ph.insert(insert_pos, hoisted);
                    if let Some(r) = hoisted_result {
                        body_refs.remove(&r);
                    }
                    changed = true;
                }
            }
        }
    }

    func
}

#[cfg(test)]
mod tests {
    use super::*;
    use prisma_ir::{BasicBlock, BinOp, BinOpKind, CondJump, Constant, Jump, OpSize, Return, Stmt};

    #[test]
    fn no_loops_is_noop() {
        let f = Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![Stmt::new(None, Op::Return(Return))],
            }],
        };
        assert_eq!(loop_invariant_motion(f.clone()), f);
    }

    #[test]
    fn hoists_invariant_binop_to_preheader() {
        // block0 (preheader): r0=const; r1=const; jump 1
        // block1 (loop header): r2 = add r0,r1 (invariant); cond 1/2
        // block2: ret
        // r2 should hoist into block0 before its terminator.
        let f = Function {
            entry: 0,
            blocks: vec![
                BasicBlock {
                    id: 0,
                    stmts: vec![
                        Stmt::new(
                            Some(0),
                            Op::Constant(Constant {
                                value: 3,
                                size: OpSize::I64,
                            }),
                        ),
                        Stmt::new(
                            Some(1),
                            Op::Constant(Constant {
                                value: 4,
                                size: OpSize::I64,
                            }),
                        ),
                        Stmt::new(None, Op::Jump(Jump { target_block: 1 })),
                    ],
                },
                BasicBlock {
                    id: 1,
                    stmts: vec![
                        Stmt::new(
                            Some(2),
                            Op::BinOp(BinOp {
                                op: BinOpKind::Add,
                                lhs: 0,
                                rhs: 1,
                                size: OpSize::I64,
                            }),
                        ),
                        Stmt::new(
                            None,
                            Op::CondJump(CondJump {
                                cond: 2,
                                if_true: 1,
                                if_false: 2,
                            }),
                        ),
                    ],
                },
                BasicBlock {
                    id: 2,
                    stmts: vec![Stmt::new(None, Op::Return(Return))],
                },
            ],
        };
        let out = loop_invariant_motion(f);
        // block0 now holds the hoisted add before its Jump terminator.
        let ph = &out.blocks[0].stmts;
        assert_eq!(ph.len(), 4);
        assert!(
            matches!(ph[2].op, Op::BinOp(_)),
            "add hoisted into preheader"
        );
        assert!(matches!(ph[3].op, Op::Jump(_)), "terminator stays last");
        // The loop header no longer contains the add (only its terminator).
        assert_eq!(out.blocks[1].stmts.len(), 1);
        assert!(matches!(out.blocks[1].stmts[0].op, Op::CondJump(_)));
    }

    #[test]
    fn variant_binop_is_not_hoisted() {
        // r2 depends on r2 (loop-carried via self) — not invariant.
        let f = Function {
            entry: 0,
            blocks: vec![
                BasicBlock {
                    id: 0,
                    stmts: vec![Stmt::new(None, Op::Jump(Jump { target_block: 1 }))],
                },
                BasicBlock {
                    id: 1,
                    stmts: vec![
                        Stmt::new(
                            Some(2),
                            Op::BinOp(BinOp {
                                op: BinOpKind::Add,
                                lhs: 2,
                                rhs: 2,
                                size: OpSize::I64,
                            }),
                        ),
                        Stmt::new(
                            None,
                            Op::CondJump(CondJump {
                                cond: 2,
                                if_true: 1,
                                if_false: 2,
                            }),
                        ),
                    ],
                },
                BasicBlock {
                    id: 2,
                    stmts: vec![Stmt::new(None, Op::Return(Return))],
                },
            ],
        };
        let out = loop_invariant_motion(f.clone());
        assert_eq!(out, f, "loop-carried value must not be hoisted");
    }
}
