//! Algebraic simplification pass.
//!
//! Mirrors C++'s `algebraic_simplify`: identities that fire when *one*
//! operand of a `BinOp` is a known constant with a special value (0, 1, or
//! all-ones), or when both operands are the same SSA ref. Two-constant folds
//! are intentionally left to `const_prop` so the two passes stay disjoint.

use std::collections::HashMap;

use prisma_ir::{BasicBlock, BinOp, BinOpKind, Constant, Function, Op, OpSize, Stmt};

use crate::Pass;

/// Canonical masked all-ones for a size (matches C++ `all_ones`).
const fn all_ones(size: OpSize) -> u64 {
    size.mask()
}

const fn zero(size: OpSize) -> Op {
    Op::Constant(Constant { value: 0, size })
}

/// Try to rewrite a `BinOp` to a simpler `Op` (always a `Constant` here).
/// Returns `None` when no identity applies.
fn try_simplify(b: &BinOp, consts: &HashMap<u32, u64>) -> Option<Op> {
    use BinOpKind as K;

    let lhs = consts.get(&b.lhs).copied();
    let rhs = consts.get(&b.rhs).copied();
    let same_ref = b.lhs == b.rhs;

    // x - x -> 0 ; x ^ x -> 0
    if same_ref && matches!(b.op, K::Sub | K::Xor) {
        return Some(zero(b.size));
    }

    if let Some(rv) = rhs {
        // x * 0 -> 0 ; x & 0 -> 0
        if matches!(b.op, K::Mul | K::And) && rv == 0 {
            return Some(zero(b.size));
        }
        // x | -1 -> -1
        if b.op == K::Or && rv == all_ones(b.size) {
            return Some(Op::Constant(Constant {
                value: all_ones(b.size),
                size: b.size,
            }));
        }
        // high half of x*0 = 0 (signed and unsigned)
        if matches!(b.op, K::UMulHi | K::SMulHi) && rv == 0 {
            return Some(zero(b.size));
        }
        // high half of x*1 = 0 (unsigned only; signed needs sign info)
        if b.op == K::UMulHi && rv == 1 {
            return Some(zero(b.size));
        }
        // x % 1 = 0 (signed and unsigned)
        if matches!(b.op, K::UMod | K::SMod) && rv == 1 {
            return Some(zero(b.size));
        }
    }

    if let Some(lv) = lhs {
        // 0 * x -> 0 ; 0 & x -> 0
        if matches!(b.op, K::Mul | K::And) && lv == 0 {
            return Some(zero(b.size));
        }
        // -1 | x -> -1
        if b.op == K::Or && lv == all_ones(b.size) {
            return Some(Op::Constant(Constant {
                value: all_ones(b.size),
                size: b.size,
            }));
        }
        // high half of 0*x = 0 ; 0/x = 0 ; 0%x = 0
        if matches!(
            b.op,
            K::UMulHi | K::SMulHi | K::UDiv | K::SDiv | K::UMod | K::SMod
        ) && lv == 0
        {
            return Some(zero(b.size));
        }
    }

    None
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Algebraic;

impl Algebraic {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Pass for Algebraic {
    fn name(&self) -> &'static str {
        "algebraic_simplify"
    }

    fn run(&self, func: Function) -> Function {
        algebraic_simplify(func)
    }
}

/// Apply algebraic identities per basic block.
pub fn algebraic_simplify(func: Function) -> Function {
    let blocks = func
        .blocks
        .into_iter()
        .map(|block| {
            let mut consts: HashMap<u32, u64> = HashMap::new();
            let mut out = Vec::with_capacity(block.stmts.len());

            for stmt in block.stmts {
                let mut new_stmt = stmt;
                if let Some(result) = new_stmt.result {
                    match &new_stmt.op {
                        Op::Constant(c) => {
                            consts.insert(result, c.value & c.size.mask());
                        }
                        Op::BinOp(b) => {
                            if let Some(simp) = try_simplify(b, &consts) {
                                if let Op::Constant(c) = &simp {
                                    consts.insert(result, c.value & c.size.mask());
                                }
                                new_stmt = Stmt {
                                    result: Some(result),
                                    op: simp,
                                };
                            }
                        }
                        _ => {}
                    }
                }
                out.push(new_stmt);
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

    fn block(stmts: Vec<Stmt>) -> Function {
        Function {
            entry: 0,
            blocks: vec![BasicBlock { id: 0, stmts }],
        }
    }

    fn cst(result: u32, value: u64, size: OpSize) -> Stmt {
        Stmt::new(Some(result), Op::Constant(Constant { value, size }))
    }

    fn binop(result: u32, op: BinOpKind, lhs: u32, rhs: u32, size: OpSize) -> Stmt {
        Stmt::new(Some(result), Op::BinOp(BinOp { op, lhs, rhs, size }))
    }

    fn folded(func: &Function, idx: usize) -> Option<u64> {
        match &func.blocks[0].stmts[idx].op {
            Op::Constant(c) => Some(c.value),
            _ => None,
        }
    }

    #[test]
    fn mul_by_zero_is_zero() {
        // ref 9 is opaque (never defined as a constant) so const_prop would
        // not fire, but algebraic should still collapse x * 0.
        let func = block(vec![
            cst(0, 0, OpSize::I64),
            binop(1, BinOpKind::Mul, 9, 0, OpSize::I64),
        ]);
        let out = algebraic_simplify(func);
        assert_eq!(folded(&out, 1), Some(0));
    }

    #[test]
    fn and_with_zero_is_zero_either_side() {
        let func = block(vec![
            cst(0, 0, OpSize::I32),
            binop(1, BinOpKind::And, 0, 9, OpSize::I32), // 0 & x
            binop(2, BinOpKind::And, 9, 0, OpSize::I32), // x & 0
        ]);
        let out = algebraic_simplify(func);
        assert_eq!(folded(&out, 1), Some(0));
        assert_eq!(folded(&out, 2), Some(0));
    }

    #[test]
    fn or_with_all_ones_is_all_ones() {
        let func = block(vec![
            cst(0, 0xFFFF, OpSize::I16),
            binop(1, BinOpKind::Or, 0, 9, OpSize::I16),
            binop(2, BinOpKind::Or, 9, 0, OpSize::I16),
        ]);
        let out = algebraic_simplify(func);
        assert_eq!(folded(&out, 1), Some(0xFFFF));
        assert_eq!(folded(&out, 2), Some(0xFFFF));
    }

    #[test]
    fn sub_and_xor_same_ref_is_zero() {
        let func = block(vec![
            binop(0, BinOpKind::Sub, 9, 9, OpSize::I64),
            binop(1, BinOpKind::Xor, 9, 9, OpSize::I64),
        ]);
        let out = algebraic_simplify(func);
        assert_eq!(folded(&out, 0), Some(0));
        assert_eq!(folded(&out, 1), Some(0));
    }

    #[test]
    fn mod_by_one_is_zero() {
        let func = block(vec![
            cst(0, 1, OpSize::I64),
            binop(1, BinOpKind::UMod, 9, 0, OpSize::I64),
            binop(2, BinOpKind::SMod, 9, 0, OpSize::I64),
        ]);
        let out = algebraic_simplify(func);
        assert_eq!(folded(&out, 1), Some(0));
        assert_eq!(folded(&out, 2), Some(0));
    }

    #[test]
    fn umulhi_by_one_is_zero_but_smulhi_is_not() {
        let func = block(vec![
            cst(0, 1, OpSize::I64),
            binop(1, BinOpKind::UMulHi, 9, 0, OpSize::I64),
            binop(2, BinOpKind::SMulHi, 9, 0, OpSize::I64),
        ]);
        let out = algebraic_simplify(func);
        assert_eq!(folded(&out, 1), Some(0));
        // SMulHi x*1 is NOT simplified (needs sign info).
        assert!(matches!(out.blocks[0].stmts[2].op, Op::BinOp(_)));
    }

    #[test]
    fn no_identity_leaves_binop_untouched() {
        let func = block(vec![
            cst(0, 7, OpSize::I64),
            binop(1, BinOpKind::Add, 9, 0, OpSize::I64), // x + 7, no identity
        ]);
        let out = algebraic_simplify(func.clone());
        assert_eq!(out, func);
    }

    #[test]
    fn idempotent_and_no_growth() {
        let func = block(vec![
            cst(0, 0, OpSize::I64),
            cst(1, 0xFFFF_FFFF_FFFF_FFFF, OpSize::I64),
            binop(2, BinOpKind::Mul, 9, 0, OpSize::I64),
            binop(3, BinOpKind::Or, 1, 9, OpSize::I64),
            binop(4, BinOpKind::Sub, 9, 9, OpSize::I64),
            binop(5, BinOpKind::Add, 9, 8, OpSize::I64),
        ]);
        let once = algebraic_simplify(func.clone());
        let twice = algebraic_simplify(once.clone());
        assert_eq!(once, twice, "algebraic must be idempotent");
        assert_eq!(
            once.blocks[0].stmts.len(),
            func.blocks[0].stmts.len(),
            "algebraic must not grow statement count"
        );
    }
}
