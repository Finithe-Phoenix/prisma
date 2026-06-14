//! Copy propagation pass.
//!
//! Current scope: local `x = Or x, x` copy-like chains only.
//! This pass rewrites SSA references to canonical aliases discovered so far
//! within each basic block.

use std::collections::HashMap;

use crate::Pass;
use prisma_ir::{BasicBlock, BinOpKind, Function, Op, OpSize, Stmt};

fn resolve_alias(r: u32, aliases: &HashMap<u32, u32>) -> u32 {
    let mut cur = r;
    loop {
        let Some(next) = aliases.get(&cur) else {
            return cur;
        };
        if *next == cur {
            return cur;
        }
        cur = *next;
    }
}

#[allow(clippy::too_many_lines)]
fn rewrite(op: Op, aliases: &HashMap<u32, u32>) -> Op {
    match op {
        Op::StoreReg(mut op) => {
            op.value = resolve_alias(op.value, aliases);
            Op::StoreReg(op)
        }
        Op::BinOp(mut op) => {
            op.lhs = resolve_alias(op.lhs, aliases);
            op.rhs = resolve_alias(op.rhs, aliases);
            Op::BinOp(op)
        }
        Op::Compare(mut op) => {
            op.lhs = resolve_alias(op.lhs, aliases);
            op.rhs = resolve_alias(op.rhs, aliases);
            Op::Compare(op)
        }
        Op::Select(mut op) => {
            op.true_value = resolve_alias(op.true_value, aliases);
            op.false_value = resolve_alias(op.false_value, aliases);
            Op::Select(op)
        }
        Op::LoadMem(mut op) => {
            op.addr = resolve_alias(op.addr, aliases);
            Op::LoadMem(op)
        }
        Op::StoreMem(mut op) => {
            op.addr = resolve_alias(op.addr, aliases);
            op.value = resolve_alias(op.value, aliases);
            Op::StoreMem(op)
        }
        Op::LoadMemTSO(mut op) => {
            op.addr = resolve_alias(op.addr, aliases);
            Op::LoadMemTSO(op)
        }
        Op::StoreMemTSO(mut op) => {
            op.addr = resolve_alias(op.addr, aliases);
            op.value = resolve_alias(op.value, aliases);
            Op::StoreMemTSO(op)
        }
        Op::CondJump(mut op) => {
            op.cond = resolve_alias(op.cond, aliases);
            Op::CondJump(op)
        }
        Op::CallReg(mut op) => {
            op.target = resolve_alias(op.target, aliases);
            Op::CallReg(op)
        }
        Op::CmpFlags(mut op) => {
            op.lhs = resolve_alias(op.lhs, aliases);
            op.rhs = resolve_alias(op.rhs, aliases);
            Op::CmpFlags(op)
        }
        Op::AluFlags(mut op) => {
            op.lhs = resolve_alias(op.lhs, aliases);
            op.rhs = resolve_alias(op.rhs, aliases);
            Op::AluFlags(op)
        }
        Op::ReadFlag(mut op) => {
            op.flags = resolve_alias(op.flags, aliases);
            Op::ReadFlag(op)
        }
        Op::CondJumpFlags(mut op) => {
            op.flags = resolve_alias(op.flags, aliases);
            Op::CondJumpFlags(op)
        }
        Op::WriteFlags(mut op) => {
            op.lhs = resolve_alias(op.lhs, aliases);
            op.rhs = resolve_alias(op.rhs, aliases);
            Op::WriteFlags(op)
        }
        Op::WriteFlagsCountZero(mut op) => {
            op.src = resolve_alias(op.src, aliases);
            op.result = resolve_alias(op.result, aliases);
            Op::WriteFlagsCountZero(op)
        }
        Op::WriteFlagsFp(mut op) => {
            op.lhs = resolve_alias(op.lhs, aliases);
            op.rhs = resolve_alias(op.rhs, aliases);
            Op::WriteFlagsFp(op)
        }
        Op::WriteFlagsPtest(mut op) => {
            op.lhs = resolve_alias(op.lhs, aliases);
            op.rhs = resolve_alias(op.rhs, aliases);
            Op::WriteFlagsPtest(op)
        }
        Op::WriteFlagsPtestYmm(mut op) => {
            op.lo_lhs = resolve_alias(op.lo_lhs, aliases);
            op.lo_rhs = resolve_alias(op.lo_rhs, aliases);
            op.hi_lhs = resolve_alias(op.hi_lhs, aliases);
            op.hi_rhs = resolve_alias(op.hi_rhs, aliases);
            Op::WriteFlagsPtestYmm(op)
        }
        Op::FpBinOp(mut op) => {
            op.lhs = resolve_alias(op.lhs, aliases);
            op.rhs = resolve_alias(op.rhs, aliases);
            Op::FpBinOp(op)
        }
        Op::IntToFpScalar(mut op) => {
            op.value = resolve_alias(op.value, aliases);
            Op::IntToFpScalar(op)
        }
        Op::FpToIntScalar(mut op) => {
            op.value = resolve_alias(op.value, aliases);
            Op::FpToIntScalar(op)
        }
        Op::FpCvtScalar(mut op) => {
            op.lhs = resolve_alias(op.lhs, aliases);
            op.src = resolve_alias(op.src, aliases);
            Op::FpCvtScalar(op)
        }
        Op::XmmFromGpr(mut op) => {
            op.value = resolve_alias(op.value, aliases);
            Op::XmmFromGpr(op)
        }
        Op::GprFromXmm(mut op) => {
            op.value = resolve_alias(op.value, aliases);
            Op::GprFromXmm(op)
        }
        Op::VecCmp(mut op) => {
            op.lhs = resolve_alias(op.lhs, aliases);
            op.rhs = resolve_alias(op.rhs, aliases);
            Op::VecCmp(op)
        }
        Op::VecUnpack(mut op) => {
            op.lhs = resolve_alias(op.lhs, aliases);
            op.rhs = resolve_alias(op.rhs, aliases);
            Op::VecUnpack(op)
        }
        Op::VecShiftImm(mut op) => {
            op.src = resolve_alias(op.src, aliases);
            Op::VecShiftImm(op)
        }
        Op::VecShiftBytes(mut op) => {
            op.src = resolve_alias(op.src, aliases);
            Op::VecShiftBytes(op)
        }
        Op::VecShuffle32x4(mut op) => {
            op.src = resolve_alias(op.src, aliases);
            Op::VecShuffle32x4(op)
        }
        Op::VecShuffle2Src(mut op) => {
            op.lhs = resolve_alias(op.lhs, aliases);
            op.rhs = resolve_alias(op.rhs, aliases);
            Op::VecShuffle2Src(op)
        }
        Op::VecInsertLane(mut op) => {
            op.lhs_xmm = resolve_alias(op.lhs_xmm, aliases);
            op.value = resolve_alias(op.value, aliases);
            Op::VecInsertLane(op)
        }
        Op::VecExtractLaneU(mut op) => {
            op.src_xmm = resolve_alias(op.src_xmm, aliases);
            Op::VecExtractLaneU(op)
        }
        Op::VecMaskMsb(mut op) => {
            op.src_xmm = resolve_alias(op.src_xmm, aliases);
            Op::VecMaskMsb(op)
        }
        Op::VecMaskFp(mut op) => {
            op.src_xmm = resolve_alias(op.src_xmm, aliases);
            Op::VecMaskFp(op)
        }
        Op::VecFpCompare(mut op) => {
            op.lhs = resolve_alias(op.lhs, aliases);
            op.rhs = resolve_alias(op.rhs, aliases);
            Op::VecFpCompare(op)
        }
        Op::VecFpScalarBinOp(mut op) => {
            op.lhs = resolve_alias(op.lhs, aliases);
            op.rhs = resolve_alias(op.rhs, aliases);
            Op::VecFpScalarBinOp(op)
        }
        Op::VecFpScalarFma(mut op) => {
            op.a = resolve_alias(op.a, aliases);
            op.b = resolve_alias(op.b, aliases);
            op.c = resolve_alias(op.c, aliases);
            op.scalar_upper = resolve_alias(op.scalar_upper, aliases);
            Op::VecFpScalarFma(op)
        }
        Op::VecFpFma(mut op) => {
            op.a = resolve_alias(op.a, aliases);
            op.b = resolve_alias(op.b, aliases);
            op.c = resolve_alias(op.c, aliases);
            Op::VecFpFma(op)
        }
        Op::VecFpRound(mut op) => {
            op.lhs = resolve_alias(op.lhs, aliases);
            op.src = resolve_alias(op.src, aliases);
            Op::VecFpRound(op)
        }
        Op::VecBlend(mut op) => {
            op.dst = resolve_alias(op.dst, aliases);
            op.src = resolve_alias(op.src, aliases);
            op.mask = resolve_alias(op.mask, aliases);
            Op::VecBlend(op)
        }
        Op::VecPshufb(mut op) => {
            op.src = resolve_alias(op.src, aliases);
            op.mask = resolve_alias(op.mask, aliases);
            Op::VecPshufb(op)
        }
        Op::VecAbs(mut op) => {
            op.src = resolve_alias(op.src, aliases);
            Op::VecAbs(op)
        }
        Op::VecAlignr(mut op) => {
            op.lhs = resolve_alias(op.lhs, aliases);
            op.rhs = resolve_alias(op.rhs, aliases);
            Op::VecAlignr(op)
        }
        Op::VecAes(mut op) => {
            op.src = resolve_alias(op.src, aliases);
            op.key = resolve_alias(op.key, aliases);
            Op::VecAes(op)
        }
        Op::VecAesKeygenAssist(mut op) => {
            op.src = resolve_alias(op.src, aliases);
            Op::VecAesKeygenAssist(op)
        }
        Op::VecSha(mut op) => {
            op.a = resolve_alias(op.a, aliases);
            op.b = resolve_alias(op.b, aliases);
            op.wk = resolve_alias(op.wk, aliases);
            Op::VecSha(op)
        }
        Op::VecTbl2(mut op) => {
            op.idx = resolve_alias(op.idx, aliases);
            Op::VecTbl2(op)
        }
        Op::Popcnt(mut op) => {
            op.value = resolve_alias(op.value, aliases);
            Op::Popcnt(op)
        }
        Op::Lzcnt(mut op) => {
            op.value = resolve_alias(op.value, aliases);
            Op::Lzcnt(op)
        }
        Op::Tzcnt(mut op) => {
            op.value = resolve_alias(op.value, aliases);
            Op::Tzcnt(op)
        }
        Op::Crc32c(mut op) => {
            op.crc = resolve_alias(op.crc, aliases);
            op.data = resolve_alias(op.data, aliases);
            Op::Crc32c(op)
        }
        Op::Bswap(mut op) => {
            op.value = resolve_alias(op.value, aliases);
            Op::Bswap(op)
        }
        Op::VecGather(mut op) => {
            op.base = resolve_alias(op.base, aliases);
            op.index = resolve_alias(op.index, aliases);
            op.mask = resolve_alias(op.mask, aliases);
            op.prev = resolve_alias(op.prev, aliases);
            Op::VecGather(op)
        }
        Op::X87Store(mut op) => {
            op.value = resolve_alias(op.value, aliases);
            Op::X87Store(op)
        }
        Op::X87Push(mut op) => {
            op.value = resolve_alias(op.value, aliases);
            Op::X87Push(op)
        }
        Op::Truncate(mut op) => {
            op.value = resolve_alias(op.value, aliases);
            Op::Truncate(op)
        }
        Op::Extend(mut op) => {
            op.value = resolve_alias(op.value, aliases);
            Op::Extend(op)
        }
        _ => op,
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CopyProp;

impl CopyProp {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Pass for CopyProp {
    fn name(&self) -> &'static str {
        "copy_prop"
    }

    fn run(&self, func: Function) -> Function {
        copy_propagate(func)
    }
}

/// Copy propagate within each basic block.
pub fn copy_propagate(func: Function) -> Function {
    let mut blocks = Vec::with_capacity(func.blocks.len());

    for mut block in func.blocks {
        let mut aliases: HashMap<u32, u32> = HashMap::new();
        let mut out = Vec::with_capacity(block.stmts.len());

        for stmt in block.stmts.drain(..) {
            let op = rewrite(stmt.op, &aliases);
            if let (Some(result), Op::BinOp(binop)) = (stmt.result, &op) {
                if binop.op == BinOpKind::Or && binop.lhs == binop.rhs && binop.lhs != result {
                    aliases.insert(result, binop.lhs);
                }
            }
            out.push(Stmt {
                result: stmt.result,
                op,
            });
        }

        block.stmts = out;
        blocks.push(block);
    }

    Function {
        blocks,
        entry: func.entry,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn propagates_self_binop_copy() {
        let func = Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![
                    Stmt::new(
                        Some(0),
                        Op::Constant(prisma_ir::Constant {
                            value: 4,
                            size: OpSize::I64,
                        }),
                    ),
                    Stmt::new(
                        Some(1),
                        Op::BinOp(prisma_ir::BinOp {
                            op: BinOpKind::Or,
                            lhs: 0,
                            rhs: 0,
                            size: OpSize::I64,
                        }),
                    ),
                    Stmt::new(
                        Some(2),
                        Op::BinOp(prisma_ir::BinOp {
                            op: BinOpKind::Add,
                            lhs: 1,
                            rhs: 0,
                            size: OpSize::I64,
                        }),
                    ),
                ],
            }],
        };

        let out = copy_propagate(func);
        match &out.blocks[0].stmts[2].op {
            Op::BinOp(binop) => {
                assert_eq!(binop.lhs, 0);
                assert_eq!(binop.rhs, 0);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
