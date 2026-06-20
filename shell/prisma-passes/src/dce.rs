//! Dead code elimination.
//!
//! Mirrors C++ `dead_code_eliminate`: a reverse liveness sweep seeds the live
//! set from the operands of side-effecting statements and propagates through
//! statements whose result is live; a forward filter then drops pure
//! statements whose result is unused. SSA + program order make a single
//! reverse pass sufficient (operand defs precede uses).
//!
//! The purity classification mirrors the C++ allowlist exactly so the Rust and
//! C++ pipelines stay differentially equivalent.

use std::collections::HashSet;

use prisma_ir::{BasicBlock, Function, Op, Ref, Stmt};

use crate::Pass;

/// True iff dropping the statement (when its result is unused) cannot change
/// observable behaviour. Side-effecting ops stay regardless of liveness.
fn is_pure_for_dce(op: &Op) -> bool {
    matches!(
        op,
        Op::Constant(_)
            | Op::LoadReg(_)
            | Op::BinOp(_)
            | Op::Compare(_)
            | Op::Select(_)
            | Op::LoadMem(_)
            | Op::WriteFlags(_)
            | Op::ReadFlag(_)
            | Op::WriteFlagsFp(_)
            | Op::VecConstant(_)
            | Op::LoadVecReg(_)
            | Op::LoadVecRegHi(_)
            | Op::WriteFlagsPtest(_)
            | Op::WriteFlagsPtestYmm(_)
            | Op::VecTbl2(_)
            | Op::VecAes(_)
            | Op::VecAesKeygenAssist(_)
            | Op::VecSha(_)
            | Op::Bswap(_)
            | Op::Crc32c(_)
            | Op::VecGather(_)
            | Op::X87Load(_)
            | Op::LoadCarry(_)
            | Op::ReadCarryOut(_)
    )
}

/// Insert every SSA `Ref` the op reads as an operand into `into`.
///
/// Arms are kept one-per-variant (mirroring the C++ visitor) even when their
/// bodies coincide, so the operand map stays auditable against `ir.hpp`.
#[allow(clippy::too_many_lines, clippy::match_same_arms)]
fn collect_operand_refs(op: &Op, into: &mut HashSet<Ref>) {
    match op {
        // No operand refs.
        Op::Constant(_)
        | Op::LoadReg(_)
        | Op::LoadSegBase(_)
        | Op::Jump(_)
        | Op::Return(_)
        | Op::JumpRel(_)
        | Op::CondJumpRel(_)
        | Op::CallRel(_)
        | Op::RetAdjusted(_)
        | Op::Cpuid(_)
        | Op::Xgetbv(_)
        | Op::Rdtsc(_)
        | Op::Syscall(_)
        | Op::Trap(_)
        | Op::Fence(_)
        | Op::GuestPc(_)
        | Op::InlineAsm(_)
        | Op::FpConstant(_)
        | Op::RspAdjust(_)
        | Op::VecConstant(_)
        | Op::LoadVecReg(_)
        | Op::LoadVecRegHi(_)
        | Op::X87Load(_)
        | Op::X87Pop(_)
        | Op::LoadCarry(_) => {}

        Op::ReadCarryOut(x) => {
            into.insert(x.flags);
        }
        Op::StoreCarry(x) => {
            into.insert(x.value);
        }
        Op::StoreReg(x) => {
            into.insert(x.value);
        }
        Op::BinOp(x) => {
            into.insert(x.lhs);
            into.insert(x.rhs);
        }
        Op::Compare(x) => {
            into.insert(x.lhs);
            into.insert(x.rhs);
        }
        Op::Select(x) => {
            into.insert(x.true_value);
            into.insert(x.false_value);
        }
        Op::LoadMem(x) => {
            into.insert(x.addr);
        }
        Op::StoreMem(x) => {
            into.insert(x.addr);
            into.insert(x.value);
        }
        Op::LoadMemTSO(x) => {
            into.insert(x.addr);
        }
        Op::StoreMemTSO(x) => {
            into.insert(x.addr);
            into.insert(x.value);
        }
        Op::CondJump(x) => {
            into.insert(x.cond);
        }
        Op::CmpFlags(x) => {
            into.insert(x.lhs);
            into.insert(x.rhs);
        }
        Op::AluFlags(x) => {
            into.insert(x.lhs);
            into.insert(x.rhs);
        }
        Op::JumpReg(x) => {
            into.insert(x.target);
        }
        Op::CallReg(x) => {
            into.insert(x.target);
        }
        Op::Extend(x) => {
            into.insert(x.value);
        }
        Op::Truncate(x) => {
            into.insert(x.value);
        }
        Op::FpBinOp(x) => {
            into.insert(x.lhs);
            into.insert(x.rhs);
        }
        Op::WriteFlags(x) => {
            into.insert(x.lhs);
            into.insert(x.rhs);
        }
        Op::ReadFlag(x) => {
            into.insert(x.flags);
        }
        Op::CondJumpFlags(x) => {
            into.insert(x.flags);
        }
        Op::VecBinOp(x) => {
            into.insert(x.lhs);
            into.insert(x.rhs);
        }
        Op::StoreVecReg(x) => {
            into.insert(x.value);
        }
        Op::LoadVec(x) => {
            into.insert(x.addr);
        }
        Op::StoreVec(x) => {
            into.insert(x.addr);
            into.insert(x.value);
        }
        Op::VecFpBinOp(x) => {
            into.insert(x.lhs);
            into.insert(x.rhs);
        }
        Op::VecFpScalarBinOp(x) => {
            into.insert(x.lhs);
            into.insert(x.rhs);
        }
        Op::XmmFromGpr(x) => {
            into.insert(x.value);
        }
        Op::GprFromXmm(x) => {
            into.insert(x.value);
        }
        Op::VecCmp(x) => {
            into.insert(x.lhs);
            into.insert(x.rhs);
        }
        Op::VecShuffle32x4(x) => {
            into.insert(x.src);
        }
        Op::VecUnpack(x) => {
            into.insert(x.lhs);
            into.insert(x.rhs);
        }
        Op::VecShiftImm(x) => {
            into.insert(x.src);
        }
        Op::VecShiftBytes(x) => {
            into.insert(x.src);
        }
        Op::IntToFpScalar(x) => {
            into.insert(x.value);
        }
        Op::FpToIntScalar(x) => {
            into.insert(x.value);
        }
        Op::FpCvtScalar(x) => {
            into.insert(x.lhs);
            into.insert(x.src);
        }
        Op::VecShuffle2Src(x) => {
            into.insert(x.lhs);
            into.insert(x.rhs);
        }
        Op::VecInsertLane(x) => {
            into.insert(x.lhs_xmm);
            into.insert(x.value);
        }
        Op::VecExtractLaneU(x) => {
            into.insert(x.src_xmm);
        }
        Op::VecMaskMsb(x) => {
            into.insert(x.src_xmm);
        }
        Op::WriteFlagsFp(x) => {
            into.insert(x.lhs);
            into.insert(x.rhs);
        }
        Op::VecShuffleH4(x) => {
            into.insert(x.src);
        }
        Op::VecMaskFp(x) => {
            into.insert(x.src_xmm);
        }
        Op::VecFpCompare(x) => {
            into.insert(x.lhs);
            into.insert(x.rhs);
        }
        Op::VecPshufb(x) => {
            into.insert(x.src);
            into.insert(x.mask);
        }
        Op::VecAbs(x) => {
            into.insert(x.src);
        }
        Op::VecAlignr(x) => {
            into.insert(x.lhs);
            into.insert(x.rhs);
        }
        Op::VecExtend(x) => {
            into.insert(x.src);
        }
        Op::VecFpRound(x) => {
            into.insert(x.lhs);
            into.insert(x.src);
        }
        Op::Popcnt(x) => {
            into.insert(x.value);
        }
        Op::Lzcnt(x) => {
            into.insert(x.value);
        }
        Op::Tzcnt(x) => {
            into.insert(x.value);
        }
        Op::WriteFlagsPopcnt(x) => {
            into.insert(x.src);
        }
        Op::WriteFlagsCountZero(x) => {
            into.insert(x.src);
            into.insert(x.result);
        }
        Op::VecBlend(x) => {
            into.insert(x.dst);
            into.insert(x.src);
            into.insert(x.mask);
        }
        Op::WriteFlagsPtest(x) => {
            into.insert(x.lhs);
            into.insert(x.rhs);
        }
        Op::WriteFlagsPtestYmm(x) => {
            into.insert(x.lo_lhs);
            into.insert(x.lo_rhs);
            into.insert(x.hi_lhs);
            into.insert(x.hi_rhs);
        }
        Op::VecTbl2(x) => {
            into.insert(x.src_lo);
            into.insert(x.src_hi);
            into.insert(x.idx);
        }
        Op::VecAes(x) => {
            into.insert(x.src);
            into.insert(x.key);
        }
        Op::VecAesKeygenAssist(x) => {
            into.insert(x.src);
        }
        Op::VecSha(x) => {
            into.insert(x.a);
            into.insert(x.b);
            into.insert(x.wk);
        }
        Op::Bswap(x) => {
            into.insert(x.value);
        }
        Op::Crc32c(x) => {
            into.insert(x.crc);
            into.insert(x.data);
        }
        Op::VecGather(x) => {
            into.insert(x.base);
            into.insert(x.index);
            into.insert(x.mask);
            into.insert(x.prev);
        }
        Op::StoreVecRegHi(x) => {
            into.insert(x.value);
        }
        Op::VecFpFma(x) => {
            into.insert(x.a);
            into.insert(x.b);
            into.insert(x.c);
        }
        Op::VecFpScalarFma(x) => {
            into.insert(x.a);
            into.insert(x.b);
            into.insert(x.c);
            into.insert(x.scalar_upper);
        }
        Op::RepStos(_) | Op::RepMovs(_) => {}
        Op::X87Store(x) => {
            into.insert(x.value);
        }
        Op::X87Push(x) => {
            into.insert(x.value);
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Dce;

impl Dce {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Pass for Dce {
    fn name(&self) -> &'static str {
        "dead_code_eliminate"
    }

    fn run(&self, func: Function) -> Function {
        dead_code_eliminate(func)
    }
}

/// Dead code elimination with **function-global** liveness.
///
/// SSA refs are function-scoped, so a value defined in one block may be used
/// in a later block. The C++ pass runs over a single flat statement vector; a
/// per-block reverse sweep would miss cross-block uses and wrongly drop a live
/// def. We therefore sweep liveness across all blocks (in program order,
/// reversed) into one shared live set before filtering. Within the function,
/// SSA + program order guarantees defs precede uses, so a single reverse pass
/// over the concatenation suffices — exactly mirroring the C++ flat scan.
pub fn dead_code_eliminate(func: Function) -> Function {
    // Pass 1: reverse liveness sweep over every statement in the function.
    let mut live: HashSet<Ref> = HashSet::new();
    for block in func.blocks.iter().rev() {
        for stmt in block.stmts.iter().rev() {
            let impure = !is_pure_for_dce(&stmt.op);
            let result_is_live = stmt.result.is_some_and(|r| live.contains(&r));
            if impure || result_is_live {
                collect_operand_refs(&stmt.op, &mut live);
            }
        }
    }

    // Pass 2: forward filter per block against the global live set.
    let blocks = func
        .blocks
        .into_iter()
        .map(|block| {
            let stmts = block
                .stmts
                .into_iter()
                .filter(|stmt| {
                    let impure = !is_pure_for_dce(&stmt.op);
                    let result_is_live = stmt.result.is_some_and(|r| live.contains(&r));
                    impure || result_is_live
                })
                .collect();
            BasicBlock {
                id: block.id,
                stmts,
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
    use prisma_ir::{BinOp, BinOpKind, Constant, Gpr, OpSize, StoreReg};

    #[test]
    fn dead_constant_is_dropped() {
        let func = Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![
                    Stmt::new(
                        Some(0),
                        Op::Constant(Constant {
                            value: 7,
                            size: OpSize::I64,
                        }),
                    ),
                    Stmt::new(None, Op::Return(prisma_ir::Return)),
                ],
            }],
        };
        let out = dead_code_eliminate(func);
        // The unused constant is removed; only Return remains.
        assert_eq!(out.blocks[0].stmts.len(), 1);
        assert!(matches!(out.blocks[0].stmts[0].op, Op::Return(_)));
    }

    #[test]
    fn live_chain_is_kept() {
        // r0 = const; r1 = r0+r0; StoreReg rax, r1  (r1, r0 live)
        let func = Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![
                    Stmt::new(
                        Some(0),
                        Op::Constant(Constant {
                            value: 7,
                            size: OpSize::I64,
                        }),
                    ),
                    Stmt::new(
                        Some(1),
                        Op::BinOp(BinOp {
                            op: BinOpKind::Add,
                            lhs: 0,
                            rhs: 0,
                            size: OpSize::I64,
                        }),
                    ),
                    Stmt::new(
                        None,
                        Op::StoreReg(StoreReg {
                            reg: Gpr::Rax,
                            value: 1,
                            size: OpSize::I64,
                        }),
                    ),
                ],
            }],
        };
        let out = dead_code_eliminate(func.clone());
        assert_eq!(out, func);
    }

    #[test]
    fn partially_dead_is_pruned() {
        // r0 used by store; r1 dead.
        let func = Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![
                    Stmt::new(
                        Some(0),
                        Op::Constant(Constant {
                            value: 1,
                            size: OpSize::I64,
                        }),
                    ),
                    Stmt::new(
                        Some(1),
                        Op::Constant(Constant {
                            value: 2,
                            size: OpSize::I64,
                        }),
                    ),
                    Stmt::new(
                        None,
                        Op::StoreReg(StoreReg {
                            reg: Gpr::Rax,
                            value: 0,
                            size: OpSize::I64,
                        }),
                    ),
                ],
            }],
        };
        let out = dead_code_eliminate(func);
        assert_eq!(out.blocks[0].stmts.len(), 2);
        // r1 constant dropped, r0 + store kept.
        assert!(matches!(out.blocks[0].stmts[1].op, Op::StoreReg(_)));
    }

    #[test]
    fn idempotent() {
        let func = Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![
                    Stmt::new(
                        Some(0),
                        Op::Constant(Constant {
                            value: 1,
                            size: OpSize::I64,
                        }),
                    ),
                    Stmt::new(
                        Some(1),
                        Op::Constant(Constant {
                            value: 2,
                            size: OpSize::I64,
                        }),
                    ),
                    Stmt::new(
                        None,
                        Op::StoreReg(StoreReg {
                            reg: Gpr::Rax,
                            value: 0,
                            size: OpSize::I64,
                        }),
                    ),
                ],
            }],
        };
        let once = dead_code_eliminate(func);
        let twice = dead_code_eliminate(once.clone());
        assert_eq!(once, twice);
    }

    #[test]
    fn cross_block_use_keeps_def() {
        // r0 defined (pure) in block 0, used by a StoreReg in block 1.
        // A per-block liveness sweep would wrongly drop r0; the global
        // sweep must keep it (regression for the Codex BLOCKER).
        let func = Function {
            entry: 0,
            blocks: vec![
                BasicBlock {
                    id: 0,
                    stmts: vec![Stmt::new(
                        Some(0),
                        Op::Constant(Constant {
                            value: 7,
                            size: OpSize::I64,
                        }),
                    )],
                },
                BasicBlock {
                    id: 1,
                    stmts: vec![Stmt::new(
                        None,
                        Op::StoreReg(StoreReg {
                            reg: Gpr::Rax,
                            value: 0,
                            size: OpSize::I64,
                        }),
                    )],
                },
            ],
        };
        let out = dead_code_eliminate(func.clone());
        assert_eq!(out, func, "cross-block live def must not be dropped");
    }
}
