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
/// Scratch register used for dynamic RSP reads/writes during stack adjustments.
const RSP_ADJUST_TMP_REG: u8 = 21;
/// Scratch register used for large RSP immediate materialization.
const RSP_ADJUST_IMM_REG: u8 = 22;
/// Scratch register used for flag-writing ALU side-effect operations.
const ALU_FLAGS_TMP_REG: u8 = 23;

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
    /// - `Select` via flag-dependent branch sequencing (`B.cond` + `MOV`)
    /// - `LoadMem`/`StoreMem` for `I8`/`I16`/`I32`/`I64` with address/value
    ///   already in registers
    /// - direct `Jump`, `JumpRel`, `CallRel` and `CallReg` between/through
    ///   registers
    /// - `RspAdjust` and `RetAdjusted` stack adjustments over `Rsp` state
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

// One match arm per IR op; the dispatch is inherently long and splitting it
// would only scatter the op->lowering mapping across helpers.
#[allow(clippy::too_many_lines)]
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
        Op::AluFlags(alu) => {
            lower_alu_flags(asm, values, alu)?;
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
        Op::LoadMemTSO(load) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("LoadMemTSO"))?;
            let addr = *values
                .get(&load.addr)
                .ok_or(LowerError::MissingValue(load.addr))?;
            let dst = value_reg(result);
            emit_load_mem(asm, load.size, dst, addr);
            asm.fence(prisma_ir::FenceKind::Mfence);
            values.insert(result, dst);
        }
        Op::StoreMemTSO(store) => {
            let addr = *values
                .get(&store.addr)
                .ok_or(LowerError::MissingValue(store.addr))?;
            let value = *values
                .get(&store.value)
                .ok_or(LowerError::MissingValue(store.value))?;
            asm.fence(prisma_ir::FenceKind::Mfence);
            emit_store_mem(asm, store.size, value, addr);
            asm.fence(prisma_ir::FenceKind::Mfence);
        }
        Op::LoadSegBase(seg) => {
            lower_load_seg_base(stmt, asm, values, seg)?;
        }
        Op::Cpuid(_) => lower_cpuid(asm),
        Op::Xgetbv(_) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("Xgetbv"))?;
            let dst = value_reg(result);
            lower_xgetbv(asm, dst);
            values.insert(result, dst);
        }
        Op::Rdtsc(_) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("Rdtsc"))?;
            let dst = value_reg(result);
            asm.mrs_cntvct(dst);
            values.insert(result, dst);
        }
        Op::Syscall(_) => {
            lower_syscall(asm);
        }
        Op::Trap(_) => {
            asm.movz_x(0, 0, 0);
            asm.ret();
        }
        Op::Extend(extend) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("Extend"))?;
            let dst = value_reg(result);
            let src = *values
                .get(&extend.value)
                .ok_or(LowerError::MissingValue(extend.value))?;
            lower_extend(
                asm,
                dst,
                src,
                extend.from_size,
                extend.to_size,
                extend.is_signed,
            );
            values.insert(result, dst);
        }
        Op::Truncate(trunc) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("Truncate"))?;
            let dst = value_reg(result);
            let src = *values
                .get(&trunc.value)
                .ok_or(LowerError::MissingValue(trunc.value))?;
            lower_truncate(asm, dst, src, trunc.to_size);
            values.insert(result, dst);
        }
        Op::Fence(fence) => match fence.kind {
            prisma_ir::FenceKind::Mfence
            | prisma_ir::FenceKind::Lfence
            | prisma_ir::FenceKind::Sfence => {
                asm.fence(fence.kind);
            }
        },
        Op::GuestPc(_) => {}
        Op::WriteFlags(write_flags) => {
            lower_write_flags(asm, values, flags, write_flags, stmt)?;
        }
        Op::WriteFlagsPopcnt(popcnt) => {
            lower_write_flags_popcnt(asm, values, popcnt)?;
        }
        Op::WriteFlagsCountZero(count_zero) => {
            lower_write_flags_count_zero(asm, values, count_zero)?;
        }
        Op::ReadFlag(flag_read) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("ReadFlag"))?;
            let dst = value_reg(result);
            lower_read_flag(asm, flags, flag_read, dst)?;
            values.insert(result, dst);
        }
        Op::Lzcnt(lzcnt) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("Lzcnt"))?;
            let src = *values
                .get(&lzcnt.value)
                .ok_or(LowerError::MissingValue(lzcnt.value))?;
            let dst = value_reg(result);
            lower_lzcnt(asm, dst, src, lzcnt.size);
            values.insert(result, dst);
        }
        Op::Tzcnt(tzcnt) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("Tzcnt"))?;
            let src = *values
                .get(&tzcnt.value)
                .ok_or(LowerError::MissingValue(tzcnt.value))?;
            let dst = value_reg(result);
            lower_tzcnt(asm, dst, src, tzcnt.size);
            values.insert(result, dst);
        }
        Op::Popcnt(popcnt) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("Popcnt"))?;
            let src = *values
                .get(&popcnt.value)
                .ok_or(LowerError::MissingValue(popcnt.value))?;
            let dst = value_reg(result);
            lower_popcnt(asm, dst, src, popcnt.size);
            values.insert(result, dst);
        }
        Op::Bswap(bswap) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("Bswap"))?;
            let src = *values
                .get(&bswap.value)
                .ok_or(LowerError::MissingValue(bswap.value))?;
            let dst = value_reg(result);
            lower_bswap(asm, dst, src, bswap.size);
            values.insert(result, dst);
        }
        Op::Crc32c(crc) => {
            let result = stmt.result.ok_or(LowerError::MissingResult("Crc32c"))?;
            let crc_reg = *values
                .get(&crc.crc)
                .ok_or(LowerError::MissingValue(crc.crc))?;
            let data_reg = *values
                .get(&crc.data)
                .ok_or(LowerError::MissingValue(crc.data))?;
            let dst = value_reg(result);
            lower_crc32c(asm, dst, crc_reg, data_reg, crc.data_size);
            values.insert(result, dst);
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
        Op::Select(select) => {
            lower_select(asm, values, select, stmt)?;
        }
        Op::JumpReg(jump) => {
            let target = *values
                .get(&jump.target)
                .ok_or(LowerError::MissingValue(jump.target))?;
            asm.br_x(target);
        }
        Op::JumpRel(jump) => {
            let target = block_label(labels, jump.target_guest_pc)?;
            asm.b_label(target);
        }
        Op::CallRel(call) => {
            let target = block_label(labels, call.target_guest_pc)?;
            asm.b_label(target);
            let _ = call.return_guest_pc;
        }
        Op::CallReg(call) => {
            let target = *values
                .get(&call.target)
                .ok_or(LowerError::MissingValue(call.target))?;
            asm.blr_x(target);
            let _ = call.return_guest_pc;
        }
        Op::RspAdjust(rsp) => {
            lower_rsp_adjust(asm, rsp)?;
        }
        Op::Return(_) => asm.ret(),
        Op::RetAdjusted(ret) => {
            lower_ret_adjusted(asm, ret.pop_bytes)?;
        }
        _ => return Err(LowerError::UnsupportedOp("unsupported")),
    }

    Ok(())
}

const FS_BASE_OFFSET: u16 = 792;
const GS_BASE_OFFSET: u16 = 800;
const KSTATE_CPUID_MAX_LEAF: u64 = 7;
const KSTATE_CPUID_VENDOR_EBX: u64 = 0x756E_6547;
const KSTATE_CPUID_VENDOR_EDX: u64 = 0x4965_6E69;
const KSTATE_CPUID_VENDOR_ECX: u64 = 0x6C65_746E;
const KSTATE_CPUID_LEAF1_EAX: u64 = 0x0002_06A7;
const KSTATE_CPUID_LEAF1_EBX: u64 = 0x0000_0800;
const KSTATE_CPUID_LEAF1_ECX: u64 = (1u64 << 0)
    | (1u64 << 9)
    | (1u64 << 12)
    | (1u64 << 13)
    | (1u64 << 19)
    | (1u64 << 22)
    | (1u64 << 23)
    | (1u64 << 27)
    | (1u64 << 28);
const KSTATE_CPUID_LEAF1_EDX: u64 =
    (1u64 << 0) | (1u64 << 4) | (1u64 << 8) | (1u64 << 15) | (1u64 << 25) | (1u64 << 26);
const KSTATE_CPUID_LEAF7_EBX: u64 = 1u64 << 8;
const KSTATE_XCR0_EAX: u64 = 0x7;

fn lower_select(
    asm: &mut Arm64Assembler,
    values: &mut HashMap<Ref, u8>,
    select: &prisma_ir::Select,
    stmt: &Stmt,
) -> Result<(), LowerError> {
    let result = stmt.result.ok_or(LowerError::MissingResult("Select"))?;
    let true_value = *values
        .get(&select.true_value)
        .ok_or(LowerError::MissingValue(select.true_value))?;
    let false_value = *values
        .get(&select.false_value)
        .ok_or(LowerError::MissingValue(select.false_value))?;

    let result_reg = value_reg(result);
    let true_label = asm.create_label();
    let end_label = asm.create_label();

    asm.b_cond_label(select.cc, true_label);
    asm.mov_x(result_reg, false_value);
    asm.b_label(end_label);
    asm.bind_label(true_label);
    asm.mov_x(result_reg, true_value);
    asm.bind_label(end_label);

    values.insert(result, result_reg);
    Ok(())
}

fn lower_bswap(asm: &mut Arm64Assembler, dst: u8, src: u8, size: OpSize) {
    match size {
        OpSize::I64 => asm.rev_x(dst, src),
        OpSize::I32 => asm.rev_w(dst, src),
        OpSize::I16 => {
            asm.rev_w(dst, src);
            emit_u64_constant(asm, FLAG_ALIGN_SHIFT_REG, 16);
            asm.lsr_x(dst, dst, FLAG_ALIGN_SHIFT_REG);
        }
        OpSize::I8 => lower_truncate(asm, dst, src, OpSize::I8),
    }
}

fn lower_crc32c(asm: &mut Arm64Assembler, dst: u8, crc: u8, data: u8, data_size: OpSize) {
    match data_size {
        OpSize::I8 => asm.crc32cb(dst, crc, data),
        OpSize::I16 => asm.crc32ch(dst, crc, data),
        OpSize::I32 => asm.crc32cw(dst, crc, data),
        OpSize::I64 => asm.crc32cx(dst, crc, data),
    }
}

fn lower_write_flags_count_zero(
    asm: &mut Arm64Assembler,
    values: &HashMap<Ref, u8>,
    count_zero: &prisma_ir::WriteFlagsCountZero,
) -> Result<(), LowerError> {
    let src = *values
        .get(&count_zero.src)
        .ok_or(LowerError::MissingValue(count_zero.src))?;
    let result = *values
        .get(&count_zero.result)
        .ok_or(LowerError::MissingValue(count_zero.result))?;

    let z_bit = FLAG_ALIGN_LHS_REG;
    let c_bit = FLAG_ALIGN_RHS_REG;
    let shift = FLAG_ALIGN_SHIFT_REG;

    if count_zero.size == OpSize::I64 {
        asm.cmp_x(result, 31);
    } else {
        lower_truncate(asm, z_bit, result, count_zero.size);
        asm.cmp_x(z_bit, 31);
    }
    asm.cset_x(z_bit, prisma_ir::CondCode::Eq);
    if count_zero.size == OpSize::I64 {
        asm.cmp_x(src, 31);
    } else {
        lower_truncate(asm, c_bit, src, count_zero.size);
        asm.cmp_x(c_bit, 31);
    }
    asm.cset_x(c_bit, prisma_ir::CondCode::Eq);
    emit_u64_constant(asm, shift, 30);
    asm.lsl_x(z_bit, z_bit, shift);
    emit_u64_constant(asm, shift, 29);
    asm.lsl_x(c_bit, c_bit, shift);
    asm.orr_x(z_bit, z_bit, c_bit);
    asm.msr_nzcv(z_bit);

    Ok(())
}

fn lower_write_flags_popcnt(
    asm: &mut Arm64Assembler,
    values: &HashMap<Ref, u8>,
    popcnt: &prisma_ir::WriteFlagsPopcnt,
) -> Result<(), LowerError> {
    let src = *values
        .get(&popcnt.src)
        .ok_or(LowerError::MissingValue(popcnt.src))?;

    let z_bit = FLAG_ALIGN_LHS_REG;
    let shift = FLAG_ALIGN_SHIFT_REG;

    if popcnt.size == OpSize::I64 {
        asm.cmp_x(src, 31);
    } else {
        lower_truncate(asm, z_bit, src, popcnt.size);
        asm.cmp_x(z_bit, 31);
    }
    asm.cset_x(z_bit, prisma_ir::CondCode::Eq);
    emit_u64_constant(asm, shift, 30);
    asm.lsl_x(z_bit, z_bit, shift);
    asm.msr_nzcv(z_bit);

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
        | BinOpKind::Rol
        | BinOpKind::Mul
        | BinOpKind::UMulHi
        | BinOpKind::SMulHi
        | BinOpKind::UDiv
        | BinOpKind::SDiv => lower_reg_binop(stmt, asm, values, bin),
        BinOpKind::UMod | BinOpKind::SMod => lower_mod_binop(stmt, asm, values, bin),
        _ => Err(LowerError::UnsupportedOp("BinOp")),
    }
}

fn lower_cpuid(asm: &mut Arm64Assembler) {
    let leaf0 = asm.create_label();
    let leaf1 = asm.create_label();
    let leaf7 = asm.create_label();
    let other = asm.create_label();
    let done = asm.create_label();

    let rax = FLAG_ALIGN_SHIFT_REG;
    let rcx = FLAG_ALIGN_RHS_REG;
    let tmp = FLAG_ALIGN_LHS_REG;
    let shift = RSP_ADJUST_TMP_REG;

    emit_load_reg(asm, OpSize::I64, rax, Gpr::Rax);
    emit_load_reg(asm, OpSize::I64, rcx, Gpr::Rcx);
    asm.uxtw_x(rax, rax);
    asm.uxtw_x(rcx, rcx);

    asm.cbz_x_label(rax, leaf0);
    emit_u64_constant(asm, tmp, 1);
    asm.eor_x(tmp, rax, tmp);
    asm.cbz_x_label(tmp, leaf1);
    emit_u64_constant(asm, tmp, 7);
    asm.eor_x(tmp, rax, tmp);
    asm.cbz_x_label(tmp, leaf7);

    emit_u64_constant(asm, tmp, 31);
    asm.lsr_x(shift, rax, tmp);
    asm.cbnz_x_label(shift, other);

    emit_u64_constant(asm, tmp, 3);
    asm.lsr_x(shift, rax, tmp);
    asm.cbnz_x_label(shift, leaf7);

    asm.b_label(other);

    asm.bind_label(leaf7);
    asm.cbnz_x_label(rcx, other);
    emit_u64_constant(asm, rax, 0);
    emit_u64_constant(asm, tmp, KSTATE_CPUID_LEAF7_EBX);
    emit_u64_constant(asm, rcx, 0);
    emit_u64_constant(asm, shift, 0);
    emit_store_reg(asm, OpSize::I64, rax, Gpr::Rax);
    emit_store_reg(asm, OpSize::I64, tmp, Gpr::Rbx);
    emit_store_reg(asm, OpSize::I64, rcx, Gpr::Rcx);
    emit_store_reg(asm, OpSize::I64, shift, Gpr::Rdx);
    asm.b_label(done);

    asm.bind_label(leaf1);
    emit_u64_constant(asm, rax, KSTATE_CPUID_LEAF1_EAX);
    emit_u64_constant(asm, tmp, KSTATE_CPUID_LEAF1_EBX);
    emit_u64_constant(asm, rcx, KSTATE_CPUID_LEAF1_ECX);
    emit_u64_constant(asm, shift, KSTATE_CPUID_LEAF1_EDX);
    emit_store_reg(asm, OpSize::I64, rax, Gpr::Rax);
    emit_store_reg(asm, OpSize::I64, tmp, Gpr::Rbx);
    emit_store_reg(asm, OpSize::I64, rcx, Gpr::Rcx);
    emit_store_reg(asm, OpSize::I64, shift, Gpr::Rdx);
    asm.b_label(done);

    asm.bind_label(leaf0);
    emit_u64_constant(asm, rax, KSTATE_CPUID_MAX_LEAF);
    emit_u64_constant(asm, tmp, KSTATE_CPUID_VENDOR_EBX);
    emit_u64_constant(asm, rcx, KSTATE_CPUID_VENDOR_ECX);
    emit_u64_constant(asm, shift, KSTATE_CPUID_VENDOR_EDX);
    emit_store_reg(asm, OpSize::I64, rax, Gpr::Rax);
    emit_store_reg(asm, OpSize::I64, tmp, Gpr::Rbx);
    emit_store_reg(asm, OpSize::I64, rcx, Gpr::Rcx);
    emit_store_reg(asm, OpSize::I64, shift, Gpr::Rdx);
    asm.b_label(done);

    asm.bind_label(other);
    emit_u64_constant(asm, rax, 0);
    emit_u64_constant(asm, tmp, 0);
    emit_u64_constant(asm, rcx, 0);
    emit_u64_constant(asm, shift, 0);
    emit_store_reg(asm, OpSize::I64, rax, Gpr::Rax);
    emit_store_reg(asm, OpSize::I64, tmp, Gpr::Rbx);
    emit_store_reg(asm, OpSize::I64, rcx, Gpr::Rcx);
    emit_store_reg(asm, OpSize::I64, shift, Gpr::Rdx);

    asm.bind_label(done);
}

fn lower_xgetbv(asm: &mut Arm64Assembler, dst: u8) {
    let rcx = RSP_ADJUST_TMP_REG;
    let other = asm.create_label();
    let done = asm.create_label();

    emit_load_reg(asm, OpSize::I64, rcx, Gpr::Rcx);
    asm.uxtw_x(rcx, rcx);
    asm.cbnz_x_label(rcx, other);
    emit_u64_constant(asm, dst, KSTATE_XCR0_EAX);
    emit_store_reg(asm, OpSize::I64, dst, Gpr::Rax);
    emit_u64_constant(asm, rcx, 0);
    emit_store_reg(asm, OpSize::I64, rcx, Gpr::Rdx);
    asm.b_label(done);

    asm.bind_label(other);
    emit_u64_constant(asm, dst, 0);
    emit_store_reg(asm, OpSize::I64, dst, Gpr::Rax);
    emit_store_reg(asm, OpSize::I64, dst, Gpr::Rdx);

    asm.bind_label(done);
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
        if let Ok(imm12) = u16::try_from(*rhs) {
            if imm12 < 4096 {
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
        BinOpKind::Rol => {
            asm.sub_x(FLAG_ALIGN_SHIFT_REG, 31, rhs);
            asm.ror_x(dst, lhs, FLAG_ALIGN_SHIFT_REG);
        }
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

fn lower_alu_flags(
    asm: &mut Arm64Assembler,
    values: &HashMap<Ref, u8>,
    alu: &prisma_ir::AluFlags,
) -> Result<(), LowerError> {
    let lhs = *values
        .get(&alu.lhs)
        .ok_or(LowerError::MissingValue(alu.lhs))?;
    let rhs = *values
        .get(&alu.rhs)
        .ok_or(LowerError::MissingValue(alu.rhs))?;
    let (lhs, rhs) = align_flag_operands(asm, alu.size, lhs, rhs);
    match alu.op {
        BinOpKind::Sub => asm.cmp_x(lhs, rhs),
        BinOpKind::Add => asm.adds_x(ALU_FLAGS_TMP_REG, lhs, rhs),
        BinOpKind::And => asm.ands_x(ALU_FLAGS_TMP_REG, lhs, rhs),
        // OR/XOR have no flag-setting ARM64 form; materialise the result
        // then re-AND it with itself so NZCV picks up N/Z and clears C/V,
        // matching x86 logical-op flags (SF/ZF from result, CF=OF=0).
        BinOpKind::Or => emit_logical_flags_or_xor(asm, lhs, rhs, false),
        BinOpKind::Xor => emit_logical_flags_or_xor(asm, lhs, rhs, true),
        _ => {
            return Err(LowerError::UnsupportedOp(
                "AluFlags only supports Sub/Add/And/Or/Xor today",
            ));
        }
    }
    Ok(())
}

/// Emit a flag-setting OR (`is_xor == false`) or XOR over the flag-aligned
/// operands. ARM64 has no `orrs`/`eors`, so we compute into the ALU flags
/// scratch register and follow with `ands Xt, Xt, Xt` to publish N/Z and
/// clear C/V.
fn emit_logical_flags_or_xor(asm: &mut Arm64Assembler, lhs: u8, rhs: u8, is_xor: bool) {
    if is_xor {
        asm.eor_x(ALU_FLAGS_TMP_REG, lhs, rhs);
    } else {
        asm.orr_x(ALU_FLAGS_TMP_REG, lhs, rhs);
    }
    asm.ands_x(ALU_FLAGS_TMP_REG, ALU_FLAGS_TMP_REG, ALU_FLAGS_TMP_REG);
}

fn lower_load_seg_base(
    stmt: &Stmt,
    asm: &mut Arm64Assembler,
    values: &mut HashMap<Ref, u8>,
    seg: &prisma_ir::LoadSegBase,
) -> Result<(), LowerError> {
    let result = stmt
        .result
        .ok_or(LowerError::MissingResult("LoadSegBase"))?;
    let dst = value_reg(result);
    match seg.seg {
        prisma_ir::SegmentReg::Fs => asm.ldr_x_unsigned(dst, abi::K_STATE_PTR_REG, FS_BASE_OFFSET),
        prisma_ir::SegmentReg::Gs => asm.ldr_x_unsigned(dst, abi::K_STATE_PTR_REG, GS_BASE_OFFSET),
        prisma_ir::SegmentReg::Es
        | prisma_ir::SegmentReg::Cs
        | prisma_ir::SegmentReg::Ss
        | prisma_ir::SegmentReg::Ds => emit_u64_constant(asm, dst, 0),
    }
    values.insert(result, dst);
    Ok(())
}

fn lower_extend(
    asm: &mut Arm64Assembler,
    dst: u8,
    src: u8,
    from_size: OpSize,
    to_size: OpSize,
    is_signed: bool,
) {
    if is_signed {
        match from_size {
            OpSize::I8 => asm.sxtb_x(dst, src),
            OpSize::I16 => asm.sxth_x(dst, src),
            OpSize::I32 => asm.sxtw_x(dst, src),
            OpSize::I64 => asm.mov_x(dst, src),
        }
        if to_size != OpSize::I64 {
            lower_truncate(asm, dst, dst, to_size);
        }
    } else {
        lower_truncate(asm, dst, src, from_size);
    }
}

fn lower_truncate(asm: &mut Arm64Assembler, dst: u8, src: u8, to_size: OpSize) {
    match to_size {
        OpSize::I8 => asm.uxtb_x(dst, src),
        OpSize::I16 => asm.uxth_x(dst, src),
        OpSize::I32 => asm.uxtw_x(dst, src),
        OpSize::I64 => asm.mov_x(dst, src),
    }
}

fn lower_write_flags(
    asm: &mut Arm64Assembler,
    values: &HashMap<Ref, u8>,
    flags: &mut HashSet<Ref>,
    write: &prisma_ir::WriteFlags,
    stmt: &Stmt,
) -> Result<(), LowerError> {
    let result = stmt.result.ok_or(LowerError::MissingResult("WriteFlags"))?;
    let lhs = *values
        .get(&write.lhs)
        .ok_or(LowerError::MissingValue(write.lhs))?;
    let rhs = *values
        .get(&write.rhs)
        .ok_or(LowerError::MissingValue(write.rhs))?;
    let (lhs, rhs) = align_flag_operands(asm, write.size, lhs, rhs);
    match write.op {
        BinOpKind::Sub => asm.cmp_x(lhs, rhs),
        BinOpKind::Add => asm.adds_x(ALU_FLAGS_TMP_REG, lhs, rhs),
        BinOpKind::And => asm.ands_x(ALU_FLAGS_TMP_REG, lhs, rhs),
        BinOpKind::Or => emit_logical_flags_or_xor(asm, lhs, rhs, false),
        BinOpKind::Xor => emit_logical_flags_or_xor(asm, lhs, rhs, true),
        _ => {
            return Err(LowerError::UnsupportedOp(
                "WriteFlags only supports Sub/Add/And/Or/Xor today",
            ));
        }
    }
    flags.insert(result);
    Ok(())
}

fn lower_read_flag(
    asm: &mut Arm64Assembler,
    flags: &HashSet<Ref>,
    read: &prisma_ir::ReadFlag,
    dst: u8,
) -> Result<(), LowerError> {
    if !flags.contains(&read.flags) {
        return Err(LowerError::MissingValue(read.flags));
    }
    let cc = match read.which {
        prisma_ir::FlagBit::Carry => prisma_ir::CondCode::Cc,
        prisma_ir::FlagBit::Zero => prisma_ir::CondCode::Eq,
        prisma_ir::FlagBit::Sign => prisma_ir::CondCode::Mi,
        prisma_ir::FlagBit::Overflow => prisma_ir::CondCode::Ov,
        prisma_ir::FlagBit::Parity | prisma_ir::FlagBit::Aux => {
            return Err(LowerError::UnsupportedOp(
                "ReadFlag(Parity/Aux) needs software emulation",
            ));
        }
    };
    asm.cset_x(dst, cc);
    Ok(())
}

fn lower_syscall(asm: &mut Arm64Assembler) {
    asm.nop();
}

fn lower_lzcnt(asm: &mut Arm64Assembler, dst: u8, src: u8, size: OpSize) {
    match size {
        OpSize::I64 => asm.clz_x(dst, src),
        OpSize::I32 => asm.clz_w(dst, src),
        OpSize::I16 | OpSize::I8 => {
            let shift = u16::try_from(64 - size.bit_width()).expect("small size shift fits");
            lower_truncate(asm, dst, src, size);
            asm.movz_x(FLAG_ALIGN_SHIFT_REG, shift, 0);
            asm.lsl_x(dst, dst, FLAG_ALIGN_SHIFT_REG);
            asm.clz_x(dst, dst);
            clamp_count_to_width(asm, dst, size);
        }
    }
}

fn lower_tzcnt(asm: &mut Arm64Assembler, dst: u8, src: u8, size: OpSize) {
    match size {
        OpSize::I64 => {
            asm.rbit_x(dst, src);
            asm.clz_x(dst, dst);
        }
        OpSize::I32 => {
            asm.rbit_w(dst, src);
            asm.clz_w(dst, dst);
        }
        OpSize::I16 | OpSize::I8 => {
            lower_truncate(asm, dst, src, size);
            asm.rbit_w(dst, dst);
            asm.clz_w(dst, dst);
            clamp_count_to_width(asm, dst, size);
        }
    }
}

fn lower_popcnt(asm: &mut Arm64Assembler, dst: u8, src: u8, size: OpSize) {
    let tmp = FLAG_ALIGN_LHS_REG;
    let mask = FLAG_ALIGN_RHS_REG;
    let shift = FLAG_ALIGN_SHIFT_REG;
    let mul = RSP_ADJUST_TMP_REG;

    lower_truncate(asm, dst, src, size);

    emit_u64_constant(asm, shift, 1);
    asm.lsr_x(tmp, dst, shift);
    emit_u64_constant(asm, mask, 0x5555_5555_5555_5555);
    asm.and_x(tmp, tmp, mask);
    asm.sub_x(dst, dst, tmp);

    emit_u64_constant(asm, shift, 2);
    asm.lsr_x(tmp, dst, shift);
    emit_u64_constant(asm, mask, 0x3333_3333_3333_3333);
    asm.and_x(dst, dst, mask);
    asm.and_x(tmp, tmp, mask);
    asm.add_x(dst, dst, tmp);

    emit_u64_constant(asm, shift, 4);
    asm.lsr_x(tmp, dst, shift);
    asm.add_x(dst, dst, tmp);
    emit_u64_constant(asm, mask, 0x0f0f_0f0f_0f0f_0f0f);
    asm.and_x(dst, dst, mask);

    emit_u64_constant(asm, mul, 0x0101_0101_0101_0101);
    asm.mul_x(dst, dst, mul);
    emit_u64_constant(asm, shift, 56);
    asm.lsr_x(dst, dst, shift);
}

fn clamp_count_to_width(asm: &mut Arm64Assembler, dst: u8, size: OpSize) {
    let done = asm.create_label();
    emit_u64_constant(asm, FLAG_ALIGN_SHIFT_REG, u64::from(size.bit_width()));
    asm.cmp_x(dst, FLAG_ALIGN_SHIFT_REG);
    asm.b_cond_label(prisma_ir::CondCode::Ule, done);
    asm.mov_x(dst, FLAG_ALIGN_SHIFT_REG);
    asm.bind_label(done);
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

fn lower_rsp_adjust(
    asm: &mut Arm64Assembler,
    adjust: &prisma_ir::RspAdjust,
) -> Result<(), LowerError> {
    emit_load_reg(asm, OpSize::I64, RSP_ADJUST_TMP_REG, Gpr::Rsp);
    emit_rsp_imm_add(asm, RSP_ADJUST_TMP_REG, adjust.delta_bytes)?;
    emit_store_reg(asm, OpSize::I64, RSP_ADJUST_TMP_REG, Gpr::Rsp);
    Ok(())
}

fn lower_ret_adjusted(asm: &mut Arm64Assembler, pop_bytes: u64) -> Result<(), LowerError> {
    emit_load_reg(asm, OpSize::I64, RSP_ADJUST_TMP_REG, Gpr::Rsp);
    if pop_bytes != 0 {
        let pop =
            i64::try_from(pop_bytes).map_err(|_| LowerError::ImmediateOutOfRange(pop_bytes))?;
        emit_rsp_imm_add(asm, RSP_ADJUST_TMP_REG, pop)?;
    }
    emit_store_reg(asm, OpSize::I64, RSP_ADJUST_TMP_REG, Gpr::Rsp);
    asm.ret();
    Ok(())
}

fn emit_rsp_imm_add(
    asm: &mut Arm64Assembler,
    register: u8,
    delta_bytes: i64,
) -> Result<(), LowerError> {
    match delta_bytes {
        0 => Ok(()),
        1..=4095 => {
            let imm = u16::try_from(delta_bytes).expect("small positive immediate fits");
            asm.add_x_imm(register, register, imm);
            Ok(())
        }
        -4095..=-1 => {
            let imm = delta_bytes
                .checked_neg()
                .and_then(|value| u16::try_from(value).ok())
                .ok_or_else(|| LowerError::ImmediateOutOfRange(delta_bytes.cast_unsigned()))?;
            asm.sub_x_imm(register, register, imm);
            Ok(())
        }
        _ => {
            if delta_bytes == i64::MIN {
                return Err(LowerError::ImmediateOutOfRange(delta_bytes.cast_unsigned()));
            }
            let abs = if delta_bytes.is_negative() {
                delta_bytes.unsigned_abs()
            } else {
                u64::try_from(delta_bytes).expect("non-negative delta in this branch")
            };
            emit_u64_constant(asm, RSP_ADJUST_IMM_REG, abs);
            if delta_bytes.is_negative() {
                asm.sub_x(register, register, RSP_ADJUST_IMM_REG);
            } else {
                asm.add_x(register, register, RSP_ADJUST_IMM_REG);
            }
            Ok(())
        }
    }
}

fn block_label(labels: &HashMap<u32, Label>, guest_pc: u64) -> Result<Label, LowerError> {
    let block_id = u32::try_from(guest_pc).map_err(|_| LowerError::UnsupportedOp("CondJumpRel"))?;
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
    use crate::abi;
    use crate::assembler::{
        add_x, add_x_imm, adds_x, ands_x, b, b_cond, blr_x, clz_w, clz_x, cmp_x, crc32cb, crc32ch,
        crc32cw, crc32cx, cset_x, eor_x, fence, ldr_x_unsigned, lsl_x, lsr_x, mov_x, movz_x,
        mrs_cntvct, msr_nzcv, orr_x, rbit_w, rbit_x, str_x_unsigned, sub_x_imm, sxtb_x, uxth_x,
        uxtw_x,
    };
    use prisma_ir::{
        AluFlags, BasicBlock, BinOp, Bswap, CmpFlags, Compare, CondCode, CondJump, CondJumpFlags,
        CondJumpRel, Constant, Crc32c, Fence, FenceKind, FlagBit, Gpr, Jump, LoadMem, LoadMemTSO,
        LoadReg, LoadSegBase, Lzcnt, Popcnt, Rdtsc, ReadFlag, Return, RspAdjust, SegmentReg,
        Select, Stmt, StoreMem, StoreMemTSO, StoreReg, Truncate, Tzcnt, WriteFlags,
        WriteFlagsCountZero, WriteFlagsPopcnt,
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
    fn lowers_segment_bases_from_state_frame() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::LoadSegBase(LoadSegBase {
                    seg: SegmentReg::Fs,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::LoadSegBase(LoadSegBase {
                    seg: SegmentReg::Gs,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::LoadSegBase(LoadSegBase {
                    seg: SegmentReg::Ds,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                ldr_x_unsigned(value_reg(0), abi::K_STATE_PTR_REG, FS_BASE_OFFSET),
                ldr_x_unsigned(value_reg(1), abi::K_STATE_PTR_REG, GS_BASE_OFFSET),
                movz_x(value_reg(2), 0, 0),
            ]
        );
    }

    #[test]
    fn lowers_extend_and_truncate_scalars() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0xff,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Extend(prisma_ir::Extend {
                    value: 0,
                    from_size: OpSize::I8,
                    to_size: OpSize::I64,
                    is_signed: true,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::Extend(prisma_ir::Extend {
                    value: 0,
                    from_size: OpSize::I16,
                    to_size: OpSize::I64,
                    is_signed: false,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::Truncate(Truncate {
                    value: 0,
                    to_size: OpSize::I32,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                movz_x(value_reg(0), 0xff, 0),
                sxtb_x(value_reg(1), value_reg(0)),
                uxth_x(value_reg(2), value_reg(0)),
                uxtw_x(value_reg(3), value_reg(0)),
            ]
        );
    }

    #[test]
    fn lowers_write_flags_then_read_flag() {
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
                Some(2),
                Op::WriteFlags(WriteFlags {
                    op: BinOpKind::Add,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::ReadFlag(ReadFlag {
                    flags: 2,
                    which: FlagBit::Carry,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                movz_x(value_reg(0), 7, 0),
                movz_x(value_reg(1), 3, 0),
                adds_x(ALU_FLAGS_TMP_REG, value_reg(0), value_reg(1)),
                cset_x(value_reg(3), CondCode::Cc),
            ]
        );
    }

    #[test]
    fn rejects_read_flag_parity_without_emulation() {
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
                Some(2),
                Op::WriteFlags(WriteFlags {
                    op: BinOpKind::Sub,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::ReadFlag(ReadFlag {
                    flags: 2,
                    which: FlagBit::Parity,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func),
            Err(LowerError::UnsupportedOp(
                "ReadFlag(Parity/Aux) needs software emulation"
            ))
        );
    }

    #[test]
    fn lowers_write_flags_count_zero_to_nzcv() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 64,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::WriteFlagsCountZero(WriteFlagsCountZero {
                    src: 0,
                    result: 1,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                movz_x(value_reg(0), 0, 0),
                movz_x(value_reg(1), 64, 0),
                cmp_x(value_reg(1), 31),
                cset_x(FLAG_ALIGN_LHS_REG, CondCode::Eq),
                cmp_x(value_reg(0), 31),
                cset_x(FLAG_ALIGN_RHS_REG, CondCode::Eq),
                movz_x(FLAG_ALIGN_SHIFT_REG, 30, 0),
                lsl_x(FLAG_ALIGN_LHS_REG, FLAG_ALIGN_LHS_REG, FLAG_ALIGN_SHIFT_REG),
                movz_x(FLAG_ALIGN_SHIFT_REG, 29, 0),
                lsl_x(FLAG_ALIGN_RHS_REG, FLAG_ALIGN_RHS_REG, FLAG_ALIGN_SHIFT_REG),
                crate::assembler::orr_x(FLAG_ALIGN_LHS_REG, FLAG_ALIGN_LHS_REG, FLAG_ALIGN_RHS_REG),
                msr_nzcv(FLAG_ALIGN_LHS_REG),
            ]
        );
    }

    #[test]
    fn lowers_write_flags_count_zero_truncates_narrow_values_before_compare() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x100,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 0x100,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::WriteFlagsCountZero(WriteFlagsCountZero {
                    src: 0,
                    result: 1,
                    size: OpSize::I8,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                movz_x(value_reg(0), 0x100, 0),
                movz_x(value_reg(1), 0x100, 0),
                crate::assembler::uxtb_x(FLAG_ALIGN_LHS_REG, value_reg(1)),
                cmp_x(FLAG_ALIGN_LHS_REG, 31),
                cset_x(FLAG_ALIGN_LHS_REG, CondCode::Eq),
                crate::assembler::uxtb_x(FLAG_ALIGN_RHS_REG, value_reg(0)),
                cmp_x(FLAG_ALIGN_RHS_REG, 31),
                cset_x(FLAG_ALIGN_RHS_REG, CondCode::Eq),
                movz_x(FLAG_ALIGN_SHIFT_REG, 30, 0),
                lsl_x(FLAG_ALIGN_LHS_REG, FLAG_ALIGN_LHS_REG, FLAG_ALIGN_SHIFT_REG),
                movz_x(FLAG_ALIGN_SHIFT_REG, 29, 0),
                lsl_x(FLAG_ALIGN_RHS_REG, FLAG_ALIGN_RHS_REG, FLAG_ALIGN_SHIFT_REG),
                crate::assembler::orr_x(FLAG_ALIGN_LHS_REG, FLAG_ALIGN_LHS_REG, FLAG_ALIGN_RHS_REG),
                msr_nzcv(FLAG_ALIGN_LHS_REG),
            ]
        );
    }

    #[test]
    fn lowers_write_flags_popcnt_to_zf_only_nzcv() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x8000_0000,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::WriteFlagsPopcnt(WriteFlagsPopcnt {
                    src: 0,
                    size: OpSize::I32,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                movz_x(value_reg(0), 0, 0),
                crate::assembler::movk_x(value_reg(0), 0x8000, 16),
                crate::assembler::uxtw_x(FLAG_ALIGN_LHS_REG, value_reg(0)),
                cmp_x(FLAG_ALIGN_LHS_REG, 31),
                cset_x(FLAG_ALIGN_LHS_REG, CondCode::Eq),
                movz_x(FLAG_ALIGN_SHIFT_REG, 30, 0),
                lsl_x(FLAG_ALIGN_LHS_REG, FLAG_ALIGN_LHS_REG, FLAG_ALIGN_SHIFT_REG),
                msr_nzcv(FLAG_ALIGN_LHS_REG),
            ]
        );
    }

    #[test]
    fn lowers_rdtsc_and_fence() {
        let func = function(vec![
            Stmt::new(Some(0), Op::Rdtsc(Rdtsc)),
            Stmt::new(
                None,
                Op::Fence(Fence {
                    kind: FenceKind::Mfence,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![mrs_cntvct(value_reg(0)), fence(FenceKind::Mfence)]
        );
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
    fn lowers_large_add_immediate_via_register_fallback() {
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
                    value: 0x1234,
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
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                movz_x(value_reg(0), 10, 0),
                movz_x(value_reg(1), 0x1234, 0),
                add_x(value_reg(2), value_reg(0), value_reg(1)),
            ]
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
    fn lowers_lzcnt_and_tzcnt_i64_i32() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x10,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Lzcnt(Lzcnt {
                    value: 0,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::Tzcnt(Tzcnt {
                    value: 0,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::Lzcnt(Lzcnt {
                    value: 0,
                    size: OpSize::I32,
                }),
            ),
            Stmt::new(
                Some(4),
                Op::Tzcnt(Tzcnt {
                    value: 0,
                    size: OpSize::I32,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                movz_x(value_reg(0), 0x10, 0),
                clz_x(value_reg(1), value_reg(0)),
                rbit_x(value_reg(2), value_reg(0)),
                clz_x(value_reg(2), value_reg(2)),
                clz_w(value_reg(3), value_reg(0)),
                rbit_w(value_reg(4), value_reg(0)),
                clz_w(value_reg(4), value_reg(4)),
            ]
        );
    }

    #[test]
    fn lowers_popcnt_i64_scalar_sequence() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x10,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Popcnt(Popcnt {
                    value: 0,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                movz_x(value_reg(0), 0x10, 0),
                mov_x(value_reg(1), value_reg(0)),
                movz_x(FLAG_ALIGN_SHIFT_REG, 1, 0),
                lsr_x(FLAG_ALIGN_LHS_REG, value_reg(1), FLAG_ALIGN_SHIFT_REG),
                movz_x(FLAG_ALIGN_RHS_REG, 0x5555, 0),
                crate::assembler::movk_x(FLAG_ALIGN_RHS_REG, 0x5555, 16),
                crate::assembler::movk_x(FLAG_ALIGN_RHS_REG, 0x5555, 32),
                crate::assembler::movk_x(FLAG_ALIGN_RHS_REG, 0x5555, 48),
                crate::assembler::and_x(FLAG_ALIGN_LHS_REG, FLAG_ALIGN_LHS_REG, FLAG_ALIGN_RHS_REG),
                crate::assembler::sub_x(value_reg(1), value_reg(1), FLAG_ALIGN_LHS_REG),
                movz_x(FLAG_ALIGN_SHIFT_REG, 2, 0),
                lsr_x(FLAG_ALIGN_LHS_REG, value_reg(1), FLAG_ALIGN_SHIFT_REG),
                movz_x(FLAG_ALIGN_RHS_REG, 0x3333, 0),
                crate::assembler::movk_x(FLAG_ALIGN_RHS_REG, 0x3333, 16),
                crate::assembler::movk_x(FLAG_ALIGN_RHS_REG, 0x3333, 32),
                crate::assembler::movk_x(FLAG_ALIGN_RHS_REG, 0x3333, 48),
                crate::assembler::and_x(value_reg(1), value_reg(1), FLAG_ALIGN_RHS_REG),
                crate::assembler::and_x(FLAG_ALIGN_LHS_REG, FLAG_ALIGN_LHS_REG, FLAG_ALIGN_RHS_REG),
                crate::assembler::add_x(value_reg(1), value_reg(1), FLAG_ALIGN_LHS_REG),
                movz_x(FLAG_ALIGN_SHIFT_REG, 4, 0),
                lsr_x(FLAG_ALIGN_LHS_REG, value_reg(1), FLAG_ALIGN_SHIFT_REG),
                crate::assembler::add_x(value_reg(1), value_reg(1), FLAG_ALIGN_LHS_REG),
                movz_x(FLAG_ALIGN_RHS_REG, 0x0f0f, 0),
                crate::assembler::movk_x(FLAG_ALIGN_RHS_REG, 0x0f0f, 16),
                crate::assembler::movk_x(FLAG_ALIGN_RHS_REG, 0x0f0f, 32),
                crate::assembler::movk_x(FLAG_ALIGN_RHS_REG, 0x0f0f, 48),
                crate::assembler::and_x(value_reg(1), value_reg(1), FLAG_ALIGN_RHS_REG),
                movz_x(RSP_ADJUST_TMP_REG, 0x0101, 0),
                crate::assembler::movk_x(RSP_ADJUST_TMP_REG, 0x0101, 16),
                crate::assembler::movk_x(RSP_ADJUST_TMP_REG, 0x0101, 32),
                crate::assembler::movk_x(RSP_ADJUST_TMP_REG, 0x0101, 48),
                crate::assembler::mul_x(value_reg(1), value_reg(1), RSP_ADJUST_TMP_REG),
                movz_x(FLAG_ALIGN_SHIFT_REG, 56, 0),
                lsr_x(value_reg(1), value_reg(1), FLAG_ALIGN_SHIFT_REG),
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
    fn lowers_tso_memory_ops_with_full_barriers() {
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
                Op::StoreMemTSO(StoreMemTSO {
                    addr: 0,
                    value: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::LoadMemTSO(LoadMemTSO {
                    addr: 0,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                movz_x(value_reg(0), 0x1000, 0),
                movz_x(value_reg(1), 0x2a, 0),
                fence(FenceKind::Mfence),
                str_x_unsigned(value_reg(1), value_reg(0), 0),
                fence(FenceKind::Mfence),
                ldr_x_unsigned(value_reg(2), value_reg(0), 0),
                fence(FenceKind::Mfence),
            ]
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
        let shift = u16::try_from(64 - OpSize::I8.bit_width()).expect("I8 fits in u16 shift");
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

        assert_eq!(Lowerer::new().lower_function(&func).unwrap(), {
            let lhs_reg = value_reg(0);
            let rhs_reg = value_reg(1);
            let result_reg = value_reg(2);
            let shift = u16::try_from(64 - OpSize::I16.bit_width()).expect("I16 fits in u16 shift");
            vec![
                movz_x(lhs_reg, 5, 0),
                movz_x(rhs_reg, 5, 0),
                movz_x(FLAG_ALIGN_SHIFT_REG, shift, 0),
                lsl_x(FLAG_ALIGN_LHS_REG, lhs_reg, FLAG_ALIGN_SHIFT_REG),
                lsl_x(FLAG_ALIGN_RHS_REG, rhs_reg, FLAG_ALIGN_SHIFT_REG),
                cmp_x(FLAG_ALIGN_LHS_REG, FLAG_ALIGN_RHS_REG),
                cset_x(result_reg, CondCode::Eq),
            ]
        });
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
        let shift = u16::try_from(64 - OpSize::I32.bit_width()).expect("I32 fits in u16 shift");
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
            vec![0x5400_0040, 0x1400_0002, 0xD65F_03C0, 0xD65F_03C0]
        );
    }

    #[test]
    fn lowers_select_false_path_from_compare() {
        let func = function(vec![
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
                Some(2),
                Op::Compare(Compare {
                    cc: CondCode::Eq,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::Select(Select {
                    cc: CondCode::Eq,
                    true_value: 0,
                    false_value: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(None, Op::Return(Return)),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                movz_x(value_reg(0), 1, 0),
                movz_x(value_reg(1), 2, 0),
                cmp_x(value_reg(0), value_reg(1)),
                cset_x(value_reg(2), CondCode::Eq),
                b_cond(CondCode::Eq, 12),
                mov_x(value_reg(3), value_reg(1)),
                b(8),
                mov_x(value_reg(3), value_reg(0)),
                0xD65F_03C0,
            ]
        );
    }

    #[test]
    fn lowers_select_true_path_from_compare() {
        let func = function(vec![
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
                Some(2),
                Op::Compare(Compare {
                    cc: CondCode::Eq,
                    lhs: 0,
                    rhs: 0,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::Select(Select {
                    cc: CondCode::Eq,
                    true_value: 0,
                    false_value: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(None, Op::Return(Return)),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                movz_x(value_reg(0), 1, 0),
                movz_x(value_reg(1), 2, 0),
                cmp_x(value_reg(0), value_reg(0)),
                cset_x(value_reg(2), CondCode::Eq),
                b_cond(CondCode::Eq, 12),
                mov_x(value_reg(3), value_reg(1)),
                b(8),
                mov_x(value_reg(3), value_reg(0)),
                0xD65F_03C0,
            ]
        );
    }

    #[test]
    fn rejects_select_missing_result() {
        let func = function(vec![
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
                Op::Select(Select {
                    cc: CondCode::Eq,
                    true_value: 0,
                    false_value: 1,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func),
            Err(LowerError::MissingResult("Select"))
        );
    }

    #[test]
    fn rejects_select_missing_true_value() {
        let func = function(vec![
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
                Some(2),
                Op::Compare(Compare {
                    cc: CondCode::Eq,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::Select(Select {
                    cc: CondCode::Eq,
                    true_value: 99,
                    false_value: 1,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func),
            Err(LowerError::MissingValue(99))
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
    fn lowers_alu_flags_sub_add_and() {
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
                Op::AluFlags(AluFlags {
                    op: BinOpKind::Sub,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::AluFlags(AluFlags {
                    op: BinOpKind::Add,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::AluFlags(AluFlags {
                    op: BinOpKind::And,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                0xD280_00E9,
                0xD280_006A,
                0xEB0A_013F,
                adds_x(ALU_FLAGS_TMP_REG, value_reg(0), value_reg(1)),
                ands_x(ALU_FLAGS_TMP_REG, value_reg(0), value_reg(1)),
            ]
        );
    }

    #[test]
    fn rejects_unsupported_alu_flags_op() {
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
                Op::AluFlags(AluFlags {
                    op: BinOpKind::Mul,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func),
            Err(LowerError::UnsupportedOp(
                "AluFlags only supports Sub/Add/And/Or/Xor today"
            ))
        );
    }

    #[test]
    fn lowers_alu_flags_or_xor() {
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
                Op::AluFlags(AluFlags {
                    op: BinOpKind::Or,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::AluFlags(AluFlags {
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
                0xD280_00E9,
                0xD280_006A,
                orr_x(ALU_FLAGS_TMP_REG, value_reg(0), value_reg(1)),
                ands_x(ALU_FLAGS_TMP_REG, ALU_FLAGS_TMP_REG, ALU_FLAGS_TMP_REG),
                eor_x(ALU_FLAGS_TMP_REG, value_reg(0), value_reg(1)),
                ands_x(ALU_FLAGS_TMP_REG, ALU_FLAGS_TMP_REG, ALU_FLAGS_TMP_REG),
            ]
        );
    }

    #[test]
    fn lowers_bswap_scalar_sizes() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Bswap(Bswap {
                    value: 0,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::Constant(Constant {
                    value: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::Bswap(Bswap {
                    value: 2,
                    size: OpSize::I32,
                }),
            ),
            Stmt::new(
                Some(4),
                Op::Constant(Constant {
                    value: 0x1122,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(5),
                Op::Bswap(Bswap {
                    value: 4,
                    size: OpSize::I16,
                }),
            ),
            Stmt::new(
                Some(6),
                Op::Constant(Constant {
                    value: 0x7f,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(7),
                Op::Bswap(Bswap {
                    value: 6,
                    size: OpSize::I8,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                crate::assembler::movz_x(value_reg(0), 1, 0),
                crate::assembler::rev_x(value_reg(1), value_reg(0)),
                crate::assembler::movz_x(value_reg(2), 1, 0),
                crate::assembler::rev_w(value_reg(3), value_reg(2)),
                crate::assembler::movz_x(value_reg(4), 0x1122, 0),
                crate::assembler::rev_w(value_reg(5), value_reg(4)),
                crate::assembler::movz_x(FLAG_ALIGN_SHIFT_REG, 16, 0),
                crate::assembler::lsr_x(value_reg(5), value_reg(5), FLAG_ALIGN_SHIFT_REG),
                crate::assembler::movz_x(value_reg(6), 0x7f, 0),
                crate::assembler::uxtb_x(value_reg(7), value_reg(6)),
            ]
        );
    }

    #[test]
    fn lowers_crc32c_scalar_sizes() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x1234,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 0xabcd,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::Crc32c(Crc32c {
                    crc: 0,
                    data: 1,
                    data_size: OpSize::I8,
                }),
            ),
            Stmt::new(
                Some(3),
                Op::Crc32c(Crc32c {
                    crc: 0,
                    data: 1,
                    data_size: OpSize::I16,
                }),
            ),
            Stmt::new(
                Some(4),
                Op::Crc32c(Crc32c {
                    crc: 0,
                    data: 1,
                    data_size: OpSize::I32,
                }),
            ),
            Stmt::new(
                Some(5),
                Op::Crc32c(Crc32c {
                    crc: 0,
                    data: 1,
                    data_size: OpSize::I64,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                crate::assembler::movz_x(value_reg(0), 0x1234, 0),
                crate::assembler::movz_x(value_reg(1), 0xabcd, 0),
                crc32cb(value_reg(2), value_reg(0), value_reg(1)),
                crc32ch(value_reg(3), value_reg(0), value_reg(1)),
                crc32cw(value_reg(4), value_reg(0), value_reg(1)),
                crc32cx(value_reg(5), value_reg(0), value_reg(1)),
            ]
        );
    }

    #[test]
    fn lowers_i8_alu_flags_are_aligned_for_flags_ops() {
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
                Op::AluFlags(AluFlags {
                    op: BinOpKind::Add,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I8,
                }),
            ),
            Stmt::new(
                None,
                Op::AluFlags(AluFlags {
                    op: BinOpKind::And,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I8,
                }),
            ),
            Stmt::new(
                None,
                Op::AluFlags(AluFlags {
                    op: BinOpKind::Sub,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I8,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                0xD280_00E9,
                0xD280_006A,
                movz_x(FLAG_ALIGN_SHIFT_REG, 56, 0),
                lsl_x(FLAG_ALIGN_LHS_REG, value_reg(0), FLAG_ALIGN_SHIFT_REG),
                lsl_x(FLAG_ALIGN_RHS_REG, value_reg(1), FLAG_ALIGN_SHIFT_REG),
                adds_x(ALU_FLAGS_TMP_REG, FLAG_ALIGN_LHS_REG, FLAG_ALIGN_RHS_REG),
                movz_x(FLAG_ALIGN_SHIFT_REG, 56, 0),
                lsl_x(FLAG_ALIGN_LHS_REG, value_reg(0), FLAG_ALIGN_SHIFT_REG),
                lsl_x(FLAG_ALIGN_RHS_REG, value_reg(1), FLAG_ALIGN_SHIFT_REG),
                ands_x(ALU_FLAGS_TMP_REG, FLAG_ALIGN_LHS_REG, FLAG_ALIGN_RHS_REG),
                movz_x(FLAG_ALIGN_SHIFT_REG, 56, 0),
                lsl_x(FLAG_ALIGN_LHS_REG, value_reg(0), FLAG_ALIGN_SHIFT_REG),
                lsl_x(FLAG_ALIGN_RHS_REG, value_reg(1), FLAG_ALIGN_SHIFT_REG),
                cmp_x(FLAG_ALIGN_LHS_REG, FLAG_ALIGN_RHS_REG),
            ]
        );
    }

    #[test]
    fn lowers_i32_alu_flags_are_aligned_for_flags_ops() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x1234,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 0x5678,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::AluFlags(AluFlags {
                    op: BinOpKind::Sub,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I32,
                }),
            ),
            Stmt::new(
                None,
                Op::AluFlags(AluFlags {
                    op: BinOpKind::Add,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I32,
                }),
            ),
            Stmt::new(
                None,
                Op::AluFlags(AluFlags {
                    op: BinOpKind::And,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I32,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                movz_x(value_reg(0), 0x1234, 0),
                movz_x(value_reg(1), 0x5678, 0),
                movz_x(FLAG_ALIGN_SHIFT_REG, 32, 0),
                lsl_x(FLAG_ALIGN_LHS_REG, value_reg(0), FLAG_ALIGN_SHIFT_REG),
                lsl_x(FLAG_ALIGN_RHS_REG, value_reg(1), FLAG_ALIGN_SHIFT_REG),
                cmp_x(FLAG_ALIGN_LHS_REG, FLAG_ALIGN_RHS_REG),
                movz_x(FLAG_ALIGN_SHIFT_REG, 32, 0),
                lsl_x(FLAG_ALIGN_LHS_REG, value_reg(0), FLAG_ALIGN_SHIFT_REG),
                lsl_x(FLAG_ALIGN_RHS_REG, value_reg(1), FLAG_ALIGN_SHIFT_REG),
                adds_x(ALU_FLAGS_TMP_REG, FLAG_ALIGN_LHS_REG, FLAG_ALIGN_RHS_REG),
                movz_x(FLAG_ALIGN_SHIFT_REG, 32, 0),
                lsl_x(FLAG_ALIGN_LHS_REG, value_reg(0), FLAG_ALIGN_SHIFT_REG),
                lsl_x(FLAG_ALIGN_RHS_REG, value_reg(1), FLAG_ALIGN_SHIFT_REG),
                ands_x(ALU_FLAGS_TMP_REG, FLAG_ALIGN_LHS_REG, FLAG_ALIGN_RHS_REG),
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

    #[test]
    fn lowers_jump_rel() {
        let func = function_with_blocks(
            vec![
                BasicBlock {
                    id: 0,
                    stmts: vec![Stmt::new(
                        None,
                        Op::JumpRel(prisma_ir::JumpRel {
                            target_guest_pc: 0x1000,
                        }),
                    )],
                },
                BasicBlock {
                    id: 0x1000,
                    stmts: Vec::new(),
                },
            ],
            0,
        );

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![0x1400_0001]
        );
    }

    #[test]
    fn lowers_call_rel_as_tail_jump() {
        let func = function_with_blocks(
            vec![
                BasicBlock {
                    id: 0,
                    stmts: vec![Stmt::new(
                        None,
                        Op::CallRel(prisma_ir::CallRel {
                            target_guest_pc: 0x1000,
                            return_guest_pc: 0x1005,
                        }),
                    )],
                },
                BasicBlock {
                    id: 0x1000,
                    stmts: Vec::new(),
                },
                BasicBlock {
                    id: 0x1005,
                    stmts: vec![Stmt::new(None, Op::Return(Return))],
                },
            ],
            0,
        );

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![0x1400_0001, 0xD65F_03C0]
        );
    }

    #[test]
    fn lowers_call_reg() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::LoadReg(LoadReg {
                    reg: Gpr::Rax,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::CallReg(prisma_ir::CallReg {
                    target: 0,
                    return_guest_pc: 0x1234,
                }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                ldr_x_unsigned(
                    value_reg(0),
                    abi::K_STATE_PTR_REG,
                    gpr_offset_bytes(Gpr::Rax)
                ),
                blr_x(value_reg(0)),
            ]
        );
    }

    #[test]
    fn lowers_rsp_adjust_push_pop() {
        let func = function(vec![
            Stmt::new(None, Op::RspAdjust(RspAdjust { delta_bytes: -8 })),
            Stmt::new(None, Op::RspAdjust(RspAdjust { delta_bytes: 16 })),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                ldr_x_unsigned(
                    RSP_ADJUST_TMP_REG,
                    abi::K_STATE_PTR_REG,
                    gpr_offset_bytes(Gpr::Rsp)
                ),
                sub_x_imm(RSP_ADJUST_TMP_REG, RSP_ADJUST_TMP_REG, 8),
                str_x_unsigned(
                    RSP_ADJUST_TMP_REG,
                    abi::K_STATE_PTR_REG,
                    gpr_offset_bytes(Gpr::Rsp)
                ),
                ldr_x_unsigned(
                    RSP_ADJUST_TMP_REG,
                    abi::K_STATE_PTR_REG,
                    gpr_offset_bytes(Gpr::Rsp)
                ),
                add_x_imm(RSP_ADJUST_TMP_REG, RSP_ADJUST_TMP_REG, 16),
                str_x_unsigned(
                    RSP_ADJUST_TMP_REG,
                    abi::K_STATE_PTR_REG,
                    gpr_offset_bytes(Gpr::Rsp)
                ),
            ]
        );
    }

    #[test]
    fn lowers_ret_adjusted_to_return() {
        let func = function(vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 1,
                    size: OpSize::I64,
                }),
            ),
            Stmt::new(
                None,
                Op::RetAdjusted(prisma_ir::RetAdjusted { pop_bytes: 16 }),
            ),
        ]);

        assert_eq!(
            Lowerer::new().lower_function(&func).unwrap(),
            vec![
                0xD280_0029,
                ldr_x_unsigned(
                    RSP_ADJUST_TMP_REG,
                    abi::K_STATE_PTR_REG,
                    gpr_offset_bytes(Gpr::Rsp)
                ),
                add_x_imm(RSP_ADJUST_TMP_REG, RSP_ADJUST_TMP_REG, 16),
                str_x_unsigned(
                    RSP_ADJUST_TMP_REG,
                    abi::K_STATE_PTR_REG,
                    gpr_offset_bytes(Gpr::Rsp)
                ),
                0xD65F_03C0
            ]
        );
    }
}
