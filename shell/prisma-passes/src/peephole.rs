//! Peephole optimization pass.
//!
//! Mirrors C++ `peephole_optimise_default`. A fixed set of local rewrite
//! rules is applied repeatedly (first-match-wins per position) up to
//! `MAX_ITERATIONS` until no rule fires. Rewrites lower to copy idioms
//! (`Truncate`/`Or`) or `Constant`, which downstream const-prop/DCE collapse.

use prisma_ir::{BasicBlock, Constant, Function, Op, OpSize, Ref, Stmt, Truncate};

use crate::Pass;

/// Termination bound (matches C++ `kPeepholeMaxIterations`).
const MAX_ITERATIONS: usize = 8;

/// Find the most recent statement before `idx` whose result is `r` and which
/// binds a `Constant`; return that constant value.
fn try_const(stmts: &[Stmt], idx: usize, r: Ref) -> Option<u64> {
    for i in (0..idx).rev() {
        if stmts[i].result == Some(r) {
            return match &stmts[i].op {
                Op::Constant(c) => Some(c.value),
                _ => None,
            };
        }
    }
    None
}

/// Try every rule at position `idx`; first match wins. Returns the
/// replacement op (always a single statement here) or `None`.
fn match_rule(stmts: &[Stmt], idx: usize) -> Option<Op> {
    use prisma_ir::BinOpKind as K;

    let s = &stmts[idx];
    // Every rule below requires a result ref; bail if absent.
    s.result?;

    let Op::BinOp(b) = &s.op else {
        // Non-BinOp rules: identity Extend.
        if let Op::Extend(e) = &s.op {
            if e.from_size == e.to_size {
                return Some(Op::Truncate(Truncate {
                    value: e.value,
                    to_size: e.to_size,
                }));
            }
        }
        return None;
    };

    // xor x,x -> 0
    if b.op == K::Xor && b.lhs == b.rhs {
        return Some(Op::Constant(Constant {
            value: 0,
            size: b.size,
        }));
    }
    // or x,x -> x ; and x,x -> x
    if matches!(b.op, K::Or | K::And) && b.lhs == b.rhs {
        return Some(Op::Truncate(Truncate {
            value: b.lhs,
            to_size: b.size,
        }));
    }
    // add: const-0 operand -> other
    if b.op == K::Add {
        let keep = if try_const(stmts, idx, b.lhs) == Some(0) {
            Some(b.rhs)
        } else if try_const(stmts, idx, b.rhs) == Some(0) {
            Some(b.lhs)
        } else {
            None
        };
        if let Some(keep) = keep {
            return Some(Op::Truncate(Truncate {
                value: keep,
                to_size: b.size,
            }));
        }
    }
    // sub x,0 -> x
    if b.op == K::Sub && try_const(stmts, idx, b.rhs) == Some(0) {
        return Some(Op::Truncate(Truncate {
            value: b.lhs,
            to_size: b.size,
        }));
    }
    // mul: const-1 operand -> other
    if b.op == K::Mul {
        let keep = if try_const(stmts, idx, b.lhs) == Some(1) {
            Some(b.rhs)
        } else if try_const(stmts, idx, b.rhs) == Some(1) {
            Some(b.lhs)
        } else {
            None
        };
        if let Some(keep) = keep {
            return Some(Op::Truncate(Truncate {
                value: keep,
                to_size: b.size,
            }));
        }
        // mul: const-0 operand -> 0
        if try_const(stmts, idx, b.lhs) == Some(0) || try_const(stmts, idx, b.rhs) == Some(0) {
            return Some(Op::Constant(Constant {
                value: 0,
                size: b.size,
            }));
        }
    }

    None
}

fn run_once(stmts: &[Stmt]) -> (Vec<Stmt>, bool) {
    let mut out = Vec::with_capacity(stmts.len());
    let mut changed = false;
    for idx in 0..stmts.len() {
        if let Some(new_op) = match_rule(stmts, idx) {
            out.push(Stmt::new(stmts[idx].result, new_op));
            changed = true;
        } else {
            out.push(stmts[idx].clone());
        }
    }
    (out, changed)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Peephole;

impl Peephole {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Pass for Peephole {
    fn name(&self) -> &'static str {
        "peephole"
    }

    fn run(&self, func: Function) -> Function {
        peephole_optimise_default(func)
    }
}

/// Per-block peephole to a fixed point.
pub fn peephole_optimise_default(func: Function) -> Function {
    let blocks = func
        .blocks
        .into_iter()
        .map(|block| {
            let mut work = block.stmts;
            for _ in 0..MAX_ITERATIONS {
                let (next, changed) = run_once(&work);
                work = next;
                if !changed {
                    break;
                }
            }
            BasicBlock {
                id: block.id,
                stmts: work,
            }
        })
        .collect();

    Function {
        blocks,
        entry: func.entry,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prisma_ir::{BinOp, BinOpKind, Extend};

    fn block(stmts: Vec<Stmt>) -> Function {
        Function {
            entry: 0,
            blocks: vec![BasicBlock { id: 0, stmts }],
        }
    }

    fn binop(result: u32, op: BinOpKind, lhs: u32, rhs: u32) -> Stmt {
        Stmt::new(
            Some(result),
            Op::BinOp(BinOp {
                op,
                lhs,
                rhs,
                size: OpSize::I64,
            }),
        )
    }

    fn cst(result: u32, value: u64) -> Stmt {
        Stmt::new(
            Some(result),
            Op::Constant(Constant {
                value,
                size: OpSize::I64,
            }),
        )
    }

    #[test]
    fn xor_self_becomes_zero() {
        let out = peephole_optimise_default(block(vec![binop(0, BinOpKind::Xor, 9, 9)]));
        match &out.blocks[0].stmts[0].op {
            Op::Constant(c) => assert_eq!(c.value, 0),
            other => panic!("expected 0, got {other:?}"),
        }
    }

    #[test]
    fn or_self_becomes_truncate_copy() {
        let out = peephole_optimise_default(block(vec![binop(0, BinOpKind::Or, 9, 9)]));
        match &out.blocks[0].stmts[0].op {
            Op::Truncate(t) => assert_eq!(t.value, 9),
            other => panic!("expected truncate, got {other:?}"),
        }
    }

    #[test]
    fn add_zero_keeps_other_operand() {
        // r0=0 ; r1 = add r9, r0  -> r1 = trunc r9
        let out = peephole_optimise_default(block(vec![cst(0, 0), binop(1, BinOpKind::Add, 9, 0)]));
        match &out.blocks[0].stmts[1].op {
            Op::Truncate(t) => assert_eq!(t.value, 9),
            other => panic!("expected truncate of r9, got {other:?}"),
        }
    }

    #[test]
    fn mul_one_keeps_other_and_mul_zero_folds() {
        let out = peephole_optimise_default(block(vec![
            cst(0, 1),
            binop(1, BinOpKind::Mul, 9, 0), // *1 -> trunc r9
            cst(2, 0),
            binop(3, BinOpKind::Mul, 9, 2), // *0 -> 0
        ]));
        match &out.blocks[0].stmts[1].op {
            Op::Truncate(t) => assert_eq!(t.value, 9),
            other => panic!("expected truncate, got {other:?}"),
        }
        match &out.blocks[0].stmts[3].op {
            Op::Constant(c) => assert_eq!(c.value, 0),
            other => panic!("expected 0, got {other:?}"),
        }
    }

    #[test]
    fn identity_extend_becomes_truncate() {
        let out = peephole_optimise_default(block(vec![Stmt::new(
            Some(0),
            Op::Extend(Extend {
                value: 9,
                from_size: OpSize::I32,
                to_size: OpSize::I32,
                is_signed: false,
            }),
        )]));
        match &out.blocks[0].stmts[0].op {
            Op::Truncate(t) => {
                assert_eq!(t.value, 9);
                assert_eq!(t.to_size, OpSize::I32);
            }
            other => panic!("expected truncate, got {other:?}"),
        }
    }

    #[test]
    fn non_identity_extend_is_untouched() {
        let func = block(vec![Stmt::new(
            Some(0),
            Op::Extend(Extend {
                value: 9,
                from_size: OpSize::I8,
                to_size: OpSize::I64,
                is_signed: true,
            }),
        )]);
        let out = peephole_optimise_default(func.clone());
        assert_eq!(out, func);
    }

    #[test]
    fn idempotent_at_fixed_point() {
        let func = block(vec![cst(0, 0), binop(1, BinOpKind::Xor, 9, 9)]);
        let once = peephole_optimise_default(func);
        let twice = peephole_optimise_default(once.clone());
        assert_eq!(once, twice);
    }
}
