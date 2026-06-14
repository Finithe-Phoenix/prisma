//! Instruction lowering facade for the Rust backend.

use std::collections::{HashMap, HashSet};

use crate::{
    abi,
    assembler::{Arm64Assembler, Label},
};
use prisma_ir::{BinOpKind, Function, Gpr, Op, OpSize, Ref, Stmt};
use thiserror::Error;

/// First temporary register used by the migration lowerer.
const FIRST_VALUE_REG: u8 = 9;

/// Number of temporary integer registers managed by this migration slice.
const VALUE_REG_COUNT: u8 = 8;

/// Scratch registers used for transient flag-alignment lowering.
const FLAG_ALIGN_LHS_REG: u8 = 17;
const FLAG_ALIGN_RHS_REG: u8 = 18;
const FLAG_ALIGN_SHIFT_REG: u8 = 19;

/// Scratch register used for quotient materialization in modulo lowering.
const MOD_QUOTIENT_REG: u8 = 20;

/// Lowering failures surfaced by the Rust backend.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LowerError {
    #[error("entry block {0} not found")]
    MissingEntryBlock(u32),
    #[error("target block {0} not found")]
    MissingTargetBlock(u32),
    #[error("statement result is required for {0}")]
    MissingResult(&'static str),
    #[error("SSA ref {0} has no assigned host register")]
    MissingValue(Ref),
    #[error("constant 0x{0:x} is not encodable by the current constant-emission slice")]
    ConstantOutOfRange(u64),
    #[error("immediate 0x{0:x} is not encodable by the current ADD/SUB immediate slice")]
    ImmediateOutOfRange(u64),
    #[error("unsupported op in Rust backend migration slice: {0}")]
    UnsupportedOp(&'static str),
}

/// Lowering strategy used by callers in later phases.
#[derive(Debug, Clone)]
pub struct Lowerer {
    /// Tracks an optional lowering budget for future tuning points.
    budget: usize,
}

impl Default for Lowerer {
    fn default() -> Self {
        Self::new()
    }
}

impl Lowerer {
    /// Constructs a default lowerer.
    #[must_use]
    pub const fn new() -> Self {
        Self { budget: 1024 }
    }

    /// Lowers an input instruction buffer into backend words.
    ///
    /// Byte-stream lowering is still owned by the decoder/backend integration
    /// work. Keep this compatibility shim empty until the Rust decoder feeds
    /// `Function` values directly into this crate.
    #[must_use]
    pub const fn lower_ir(&self, _bytes: &[u8]) -> Vec<u32> {
        let _ = self.budget;
        Vec::new()
    }

    /// Lowers a minimal `prisma-ir` function into `AArch64` instruction words.
    ///
    /// Current migration coverage:
    /// - `Constant` values via `MOVZ` + `MOVK`
    /// - `LoadReg`/`StoreReg` over `CpuStateFrame::gpr[]`
    /// - `BinOp Add/Sub` with a register rhs or 12-bit constant rhs
    /// - `BinOp And/Or/Xor/Shl/Shr/Sar/Ror/Mul/UMulHi/SMulHi/UDiv/SDiv/UMod/SMod`
    ///   with previously-lowered register operands
    /// - `Compare` values via `CMP` + `CSET` with sized-operand alignment for
    ///   non-I64 compares
    /// - `CmpFlags`/`CondJumpFlags` through ARM64 NZCV + `B.cond`
    /// - `CondJumpRel` through ARM64 `B.cond`
    /// - `LoadMem`/`StoreMem` for `I8`/`I16`/`I32`/`I64` with address/value
    ///   already in registers
    /// - direct `Jump` and `CondJump` between basic blocks
    /// - `Return`
    ///
    /// This is deliberately small, but it is a real IR-to-backend path with
    /// exact instruction tests.
    ///
    /// # Errors
    ///
    /// Returns `LowerError` when the function uses IR outside the current
    /// migration slice or references values that have not been lowered.
    pub fn lower_function(&self, func: &Function) -> Result<Vec<u32>, LowerError> {
        let mut asm = Arm64Assembler::new();
        let mut labels = HashMap::<u32, Label>::new();
        for block in &func.blocks {
            labels.insert(block.id, asm.create_label());
        }

        if !labels.contains_key(&func.entry) {
            return Err(LowerError::MissingEntryBlock(func.entry));
        }

        let mut values = HashMap::<Ref, u8>::new();
        let mut constants = HashMap::<Ref, u64>::new();
        let mut flags = HashSet::<Ref>::new();

        let mut block_order = Vec::with_capacity(func.blocks.len());
        let entry_block = func
            .blocks
            .iter()
            .find(|block| block.id == func.entry)
            .ok_or(LowerError::MissingEntryBlock(func.entry))?;
        block_order.push(entry_block);
        block_order.extend(func.blocks.iter().filter(|block| block.id != func.entry));

        for block in block_order {
            let label = labels
                .get(&block.id)
                .copied()
                .ok_or(LowerError::MissingTargetBlock(block.id))?;
            asm.bind_label(label);

            for stmt in &block.stmts {
                lower_stmt(
                    stmt,
                    &mut asm,
                    &labels,
                    &mut values,
                    &mut constants,
                    &mut flags,
                )?;
            }
        }

        Ok(asm.finish())
    }
}

fn lower_stmt(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    labels: &HashMap<u32, Label>,
    values: &mut HashMap<Ref, u8>,
    constants: &mut HashMap<Ref, u64>,
    flags: &mut HashSet<Ref>,
) -> Result<(), LowerError> {
    match &stmt.op {
        Op::Constant(c) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("Constant"))?;
            let reg = value_reg(result);
            emit_u64_constant(asm, reg, c.value);
            values.insert(result, reg);
            constants.insert(result, c.value);
        }
        Op::LoadReg(load) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("LoadReg"))?;
            let dst = value_reg(result);
            emit_load_reg(asm, load.size, dst, load.reg);
            values.insert(result, dst);
        }
        Op::StoreReg(store) => {
            let value = *values
                .get(&store.value)
                .ok_or(LowerError::MissingValue(store.value))?;
            emit_store_reg(asm, store.size, value, store.reg);
        }
        Op::BinOp(bin) => {
            lower_binop(stmt, asm, values, constants, bin)?;
        }
        Op::LoadMem(load) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("LoadMem"))?;
            let addr = *values
                .get(&load.addr)
                .ok_or(LowerError::MissingValue(load.addr))?;
            let dst = value_reg(result);
            emit_load_mem(asm, load.size, dst, addr);
            values.insert(result, dst);
        }
        Op::Compare(compare) => {
            lower_compare(stmt, asm, values, compare)?;
        }
        Op::CmpFlags(cmp) => {
            lower_cmp_flags(stmt, asm, values, flags, cmp)?;
        }
        Op::StoreMem(store) => {
            let addr = *values
                .get(&store.addr)
                .ok_or(LowerError::MissingValue(store.addr))?;
            let value = *values
                .get(&store.value)
                .ok_or(LowerError::MissingValue(store.value))?;
            emit_store_mem(asm, store.size, value, addr);
        }
        Op::Jump(jump) => {
            let target = labels
                .get(&jump.target_block)
                .copied()
                .ok_or(LowerError::MissingTargetBlock(jump.target_block))?;
            asm.b_label(target);
        }
        Op::CondJump(jump) => {
            lower_cond_jump(asm, labels, values, jump)?;
        }
        Op::CondJumpFlags(jump) => {
            lower_cond_jump_flags(asm, labels, flags, jump)?;
        }
        Op::CondJumpRel(jump) => {
            lower_cond_jump_rel(asm, labels, jump)?;
        }
        Op::JumpReg(jump) => {
            let target = *values
                .get(&jump.target)
                .ok_or(LowerError::MissingValue(jump.target))?;
            asm.br_x(target);
        }
        Op::Return(_) => asm.ret(),
        _ => return Err(LowerError::UnsupportedOp("unsupported")),
    }

    Ok(())
}

fn lower_binop(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    values: &mut HashMap<Ref, u8>,
    constants: &HashMap<Ref, u64>,
    bin: &prisma_ir::BinOp,
) -> Result<(), LowerError> {
    match bin.op {
        BinOpKind::Add | BinOpKind::Sub => lower_add_sub(stmt, asm, values, constants, bin),
        BinOpKind::And
        | BinOpKind::Or
        | BinOpKind::Xor
        | BinOpKind::Shl
        | BinOpKind::Shr
        | BinOpKind::Sar
        | BinOpKind::Ror
        | BinOpKind::Mul
        | BinOpKind::UMulHi
        | BinOpKind::SMulHi
        | BinOpKind::UDiv
        | BinOpKind::SDiv => lower_reg_binop(stmt, asm, values, bin),
        BinOpKind::UMod | BinOpKind::SMod => lower_mod_binop(stmt, asm, values, bin),
        _ => Err(LowerError::UnsupportedOp("BinOp")),
    }
}

fn lower_add_sub(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    values: &mut HashMap<Ref, u8>,
    constants: &HashMap<Ref, u64>,
    bin: &prisma_ir::BinOp,
) -> Result<(), LowerError> {
    let result = stmt.result.ok_or(LowerError::MissingResult("BinOp"))?;
    let lhs = *values
        .get(&bin.lhs)
        .ok_or(LowerError::MissingValue(bin.lhs))?;
    let dst = value_reg(result);
    if let Some(rhs) = constants.get(&bin.rhs) {
        let imm12 = u16::try_from(*rhs).map_err(|_| LowerError::ImmediateOutOfRange(*rhs))?;
        if imm12 >= 4096 {
            return Err(LowerError::ImmediateOutOfRange(*rhs));
        }
        match bin.op {
            BinOpKind::Add => asm.add_x_imm(dst, lhs, imm12),
            BinOpKind::Sub => asm.sub_x_imm(dst, lhs, imm12),
            _ => unreachable!("called only for Add/Sub"),
        }
    } else {
        let rhs = *values
            .get(&bin.rhs)
            .ok_or(LowerError::MissingValue(bin.rhs))?;
        match bin.op {
            BinOpKind::Add => asm.add_x(dst, lhs, rhs),
            BinOpKind::Sub => asm.sub_x(dst, lhs, rhs),
            _ => unreachable!("called only for Add/Sub"),
        }
    }
    values.insert(result, dst);
    Ok(())
}

fn lower_reg_binop(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    values: &mut HashMap<Ref, u8>,
    bin: &prisma_ir::BinOp,
) -> Result<(), LowerError> {
    let result = stmt.result.ok_or(LowerError::MissingResult("BinOp"))?;
    let lhs = *values
        .get(&bin.lhs)
        .ok_or(LowerError::MissingValue(bin.lhs))?;
    let rhs = *values
        .get(&bin.rhs)
        .ok_or(LowerError::MissingValue(bin.rhs))?;
    let dst = value_reg(result);
    match bin.op {
        BinOpKind::And => asm.and_x(dst, lhs, rhs),
        BinOpKind::Or => asm.orr_x(dst, lhs, rhs),
        BinOpKind::Xor => asm.eor_x(dst, lhs, rhs),
        BinOpKind::Shl => asm.lsl_x(dst, lhs, rhs),
        BinOpKind::Shr => asm.lsr_x(dst, lhs, rhs),
        BinOpKind::Sar => asm.asr_x(dst, lhs, rhs),
        BinOpKind::Ror => asm.ror_x(dst, lhs, rhs),
        BinOpKind::Mul => asm.mul_x(dst, lhs, rhs),
        BinOpKind::UMulHi => asm.umulh_x(dst, lhs, rhs),
        BinOpKind::SMulHi => asm.smulh_x(dst, lhs, rhs),
        BinOpKind::UDiv => asm.udiv_x(dst, lhs, rhs),
        BinOpKind::SDiv => asm.sdiv_x(dst, lhs, rhs),
        _ => unreachable!("called only for register-register binops"),
    }
    values.insert(result, dst);
    Ok(())
}

fn lower_mod_binop(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    values: &mut HashMap<Ref, u8>,
    bin: &prisma_ir::BinOp,
) -> Result<(), LowerError> {
    let result = stmt.result.ok_or(LowerError::MissingResult("BinOp"))?;
    let lhs = *values
        .get(&bin.lhs)
        .ok_or(LowerError::MissingValue(bin.lhs))?;
    let rhs = *values
        .get(&bin.rhs)
        .ok_or(LowerError::MissingValue(bin.rhs))?;
    let dst = value_reg(result);
    match bin.op {
        BinOpKind::UMod => asm.udiv_x(MOD_QUOTIENT_REG, lhs, rhs),
        BinOpKind::SMod => asm.sdiv_x(MOD_QUOTIENT_REG, lhs, rhs),
        _ => unreachable!("called only for modulo binops"),
    }
    asm.msub_x(dst, MOD_QUOTIENT_REG, rhs, lhs);
    values.insert(result, dst);
    Ok(())
}

fn lower_compare(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    values: &mut HashMap<Ref, u8>,
    compare: &prisma_ir::Compare,
) -> Result<(), LowerError> {
    let result = stmt.result.ok_or(LowerError::MissingResult("Compare"))?;
    let lhs = *values
        .get(&compare.lhs)
        .ok_or(LowerError::MissingValue(compare.lhs))?;
    let rhs = *values
        .get(&compare.rhs)
        .ok_or(LowerError::MissingValue(compare.rhs))?;
    let dst = value_reg(result);
    let (lhs, rhs) = align_flag_operands(asm, compare.size, lhs, rhs);
    asm.cmp_x(lhs, rhs);
    asm.cset_x(dst, compare.cc);
    values.insert(result, dst);
    Ok(())
}

fn lower_cmp_flags(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    values: &HashMap<Ref, u8>,
    flags: &mut HashSet<Ref>,
    cmp: &prisma_ir::CmpFlags,
) -> Result<(), LowerError> {
    let result = stmt.result.ok_or(LowerError::MissingResult("CmpFlags"))?;
    let lhs = *values
        .get(&cmp.lhs)
        .ok_or(LowerError::MissingValue(cmp.lhs))?;
    let rhs = *values
        .get(&cmp.rhs)
        .ok_or(LowerError::MissingValue(cmp.rhs))?;
    let (lhs, rhs) = align_flag_operands(asm, cmp.size, lhs, rhs);
    asm.cmp_x(lhs, rhs);
    flags.insert(result);
    Ok(())
}

fn align_flag_operands(asm: &mut Arm64Assembler, size: OpSize, lhs: u8, rhs: u8) -> (u8, u8) {
    if size == OpSize::I64 {
        return (lhs, rhs);
    }

    let shift = u16::try_from(64 - size.bit_width()).expect("OpSize bit width is <= 64");
    asm.movz_x(FLAG_ALIGN_SHIFT_REG, shift, 0);
    asm.lsl_x(FLAG_ALIGN_LHS_REG, lhs, FLAG_ALIGN_SHIFT_REG);
    asm.lsl_x(FLAG_ALIGN_RHS_REG, rhs, FLAG_ALIGN_SHIFT_REG);
    (FLAG_ALIGN_LHS_REG, FLAG_ALIGN_RHS_REG)
}

fn lower_cond_jump(
    asm: &mut Arm64Assembler,
    labels: &HashMap<u32, Label>,
    values: &HashMap<Ref, u8>,
    jump: &prisma_ir::CondJump,
) -> Result<(), LowerError> {
    let cond = *values
        .get(&jump.cond)
        .ok_or(LowerError::MissingValue(jump.cond))?;
    let if_true = labels
        .get(&jump.if_true)
        .copied()
        .ok_or(LowerError::MissingTargetBlock(jump.if_true))?;
    let if_false = labels
        .get(&jump.if_false)
        .copied()
        .ok_or(LowerError::MissingTargetBlock(jump.if_false))?;
    asm.cbnz_x_label(cond, if_true);
    asm.b_label(if_false);
    Ok(())
}

fn lower_cond_jump_flags(
    asm: &mut Arm64Assembler,
    labels: &HashMap<u32, Label>,
    flags: &HashSet<Ref>,
    jump: &prisma_ir::CondJumpFlags,
) -> Result<(), LowerError> {
    if !flags.contains(&jump.flags) {
        return Err(LowerError::MissingValue(jump.flags));
    }
    let if_true = labels
        .get(&jump.if_true)
        .copied()
        .ok_or(LowerError::MissingTargetBlock(jump.if_true))?;
    let if_false = labels
        .get(&jump.if_false)
        .copied()
        .ok_or(LowerError::MissingTargetBlock(jump.if_false))?;
    asm.b_cond_label(jump.cc, if_true);
    asm.b_label(if_false);
    Ok(())
}

fn lower_cond_jump_rel(
    asm: &mut Arm64Assembler,
    labels: &HashMap<u32, Label>,
    jump: &prisma_ir::CondJumpRel,
) -> Result<(), LowerError> {
    let if_true = block_label(labels, jump.target_guest_pc)?;
    let if_false = block_label(labels, jump.fallthrough_guest_pc)?;
    asm.b_cond_label(jump.cc, if_true);
    asm.b_label(if_false);
    Ok(())
}

fn block_label(labels: &HashMap<u32, Label>, guest_pc: u64) -> Result<Label, LowerError> {
    let block_id =
        u32::try_from(guest_pc).map_err(|_| LowerError::UnsupportedOp("CondJumpRel"))?;
    labels
        .get(&block_id)
        .copied()
        .ok_or(LowerError::MissingTargetBlock(block_id))
}

fn value_reg(reference: Ref) -> u8 {
    let slot = reference % u32::from(VALUE_REG_COUNT);
    FIRST_VALUE_REG + u8::try_from(slot).expect("slot is bounded by VALUE_REG_COUNT")
}

fn emit_u64_constant(asm: &mut Arm64Assembler, dst: u8, value: u64) {
    let low = u16::try_from(value & 0xffff).expect("masked to 16 bits");
    asm.movz_x(dst, low, 0);
    for chunk in 1..4 {
        let part = u16::try_from((value >> (chunk * 16)) & 0xffff).expect("masked to 16 bits");
        if part != 0 {
            asm.movk_x(
                dst,
                part,
                u8::try_from(chunk * 16).expect("valid MOVK shift"),
            );
        }
    }
}

fn emit_load_mem(asm: &mut Arm64Assembler, size: OpSize, dst: u8, addr: u8) {
    match size {
        OpSize::I8 => asm.ldrb_unsigned(dst, addr, 0),
        OpSize::I16 => asm.ldrh_unsigned(dst, addr, 0),
        OpSize::I32 => asm.ldr_w_unsigned(dst, addr, 0),
        OpSize::I64 => asm.ldr_x_unsigned(dst, addr, 0),
    }
}

fn emit_load_reg(asm: &mut Arm64Assembler, size: OpSize, dst: u8, reg: Gpr) {
    let offset = gpr_offset_bytes(reg);
    match size {
        OpSize::I8 => asm.ldrb_unsigned(dst, abi::K_STATE_PTR_REG, offset),
        OpSize::I16 => asm.ldrh_unsigned(dst, abi::K_STATE_PTR_REG, offset),
        OpSize::I32 => asm.ldr_w_unsigned(dst, abi::K_STATE_PTR_REG, offset),
        OpSize::I64 => asm.ldr_x_unsigned(dst, abi::K_STATE_PTR_REG, offset),
    }
}

fn emit_store_mem(asm: &mut Arm64Assembler, size: OpSize, value: u8, addr: u8) {
    match size {
        OpSize::I8 => asm.strb_unsigned(value, addr, 0),
        OpSize::I16 => asm.strh_unsigned(value, addr, 0),
        OpSize::I32 => asm.str_w_unsigned(value, addr, 0),
        OpSize::I64 => asm.str_x_unsigned(value, addr, 0),
    }
}

fn emit_store_reg(asm: &mut Arm64Assembler, size: OpSize, value: u8, reg: Gpr) {
    let offset = gpr_offset_bytes(reg);
    match size {
        OpSize::I8 => asm.strb_unsigned(value, abi::K_STATE_PTR_REG, offset),
        OpSize::I16 => asm.strh_unsigned(value, abi::K_STATE_PTR_REG, offset),
        OpSize::I32 => asm.str_w_unsigned(value, abi::K_STATE_PTR_REG, offset),
        OpSize::I64 => asm.str_x_unsigned(value, abi::K_STATE_PTR_REG, offset),
    }
}

fn gpr_offset_bytes(reg: Gpr) -> u16 {
    u16::from(reg as u8) * 8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assembler::{cmp_x, cset_x, lsl_x, movz_x};
    use prisma_ir::{
        BasicBlock, BinOp, CmpFlags, Compare, CondCode, CondJump, CondJumpFlags, CondJumpRel, Constant,
        Jump, LoadMem, LoadReg, Return, Stmt, StoreMem, StoreReg,
    };

    fn function(stmts: Vec<Stmt>) -> Function {
        Function {
            blocks: vec![BasicBlock { id: 0, stmts }],
            entry: 0,
        }
    }

    fn function_with_blocks(blocks: Vec<BasicBlock>, entry: u32) -> Function {
        Function { blocks, entry }
    }

    #[test]
    fn lowers_constant_add_and_return() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 10,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 7,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::BinOp(BinOp {
                    op: BinOpKind::Add,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(None, Op::Return(Return)),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![0xD280_0149, 0xD280_00EA, 0x9100_1D2B, 0xD65F_03C0]
        );
    }

    #[test]
    fn lowers_sub_immediate() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 10,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 3,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::BinOp(BinOp {
                    op: BinOpKind::Sub,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![0xD280_0149, 0xD280_006A, 0xD100_0D2B]
        );
    }

    #[test]
    fn lowers_logical_register_ops() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0xf0,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 0x0f,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::BinOp(BinOp {
                    op: BinOpKind::And,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::BinOp(BinOp {
                    op: BinOpKind::Or,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(4),
                Op::BinOp(BinOp {
                    op: BinOpKind::Xor,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                0xD280_1E09,
                0xD280_01EA,
                0x8A0A_012B,
                0xAA0A_012C,
                0xCA0A_012D,
            ]
        );
    }

    #[test]
    fn lowers_shift_register_ops() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x80,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 3,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::BinOp(BinOp {
                    op: BinOpKind::Shl,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::BinOp(BinOp {
                    op: BinOpKind::Shr,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(4),
                Op::BinOp(BinOp {
                    op: BinOpKind::Sar,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(5),
                Op::BinOp(BinOp {
                    op: BinOpKind::Ror,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                0xD280_1009,
                0xD280_006A,
                0x9ACA_212B,
                0x9ACA_252C,
                0x9ACA_292D,
                0x9ACA_2D2E,
            ]
        );
    }

    #[test]
    fn lowers_multiply_divide_register_ops() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 21,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 3,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::BinOp(BinOp {
                    op: BinOpKind::Mul,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::BinOp(BinOp {
                    op: BinOpKind::UMulHi,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(4),
                Op::BinOp(BinOp {
                    op: BinOpKind::SMulHi,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(5),
                Op::BinOp(BinOp {
                    op: BinOpKind::UDiv,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(6),
                Op::BinOp(BinOp {
                    op: BinOpKind::SDiv,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                0xD280_02A9,
                0xD280_006A,
                0x9B0A_7D2B,
                0x9BCA_7D2C,
                0x9B4A_7D2D,
                0x9ACA_092E,
                0x9ACA_0D2F,
            ]
        );
    }

    #[test]
    fn lowers_modulo_register_ops() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 21,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 5,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::BinOp(BinOp {
                    op: BinOpKind::UMod,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::BinOp(BinOp {
                    op: BinOpKind::SMod,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                0xD280_02A9,
                0xD280_00AA,
                0x9ACA_0934,
                0x9B0A_A68B,
                0x9ACA_0D34,
                0x9B0A_A68C,
            ]
        );
    }

    #[test]
    fn rejects_unsupported_binop() {
        let func = function(vec![Stmt::new(
            Some(0),
            Op::BinOp(BinOp {
                op: BinOpKind::Pdep,
                lhs: 1,
                rhs: 2,
                size: OpSize::I64,
            }),
        )]);

        assert_eq!(
            Lowerer::new().lower_function(&func),
            Err(LowerError::UnsupportedOp("BinOp"))
        );
    }

    #[test]
    fn rejects_missing_entry() {
        let func = Function {
            blocks: Vec::new(),
            entry: 42,
        };
        assert_eq!(
            Lowerer::new().lower_function(&func),
            Err(LowerError::MissingEntryBlock(42))
        );
    }

    #[test]
    fn lowers_wide_constant_with_movk() {
        let func = function(vec![Stmt::new(
            Some(0),
            Op::Constant(Constant {
                value: 0x1234_5678_9ABC_DEF0,
                size: OpSize::I64,
            }),
        )]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![0xD29B_DE09, 0xF2B3_5789, 0xF2CA_CF09, 0xF2E2_4689]
        );
    }

    #[test]
    fn lowers_store_reg_from_constant() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x42,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::StoreReg(StoreReg {
                    reg: prisma_ir::Gpr::Rax,
                    value: 0,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![0xD280_0849, 0xF900_0369]
        );
    }

    #[test]
    fn lowers_load_reg_to_value() {
        let func = function(vec![Stmt::new(
            Some(0),
            Op::LoadReg(LoadReg {
                reg: prisma_ir::Gpr::Rcx,
                size: OpSize::I64,
            }),
        )]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![0xF940_0769]
        );
    }

    #[test]
    fn lowers_add_registers_and_stores_result() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::LoadReg(LoadReg {
                    reg: prisma_ir::Gpr::Rcx,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::LoadReg(LoadReg {
                    reg: prisma_ir::Gpr::Rax,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::BinOp(BinOp {
                    op: BinOpKind::Add,
                    lhs: 1,
                    rhs: 0,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::StoreReg(StoreReg {
                    reg: prisma_ir::Gpr::Rax,
                    value: 2,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![0xF940_0769, 0xF940_036A, 0x8B09_014B, 0xF900_036B]
        );
    }

    #[test]
    fn lowers_i64_store_and_load_mem() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x1000,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 0x2a,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::StoreMem(StoreMem {
                    addr: 0,
                    value: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::LoadMem(LoadMem {
                    addr: 0,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![0xD282_0009, 0xD280_054A, 0xF900_012A, 0xF940_012B]
        );
    }

    #[test]
    fn lowers_i8_i16_i32_memory_ops() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x1000,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 0x2a,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::StoreMem(StoreMem {
                    addr: 0,
                    value: 1,
                    size: OpSize::I8,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::LoadMem(LoadMem {
                    addr: 0,
                    size: OpSize::I8,
                }),
            ),
            Stmt::new(
                None,
                Op::StoreMem(StoreMem {
                    addr: 0,
                    value: 1,
                    size: OpSize::I16,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::LoadMem(LoadMem {
                    addr: 0,
                    size: OpSize::I16,
                }),
            ),
            Stmt::new(
                None,
                Op::StoreMem(StoreMem {
                    addr: 0,
                    value: 1,
                    size: OpSize::I32,
                }),
            ),
            Stmt::new(
                Some(4),
                Op::LoadMem(LoadMem {
                    addr: 0,
                    size: OpSize::I32,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                0xD282_0009,
                0xD280_054A,
                0x3900_012A,
                0x3940_012B,
                0x7900_012A,
                0x7940_012C,
                0xB900_012A,
                0xB940_012D,
            ]
        );
    }

    #[test]
    fn lowers_compare_eq() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 7,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 7,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::Compare(Compare {
                    cc: CondCode::Eq,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![0xD280_00E9, 0xD280_00EA, 0xEB0A_013F, 0x9A9F_17EB]
        );
    }

    #[test]
    fn lowers_i8_compare_eq_with_operand_alignment() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x7f,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 0x7f,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::Compare(Compare {
                    cc: CondCode::Eq,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I8,
                }),
            ),
        ]);

        let lhs_reg = value_reg(0);
        let rhs_reg = value_reg(1);
        let result_reg = value_reg(2);
        let shift = u16::try_from(64 - OpSize::I8.bit_width())
            .expect("I8 fits in u16 shift");
        let expected = vec![
            movz_x(lhs_reg, 0x7f, 0),
            movz_x(rhs_reg, 0x7f, 0),
            movz_x(FLAG_ALIGN_SHIFT_REG, shift, 0),
            lsl_x(FLAG_ALIGN_LHS_REG, lhs_reg, FLAG_ALIGN_SHIFT_REG),
            lsl_x(FLAG_ALIGN_RHS_REG, rhs_reg, FLAG_ALIGN_SHIFT_REG),
            cmp_x(FLAG_ALIGN_LHS_REG, FLAG_ALIGN_RHS_REG),
            cset_x(result_reg, CondCode::Eq),
        ];

        assert_eq!(Lowerer::new().lower_function(&func).unwrap(), expected);
    }

    #[test]
    fn lowers_i16_compare_eq_with_operand_alignment() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 5,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 5,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::Compare(Compare {
                    cc: CondCode::Eq,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I16,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            {
                let lhs_reg = value_reg(0);
                let rhs_reg = value_reg(1);
                let result_reg = value_reg(2);
                let shift = u16::try_from(64 - OpSize::I16.bit_width())
                    .expect("I16 fits in u16 shift");
                vec![
                    movz_x(lhs_reg, 5, 0),
                    movz_x(rhs_reg, 5, 0),
                    movz_x(FLAG_ALIGN_SHIFT_REG, shift, 0),
                    lsl_x(FLAG_ALIGN_LHS_REG, lhs_reg, FLAG_ALIGN_SHIFT_REG),
                    lsl_x(FLAG_ALIGN_RHS_REG, rhs_reg, FLAG_ALIGN_SHIFT_REG),
                    cmp_x(FLAG_ALIGN_LHS_REG, FLAG_ALIGN_RHS_REG),
                    cset_x(result_reg, CondCode::Eq),
                ]
            }
        );
    }

    #[test]
    fn lowers_i32_compare_eq_with_operand_alignment() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 10,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 10,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::Compare(Compare {
                    cc: CondCode::Eq,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I32,
                }),
            ),
        ]);

        let lhs_reg = value_reg(0);
        let rhs_reg = value_reg(1);
        let result_reg = value_reg(2);
        let shift = u16::try_from(64 - OpSize::I32.bit_width())
            .expect("I32 fits in u16 shift");
        let expected = vec![
            movz_x(lhs_reg, 10, 0),
            movz_x(rhs_reg, 10, 0),
            movz_x(FLAG_ALIGN_SHIFT_REG, shift, 0),
            lsl_x(FLAG_ALIGN_LHS_REG, lhs_reg, FLAG_ALIGN_SHIFT_REG),
            lsl_x(FLAG_ALIGN_RHS_REG, rhs_reg, FLAG_ALIGN_SHIFT_REG),
            cmp_x(FLAG_ALIGN_LHS_REG, FLAG_ALIGN_RHS_REG),
            cset_x(result_reg, CondCode::Eq),
        ];

        assert_eq!(Lowerer::new().lower_function(&func).unwrap(), expected);
    }

    #[test]
    fn lowers_forward_jump_between_blocks() {
        let func = function_with_blocks(
            vec![
                BasicBlock {
                    id: 0,
                    stmts: vec![
                        Stmt::new(
                            Some(0),
                            Op::Constant(Constant {
                                value: 1,
                                size: OpSize::I64,
                            }),
                        ),
                        Stmt::new(None, Op::Jump(Jump { target_block: 1 })),
                    ],
                },
                BasicBlock {
                    id: 1,
                    stmts: vec![Stmt::new(None, Op::Return(Return))],
                },
            ],
            0,
        );

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![0xD280_0029, 0x1400_0001, 0xD65F_03C0]
        );
    }

    #[test]
    fn lowers_backward_jump_between_blocks() {
        let func = function_with_blocks(
            vec![
                BasicBlock {
                    id: 0,
                    stmts: vec![Stmt::new(None, Op::Jump(Jump { target_block: 1 }))],
                },
                BasicBlock {
                    id: 1,
                    stmts: vec![Stmt::new(None, Op::Jump(Jump { target_block: 0 }))],
                },
            ],
            0,
        );

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![0x1400_0001, 0x17FF_FFFF]
        );
    }

    #[test]
    fn rejects_jump_to_missing_block() {
        let func = function(vec![Stmt::new(None, Op::Jump(Jump { target_block: 42 }))]);

        assert_eq!(
            Lowerer::new().lower_function(&func),
            Err(LowerError::MissingTargetBlock(42))
        );
    }

    #[test]
    fn lowers_cond_jump_between_blocks() {
        let func = function_with_blocks(
            vec![
                BasicBlock {
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
                            None,
                            Op::CondJump(CondJump {
                                cond: 0,
                                if_true: 1,
                                if_false: 2,
                            }),
                        ),
                    ],
                },
                BasicBlock {
                    id: 1,
                    stmts: vec![Stmt::new(None, Op::Return(Return))],
                },
                BasicBlock {
                    id: 2,
                    stmts: vec![Stmt::new(None, Op::Return(Return))],
                },
            ],
            0,
        );

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                0xD280_0029,
                0xB500_0049,
                0x1400_0002,
                0xD65F_03C0,
                0xD65F_03C0,
            ]
        );
    }

    #[test]
    fn lowers_cond_jump_rel_between_blocks() {
        let func = function_with_blocks(
            vec![
                BasicBlock {
                    id: 0,
                    stmts: vec![Stmt::new(
                        None,
                        Op::CondJumpRel(CondJumpRel {
                            cc: CondCode::Eq,
                            target_guest_pc: 1,
                            fallthrough_guest_pc: 2,
                        }),
                    )],
                },
                BasicBlock {
                    id: 1,
                    stmts: vec![Stmt::new(None, Op::Return(Return))],
                },
                BasicBlock {
                    id: 2,
                    stmts: vec![Stmt::new(None, Op::Return(Return))],
                },
            ],
            0,
        );

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![0x5400_0041, 0x1400_0002, 0xD65F_03C0, 0xD65F_03C0]
        );
    }

    #[test]
    fn lowers_compare_then_cond_jump() {
        let func = function_with_blocks(
            vec![
                BasicBlock {
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
                            Op::Constant(Constant {
                                value: 3,
                                size: OpSize::I64,
                            }),
                        ),
                        Stmt::new(
                            Some(2),
                            Op::Compare(Compare {
                                cc: CondCode::Ne,
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
                    id: 1,
                    stmts: vec![Stmt::new(None, Op::Return(Return))],
                },
                BasicBlock {
                    id: 2,
                    stmts: vec![Stmt::new(None, Op::Return(Return))],
                },
            ],
            0,
        );

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                0xD280_00E9,
                0xD280_006A,
                0xEB0A_013F,
                0x9A9F_07EB,
                0xB500_004B,
                0x1400_0002,
                0xD65F_03C0,
                0xD65F_03C0,
            ]
        );
    }

    #[test]
    fn lowers_cmp_flags_then_cond_jump_flags() {
        let func = function_with_blocks(
            vec![
                BasicBlock {
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
                            Op::Constant(Constant {
                                value: 3,
                                size: OpSize::I64,
                            }),
                        ),
                        Stmt::new(
                            Some(2),
                            Op::CmpFlags(CmpFlags {
                                lhs: 0,
                                rhs: 1,
                                size: OpSize::I64,
                            }),
                        ),
                        Stmt::new(
                            None,
                            Op::CondJumpFlags(CondJumpFlags {
                                flags: 2,
                                cc: CondCode::Ne,
                                if_true: 1,
                                if_false: 2,
                            }),
                        ),
                    ],
                },
                BasicBlock {
                    id: 1,
                    stmts: vec![Stmt::new(None, Op::Return(Return))],
                },
                BasicBlock {
                    id: 2,
                    stmts: vec![Stmt::new(None, Op::Return(Return))],
                },
            ],
            0,
        );

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                0xD280_00E9,
                0xD280_006A,
                0xEB0A_013F,
                0x5400_0041,
                0x1400_0002,
                0xD65F_03C0,
                0xD65F_03C0,
            ]
        );
    }

    #[test]
    fn lowers_i8_cmp_flags_with_operand_alignment() {
        let func = function_with_blocks(
            vec![
                BasicBlock {
                    id: 0,
                    stmts: vec![
                        Stmt::new(
                            Some(0),
                            Op::Constant(Constant {
                                value: 0xff,
                                size: OpSize::I64,
                            }),
                        ),
                        Stmt::new(
                            Some(1),
                            Op::Constant(Constant {
                                value: 1,
                                size: OpSize::I64,
                            }),
                        ),
                        Stmt::new(
                            Some(2),
                            Op::CmpFlags(CmpFlags {
                                lhs: 0,
                                rhs: 1,
                                size: OpSize::I8,
                            }),
                        ),
                        Stmt::new(
                            None,
                            Op::CondJumpFlags(CondJumpFlags {
                                flags: 2,
                                cc: CondCode::Ult,
                                if_true: 1,
                                if_false: 2,
                            }),
                        ),
                    ],
                },
                BasicBlock {
                    id: 1,
                    stmts: vec![Stmt::new(None, Op::Return(Return))],
                },
                BasicBlock {
                    id: 2,
                    stmts: vec![Stmt::new(None, Op::Return(Return))],
                },
            ],
            0,
        );

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                0xD280_1FE9,
                0xD280_002A,
                0xD280_0713,
                0x9AD3_2131,
                0x9AD3_2152,
                0xEB12_023F,
                0x5400_0043,
                0x1400_0002,
                0xD65F_03C0,
                0xD65F_03C0,
            ]
        );
    }

    #[test]
    fn lowers_i32_cmp_flags_with_operand_alignment() {
        let func = function_with_blocks(
            vec![
                BasicBlock {
                    id: 0,
                    stmts: vec![
                        Stmt::new(
                            Some(0),
                            Op::Constant(Constant {
                                value: 0x8000_0000,
                                size: OpSize::I64,
                            }),
                        ),
                        Stmt::new(
                            Some(1),
                            Op::Constant(Constant {
                                value: 1,
                                size: OpSize::I64,
                            }),
                        ),
                        Stmt::new(
                            Some(2),
                            Op::CmpFlags(CmpFlags {
                                lhs: 0,
                                rhs: 1,
                                size: OpSize::I32,
                            }),
                        ),
                        Stmt::new(
                            None,
                            Op::CondJumpFlags(CondJumpFlags {
                                flags: 2,
                                cc: CondCode::Slt,
                                if_true: 1,
                                if_false: 2,
                            }),
                        ),
                    ],
                },
                BasicBlock {
                    id: 1,
                    stmts: vec![Stmt::new(None, Op::Return(Return))],
                },
                BasicBlock {
                    id: 2,
                    stmts: vec![Stmt::new(None, Op::Return(Return))],
                },
            ],
            0,
        );

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                0xD280_0009,
                0xF2B0_0009,
                0xD280_002A,
                0xD280_0413,
                0x9AD3_2131,
                0x9AD3_2152,
                0xEB12_023F,
                0x5400_004B,
                0x1400_0002,
                0xD65F_03C0,
                0xD65F_03C0,
            ]
        );
    }

    #[test]
    fn rejects_cmp_flags_missing_result() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 7,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 3,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::CmpFlags(CmpFlags {
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func),
            Err(LowerError::MissingResult("CmpFlags"))
        );
    }

    #[test]
    fn rejects_cond_jump_flags_missing_flags() {
        let func = function_with_blocks(
            vec![
                BasicBlock {
                    id: 0,
                    stmts: vec![Stmt::new(
                        None,
                        Op::CondJumpFlags(CondJumpFlags {
                            flags: 99,
                            cc: CondCode::Eq,
                            if_true: 1,
                            if_false: 2,
                        }),
                    )],
                },
                BasicBlock {
                    id: 1,
                    stmts: Vec::new(),
                },
                BasicBlock {
                    id: 2,
                    stmts: Vec::new(),
                },
            ],
            0,
        );

        assert_eq!(
            Lowerer::new().lower_function(&func),
            Err(LowerError::MissingValue(99))
        );
    }

    #[test]
    fn rejects_cond_jump_missing_condition() {
        let func = function_with_blocks(
            vec![
                BasicBlock {
                    id: 0,
                    stmts: vec![Stmt::new(
                        None,
                        Op::CondJump(CondJump {
                            cond: 7,
                            if_true: 1,
                            if_false: 2,
                        }),
                    )],
                },
                BasicBlock {
                    id: 1,
                    stmts: Vec::new(),
                },
                BasicBlock {
                    id: 2,
                    stmts: Vec::new(),
                },
            ],
            0,
        );

        assert_eq!(
            Lowerer::new().lower_function(&func),
            Err(LowerError::MissingValue(7))
        );
    }

    #[test]
    fn lowers_jump_reg() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x1000,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(None, Op::JumpReg(prisma_ir::JumpReg { target: 0 })),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![0xD282_0009, 0xD61F_0120]
        );
    }

    #[test]
    fn rejects_jump_reg_missing_target() {
        let func = function(vec![Stmt::new(
            None,
            Op::JumpReg(prisma_ir::JumpReg { target: 99 }),
        )]);

        assert_eq!(
            Lowerer::new().lower_function(&func),
            Err(LowerError::MissingValue(99))
        );
    }
}
