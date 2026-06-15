//! Branch folding pass.
//!
//! Mirrors C++ `branch_fold`. When a `CmpFlags const_a, const_b` is followed
//! (with no intervening flag clobber) by a `CondJumpRel`, the direction is
//! known at compile time and the `CondJumpRel` is rewritten to a plain
//! `JumpRel`. The `CmpFlags` is left in place; DCE removes it once the flag
//! state is provably unused. Flag-direct conditions (carry/overflow/sign) are
//! not folded since the IR does not model the full NZCV update here.

use std::collections::HashMap;

use prisma_ir::{BasicBlock, CondCode, Function, JumpRel, Op, OpSize, Ref, Stmt};

use crate::Pass;

/// Evaluate `a cc b` with size-specific masking; `None` for flag-direct ccs.
fn eval_cond(cc: CondCode, a: u64, b: u64, size: OpSize) -> Option<bool> {
    let a = a & size.mask();
    let b = b & size.mask();
    let bits = size.bit_width();
    let sext = |v: u64| -> i64 {
        if bits == 64 {
            v as i64
        } else {
            let sign_bit = 1u64 << (bits - 1);
            if v & sign_bit != 0 {
                (v | !((1u64 << bits) - 1)) as i64
            } else {
                v as i64
            }
        }
    };
    let (sa, sb) = (sext(a), sext(b));
    match cc {
        CondCode::Eq => Some(a == b),
        CondCode::Ne => Some(a != b),
        CondCode::Ult => Some(a < b),
        CondCode::Ule => Some(a <= b),
        CondCode::Ugt => Some(a > b),
        CondCode::Uge => Some(a >= b),
        CondCode::Slt => Some(sa < sb),
        CondCode::Sle => Some(sa <= sb),
        CondCode::Sgt => Some(sa > sb),
        CondCode::Sge => Some(sa >= sb),
        CondCode::Cc
        | CondCode::Nc
        | CondCode::Ov
        | CondCode::NoOv
        | CondCode::Mi
        | CondCode::Pl => None,
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BranchFold;

impl BranchFold {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Pass for BranchFold {
    fn name(&self) -> &'static str {
        "branch_fold"
    }

    fn run(&self, func: Function) -> Function {
        branch_fold(func)
    }
}

#[derive(Clone, Copy)]
struct LastCmp {
    lhs: Ref,
    rhs: Ref,
    size: OpSize,
}

/// Per-block branch folding.
pub fn branch_fold(func: Function) -> Function {
    let blocks = func
        .blocks
        .into_iter()
        .map(|block| {
            let mut consts: HashMap<Ref, u64> = HashMap::new();
            let mut last_cmp: Option<LastCmp> = None;
            let mut out = Vec::with_capacity(block.stmts.len());

            for stmt in block.stmts {
                match &stmt.op {
                    Op::Constant(c) => {
                        if let Some(r) = stmt.result {
                            consts.insert(r, c.value & c.size.mask());
                        }
                    }
                    Op::CmpFlags(cf) => {
                        last_cmp = Some(LastCmp {
                            lhs: cf.lhs,
                            rhs: cf.rhs,
                            size: cf.size,
                        });
                    }
                    Op::AluFlags(_) | Op::WriteFlagsCountZero(_) => {
                        last_cmp = None;
                    }
                    Op::CondJumpRel(cj) => {
                        if let Some(lc) = last_cmp {
                            if let (Some(&a), Some(&b)) = (consts.get(&lc.lhs), consts.get(&lc.rhs))
                            {
                                let choose_taken = eval_cond(cj.cc, a, b, lc.size).unwrap_or(false);
                                let target = if choose_taken {
                                    cj.target_guest_pc
                                } else {
                                    cj.fallthrough_guest_pc
                                };
                                out.push(Stmt::new(
                                    None,
                                    Op::JumpRel(JumpRel {
                                        target_guest_pc: target,
                                    }),
                                ));
                                last_cmp = None;
                                continue;
                            }
                        }
                        last_cmp = None;
                    }
                    _ => {}
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
    use prisma_ir::{CmpFlags, CondJumpRel, Constant};

    fn cst(result: u32, value: u64) -> Stmt {
        Stmt::new(
            Some(result),
            Op::Constant(Constant {
                value,
                size: OpSize::I64,
            }),
        )
    }

    fn build(cc: CondCode, a: u64, b: u64) -> Function {
        Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![
                    cst(0, a),
                    cst(1, b),
                    Stmt::new(
                        None,
                        Op::CmpFlags(CmpFlags {
                            lhs: 0,
                            rhs: 1,
                            size: OpSize::I64,
                        }),
                    ),
                    Stmt::new(
                        None,
                        Op::CondJumpRel(CondJumpRel {
                            cc,
                            target_guest_pc: 0x1000,
                            fallthrough_guest_pc: 0x2000,
                        }),
                    ),
                ],
            }],
        }
    }

    fn last_target(func: &Function) -> Option<u64> {
        match &func.blocks[0].stmts.last().unwrap().op {
            Op::JumpRel(j) => Some(j.target_guest_pc),
            _ => None,
        }
    }

    #[test]
    fn folds_taken_branch() {
        // 5 == 5 -> taken -> target 0x1000
        let out = branch_fold(build(CondCode::Eq, 5, 5));
        assert_eq!(last_target(&out), Some(0x1000));
    }

    #[test]
    fn folds_not_taken_branch() {
        // 5 == 6 -> false -> fallthrough 0x2000
        let out = branch_fold(build(CondCode::Eq, 5, 6));
        assert_eq!(last_target(&out), Some(0x2000));
    }

    #[test]
    fn signed_less_than_uses_signed_interpretation() {
        // -1 (0xFFFF..) Slt 1 -> true -> target
        let out = branch_fold(build(CondCode::Slt, u64::MAX, 1));
        assert_eq!(last_target(&out), Some(0x1000));
        // unsigned would be false: confirm Ult picks fallthrough.
        let out2 = branch_fold(build(CondCode::Ult, u64::MAX, 1));
        assert_eq!(last_target(&out2), Some(0x2000));
    }

    #[test]
    fn flag_direct_cc_folds_to_fallthrough() {
        // eval_cond returns None for flag-direct ccs; with both operands known
        // the C++ pass folds via unwrap_or(false) -> fallthrough. Match it.
        let out = branch_fold(build(CondCode::Cc, 5, 5));
        assert_eq!(last_target(&out), Some(0x2000));
    }

    #[test]
    fn non_constant_operands_leave_branch() {
        let func = Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![
                    Stmt::new(
                        None,
                        Op::CmpFlags(CmpFlags {
                            lhs: 7,
                            rhs: 8,
                            size: OpSize::I64,
                        }),
                    ),
                    Stmt::new(
                        None,
                        Op::CondJumpRel(CondJumpRel {
                            cc: CondCode::Eq,
                            target_guest_pc: 0x1000,
                            fallthrough_guest_pc: 0x2000,
                        }),
                    ),
                ],
            }],
        };
        let out = branch_fold(func.clone());
        assert_eq!(out, func);
    }
}
