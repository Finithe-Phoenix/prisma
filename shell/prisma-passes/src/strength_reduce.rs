//! Strength reduction pass.
//!
//! Mirrors C++ `strength_reduce`. MVP pattern only:
//!
//!   `Mul x, (1 << k)`  ->  `Shl x, k`   for a power-of-two constant operand.
//!
//! A fresh `Constant` ref is minted for the shift count; the old pow-of-two
//! constant is left for DCE to clean up. Signed/unsigned division-by-pow2 is
//! intentionally not handled (rounding direction differs for signed).

use std::collections::HashMap;

use prisma_ir::{BasicBlock, BinOp, BinOpKind, Constant, Function, Op, Ref, Stmt};

use crate::Pass;

/// If `v` is a non-zero power of two, return its log2.
fn log2_pow2(v: u64) -> Option<u64> {
    if v == 0 || (v & (v - 1)) != 0 {
        return None;
    }
    Some(u64::from(v.trailing_zeros()))
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct StrengthReduce;

impl StrengthReduce {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Pass for StrengthReduce {
    fn name(&self) -> &'static str {
        "strength_reduce"
    }

    fn run(&self, func: Function) -> Function {
        strength_reduce(func)
    }
}

/// Apply strength reduction, threading a function-global ref allocator so the
/// minted shift-count constants never collide across blocks.
pub fn strength_reduce(func: Function) -> Function {
    let mut next_ref: Ref = func
        .blocks
        .iter()
        .flat_map(|b| b.stmts.iter())
        .filter_map(|s| s.result)
        .map(|r| r + 1)
        .max()
        .unwrap_or(0);

    let blocks = func
        .blocks
        .into_iter()
        .map(|block| {
            let mut consts: HashMap<Ref, u64> = HashMap::new();
            let mut out = Vec::with_capacity(block.stmts.len());

            for stmt in block.stmts {
                if let (Some(result), Op::Constant(c)) = (stmt.result, &stmt.op) {
                    consts.insert(result, c.value & c.size.mask());
                }

                if let (Some(result), Op::BinOp(b)) = (stmt.result, &stmt.op) {
                    if b.op == BinOpKind::Mul {
                        let (k, shift_operand) = match (
                            consts.get(&b.rhs).and_then(|&v| log2_pow2(v)),
                            consts.get(&b.lhs).and_then(|&v| log2_pow2(v)),
                        ) {
                            (Some(k), _) => (Some(k), b.lhs),
                            (None, Some(k)) => (Some(k), b.rhs),
                            (None, None) => (None, 0),
                        };

                        if let Some(k) = k {
                            let ref_k = next_ref;
                            next_ref += 1;
                            consts.insert(ref_k, k);
                            out.push(Stmt::new(
                                Some(ref_k),
                                Op::Constant(Constant { value: k, size: b.size }),
                            ));
                            out.push(Stmt::new(
                                Some(result),
                                Op::BinOp(BinOp {
                                    op: BinOpKind::Shl,
                                    lhs: shift_operand,
                                    rhs: ref_k,
                                    size: b.size,
                                }),
                            ));
                            continue;
                        }
                    }
                }

                out.push(stmt);
            }

            BasicBlock {
                id: block.id,
                stmts: out,
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
    use prisma_ir::OpSize;

    #[test]
    fn mul_by_pow2_becomes_shift() {
        // r0 = 8 ; r1 = mul r9, r0   ->   r0=8 ; r2=3 ; r1 = shl r9, r2
        let func = Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![
                    Stmt::new(Some(0), Op::Constant(Constant { value: 8, size: OpSize::I64 })),
                    Stmt::new(
                        Some(1),
                        Op::BinOp(BinOp { op: BinOpKind::Mul, lhs: 9, rhs: 0, size: OpSize::I64 }),
                    ),
                ],
            }],
        };
        let out = strength_reduce(func);
        let stmts = &out.blocks[0].stmts;
        assert_eq!(stmts.len(), 3);
        // minted shift count = log2(8) = 3
        match &stmts[1].op {
            Op::Constant(c) => assert_eq!(c.value, 3),
            other => panic!("expected minted constant, got {other:?}"),
        }
        match &stmts[2].op {
            Op::BinOp(b) => {
                assert_eq!(b.op, BinOpKind::Shl);
                assert_eq!(b.lhs, 9);
                assert_eq!(b.rhs, stmts[1].result.unwrap());
            }
            other => panic!("expected shl, got {other:?}"),
        }
    }

    #[test]
    fn mul_by_non_pow2_is_untouched() {
        let func = Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![
                    Stmt::new(Some(0), Op::Constant(Constant { value: 6, size: OpSize::I64 })),
                    Stmt::new(
                        Some(1),
                        Op::BinOp(BinOp { op: BinOpKind::Mul, lhs: 9, rhs: 0, size: OpSize::I64 }),
                    ),
                ],
            }],
        };
        let out = strength_reduce(func.clone());
        assert_eq!(out, func);
    }

    #[test]
    fn idempotent() {
        let func = Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![
                    Stmt::new(Some(0), Op::Constant(Constant { value: 16, size: OpSize::I32 })),
                    Stmt::new(
                        Some(1),
                        Op::BinOp(BinOp { op: BinOpKind::Mul, lhs: 0, rhs: 9, size: OpSize::I32 }),
                    ),
                ],
            }],
        };
        let once = strength_reduce(func);
        let twice = strength_reduce(once.clone());
        assert_eq!(once, twice);
    }
}
