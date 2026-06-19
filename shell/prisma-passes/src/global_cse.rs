//! Global common subexpression elimination over a `Function`.
//!
//! Mirrors C++ `global_cse`. Forwards the intra-block available-expression
//! table along dominator-tree edges where the dominated child has exactly one
//! CFG predecessor and that predecessor is its immediate dominator. Join
//! blocks (multiple predecessors) start with an empty table — conservative but
//! sound. Within each block the logic matches `cse`.

use std::collections::HashMap;

use prisma_ir::{BinOpKind, Function, Op, OpSize, Ref};

use crate::cfg;
use crate::Pass;

type BinOpKey = (BinOpKind, Ref, Ref, OpSize);
type AvailTable = HashMap<BinOpKey, Ref>;

fn is_flushing_op(op: &Op) -> bool {
    matches!(
        op,
        Op::StoreReg(_)
            | Op::StoreMem(_)
            | Op::StoreMemTSO(_)
            | Op::CmpFlags(_)
            | Op::AluFlags(_)
            | Op::WriteFlagsPopcnt(_)
            | Op::WriteFlagsCountZero(_)
            | Op::Syscall(_)
    )
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GlobalCse;

impl GlobalCse {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Pass for GlobalCse {
    fn name(&self) -> &'static str {
        "global_cse"
    }

    fn run(&self, func: Function) -> Function {
        global_cse(func)
    }
}

/// Dominator-tree-forwarded CSE over the whole function.
pub fn global_cse(mut func: Function) -> Function {
    if func.blocks.is_empty() {
        return func;
    }

    // id -> position in func.blocks
    let idx: HashMap<u32, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.id, i))
        .collect();

    // Predecessor map from successors().
    let mut preds: HashMap<u32, Vec<u32>> = HashMap::new();
    for b in &func.blocks {
        for s in cfg::successors(&func, b.id) {
            preds.entry(s).or_default().push(b.id);
        }
    }

    let idoms = cfg::dominators(&func);
    if !idx.contains_key(&func.entry) {
        return func;
    }

    // Reverse postorder: each block visited after its dominators.
    let mut rpo = cfg::postorder(&func);
    rpo.reverse();

    let mut end_table: HashMap<u32, AvailTable> = HashMap::new();

    for block_id in rpo {
        let mut seen: AvailTable = AvailTable::new();
        if block_id != func.entry {
            if let Some(bpreds) = preds.get(&block_id) {
                if bpreds.len() == 1 {
                    let pred = bpreds[0];
                    if idoms.get(&block_id) == Some(&pred) {
                        if let Some(t) = end_table.get(&pred) {
                            seen.clone_from(t);
                        }
                    }
                }
            }
        }

        let Some(&block_pos) = idx.get(&block_id) else {
            continue; // defensive: postorder only yields real blocks, but guard anyway
        };
        let block = &mut func.blocks[block_pos];
        for stmt in &mut block.stmts {
            if is_flushing_op(&stmt.op) {
                seen.clear();
                continue;
            }
            if let (Some(result), Op::BinOp(b)) = (stmt.result, &stmt.op) {
                let key = (b.op, b.lhs, b.rhs, b.size);
                if let Some(&prev) = seen.get(&key) {
                    let size = b.size;
                    stmt.op = Op::BinOp(prisma_ir::BinOp {
                        op: BinOpKind::Or,
                        lhs: prev,
                        rhs: prev,
                        size,
                    });
                } else {
                    seen.insert(key, result);
                }
            }
        }

        end_table.insert(block_id, seen);
    }

    func
}

#[cfg(test)]
mod tests {
    use super::*;
    use prisma_ir::{BasicBlock, BinOp, Jump, Stmt};

    fn binop(result: u32, lhs: u32, rhs: u32) -> Stmt {
        Stmt::new(
            Some(result),
            Op::BinOp(BinOp {
                op: BinOpKind::Add,
                lhs,
                rhs,
                size: OpSize::I64,
            }),
        )
    }

    #[test]
    fn forwards_across_single_pred_idom_edge() {
        // block0: r2 = add r0,r1 ; jump 1
        // block1: r3 = add r0,r1  -> rewritten to Or r2,r2 (forwarded)
        let f = Function {
            entry: 0,
            blocks: vec![
                BasicBlock {
                    id: 0,
                    stmts: vec![
                        binop(2, 0, 1),
                        Stmt::new(None, Op::Jump(Jump { target_block: 1 })),
                    ],
                },
                BasicBlock {
                    id: 1,
                    stmts: vec![binop(3, 0, 1)],
                },
            ],
        };
        let out = global_cse(f);
        match &out.blocks[1].stmts[0].op {
            Op::BinOp(b) => {
                assert_eq!(b.op, BinOpKind::Or);
                assert_eq!((b.lhs, b.rhs), (2, 2));
            }
            other => panic!("expected forwarded copy, got {other:?}"),
        }
    }

    #[test]
    fn join_block_does_not_forward() {
        // 0 -> {1,2} -> 3. block3 has two preds, so no forwarding even if
        // both branches compute the same expr.
        let f = Function {
            entry: 0,
            blocks: vec![
                BasicBlock {
                    id: 0,
                    stmts: vec![
                        binop(2, 0, 1),
                        Stmt::new(
                            None,
                            Op::CondJump(prisma_ir::CondJump {
                                cond: 2,
                                if_true: 1,
                                if_false: 2,
                            }),
                        ),
                    ],
                },
                BasicBlock {
                    id: 1,
                    stmts: vec![Stmt::new(None, Op::Jump(Jump { target_block: 3 }))],
                },
                BasicBlock {
                    id: 2,
                    stmts: vec![Stmt::new(None, Op::Jump(Jump { target_block: 3 }))],
                },
                BasicBlock {
                    id: 3,
                    stmts: vec![binop(5, 0, 1)],
                },
            ],
        };
        let out = global_cse(f);
        // block3's add is NOT forwarded (join block starts empty).
        match &out.blocks[3].stmts[0].op {
            Op::BinOp(b) => assert_eq!(b.op, BinOpKind::Add),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn intra_block_dedup_still_works() {
        let f = Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![binop(2, 0, 1), binop(3, 0, 1)],
            }],
        };
        let out = global_cse(f);
        match &out.blocks[0].stmts[1].op {
            Op::BinOp(b) => assert_eq!((b.op, b.lhs, b.rhs), (BinOpKind::Or, 2, 2)),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn empty_function_unchanged() {
        let f = Function {
            entry: 0,
            blocks: vec![],
        };
        assert_eq!(global_cse(f.clone()), f);
    }
}
