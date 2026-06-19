use prisma_ir::*;

use crate::modrm::{self, ModRm};
use crate::prefixes::{operand_size, PrefixSet, RexPrefix};
use crate::tables;

/// Result of decoding a single x86 instruction.
#[derive(Debug, Clone, PartialEq)]
pub struct Decoded {
    pub stmts: Vec<Stmt>,
    pub bytes_consumed: usize,
}

/// Decode one x86 instruction at `offset`.
pub fn decode_one(bytes: &[u8], offset: usize) -> Result<Decoded, crate::DecodeError> {
    decode_one_at(bytes, offset, 0)
}

/// Decode one x86 instruction at `offset`, using `instruction_guest_pc` for
/// address forms whose semantics depend on the instruction pointer.
pub fn decode_one_at(
    bytes: &[u8],
    offset: usize,
    instruction_guest_pc: u64,
) -> Result<Decoded, crate::DecodeError> {
    let (prefixes, cursor) = crate::prefixes::parse_prefixes(bytes, offset);
    let opcode = *bytes.get(cursor).ok_or(crate::DecodeError::Truncated)?;
    let opcode_guest_pc = instruction_guest_pc.wrapping_add((cursor - offset) as u64);
    let mut stmts = Vec::new();

    // Dispatch based on opcode
    let consumed = match tables::classify_one_byte(opcode) {
        tables::OneByteOpcode::MovRmR8 => {
            decode_mov_rm_r_with_size(&prefixes, bytes, cursor, &mut stmts, OpSize::I8)?
        }
        tables::OneByteOpcode::MovRmR => decode_mov_rm_r(&prefixes, bytes, cursor, &mut stmts)?,
        tables::OneByteOpcode::MovR8Rm => {
            decode_mov_r_rm_with_size(&prefixes, bytes, cursor, &mut stmts, OpSize::I8)?
        }
        tables::OneByteOpcode::MovRRm => decode_mov_r_rm(&prefixes, bytes, cursor, &mut stmts)?,
        tables::OneByteOpcode::PopRm => decode_pop_rm(&prefixes, bytes, cursor, &mut stmts)?,
        tables::OneByteOpcode::Lea => decode_lea(&prefixes, bytes, cursor, &mut stmts)?,
        tables::OneByteOpcode::MovMoffsToAcc => {
            decode_mov_moffs(&prefixes, opcode, bytes, cursor, &mut stmts)?
        }
        tables::OneByteOpcode::MovAccToMoffs => {
            decode_mov_moffs(&prefixes, opcode, bytes, cursor, &mut stmts)?
        }
        tables::OneByteOpcode::Movsb => decode_movs(&prefixes, opcode, &mut stmts)?,
        tables::OneByteOpcode::Cmpsb => decode_cmps(&prefixes, opcode, &mut stmts)?,
        tables::OneByteOpcode::Stosb => decode_stos(&prefixes, opcode, &mut stmts)?,
        tables::OneByteOpcode::Lodsb => decode_lods(&prefixes, opcode, &mut stmts)?,
        tables::OneByteOpcode::Scasb => decode_scas(&prefixes, opcode, &mut stmts)?,
        tables::OneByteOpcode::MovR8I8 => {
            decode_mov_r8_i8(&prefixes, opcode, bytes, cursor, &mut stmts)?
        }
        tables::OneByteOpcode::MovRI => {
            decode_mov_ri(&prefixes, opcode, bytes, cursor, &mut stmts)?
        }
        tables::OneByteOpcode::AddRmR8 => decode_binop_rm_r_with_size(
            BinOpKind::Add,
            &prefixes,
            bytes,
            cursor,
            &mut stmts,
            OpSize::I8,
        )?,
        tables::OneByteOpcode::AddRmR => {
            decode_binop_rm_r(BinOpKind::Add, &prefixes, bytes, cursor, &mut stmts)?
        }
        tables::OneByteOpcode::AddR8Rm => decode_binop_r_rm_with_size(
            BinOpKind::Add,
            &prefixes,
            bytes,
            cursor,
            &mut stmts,
            OpSize::I8,
        )?,
        tables::OneByteOpcode::AddRRm => {
            decode_binop_r_rm(BinOpKind::Add, &prefixes, bytes, cursor, &mut stmts)?
        }
        tables::OneByteOpcode::AccImm => {
            decode_acc_imm(&prefixes, opcode, bytes, cursor, &mut stmts)?
        }
        tables::OneByteOpcode::AluRmImm => decode_alu_rm_imm(
            &prefixes,
            opcode,
            bytes,
            cursor,
            opcode_guest_pc,
            &mut stmts,
        )?,
        tables::OneByteOpcode::AluRmImm8 => decode_alu_rm_imm(
            &prefixes,
            opcode,
            bytes,
            cursor,
            opcode_guest_pc,
            &mut stmts,
        )?,
        tables::OneByteOpcode::Group2 => {
            decode_group2(&prefixes, opcode, bytes, cursor, &mut stmts)?
        }
        tables::OneByteOpcode::SubRmR8 => decode_binop_rm_r_with_size(
            BinOpKind::Sub,
            &prefixes,
            bytes,
            cursor,
            &mut stmts,
            OpSize::I8,
        )?,
        tables::OneByteOpcode::SubRmR => {
            decode_binop_rm_r(BinOpKind::Sub, &prefixes, bytes, cursor, &mut stmts)?
        }
        tables::OneByteOpcode::SubR8Rm => decode_binop_r_rm_with_size(
            BinOpKind::Sub,
            &prefixes,
            bytes,
            cursor,
            &mut stmts,
            OpSize::I8,
        )?,
        tables::OneByteOpcode::SubRRm => {
            decode_binop_r_rm(BinOpKind::Sub, &prefixes, bytes, cursor, &mut stmts)?
        }
        tables::OneByteOpcode::AndRmR8 => decode_binop_rm_r_with_size(
            BinOpKind::And,
            &prefixes,
            bytes,
            cursor,
            &mut stmts,
            OpSize::I8,
        )?,
        tables::OneByteOpcode::AndRmR => {
            decode_binop_rm_r(BinOpKind::And, &prefixes, bytes, cursor, &mut stmts)?
        }
        tables::OneByteOpcode::AndR8Rm => decode_binop_r_rm_with_size(
            BinOpKind::And,
            &prefixes,
            bytes,
            cursor,
            &mut stmts,
            OpSize::I8,
        )?,
        tables::OneByteOpcode::AndRRm => {
            decode_binop_r_rm(BinOpKind::And, &prefixes, bytes, cursor, &mut stmts)?
        }
        tables::OneByteOpcode::OrRmR8 => decode_binop_rm_r_with_size(
            BinOpKind::Or,
            &prefixes,
            bytes,
            cursor,
            &mut stmts,
            OpSize::I8,
        )?,
        tables::OneByteOpcode::OrRmR => {
            decode_binop_rm_r(BinOpKind::Or, &prefixes, bytes, cursor, &mut stmts)?
        }
        tables::OneByteOpcode::OrR8Rm => decode_binop_r_rm_with_size(
            BinOpKind::Or,
            &prefixes,
            bytes,
            cursor,
            &mut stmts,
            OpSize::I8,
        )?,
        tables::OneByteOpcode::OrRRm => {
            decode_binop_r_rm(BinOpKind::Or, &prefixes, bytes, cursor, &mut stmts)?
        }
        tables::OneByteOpcode::XorRmR8 => decode_binop_rm_r_with_size(
            BinOpKind::Xor,
            &prefixes,
            bytes,
            cursor,
            &mut stmts,
            OpSize::I8,
        )?,
        tables::OneByteOpcode::XorRmR => {
            decode_binop_rm_r(BinOpKind::Xor, &prefixes, bytes, cursor, &mut stmts)?
        }
        tables::OneByteOpcode::XorR8Rm => decode_binop_r_rm_with_size(
            BinOpKind::Xor,
            &prefixes,
            bytes,
            cursor,
            &mut stmts,
            OpSize::I8,
        )?,
        tables::OneByteOpcode::XorRRm => {
            decode_binop_r_rm(BinOpKind::Xor, &prefixes, bytes, cursor, &mut stmts)?
        }
        tables::OneByteOpcode::CmpRmR8 => {
            decode_cmp_rm_r_with_size(&prefixes, bytes, cursor, &mut stmts, OpSize::I8)?
        }
        tables::OneByteOpcode::CmpRmR => decode_cmp_rm_r(&prefixes, bytes, cursor, &mut stmts)?,
        tables::OneByteOpcode::CmpR8Rm => {
            decode_cmp_r_rm_with_size(&prefixes, bytes, cursor, &mut stmts, OpSize::I8)?
        }
        tables::OneByteOpcode::CmpRRm => decode_cmp_r_rm_with_size(
            &prefixes,
            bytes,
            cursor,
            &mut stmts,
            operand_size(&prefixes, true),
        )?,
        tables::OneByteOpcode::TestRmR8 => {
            decode_test_rm_r_with_size(&prefixes, bytes, cursor, &mut stmts, OpSize::I8)?
        }
        tables::OneByteOpcode::TestRmR => decode_test_rm_r(&prefixes, bytes, cursor, &mut stmts)?,
        tables::OneByteOpcode::LegacyInvalidTrap => {
            decode_one_byte_trap(&prefixes, opcode, TrapKind::Sigill, &mut stmts)?
        }
        tables::OneByteOpcode::LegacyInvalidImm8 => {
            decode_legacy_invalid_imm8(&prefixes, opcode, bytes, cursor, &mut stmts)?
        }
        tables::OneByteOpcode::Movsxd => decode_movsxd(&prefixes, bytes, cursor, &mut stmts)?,
        tables::OneByteOpcode::ImulRmImm => {
            decode_imul_imm(&prefixes, 0x69, bytes, cursor, &mut stmts)?
        }
        tables::OneByteOpcode::ImulRmImm8 => {
            decode_imul_imm(&prefixes, 0x6B, bytes, cursor, &mut stmts)?
        }
        tables::OneByteOpcode::MovRmImm => {
            decode_mov_rm_imm(&prefixes, opcode, bytes, cursor, &mut stmts)?
        }
        tables::OneByteOpcode::CallRel32 => {
            if prefixes.rex.present || prefixes.lock || prefixes.segment.is_some() {
                return Err(crate::DecodeError::UnsupportedOpcode(opcode));
            }
            if prefixes.operand_override {
                return Err(crate::DecodeError::UnsupportedOpcode(opcode));
            }
            decode_call_rel32(&prefixes, bytes, cursor, opcode_guest_pc, &mut stmts)?
        }
        tables::OneByteOpcode::JumpRel8 => {
            if prefixes.rex.present || prefixes.lock || prefixes.segment.is_some() {
                return Err(crate::DecodeError::UnsupportedOpcode(opcode));
            }
            if prefixes.operand_override {
                return Err(crate::DecodeError::UnsupportedOpcode(opcode));
            }
            decode_jump_rel8(&prefixes, bytes, cursor, opcode_guest_pc, &mut stmts)?
        }
        tables::OneByteOpcode::JrcxzRel8 => {
            decode_jrcxz_rel8(&prefixes, bytes, cursor, opcode_guest_pc, &mut stmts)?
        }
        tables::OneByteOpcode::LoopRel8 => {
            decode_loop_rel8(&prefixes, bytes, cursor, opcode_guest_pc, &mut stmts)?
        }
        tables::OneByteOpcode::IoTrapImm8 => {
            decode_io_trap_imm8(&prefixes, opcode, bytes, cursor, &mut stmts)?
        }
        tables::OneByteOpcode::JumpRel32 => {
            if prefixes.rex.present || prefixes.lock || prefixes.segment.is_some() {
                return Err(crate::DecodeError::UnsupportedOpcode(opcode));
            }
            if prefixes.operand_override {
                return Err(crate::DecodeError::UnsupportedOpcode(opcode));
            }
            decode_jump_rel32(&prefixes, bytes, cursor, opcode_guest_pc, &mut stmts)?
        }
        tables::OneByteOpcode::IoTrapDx => {
            decode_one_byte_trap(&prefixes, opcode, TrapKind::Sigill, &mut stmts)?
        }
        tables::OneByteOpcode::Nop => {
            if prefixes.rep == Some(0xF3)
                && !prefixes.rex.present
                && !prefixes.lock
                && prefixes.segment.is_none()
                && !prefixes.operand_override
                && !prefixes.addr_override
            {
                1
            } else if prefixes.rep.is_some() || prefixes.rex.w {
                return Err(crate::DecodeError::UnsupportedOpcode(opcode));
            } else {
                1
            }
        }
        tables::OneByteOpcode::Fwait => {
            if prefixes.rex.present
                || prefixes.lock
                || prefixes.segment.is_some()
                || prefixes.rep.is_some()
                || prefixes.operand_override
                || prefixes.addr_override
            {
                return Err(crate::DecodeError::UnsupportedOpcode(opcode));
            }
            1
        }
        tables::OneByteOpcode::Cld => {
            if prefixes.rex.present
                || prefixes.lock
                || prefixes.segment.is_some()
                || prefixes.rep.is_some()
                || prefixes.operand_override
                || prefixes.addr_override
            {
                return Err(crate::DecodeError::UnsupportedOpcode(opcode));
            }
            1
        }
        tables::OneByteOpcode::XchgAcc => decode_xchg_acc(&prefixes, opcode, &mut stmts),
        tables::OneByteOpcode::SignExtendAcc => decode_sign_extend_acc(&prefixes, &mut stmts),
        tables::OneByteOpcode::SignExtendAccToDx => {
            decode_sign_extend_acc_to_dx(&prefixes, &mut stmts)
        }
        tables::OneByteOpcode::Xlat => decode_xlat(&prefixes, &mut stmts)?,
        tables::OneByteOpcode::X87D9 => decode_x87_d9(&prefixes, opcode, bytes, cursor)?,
        tables::OneByteOpcode::X87DB => decode_x87_db(&prefixes, opcode, bytes, cursor)?,
        tables::OneByteOpcode::CondJumpRel8 => {
            if prefixes.rex.present || prefixes.operand_override {
                return Err(crate::DecodeError::UnsupportedOpcode(opcode));
            }
            decode_cond_jump_rel8(&prefixes, bytes, cursor, opcode_guest_pc, &mut stmts)?
        }
        tables::OneByteOpcode::Xchg => decode_xchg(&prefixes, opcode, bytes, cursor, &mut stmts)?,
        tables::OneByteOpcode::Ret => decode_ret(&prefixes, bytes, cursor, &mut stmts)?,
        tables::OneByteOpcode::RetImm16 => decode_ret_imm16(&prefixes, bytes, cursor, &mut stmts)?,
        tables::OneByteOpcode::IretTrap => {
            decode_one_byte_trap(&prefixes, opcode, TrapKind::Sigill, &mut stmts)?
        }
        tables::OneByteOpcode::Enter => decode_enter(&prefixes, bytes, cursor, &mut stmts)?,
        tables::OneByteOpcode::Leave => decode_leave(&prefixes, &mut stmts)?,
        tables::OneByteOpcode::Int3 => {
            decode_one_byte_trap(&prefixes, opcode, TrapKind::Sigtrap, &mut stmts)?
        }
        tables::OneByteOpcode::IntImm8 => {
            decode_int_imm8(&prefixes, opcode, bytes, cursor, &mut stmts)?
        }
        tables::OneByteOpcode::Group3 => {
            decode_group3(&prefixes, opcode, bytes, cursor, &mut stmts)?
        }
        tables::OneByteOpcode::Group4 => decode_group4(&prefixes, bytes, cursor, &mut stmts)?,
        tables::OneByteOpcode::Group5 => {
            decode_group5(&prefixes, bytes, cursor, opcode_guest_pc, &mut stmts)?
        }
        tables::OneByteOpcode::PushReg => decode_push_r(&prefixes, opcode, &mut stmts),
        tables::OneByteOpcode::PushImm => {
            decode_push_imm(&prefixes, opcode, bytes, cursor, &mut stmts)?
        }
        tables::OneByteOpcode::PopReg => decode_pop_r(&prefixes, opcode, &mut stmts),
        tables::OneByteOpcode::Icebp => {
            decode_one_byte_trap(&prefixes, opcode, TrapKind::Sigtrap, &mut stmts)?
        }
        tables::OneByteOpcode::Hlt => {
            decode_one_byte_trap(&prefixes, opcode, TrapKind::Sigill, &mut stmts)?
        }
        tables::OneByteOpcode::PrivilegedTrap => {
            decode_one_byte_trap(&prefixes, opcode, TrapKind::Sigill, &mut stmts)?
        }
        tables::OneByteOpcode::TwoBytePrefix => {
            decode_two_byte(&prefixes, bytes, cursor, opcode_guest_pc, &mut stmts)?
        }
        tables::OneByteOpcode::Unsupported => {
            return Err(crate::DecodeError::UnsupportedOpcode(opcode))
        }
    };

    let total_bytes = cursor - offset + consumed;
    Ok(Decoded {
        stmts,
        bytes_consumed: total_bytes,
    })
}

// ---------------------------------------------------------------------------
// Two-byte opcodes (0F xx)
// ---------------------------------------------------------------------------

fn decode_two_byte(
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    opcode_guest_pc: u64,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let op2 = *bytes.get(cursor + 1).ok_or(crate::DecodeError::Truncated)?;
    let start = cursor + 1;
    let consumed_after_escape = match tables::classify_two_byte(op2) {
        tables::TwoByteOpcode::DescriptorGroup => {
            decode_descriptor_group(prefixes, op2, bytes, start, stmts)
        }
        tables::TwoByteOpcode::SystemGroup => {
            decode_system_group(prefixes, op2, bytes, start, stmts)
        }
        tables::TwoByteOpcode::SystemTrap => {
            decode_two_byte_trap(prefixes, op2, TrapKind::Sigill, stmts)
        }
        tables::TwoByteOpcode::SystemTrapRm => {
            if op2 == 0x20 || op2 == 0x21 || op2 == 0x22 || op2 == 0x23 {
                decode_two_byte_rm_trap(prefixes, op2, bytes, start, TrapKind::Sigill, stmts)
            } else {
                decode_two_byte_any_rm_trap(prefixes, op2, bytes, start, TrapKind::Sigill, stmts)
            }
        }
        tables::TwoByteOpcode::Syscall => decode_syscall(prefixes, op2, stmts),
        tables::TwoByteOpcode::CacheTrap => decode_cache_trap(prefixes, op2, stmts),
        tables::TwoByteOpcode::Ud2 => decode_ud2(prefixes, op2, stmts),
        tables::TwoByteOpcode::Prefetch => decode_prefetch(op2, bytes, start),
        tables::TwoByteOpcode::Prefetchw => decode_prefetchw(op2, bytes, start),
        tables::TwoByteOpcode::MmxStateNoop => decode_mmx_state_noop(prefixes, op2),
        tables::TwoByteOpcode::Endbr => decode_endbr(prefixes, op2, bytes, start),
        tables::TwoByteOpcode::NopRm => decode_nop_rm(op2, bytes, start),
        tables::TwoByteOpcode::Rdtsc => decode_rdtsc(prefixes, op2, stmts),
        tables::TwoByteOpcode::Cmov => decode_cmov(prefixes, op2, bytes, start, stmts),
        tables::TwoByteOpcode::MovzxI8 => decode_movzx(prefixes, bytes, start, stmts, OpSize::I8),
        tables::TwoByteOpcode::MovzxI16 => decode_movzx(prefixes, bytes, start, stmts, OpSize::I16),
        tables::TwoByteOpcode::UndefinedRm => {
            decode_two_byte_any_rm_trap(prefixes, op2, bytes, start, TrapKind::Sigill, stmts)
        }
        tables::TwoByteOpcode::ImulRm => {
            decode_binop_r_rm(BinOpKind::Mul, prefixes, bytes, start, stmts)
        }
        tables::TwoByteOpcode::MovsxI8 => decode_movsx(prefixes, bytes, start, stmts, OpSize::I8),
        tables::TwoByteOpcode::MovsxI16 => decode_movsx(prefixes, bytes, start, stmts, OpSize::I16),
        tables::TwoByteOpcode::Popcnt => {
            if prefixes.rep == Some(0xF3) {
                decode_popcnt(prefixes, bytes, start, stmts)
            } else {
                Err(crate::DecodeError::UnsupportedOpcode(op2))
            }
        }
        tables::TwoByteOpcode::Fence => decode_fence(prefixes, op2, bytes, start, stmts),
        tables::TwoByteOpcode::Xadd => decode_xadd(prefixes, op2, bytes, start, stmts),
        tables::TwoByteOpcode::Movnti => decode_movnti(prefixes, op2, bytes, start, stmts),
        tables::TwoByteOpcode::Lzcnt => {
            if prefixes.rep == Some(0xF3) {
                decode_lzcnt(prefixes, bytes, start, stmts)
            } else {
                // Bare 0F BD is BSR (F3 makes it LZCNT).
                decode_bsf_bsr(prefixes, op2, bytes, start, true, stmts)
            }
        }
        tables::TwoByteOpcode::Tzcnt => {
            if prefixes.rep == Some(0xF3) {
                decode_tzcnt(prefixes, bytes, start, stmts)
            } else {
                // Bare 0F BC is BSF (F3 makes it TZCNT).
                decode_bsf_bsr(prefixes, op2, bytes, start, false, stmts)
            }
        }
        tables::TwoByteOpcode::Bswap => decode_bswap(prefixes, op2, stmts),
        tables::TwoByteOpcode::Cmpxchg => decode_cmpxchg(prefixes, op2, bytes, start, stmts),
        tables::TwoByteOpcode::ThreeByte0F38 => {
            decode_three_byte_0f38(prefixes, bytes, start, stmts)
        }
        tables::TwoByteOpcode::CondJumpRel32 => {
            if prefixes.rex.present || prefixes.rep == Some(0xF3) || prefixes.operand_override {
                Err(crate::DecodeError::UnsupportedOpcode(op2))
            } else {
                decode_cond_jump_rel32(op2, bytes, cursor, opcode_guest_pc, stmts)
            }
        }
        tables::TwoByteOpcode::Setcc => decode_setcc(prefixes, op2, bytes, start, stmts),
        tables::TwoByteOpcode::Cpuid => decode_cpuid(prefixes, op2, stmts),
        tables::TwoByteOpcode::Unsupported => Err(crate::DecodeError::UnsupportedOpcode(op2)),
    }?;
    Ok(1 + consumed_after_escape)
}

fn decode_descriptor_group(
    prefixes: &PrefixSet,
    opcode: u8,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let (modrm, _) = modrm::parse_modrm(bytes, cursor + 1)?;
    if (modrm.reg == 2 || modrm.reg == 3)
        && !prefixes.rex.present
        && !prefixes.lock
        && prefixes.segment.is_none()
        && prefixes.rep.is_none()
        && !prefixes.operand_override
        && !prefixes.addr_override
    {
        let modrm_bytes = bytes_for_modrm(&modrm, bytes, cursor + 1)?;
        stmts.push(Stmt::new(
            None,
            Op::Trap(Trap {
                kind: TrapKind::Sigill,
            }),
        ));
        return Ok(1 + modrm_bytes);
    }
    Err(crate::DecodeError::UnsupportedOpcode(opcode))
}

fn decode_two_byte_rm_trap(
    prefixes: &PrefixSet,
    opcode: u8,
    bytes: &[u8],
    cursor: usize,
    kind: TrapKind,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.rex.present
        || prefixes.lock
        || prefixes.segment.is_some()
        || prefixes.rep.is_some()
        || prefixes.operand_override
        || prefixes.addr_override
    {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }
    let (modrm, _) = modrm::parse_modrm(bytes, cursor + 1)?;
    if modrm.mod_ != 3 {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }
    let modrm_bytes = bytes_for_modrm(&modrm, bytes, cursor + 1)?;
    stmts.push(Stmt::new(None, Op::Trap(Trap { kind })));
    Ok(1 + modrm_bytes)
}

fn decode_two_byte_any_rm_trap(
    prefixes: &PrefixSet,
    opcode: u8,
    bytes: &[u8],
    cursor: usize,
    kind: TrapKind,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.rex.present
        || prefixes.lock
        || prefixes.segment.is_some()
        || prefixes.rep.is_some()
        || prefixes.operand_override
        || prefixes.addr_override
    {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }
    let (modrm, _) = modrm::parse_modrm(bytes, cursor + 1)?;
    let modrm_bytes = bytes_for_modrm(&modrm, bytes, cursor + 1)?;
    stmts.push(Stmt::new(None, Op::Trap(Trap { kind })));
    Ok(1 + modrm_bytes)
}

fn decode_nop_rm(opcode: u8, bytes: &[u8], cursor: usize) -> Result<usize, crate::DecodeError> {
    let (modrm, _) = modrm::parse_modrm(bytes, cursor + 1)?;
    if modrm.reg != 0 {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }
    let modrm_bytes = bytes_for_modrm(&modrm, bytes, cursor + 1)?;
    Ok(1 + modrm_bytes)
}

fn decode_system_group(
    prefixes: &PrefixSet,
    opcode: u8,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let (modrm, _) = modrm::parse_modrm(bytes, cursor + 1)?;
    if modrm.mod_ == 3 && modrm.reg == 2 && modrm.rm == 0 {
        return decode_xgetbv(prefixes, opcode, stmts);
    }
    if modrm.mod_ == 3
        && ((modrm.reg == 0 && matches!(modrm.rm, 1..=4))
            || (modrm.reg == 1 && (modrm.rm <= 3 || modrm.rm == 7))
            || (modrm.reg == 2 && matches!(modrm.rm, 1 | 4))
            || modrm.reg == 3
            || (modrm.reg == 7 && modrm.rm == 0))
    {
        if prefixes.rex.present
            || prefixes.lock
            || prefixes.segment.is_some()
            || prefixes.rep.is_some()
            || prefixes.operand_override
            || prefixes.addr_override
        {
            return Err(crate::DecodeError::UnsupportedOpcode(opcode));
        }
        stmts.push(Stmt::new(
            None,
            Op::Trap(Trap {
                kind: TrapKind::Sigill,
            }),
        ));
        return Ok(2);
    }
    if ((modrm.reg == 2 || modrm.reg == 3 || modrm.reg == 7) && modrm.mod_ != 3) || modrm.reg == 6 {
        if prefixes.rex.present
            || prefixes.lock
            || prefixes.segment.is_some()
            || prefixes.rep.is_some()
            || prefixes.operand_override
            || prefixes.addr_override
        {
            return Err(crate::DecodeError::UnsupportedOpcode(opcode));
        }
        let modrm_bytes = bytes_for_modrm(&modrm, bytes, cursor + 1)?;
        stmts.push(Stmt::new(
            None,
            Op::Trap(Trap {
                kind: TrapKind::Sigill,
            }),
        ));
        return Ok(1 + modrm_bytes);
    }
    Err(crate::DecodeError::UnsupportedOpcode(opcode))
}

fn decode_prefetch(opcode: u8, bytes: &[u8], cursor: usize) -> Result<usize, crate::DecodeError> {
    let (modrm, _) = modrm::parse_modrm(bytes, cursor + 1)?;
    if modrm.mod_ == 3 || modrm.reg > 3 {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }
    let modrm_bytes = bytes_for_modrm(&modrm, bytes, cursor + 1)?;
    Ok(1 + modrm_bytes)
}

fn decode_prefetchw(opcode: u8, bytes: &[u8], cursor: usize) -> Result<usize, crate::DecodeError> {
    let (modrm, _) = modrm::parse_modrm(bytes, cursor + 1)?;
    if modrm.mod_ == 3 || modrm.reg > 1 {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }
    let modrm_bytes = bytes_for_modrm(&modrm, bytes, cursor + 1)?;
    Ok(1 + modrm_bytes)
}

fn decode_mmx_state_noop(prefixes: &PrefixSet, opcode: u8) -> Result<usize, crate::DecodeError> {
    if prefixes.rex.present
        || prefixes.lock
        || prefixes.segment.is_some()
        || prefixes.rep.is_some()
        || prefixes.operand_override
        || prefixes.addr_override
    {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }
    Ok(1)
}

fn decode_endbr(
    prefixes: &PrefixSet,
    opcode: u8,
    bytes: &[u8],
    cursor: usize,
) -> Result<usize, crate::DecodeError> {
    if prefixes.rep != Some(0xF3)
        || prefixes.rex.present
        || prefixes.operand_override
        || prefixes.addr_override
        || prefixes.lock
        || prefixes.segment.is_some()
    {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }
    match bytes.get(cursor + 1).copied() {
        Some(0xFA | 0xFB) => Ok(2),
        Some(_) => Err(crate::DecodeError::UnsupportedOpcode(opcode)),
        None => Err(crate::DecodeError::Truncated),
    }
}

fn decode_fence(
    prefixes: &PrefixSet,
    opcode: u8,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let (modrm, _) = modrm::parse_modrm(bytes, cursor + 1)?;
    if modrm.mod_ != 3
        && (modrm.reg == 7 || (modrm.reg == 6 && prefixes.operand_override))
        && !prefixes.rex.present
        && !prefixes.lock
        && prefixes.segment.is_none()
        && prefixes.rep.is_none()
        && !prefixes.addr_override
    {
        let modrm_bytes = bytes_for_modrm(&modrm, bytes, cursor + 1)?;
        return Ok(1 + modrm_bytes);
    }
    if prefixes.rex.present
        || prefixes.lock
        || prefixes.segment.is_some()
        || prefixes.rep.is_some()
        || prefixes.operand_override
        || prefixes.addr_override
    {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }
    if modrm.mod_ != 3 || modrm.rm != 0 || !(5..=7).contains(&modrm.reg) {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }
    let kind = match modrm.reg {
        5 => FenceKind::Lfence,
        6 => FenceKind::Mfence,
        7 => FenceKind::Sfence,
        _ => unreachable!(),
    };
    stmts.push(Stmt::new(None, Op::Fence(Fence { kind })));
    Ok(2)
}

fn decode_rdtsc(
    prefixes: &PrefixSet,
    opcode: u8,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.rex.present
        || prefixes.lock
        || prefixes.segment.is_some()
        || prefixes.rep.is_some()
        || prefixes.operand_override
        || prefixes.addr_override
    {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }
    let counter = stmts.len() as u32;
    stmts.push(Stmt::new(Some(counter), Op::Rdtsc(Rdtsc)));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: Gpr::Rax,
            value: counter,
            size: OpSize::I32,
        }),
    ));
    let shift = stmts.len() as u32;
    stmts.push(Stmt::new(
        Some(shift),
        Op::Constant(Constant {
            value: 32,
            size: OpSize::I64,
        }),
    ));
    let high = stmts.len() as u32;
    stmts.push(Stmt::new(
        Some(high),
        Op::BinOp(BinOp {
            op: BinOpKind::Shr,
            lhs: counter,
            rhs: shift,
            size: OpSize::I64,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: Gpr::Rdx,
            value: high,
            size: OpSize::I32,
        }),
    ));
    Ok(1)
}

fn decode_xgetbv(
    prefixes: &PrefixSet,
    opcode: u8,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.rex.present
        || prefixes.lock
        || prefixes.segment.is_some()
        || prefixes.rep.is_some()
        || prefixes.operand_override
        || prefixes.addr_override
    {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }
    let value = stmts.len() as u32;
    stmts.push(Stmt::new(Some(value), Op::Xgetbv(Xgetbv)));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: Gpr::Rax,
            value,
            size: OpSize::I32,
        }),
    ));
    let shift = stmts.len() as u32;
    stmts.push(Stmt::new(
        Some(shift),
        Op::Constant(Constant {
            value: 32,
            size: OpSize::I64,
        }),
    ));
    let high = stmts.len() as u32;
    stmts.push(Stmt::new(
        Some(high),
        Op::BinOp(BinOp {
            op: BinOpKind::Shr,
            lhs: value,
            rhs: shift,
            size: OpSize::I64,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: Gpr::Rdx,
            value: high,
            size: OpSize::I32,
        }),
    ));
    Ok(2)
}

fn decode_syscall(
    prefixes: &PrefixSet,
    opcode: u8,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.rex.present
        || prefixes.lock
        || prefixes.segment.is_some()
        || prefixes.rep.is_some()
        || prefixes.operand_override
        || prefixes.addr_override
    {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }
    stmts.push(Stmt::new(None, Op::Syscall(Syscall)));
    Ok(1)
}

fn decode_ud2(
    prefixes: &PrefixSet,
    opcode: u8,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.rex.present
        || prefixes.lock
        || prefixes.segment.is_some()
        || prefixes.rep.is_some()
        || prefixes.operand_override
        || prefixes.addr_override
    {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }
    stmts.push(Stmt::new(
        None,
        Op::Trap(Trap {
            kind: TrapKind::Sigill,
        }),
    ));
    Ok(1)
}

fn decode_one_byte_trap(
    prefixes: &PrefixSet,
    opcode: u8,
    kind: TrapKind,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.rex.present
        || prefixes.lock
        || prefixes.segment.is_some()
        || prefixes.rep.is_some()
        || prefixes.operand_override
        || prefixes.addr_override
    {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }
    stmts.push(Stmt::new(None, Op::Trap(Trap { kind })));
    Ok(1)
}

fn decode_two_byte_trap(
    prefixes: &PrefixSet,
    opcode: u8,
    kind: TrapKind,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.rex.present
        || prefixes.lock
        || prefixes.segment.is_some()
        || prefixes.rep.is_some()
        || prefixes.operand_override
        || prefixes.addr_override
    {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }
    stmts.push(Stmt::new(None, Op::Trap(Trap { kind })));
    Ok(1)
}

fn decode_cache_trap(
    prefixes: &PrefixSet,
    opcode: u8,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let valid_plain = prefixes.rep.is_none();
    let valid_wbnoinvd = opcode == 0x09 && prefixes.rep == Some(0xF3);
    if prefixes.rex.present
        || prefixes.lock
        || prefixes.segment.is_some()
        || prefixes.operand_override
        || prefixes.addr_override
        || (!valid_plain && !valid_wbnoinvd)
    {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }
    stmts.push(Stmt::new(
        None,
        Op::Trap(Trap {
            kind: TrapKind::Sigill,
        }),
    ));
    Ok(1)
}

fn decode_int_imm8(
    prefixes: &PrefixSet,
    opcode: u8,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.rex.present
        || prefixes.lock
        || prefixes.segment.is_some()
        || prefixes.rep.is_some()
        || prefixes.operand_override
        || prefixes.addr_override
    {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }
    bytes.get(cursor + 1).ok_or(crate::DecodeError::Truncated)?;
    stmts.push(Stmt::new(
        None,
        Op::Trap(Trap {
            kind: TrapKind::Sigtrap,
        }),
    ));
    Ok(2)
}

fn decode_legacy_invalid_imm8(
    prefixes: &PrefixSet,
    opcode: u8,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.rex.present
        || prefixes.lock
        || prefixes.segment.is_some()
        || prefixes.rep.is_some()
        || prefixes.operand_override
        || prefixes.addr_override
    {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }
    bytes.get(cursor + 1).ok_or(crate::DecodeError::Truncated)?;
    stmts.push(Stmt::new(
        None,
        Op::Trap(Trap {
            kind: TrapKind::Sigill,
        }),
    ));
    Ok(2)
}

fn decode_io_trap_imm8(
    prefixes: &PrefixSet,
    opcode: u8,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.rex.present
        || prefixes.lock
        || prefixes.segment.is_some()
        || prefixes.rep.is_some()
        || prefixes.operand_override
        || prefixes.addr_override
    {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }
    bytes.get(cursor + 1).ok_or(crate::DecodeError::Truncated)?;
    stmts.push(Stmt::new(
        None,
        Op::Trap(Trap {
            kind: TrapKind::Sigill,
        }),
    ));
    Ok(2)
}

fn decode_cpuid(
    prefixes: &PrefixSet,
    opcode: u8,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.rex.present
        || prefixes.lock
        || prefixes.segment.is_some()
        || prefixes.rep.is_some()
        || prefixes.operand_override
        || prefixes.addr_override
    {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }
    stmts.push(Stmt::new(None, Op::Cpuid(Cpuid)));
    Ok(1)
}

fn decode_cond_jump_rel8(
    _prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    opcode_guest_pc: u64,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let opcode = *bytes.get(cursor).ok_or(crate::DecodeError::Truncated)?;
    let cc = jcc_condition(opcode).ok_or(crate::DecodeError::UnsupportedOpcode(opcode))?;
    let rel = i64::from(i8::from_le_bytes([*bytes
        .get(cursor + 1)
        .ok_or(crate::DecodeError::Truncated)?]));
    let fallthrough = add_signed_disp(opcode_guest_pc, 2);
    let target = add_signed_disp(fallthrough, rel);
    stmts.push(Stmt::new(
        None,
        Op::CondJumpRel(CondJumpRel {
            cc,
            target_guest_pc: target,
            fallthrough_guest_pc: fallthrough,
        }),
    ));
    Ok(2)
}

fn decode_cond_jump_rel32(
    opcode: u8,
    bytes: &[u8],
    cursor: usize,
    opcode_guest_pc: u64,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let cc = jcc_condition(opcode).ok_or(crate::DecodeError::UnsupportedOpcode(opcode))?;
    let rel_bytes = bytes
        .get(cursor + 2..cursor + 6)
        .ok_or(crate::DecodeError::Truncated)?;
    let rel = i64::from(i32::from_le_bytes(
        rel_bytes
            .try_into()
            .map_err(|_| crate::DecodeError::Truncated)?,
    ));
    let fallthrough = add_signed_disp(opcode_guest_pc, 6);
    let target = add_signed_disp(fallthrough, rel);
    stmts.push(Stmt::new(
        None,
        Op::CondJumpRel(CondJumpRel {
            cc,
            target_guest_pc: target,
            fallthrough_guest_pc: fallthrough,
        }),
    ));
    Ok(5)
}

fn decode_call_rel32(
    _prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    opcode_guest_pc: u64,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let imm = read_imm(bytes, cursor + 1, 4)?;
    let rel = i64::from(i32::from_le_bytes(
        imm.to_le_bytes()[0..4]
            .try_into()
            .map_err(|_| crate::DecodeError::Truncated)?,
    ));
    let fallthrough = add_signed_disp(opcode_guest_pc, 5);
    let target = add_signed_disp(fallthrough, rel);
    stmts.push(Stmt::new(
        None,
        Op::CallRel(CallRel {
            target_guest_pc: target,
            return_guest_pc: fallthrough,
        }),
    ));
    Ok(5)
}

fn decode_jump_rel8(
    _prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    opcode_guest_pc: u64,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let rel = i64::from(i8::from_le_bytes([*bytes
        .get(cursor + 1)
        .ok_or(crate::DecodeError::Truncated)?]));
    let target = add_signed_disp(add_signed_disp(opcode_guest_pc, 2), rel);
    stmts.push(Stmt::new(
        None,
        Op::JumpRel(JumpRel {
            target_guest_pc: target,
        }),
    ));
    Ok(2)
}

fn decode_jump_rel32(
    _prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    opcode_guest_pc: u64,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let imm = i64::from(i32::from_le_bytes(
        read_imm(bytes, cursor + 1, 4)?.to_le_bytes()[0..4]
            .try_into()
            .map_err(|_| crate::DecodeError::Truncated)?,
    ));
    let target = add_signed_disp(add_signed_disp(opcode_guest_pc, 5), imm);
    stmts.push(Stmt::new(
        None,
        Op::JumpRel(JumpRel {
            target_guest_pc: target,
        }),
    ));
    Ok(5)
}

fn decode_jrcxz_rel8(
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    opcode_guest_pc: u64,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.rex.present
        || prefixes.operand_override
        || prefixes.lock
        || prefixes.segment.is_some()
        || prefixes.rep.is_some()
    {
        return Err(crate::DecodeError::UnsupportedOpcode(0xE3));
    }

    let rel = i64::from(i8::from_le_bytes([*bytes
        .get(cursor + 1)
        .ok_or(crate::DecodeError::Truncated)?]));
    let fallthrough = add_signed_disp(opcode_guest_pc, 2);
    let target = add_signed_disp(fallthrough, rel);
    let size = if prefixes.addr_override {
        OpSize::I32
    } else {
        OpSize::I64
    };
    let rcx = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(rcx),
        Op::LoadReg(LoadReg {
            reg: Gpr::Rcx,
            size,
        }),
    ));
    let zero = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(zero),
        Op::Constant(Constant { value: 0, size }),
    ));
    let flags = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(flags),
        Op::CmpFlags(CmpFlags {
            lhs: rcx,
            rhs: zero,
            size,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::CondJumpRel(CondJumpRel {
            cc: CondCode::Eq,
            target_guest_pc: target,
            fallthrough_guest_pc: fallthrough,
        }),
    ));
    Ok(2)
}

fn decode_loop_rel8(
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    opcode_guest_pc: u64,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.rex.present
        || prefixes.operand_override
        || prefixes.lock
        || prefixes.segment.is_some()
        || prefixes.rep.is_some()
    {
        return Err(crate::DecodeError::UnsupportedOpcode(0xE2));
    }

    let rel = i64::from(i8::from_le_bytes([*bytes
        .get(cursor + 1)
        .ok_or(crate::DecodeError::Truncated)?]));
    let fallthrough = add_signed_disp(opcode_guest_pc, 2);
    let target = add_signed_disp(fallthrough, rel);
    let size = if prefixes.addr_override {
        OpSize::I32
    } else {
        OpSize::I64
    };

    let counter = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(counter),
        Op::LoadReg(LoadReg {
            reg: Gpr::Rcx,
            size,
        }),
    ));
    let one = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(one),
        Op::Constant(Constant { value: 1, size }),
    ));
    let decremented = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(decremented),
        Op::BinOp(BinOp {
            op: BinOpKind::Sub,
            lhs: counter,
            rhs: one,
            size,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: Gpr::Rcx,
            value: decremented,
            size,
        }),
    ));
    let zero = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(zero),
        Op::Constant(Constant { value: 0, size }),
    ));
    let flags = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(flags),
        Op::CmpFlags(CmpFlags {
            lhs: decremented,
            rhs: zero,
            size,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::CondJumpRel(CondJumpRel {
            cc: CondCode::Ne,
            target_guest_pc: target,
            fallthrough_guest_pc: fallthrough,
        }),
    ));
    Ok(2)
}

fn decode_ret(
    prefixes: &PrefixSet,
    _bytes: &[u8],
    _cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.rex.present
        || prefixes.operand_override
        || prefixes.lock
        || prefixes.segment.is_some()
    {
        return Err(crate::DecodeError::UnsupportedOpcode(0xC3));
    }
    stmts.push(Stmt::new(None, Op::Return(Return)));
    Ok(1)
}

fn decode_ret_imm16(
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.rex.present
        || prefixes.operand_override
        || prefixes.lock
        || prefixes.segment.is_some()
    {
        return Err(crate::DecodeError::UnsupportedOpcode(0xC2));
    }
    let imm = read_imm(bytes, cursor + 1, 2)?;
    let pop_bytes = imm + 8;
    stmts.push(Stmt::new(None, Op::RetAdjusted(RetAdjusted { pop_bytes })));
    Ok(3)
}

fn decode_leave(prefixes: &PrefixSet, stmts: &mut Vec<Stmt>) -> Result<usize, crate::DecodeError> {
    if prefixes.rex.present
        || prefixes.operand_override
        || prefixes.lock
        || prefixes.segment.is_some()
    {
        return Err(crate::DecodeError::UnsupportedOpcode(0xC9));
    }

    let rbp = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(rbp),
        Op::LoadReg(LoadReg {
            reg: Gpr::Rbp,
            size: OpSize::I64,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: Gpr::Rsp,
            value: rbp,
            size: OpSize::I64,
        }),
    ));
    let rsp = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(rsp),
        Op::LoadReg(LoadReg {
            reg: Gpr::Rsp,
            size: OpSize::I64,
        }),
    ));
    let value = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(value),
        Op::LoadMem(LoadMem {
            addr: rsp,
            size: OpSize::I64,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: Gpr::Rbp,
            value,
            size: OpSize::I64,
        }),
    ));
    stmts.push(Stmt::new(None, Op::RspAdjust(RspAdjust { delta_bytes: 8 })));
    Ok(1)
}

fn decode_enter(
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.rex.present
        || prefixes.operand_override
        || prefixes.addr_override
        || prefixes.lock
        || prefixes.segment.is_some()
        || prefixes.rep.is_some()
    {
        return Err(crate::DecodeError::UnsupportedOpcode(0xC8));
    }

    let alloc_size = read_imm(bytes, cursor + 1, 2)?;
    let nesting = *bytes.get(cursor + 3).ok_or(crate::DecodeError::Truncated)?;
    if nesting != 0 {
        return Err(crate::DecodeError::UnsupportedOpcode(0xC8));
    }

    let rbp = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(rbp),
        Op::LoadReg(LoadReg {
            reg: Gpr::Rbp,
            size: OpSize::I64,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::RspAdjust(RspAdjust { delta_bytes: -8 }),
    ));
    let rsp_after_push = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(rsp_after_push),
        Op::LoadReg(LoadReg {
            reg: Gpr::Rsp,
            size: OpSize::I64,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreMem(StoreMem {
            addr: rsp_after_push,
            value: rbp,
            size: OpSize::I64,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: Gpr::Rbp,
            value: rsp_after_push,
            size: OpSize::I64,
        }),
    ));
    if alloc_size != 0 {
        stmts.push(Stmt::new(
            None,
            Op::RspAdjust(RspAdjust {
                delta_bytes: -(alloc_size as i64),
            }),
        ));
    }
    Ok(4)
}

fn decode_group4(
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.rep.is_some() || prefixes.lock || prefixes.segment.is_some() {
        return Err(crate::DecodeError::UnsupportedOpcode(0xFE));
    }

    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;
    let kind = match modrm.reg {
        0 => BinOpKind::Add,
        1 => BinOpKind::Sub,
        _ => return Err(crate::DecodeError::UnsupportedOpcode(0xFE)),
    };
    let size = OpSize::I8;
    let one = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(one),
        Op::Constant(Constant { value: 1, size }),
    ));

    if modrm.mod_ == 3 {
        let reg = map_reg(modrm.rm, &prefixes.rex, false);
        let value = alloc_ref(stmts);
        stmts.push(Stmt::new(Some(value), Op::LoadReg(LoadReg { reg, size })));
        let result = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(result),
            Op::BinOp(BinOp {
                op: kind,
                lhs: value,
                rhs: one,
                size,
            }),
        ));
        stmts.push(Stmt::new(
            None,
            Op::StoreReg(StoreReg {
                reg,
                value: result,
                size,
            }),
        ));
        Ok(2)
    } else {
        let (addr, used) = emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
        let value = alloc_ref(stmts);
        stmts.push(Stmt::new(Some(value), Op::LoadMem(LoadMem { addr, size })));
        let result = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(result),
            Op::BinOp(BinOp {
                op: kind,
                lhs: value,
                rhs: one,
                size,
            }),
        ));
        stmts.push(Stmt::new(
            None,
            Op::StoreMem(StoreMem {
                addr,
                value: result,
                size,
            }),
        ));
        Ok(1 + used)
    }
}

fn decode_group5(
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    opcode_guest_pc: u64,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let size = operand_size(prefixes, true);
    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;

    match modrm.reg {
        // inc r/m
        0 => {
            let one = alloc_ref(stmts);
            stmts.push(Stmt::new(
                Some(one),
                Op::Constant(Constant { value: 1, size }),
            ));
            if modrm.mod_ == 3 {
                let dst_reg = map_reg(modrm.rm, &prefixes.rex, false);
                let value_ref = alloc_ref(stmts);
                stmts.push(Stmt::new(
                    Some(value_ref),
                    Op::LoadReg(LoadReg { reg: dst_reg, size }),
                ));
                let result_ref = alloc_ref(stmts);
                stmts.push(Stmt::new(
                    Some(result_ref),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Add,
                        lhs: value_ref,
                        rhs: one,
                        size,
                    }),
                ));
                stmts.push(Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: dst_reg,
                        value: result_ref,
                        size,
                    }),
                ));
                Ok(2)
            } else {
                let (addr_ref, used) =
                    emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
                let value_ref = alloc_ref(stmts);
                stmts.push(Stmt::new(
                    Some(value_ref),
                    Op::LoadMem(LoadMem {
                        addr: addr_ref,
                        size,
                    }),
                ));
                let result_ref = alloc_ref(stmts);
                stmts.push(Stmt::new(
                    Some(result_ref),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Add,
                        lhs: value_ref,
                        rhs: one,
                        size,
                    }),
                ));
                stmts.push(Stmt::new(
                    None,
                    Op::StoreMem(StoreMem {
                        addr: addr_ref,
                        value: result_ref,
                        size,
                    }),
                ));
                Ok(1 + used)
            }
        }
        // dec r/m
        1 => {
            let one = alloc_ref(stmts);
            stmts.push(Stmt::new(
                Some(one),
                Op::Constant(Constant { value: 1, size }),
            ));
            if modrm.mod_ == 3 {
                let dst_reg = map_reg(modrm.rm, &prefixes.rex, false);
                let value_ref = alloc_ref(stmts);
                stmts.push(Stmt::new(
                    Some(value_ref),
                    Op::LoadReg(LoadReg { reg: dst_reg, size }),
                ));
                let result_ref = alloc_ref(stmts);
                stmts.push(Stmt::new(
                    Some(result_ref),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Sub,
                        lhs: value_ref,
                        rhs: one,
                        size,
                    }),
                ));
                stmts.push(Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: dst_reg,
                        value: result_ref,
                        size,
                    }),
                ));
                Ok(2)
            } else {
                let (addr_ref, used) =
                    emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
                let value_ref = alloc_ref(stmts);
                stmts.push(Stmt::new(
                    Some(value_ref),
                    Op::LoadMem(LoadMem {
                        addr: addr_ref,
                        size,
                    }),
                ));
                let result_ref = alloc_ref(stmts);
                stmts.push(Stmt::new(
                    Some(result_ref),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Sub,
                        lhs: value_ref,
                        rhs: one,
                        size,
                    }),
                ));
                stmts.push(Stmt::new(
                    None,
                    Op::StoreMem(StoreMem {
                        addr: addr_ref,
                        value: result_ref,
                        size,
                    }),
                ));
                Ok(1 + used)
            }
        }
        // call r/m
        2 => {
            let target_ref = alloc_ref(stmts);
            if modrm.mod_ == 3 {
                let src_reg = map_reg(modrm.rm, &prefixes.rex, false);
                stmts.push(Stmt::new(
                    Some(target_ref),
                    Op::LoadReg(LoadReg { reg: src_reg, size }),
                ));
                stmts.push(Stmt::new(
                    None,
                    Op::CallReg(CallReg {
                        target: target_ref,
                        return_guest_pc: add_signed_disp(opcode_guest_pc, 2),
                    }),
                ));
                Ok(2)
            } else {
                let (addr_ref, used) =
                    emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
                stmts.push(Stmt::new(
                    Some(target_ref),
                    Op::LoadMem(LoadMem {
                        addr: addr_ref,
                        size,
                    }),
                ));
                stmts.push(Stmt::new(
                    None,
                    Op::CallReg(CallReg {
                        target: target_ref,
                        return_guest_pc: add_signed_disp(opcode_guest_pc, (1 + used) as i64),
                    }),
                ));
                Ok(1 + used)
            }
        }
        // jmp r/m
        4 => {
            let target_ref = alloc_ref(stmts);
            if modrm.mod_ == 3 {
                let src_reg = map_reg(modrm.rm, &prefixes.rex, false);
                stmts.push(Stmt::new(
                    Some(target_ref),
                    Op::LoadReg(LoadReg { reg: src_reg, size }),
                ));
                stmts.push(Stmt::new(None, Op::JumpReg(JumpReg { target: target_ref })));
                Ok(2)
            } else {
                let (addr_ref, used) =
                    emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
                stmts.push(Stmt::new(
                    Some(target_ref),
                    Op::LoadMem(LoadMem {
                        addr: addr_ref,
                        size,
                    }),
                ));
                stmts.push(Stmt::new(None, Op::JumpReg(JumpReg { target: target_ref })));
                Ok(1 + used)
            }
        }
        // push r/m64
        6 => {
            let value = if modrm.mod_ == 3 {
                let src_reg = map_reg(modrm.rm, &prefixes.rex, false);
                let value = alloc_ref(stmts);
                stmts.push(Stmt::new(
                    Some(value),
                    Op::LoadReg(LoadReg {
                        reg: src_reg,
                        size: OpSize::I64,
                    }),
                ));
                value
            } else {
                let (addr_ref, used) =
                    emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
                let value = alloc_ref(stmts);
                stmts.push(Stmt::new(
                    Some(value),
                    Op::LoadMem(LoadMem {
                        addr: addr_ref,
                        size: OpSize::I64,
                    }),
                ));
                stmts.push(Stmt::new(
                    None,
                    Op::RspAdjust(RspAdjust { delta_bytes: -8 }),
                ));
                let rsp = alloc_ref(stmts);
                stmts.push(Stmt::new(
                    Some(rsp),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rsp,
                        size: OpSize::I64,
                    }),
                ));
                stmts.push(Stmt::new(
                    None,
                    Op::StoreMem(StoreMem {
                        addr: rsp,
                        value,
                        size: OpSize::I64,
                    }),
                ));
                return Ok(1 + used);
            };

            stmts.push(Stmt::new(
                None,
                Op::RspAdjust(RspAdjust { delta_bytes: -8 }),
            ));
            let rsp = alloc_ref(stmts);
            stmts.push(Stmt::new(
                Some(rsp),
                Op::LoadReg(LoadReg {
                    reg: Gpr::Rsp,
                    size: OpSize::I64,
                }),
            ));
            stmts.push(Stmt::new(
                None,
                Op::StoreMem(StoreMem {
                    addr: rsp,
                    value,
                    size: OpSize::I64,
                }),
            ));
            Ok(2)
        }
        _ => Err(crate::DecodeError::UnsupportedOpcode(0xFF)),
    }
}

// MUL/IMUL (Group 3 /4,/5) and DIV/IDIV (/6,/7), register-direct only.
// Mirrors the C++ MVP (`decode_mul_imul_from_rm` / `decode_div_from_rm`):
// the operand is RAX-only, full 128-bit RDX:RAX is deferred — most
// compiler output zero/sign-extends RDX first, so this covers the common
// case. low/quotient -> RAX, high/remainder -> RDX.
fn emit_rax_rdx_pair(
    stmts: &mut Vec<Stmt>,
    src_reg: Gpr,
    size: OpSize,
    lo_kind: BinOpKind,
    hi_kind: BinOpKind,
) {
    let rax_ref = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(rax_ref),
        Op::LoadReg(LoadReg {
            reg: Gpr::Rax,
            size,
        }),
    ));
    let rhs_ref = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(rhs_ref),
        Op::LoadReg(LoadReg { reg: src_reg, size }),
    ));
    let lo_ref = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(lo_ref),
        Op::BinOp(BinOp {
            op: lo_kind,
            lhs: rax_ref,
            rhs: rhs_ref,
            size,
        }),
    ));
    let hi_ref = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(hi_ref),
        Op::BinOp(BinOp {
            op: hi_kind,
            lhs: rax_ref,
            rhs: rhs_ref,
            size,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: Gpr::Rax,
            value: lo_ref,
            size,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: Gpr::Rdx,
            value: hi_ref,
            size,
        }),
    ));
}

fn decode_group3(
    prefixes: &PrefixSet,
    opcode: u8,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let size = if opcode == 0xF6 {
        OpSize::I8
    } else {
        operand_size(prefixes, true)
    };
    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;
    let all_ones = size.mask();

    match modrm.reg {
        // test r/m, imm: flags = dst & imm
        0 => {
            let modrm_bytes = bytes_for_modrm(&modrm, bytes, cursor + 1)?;
            let imm_offset = cursor + 1 + modrm_bytes;
            let imm_bytes = if opcode == 0xF6 {
                1
            } else if size == OpSize::I16 {
                2
            } else {
                4
            };
            let mut imm = read_imm(bytes, imm_offset, imm_bytes)?;
            if opcode == 0xF7 && size == OpSize::I64 {
                imm = i64::from(imm as i32).cast_unsigned();
            }
            let rhs = alloc_ref(stmts);
            stmts.push(Stmt::new(
                Some(rhs),
                Op::Constant(Constant { value: imm, size }),
            ));
            let lhs = if modrm.mod_ == 3 {
                let lhs_reg = map_reg(modrm.rm, &prefixes.rex, false);
                let r = alloc_ref(stmts);
                stmts.push(Stmt::new(
                    Some(r),
                    Op::LoadReg(LoadReg { reg: lhs_reg, size }),
                ));
                r
            } else {
                let (addr_ref, used) =
                    emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
                debug_assert_eq!(used, modrm_bytes);
                let r = alloc_ref(stmts);
                stmts.push(Stmt::new(
                    Some(r),
                    Op::LoadMem(LoadMem {
                        addr: addr_ref,
                        size,
                    }),
                ));
                r
            };
            stmts.push(Stmt::new(
                None,
                Op::AluFlags(AluFlags {
                    op: BinOpKind::And,
                    lhs,
                    rhs,
                    size,
                }),
            ));
            Ok(1 + modrm_bytes + imm_bytes)
        }
        // not (ones complement): dst = all_ones XOR dst
        2 => {
            if modrm.mod_ == 3 {
                let dst_reg = map_reg(modrm.rm, &prefixes.rex, false);
                let value_ref = alloc_ref(stmts);
                stmts.push(Stmt::new(
                    Some(value_ref),
                    Op::LoadReg(LoadReg { reg: dst_reg, size }),
                ));
                let mask_ref = alloc_ref(stmts);
                stmts.push(Stmt::new(
                    Some(mask_ref),
                    Op::Constant(Constant {
                        value: all_ones,
                        size,
                    }),
                ));
                let result_ref = alloc_ref(stmts);
                stmts.push(Stmt::new(
                    Some(result_ref),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Xor,
                        lhs: value_ref,
                        rhs: mask_ref,
                        size,
                    }),
                ));
                stmts.push(Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: dst_reg,
                        value: result_ref,
                        size,
                    }),
                ));
                Ok(2)
            } else {
                let (addr_ref, used) =
                    emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
                let value_ref = alloc_ref(stmts);
                stmts.push(Stmt::new(
                    Some(value_ref),
                    Op::LoadMem(LoadMem {
                        addr: addr_ref,
                        size,
                    }),
                ));
                let mask_ref = alloc_ref(stmts);
                stmts.push(Stmt::new(
                    Some(mask_ref),
                    Op::Constant(Constant {
                        value: all_ones,
                        size,
                    }),
                ));
                let result_ref = alloc_ref(stmts);
                stmts.push(Stmt::new(
                    Some(result_ref),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Xor,
                        lhs: value_ref,
                        rhs: mask_ref,
                        size,
                    }),
                ));
                stmts.push(Stmt::new(
                    None,
                    Op::StoreMem(StoreMem {
                        addr: addr_ref,
                        value: result_ref,
                        size,
                    }),
                ));
                Ok(1 + used)
            }
        }
        // neg: dst = 0 - dst
        3 => {
            if modrm.mod_ == 3 {
                let dst_reg = map_reg(modrm.rm, &prefixes.rex, false);
                let value_ref = alloc_ref(stmts);
                stmts.push(Stmt::new(
                    Some(value_ref),
                    Op::LoadReg(LoadReg { reg: dst_reg, size }),
                ));
                let zero_ref = alloc_ref(stmts);
                stmts.push(Stmt::new(
                    Some(zero_ref),
                    Op::Constant(Constant { value: 0, size }),
                ));
                let result_ref = alloc_ref(stmts);
                stmts.push(Stmt::new(
                    Some(result_ref),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Sub,
                        lhs: zero_ref,
                        rhs: value_ref,
                        size,
                    }),
                ));
                stmts.push(Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: dst_reg,
                        value: result_ref,
                        size,
                    }),
                ));
                Ok(2)
            } else {
                let (addr_ref, used) =
                    emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
                let value_ref = alloc_ref(stmts);
                stmts.push(Stmt::new(
                    Some(value_ref),
                    Op::LoadMem(LoadMem {
                        addr: addr_ref,
                        size,
                    }),
                ));
                let zero_ref = alloc_ref(stmts);
                stmts.push(Stmt::new(
                    Some(zero_ref),
                    Op::Constant(Constant { value: 0, size }),
                ));
                let result_ref = alloc_ref(stmts);
                stmts.push(Stmt::new(
                    Some(result_ref),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Sub,
                        lhs: zero_ref,
                        rhs: value_ref,
                        size,
                    }),
                ));
                stmts.push(Stmt::new(
                    None,
                    Op::StoreMem(StoreMem {
                        addr: addr_ref,
                        value: result_ref,
                        size,
                    }),
                ));
                Ok(1 + used)
            }
        }
        // mul/imul (/4,/5) + div/idiv (/6,/7), register-direct only.
        4..=7 if modrm.mod_ == 3 => {
            let src_reg = map_reg(modrm.rm, &prefixes.rex, false);
            let (lo_kind, hi_kind) = match modrm.reg {
                4 => (BinOpKind::Mul, BinOpKind::UMulHi), // mul
                5 => (BinOpKind::Mul, BinOpKind::SMulHi), // imul
                6 => (BinOpKind::UDiv, BinOpKind::UMod),  // div
                _ => (BinOpKind::SDiv, BinOpKind::SMod),  // idiv (7)
            };
            emit_rax_rdx_pair(stmts, src_reg, size, lo_kind, hi_kind);
            Ok(2)
        }
        _ => Err(crate::DecodeError::UnsupportedOpcode(opcode)),
    }
}

fn jcc_condition(opcode: u8) -> Option<CondCode> {
    match opcode & 0x0F {
        0x0 => Some(CondCode::Ov),
        0x1 => Some(CondCode::NoOv),
        0x2 => Some(CondCode::Nc),
        0x3 => Some(CondCode::Cc),
        0x4 => Some(CondCode::Eq),
        0x5 => Some(CondCode::Ne),
        0x6 => Some(CondCode::Ule),
        0x7 => Some(CondCode::Ugt),
        0x8 => Some(CondCode::Mi),
        0x9 => Some(CondCode::Pl),
        0xC => Some(CondCode::Slt),
        0xD => Some(CondCode::Sge),
        0xE => Some(CondCode::Sle),
        0xF => Some(CondCode::Sgt),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Instruction decoders
// ---------------------------------------------------------------------------

fn decode_mov_rm_r(
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let size = operand_size(prefixes, true);
    decode_mov_rm_r_with_size(prefixes, bytes, cursor, stmts, size)
}

fn decode_mov_rm_r_with_size(
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
    size: OpSize,
) -> Result<usize, crate::DecodeError> {
    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;
    let source_reg = map_reg(modrm.reg, &prefixes.rex, true);
    let value_ref = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(value_ref),
        Op::LoadReg(LoadReg {
            reg: source_reg,
            size,
        }),
    ));

    if modrm.mod_ == 3 {
        let dst_reg = map_reg(modrm.rm, &prefixes.rex, false);
        stmts.push(Stmt::new(
            None,
            Op::StoreReg(StoreReg {
                reg: dst_reg,
                value: value_ref,
                size,
            }),
        ));
        Ok(2)
    } else {
        let (addr_ref, used) = emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
        stmts.push(Stmt::new(
            None,
            Op::StoreMem(StoreMem {
                addr: addr_ref,
                value: value_ref,
                size,
            }),
        ));
        Ok(1 + used)
    }
}

fn decode_mov_moffs(
    prefixes: &PrefixSet,
    opcode: u8,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.lock || prefixes.rep.is_some() {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }

    let size = match opcode {
        0xA0 | 0xA2 => OpSize::I8,
        0xA1 | 0xA3 => operand_size(prefixes, true),
        _ => return Err(crate::DecodeError::UnsupportedOpcode(opcode)),
    };
    let addr_bytes = if prefixes.addr_override { 4 } else { 8 };
    let addr_value = read_imm(bytes, cursor + 1, addr_bytes)?;
    let addr = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(addr),
        Op::Constant(Constant {
            value: addr_value,
            size: OpSize::I64,
        }),
    ));

    if opcode == 0xA0 || opcode == 0xA1 {
        let value = alloc_ref(stmts);
        stmts.push(Stmt::new(Some(value), Op::LoadMem(LoadMem { addr, size })));
        stmts.push(Stmt::new(
            None,
            Op::StoreReg(StoreReg {
                reg: Gpr::Rax,
                value,
                size,
            }),
        ));
    } else {
        let value = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(value),
            Op::LoadReg(LoadReg {
                reg: Gpr::Rax,
                size,
            }),
        ));
        stmts.push(Stmt::new(
            None,
            Op::StoreMem(StoreMem { addr, value, size }),
        ));
    }

    Ok(1 + addr_bytes)
}

fn decode_mov_r_rm(
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let size = operand_size(prefixes, true);
    decode_mov_r_rm_with_size(prefixes, bytes, cursor, stmts, size)
}

fn decode_mov_r_rm_with_size(
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
    size: OpSize,
) -> Result<usize, crate::DecodeError> {
    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;
    let dst_reg = map_reg(modrm.reg, &prefixes.rex, true);

    let (src, used) = if modrm.mod_ == 3 {
        let src_reg = map_reg(modrm.rm, &prefixes.rex, false);
        let src = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(src),
            Op::LoadReg(LoadReg { reg: src_reg, size }),
        ));
        (src, 1)
    } else {
        let (addr_ref, used) = emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
        let src = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(src),
            Op::LoadMem(LoadMem {
                addr: addr_ref,
                size,
            }),
        ));
        (src, used)
    };

    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: dst_reg,
            value: src,
            size,
        }),
    ));
    Ok(1 + used)
}

fn decode_lea(
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.rep.is_some()
        || prefixes.lock
        || prefixes.segment.is_some()
        || prefixes.operand_override
        || prefixes.addr_override
    {
        return Err(crate::DecodeError::UnsupportedOpcode(0x8D));
    }

    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;
    if modrm.mod_ == 3 {
        return Err(crate::DecodeError::UnsupportedOpcode(0x8D));
    }

    let dst_reg = map_reg(modrm.reg, &prefixes.rex, true);
    let size = if prefixes.rex.w {
        OpSize::I64
    } else {
        OpSize::I32
    };
    let (addr, used) = emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: dst_reg,
            value: addr,
            size,
        }),
    ));
    Ok(1 + used)
}

fn decode_mov_ri(
    prefixes: &PrefixSet,
    opcode: u8,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let size = operand_size(prefixes, true);
    let reg_idx = opcode - 0xB8;
    let reg = map_reg_raw(reg_idx, &prefixes.rex);
    let imm_bytes = size.bit_width() as usize / 8;
    let imm = read_imm(bytes, cursor + 1, imm_bytes)?;
    let r = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(r),
        Op::Constant(Constant { value: imm, size }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg,
            value: r,
            size,
        }),
    ));
    Ok(1 + imm_bytes)
}

fn decode_mov_r8_i8(
    prefixes: &PrefixSet,
    opcode: u8,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let reg = map_reg_raw(opcode - 0xB0, &prefixes.rex);
    let imm = u64::from(*bytes.get(cursor + 1).ok_or(crate::DecodeError::Truncated)?);
    let r = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(r),
        Op::Constant(Constant {
            value: imm,
            size: OpSize::I8,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg,
            value: r,
            size: OpSize::I8,
        }),
    ));
    Ok(2)
}

fn decode_mov_rm_imm(
    prefixes: &PrefixSet,
    opcode: u8,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.rep.is_some() || prefixes.lock || prefixes.segment.is_some() {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }

    let size = if opcode == 0xC6 {
        OpSize::I8
    } else {
        operand_size(prefixes, true)
    };
    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;
    if modrm.reg != 0 {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }

    let modrm_bytes = bytes_for_modrm(&modrm, bytes, cursor + 1)?;
    let imm_offset = cursor + 1 + modrm_bytes;
    let imm_bytes = match (opcode, size) {
        (0xC6, _) => 1,
        (0xC7, OpSize::I16) => 2,
        (0xC7, OpSize::I32 | OpSize::I64) => 4,
        _ => return Err(crate::DecodeError::UnsupportedOpcode(opcode)),
    };
    let mut imm = read_imm(bytes, imm_offset, imm_bytes)?;
    if opcode == 0xC7 && size == OpSize::I64 {
        imm = i64::from(imm as i32).cast_unsigned();
    }

    let value = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(value),
        Op::Constant(Constant { value: imm, size }),
    ));

    if modrm.mod_ == 3 {
        let reg = map_reg(modrm.rm, &prefixes.rex, false);
        stmts.push(Stmt::new(None, Op::StoreReg(StoreReg { reg, value, size })));
    } else {
        let (addr, used) = emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
        stmts.push(Stmt::new(
            None,
            Op::StoreMem(StoreMem { addr, value, size }),
        ));
        return Ok(1 + used + imm_bytes);
    }

    Ok(1 + modrm_bytes + imm_bytes)
}

fn decode_acc_imm(
    prefixes: &PrefixSet,
    opcode: u8,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.rep.is_some() || prefixes.lock || prefixes.segment.is_some() {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }

    let is_byte = matches!(
        opcode,
        0x04 | 0x0C | 0x14 | 0x1C | 0x24 | 0x2C | 0x34 | 0x3C | 0xA8
    );
    let size = if is_byte {
        OpSize::I8
    } else {
        operand_size(prefixes, true)
    };
    let imm_bytes = if is_byte {
        1
    } else if size == OpSize::I16 {
        2
    } else {
        4
    };
    let mut imm = read_imm(bytes, cursor + 1, imm_bytes)?;
    if !is_byte && size == OpSize::I64 {
        imm = i64::from(imm as i32).cast_unsigned();
    }

    let lhs = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(lhs),
        Op::LoadReg(LoadReg {
            reg: Gpr::Rax,
            size,
        }),
    ));
    let rhs = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(rhs),
        Op::Constant(Constant { value: imm, size }),
    ));

    match opcode {
        0x3C | 0x3D => {
            let flags = alloc_ref(stmts);
            stmts.push(Stmt::new(
                Some(flags),
                Op::CmpFlags(CmpFlags { lhs, rhs, size }),
            ));
        }
        0xA8 | 0xA9 => {
            stmts.push(Stmt::new(
                None,
                Op::AluFlags(AluFlags {
                    op: BinOpKind::And,
                    lhs,
                    rhs,
                    size,
                }),
            ));
        }
        _ => {
            let op = match opcode {
                0x04 | 0x05 => BinOpKind::Add,
                0x0C | 0x0D => BinOpKind::Or,
                0x14 | 0x15 => BinOpKind::Add,
                0x1C | 0x1D => BinOpKind::Sub,
                0x24 | 0x25 => BinOpKind::And,
                0x2C | 0x2D => BinOpKind::Sub,
                0x34 | 0x35 => BinOpKind::Xor,
                _ => return Err(crate::DecodeError::UnsupportedOpcode(opcode)),
            };
            let result = alloc_ref(stmts);
            stmts.push(Stmt::new(
                Some(result),
                Op::BinOp(BinOp { op, lhs, rhs, size }),
            ));
            stmts.push(Stmt::new(
                None,
                Op::StoreReg(StoreReg {
                    reg: Gpr::Rax,
                    value: result,
                    size,
                }),
            ));
        }
    }

    Ok(1 + imm_bytes)
}

fn decode_binop_rm_r(
    kind: BinOpKind,
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let size = operand_size(prefixes, true);
    decode_binop_rm_r_with_size(kind, prefixes, bytes, cursor, stmts, size)
}

fn decode_binop_rm_r_with_size(
    kind: BinOpKind,
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
    size: OpSize,
) -> Result<usize, crate::DecodeError> {
    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;
    let rhs_ref = alloc_ref(stmts);
    let rhs_reg = map_reg(modrm.reg, &prefixes.rex, true);
    stmts.push(Stmt::new(
        Some(rhs_ref),
        Op::LoadReg(LoadReg { reg: rhs_reg, size }),
    ));
    if modrm.mod_ == 3 {
        let lhs_reg = map_reg(modrm.rm, &prefixes.rex, false);
        let lhs_ref = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(lhs_ref),
            Op::LoadReg(LoadReg { reg: lhs_reg, size }),
        ));
        let result = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(result),
            Op::BinOp(BinOp {
                op: kind,
                lhs: lhs_ref,
                rhs: rhs_ref,
                size,
            }),
        ));
        stmts.push(Stmt::new(
            None,
            Op::StoreReg(StoreReg {
                reg: lhs_reg,
                value: result,
                size,
            }),
        ));
        Ok(2)
    } else {
        let (addr_ref, used) = emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
        let mem_val = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(mem_val),
            Op::LoadMem(LoadMem {
                addr: addr_ref,
                size,
            }),
        ));
        let result = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(result),
            Op::BinOp(BinOp {
                op: kind,
                lhs: mem_val,
                rhs: rhs_ref,
                size,
            }),
        ));
        stmts.push(Stmt::new(
            None,
            Op::StoreMem(StoreMem {
                addr: addr_ref,
                value: result,
                size,
            }),
        ));
        Ok(1 + used)
    }
}

fn decode_binop_r_rm(
    kind: BinOpKind,
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let size = operand_size(prefixes, true);
    decode_binop_r_rm_with_size(kind, prefixes, bytes, cursor, stmts, size)
}

fn decode_binop_r_rm_with_size(
    kind: BinOpKind,
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
    size: OpSize,
) -> Result<usize, crate::DecodeError> {
    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;
    let dst_reg = map_reg(modrm.reg, &prefixes.rex, true);
    let dst_val = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(dst_val),
        Op::LoadReg(LoadReg { reg: dst_reg, size }),
    ));

    let (rhs, used) = if modrm.mod_ == 3 {
        let rhs_reg = map_reg(modrm.rm, &prefixes.rex, false);
        let r = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(r),
            Op::LoadReg(LoadReg { reg: rhs_reg, size }),
        ));
        (r, 1)
    } else {
        let (addr_ref, used) = emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
        let r = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(r),
            Op::LoadMem(LoadMem {
                addr: addr_ref,
                size,
            }),
        ));
        (r, used)
    };

    let result = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(result),
        Op::BinOp(BinOp {
            op: kind,
            lhs: dst_val,
            rhs,
            size,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: dst_reg,
            value: result,
            size,
        }),
    ));
    Ok(1 + used)
}

fn decode_xadd(
    prefixes: &PrefixSet,
    opcode: u8,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.rep.is_some() || prefixes.lock || prefixes.segment.is_some() {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }

    let size = if opcode == 0xC0 {
        OpSize::I8
    } else {
        operand_size(prefixes, true)
    };
    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;
    let src_reg = map_reg(modrm.reg, &prefixes.rex, true);
    let src = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(src),
        Op::LoadReg(LoadReg { reg: src_reg, size }),
    ));

    if modrm.mod_ == 3 {
        let dst_reg = map_reg(modrm.rm, &prefixes.rex, false);
        let dst = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(dst),
            Op::LoadReg(LoadReg { reg: dst_reg, size }),
        ));
        let result = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(result),
            Op::BinOp(BinOp {
                op: BinOpKind::Add,
                lhs: dst,
                rhs: src,
                size,
            }),
        ));
        stmts.push(Stmt::new(
            None,
            Op::StoreReg(StoreReg {
                reg: src_reg,
                value: dst,
                size,
            }),
        ));
        stmts.push(Stmt::new(
            None,
            Op::StoreReg(StoreReg {
                reg: dst_reg,
                value: result,
                size,
            }),
        ));
        Ok(2)
    } else {
        let (addr, used) = emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
        let dst = alloc_ref(stmts);
        stmts.push(Stmt::new(Some(dst), Op::LoadMem(LoadMem { addr, size })));
        let result = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(result),
            Op::BinOp(BinOp {
                op: BinOpKind::Add,
                lhs: dst,
                rhs: src,
                size,
            }),
        ));
        stmts.push(Stmt::new(
            None,
            Op::StoreReg(StoreReg {
                reg: src_reg,
                value: dst,
                size,
            }),
        ));
        stmts.push(Stmt::new(
            None,
            Op::StoreMem(StoreMem {
                addr,
                value: result,
                size,
            }),
        ));
        Ok(1 + used)
    }
}

/// CMPXCHG r/m8, r8 (0F B0) and r/m, r (0F B1), register-direct (mod==3) only.
///
/// Semantics (mirrors `decode_cmpxchg_rm_r` in the C++ reference):
///   if RAX == dst: dst = src (ZF=1) else RAX = dst (ZF=0)
/// On success the accumulator is NOT written; failure writes the accumulator:
/// r/m16 writes AX only, r/m32/64 writes the full RAX (I32 zero-extends).
/// The accumulator writeback is emitted FIRST, then the destination, so that
/// when the r/m operand aliases RAX (e.g. `cmpxchg rax, rcx`) the compare
/// always succeeds and the dst write wins (SDM accumulator-then-DEST order).
///
/// Memory and LOCK forms (and CMPXCHG8B/16B) are deferred.
fn decode_cmpxchg(
    prefixes: &PrefixSet,
    opcode: u8,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let size = if opcode == 0xB0 {
        OpSize::I8
    } else {
        operand_size(prefixes, true)
    };
    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;
    if modrm.mod_ != 3 {
        // Register-direct only; memory/LOCK forms are deferred.
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }

    let src_reg = map_reg(modrm.reg, &prefixes.rex, true);
    let dst_reg = map_reg(modrm.rm, &prefixes.rex, false);

    // ref_acc = RAX at the operand size (the comparison operand).
    let acc = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(acc),
        Op::LoadReg(LoadReg {
            reg: Gpr::Rax,
            size,
        }),
    ));
    // For I32/I64, capture the full RAX so the failure writeback preserves the
    // upper bits on success and zero-extends on a 32-bit failure.
    let rax_full = if size == OpSize::I16 {
        None
    } else {
        let r = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(r),
            Op::LoadReg(LoadReg {
                reg: Gpr::Rax,
                size: OpSize::I64,
            }),
        ));
        Some(r)
    };

    // ref_src = explicit register operand (ModRM.reg).
    let src = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(src),
        Op::LoadReg(LoadReg { reg: src_reg, size }),
    ));
    // ref_dst = register-direct destination (ModRM.rm).
    let dst = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(dst),
        Op::LoadReg(LoadReg { reg: dst_reg, size }),
    ));

    // ZF = (acc == dst). The Rust lowerer requires CmpFlags to carry a result
    // ref (unlike the C++ nullopt); the following Selects read NZCV by cc.
    let cmp = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(cmp),
        Op::CmpFlags(CmpFlags {
            lhs: acc,
            rhs: dst,
            size,
        }),
    ));

    // equal -> dst = src ; else dst unchanged.
    let new_dst = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(new_dst),
        Op::Select(Select {
            cc: CondCode::Eq,
            true_value: src,
            false_value: dst,
            size,
        }),
    ));
    // Accumulator writeback value: success keeps the accumulator, failure
    // captures dst. I16 writes AX only; I32/I64 select into the full RAX.
    let new_rax = alloc_ref(stmts);
    let (rax_keep, rax_size) = if size == OpSize::I16 {
        (acc, OpSize::I16)
    } else {
        (rax_full.expect("rax_full loaded for non-I16"), OpSize::I64)
    };
    stmts.push(Stmt::new(
        Some(new_rax),
        Op::Select(Select {
            cc: CondCode::Eq,
            true_value: rax_keep,
            false_value: dst,
            size: rax_size,
        }),
    ));

    // Accumulator writeback FIRST, then DEST (matters when rm aliases RAX).
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: Gpr::Rax,
            value: new_rax,
            size: rax_size,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: dst_reg,
            value: new_dst,
            size,
        }),
    ));
    Ok(2)
}

fn decode_movnti(
    prefixes: &PrefixSet,
    opcode: u8,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.rep.is_some() || prefixes.lock || prefixes.segment.is_some() {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }

    let size = operand_size(prefixes, true);
    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;
    if modrm.mod_ == 3 {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }

    let src_reg = map_reg(modrm.reg, &prefixes.rex, true);
    let value = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(value),
        Op::LoadReg(LoadReg { reg: src_reg, size }),
    ));
    let (addr, used) = emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
    stmts.push(Stmt::new(
        None,
        Op::StoreMem(StoreMem { addr, value, size }),
    ));
    Ok(1 + used)
}

#[allow(clippy::too_many_lines)]
fn decode_alu_rm_imm(
    prefixes: &PrefixSet,
    opcode: u8,
    bytes: &[u8],
    cursor: usize,
    opcode_guest_pc: u64,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let size = if opcode == 0x80 {
        OpSize::I8
    } else {
        operand_size(prefixes, true)
    };
    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;
    let kind = match modrm.reg {
        1 => Some(BinOpKind::Or),
        4 => Some(BinOpKind::And),
        6 => Some(BinOpKind::Xor),
        0 | 2 => Some(BinOpKind::Add),
        3 | 5 => Some(BinOpKind::Sub),
        7 => None,
        _ => return Err(crate::DecodeError::UnsupportedOpcode(opcode)),
    };
    let modrm_bytes = bytes_for_modrm(&modrm, bytes, cursor + 1)?;
    let imm_offset = cursor + 1 + modrm_bytes;
    let imm_bytes = match opcode {
        0x80 | 0x83 => 1,
        0x81 if size == OpSize::I16 => 2,
        0x81 => 4,
        _ => return Err(crate::DecodeError::UnsupportedOpcode(opcode)),
    };
    let mut imm = if opcode == 0x83 {
        i64::from(*bytes.get(imm_offset).ok_or(crate::DecodeError::Truncated)? as i8)
            .cast_unsigned()
    } else {
        read_imm(bytes, imm_offset, imm_bytes)?
    };
    if opcode == 0x81 && size == OpSize::I64 {
        imm = i64::from(imm as i32).cast_unsigned();
    }
    let imm_ref = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(imm_ref),
        Op::Constant(Constant { value: imm, size }),
    ));

    if modrm.mod_ == 3 {
        let lhs_reg = map_reg(modrm.rm, &prefixes.rex, false);
        let lhs_ref = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(lhs_ref),
            Op::LoadReg(LoadReg { reg: lhs_reg, size }),
        ));
        if kind.is_none() {
            let flags_ref = alloc_ref(stmts);
            stmts.push(Stmt::new(
                Some(flags_ref),
                Op::CmpFlags(CmpFlags {
                    lhs: lhs_ref,
                    rhs: imm_ref,
                    size,
                }),
            ));
            return Ok(1 + modrm_bytes + imm_bytes);
        }
        let result = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(result),
            Op::BinOp(BinOp {
                op: kind.expect("non-CMP Group 1 op has BinOpKind"),
                lhs: lhs_ref,
                rhs: imm_ref,
                size,
            }),
        ));
        stmts.push(Stmt::new(
            None,
            Op::StoreReg(StoreReg {
                reg: lhs_reg,
                value: result,
                size,
            }),
        ));
    } else {
        let rip_after = opcode_guest_pc.wrapping_add(1 + modrm_bytes as u64 + imm_bytes as u64);
        let (addr_ref, used) =
            emit_addr_with_size_at(modrm, prefixes, bytes, cursor + 1, rip_after, stmts)?;
        debug_assert_eq!(used, modrm_bytes);
        let mem_val = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(mem_val),
            Op::LoadMem(LoadMem {
                addr: addr_ref,
                size,
            }),
        ));
        if kind.is_none() {
            let flags_ref = alloc_ref(stmts);
            stmts.push(Stmt::new(
                Some(flags_ref),
                Op::CmpFlags(CmpFlags {
                    lhs: mem_val,
                    rhs: imm_ref,
                    size,
                }),
            ));
            return Ok(1 + modrm_bytes + imm_bytes);
        }
        let result = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(result),
            Op::BinOp(BinOp {
                op: kind.expect("non-CMP Group 1 op has BinOpKind"),
                lhs: mem_val,
                rhs: imm_ref,
                size,
            }),
        ));
        stmts.push(Stmt::new(
            None,
            Op::StoreMem(StoreMem {
                addr: addr_ref,
                value: result,
                size,
            }),
        ));
    }
    Ok(1 + modrm_bytes + imm_bytes)
}

fn decode_group2(
    prefixes: &PrefixSet,
    opcode: u8,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let size = if matches!(opcode, 0xC0 | 0xD0 | 0xD2) {
        OpSize::I8
    } else {
        operand_size(prefixes, true)
    };
    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;
    let op = match modrm.reg {
        0 => BinOpKind::Rol,
        1 => BinOpKind::Ror,
        4 => BinOpKind::Shl,
        5 => BinOpKind::Shr,
        7 => BinOpKind::Sar,
        _ => return Err(crate::DecodeError::UnsupportedOpcode(opcode)),
    };
    let modrm_bytes = bytes_for_modrm(&modrm, bytes, cursor + 1)?;
    let (shift, extra_bytes) = match opcode {
        0xC0 | 0xC1 => {
            let imm_offset = cursor + 1 + modrm_bytes;
            let imm = u64::from(*bytes.get(imm_offset).ok_or(crate::DecodeError::Truncated)?);
            let shift = alloc_ref(stmts);
            stmts.push(Stmt::new(
                Some(shift),
                Op::Constant(Constant {
                    value: imm,
                    size: OpSize::I8,
                }),
            ));
            (shift, 1)
        }
        0xD0 | 0xD1 => {
            let shift = alloc_ref(stmts);
            stmts.push(Stmt::new(
                Some(shift),
                Op::Constant(Constant {
                    value: 1,
                    size: OpSize::I8,
                }),
            ));
            (shift, 0)
        }
        0xD2 | 0xD3 => {
            let shift = alloc_ref(stmts);
            stmts.push(Stmt::new(
                Some(shift),
                Op::LoadReg(LoadReg {
                    reg: Gpr::Rcx,
                    size: OpSize::I8,
                }),
            ));
            (shift, 0)
        }
        _ => return Err(crate::DecodeError::UnsupportedOpcode(opcode)),
    };

    if modrm.mod_ == 3 {
        let reg = map_reg(modrm.rm, &prefixes.rex, false);
        let value = alloc_ref(stmts);
        stmts.push(Stmt::new(Some(value), Op::LoadReg(LoadReg { reg, size })));
        let result = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(result),
            Op::BinOp(BinOp {
                op,
                lhs: value,
                rhs: shift,
                size,
            }),
        ));
        stmts.push(Stmt::new(
            None,
            Op::StoreReg(StoreReg {
                reg,
                value: result,
                size,
            }),
        ));
    } else {
        let (addr, used) = emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
        debug_assert_eq!(used, modrm_bytes);
        let value = alloc_ref(stmts);
        stmts.push(Stmt::new(Some(value), Op::LoadMem(LoadMem { addr, size })));
        let result = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(result),
            Op::BinOp(BinOp {
                op,
                lhs: value,
                rhs: shift,
                size,
            }),
        ));
        stmts.push(Stmt::new(
            None,
            Op::StoreMem(StoreMem {
                addr,
                value: result,
                size,
            }),
        ));
    }

    Ok(1 + modrm_bytes + extra_bytes)
}

fn decode_cmp_rm_r(
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let size = operand_size(prefixes, true);
    decode_cmp_rm_r_with_size(prefixes, bytes, cursor, stmts, size)
}

fn decode_cmp_rm_r_with_size(
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
    size: OpSize,
) -> Result<usize, crate::DecodeError> {
    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;
    let rhs_reg = map_reg(modrm.reg, &prefixes.rex, true);
    let rhs = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(rhs),
        Op::LoadReg(LoadReg { reg: rhs_reg, size }),
    ));

    let (lhs, used) = if modrm.mod_ == 3 {
        let lhs_reg = map_reg(modrm.rm, &prefixes.rex, false);
        let r = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(r),
            Op::LoadReg(LoadReg { reg: lhs_reg, size }),
        ));
        (r, 1)
    } else {
        let (addr_ref, used) = emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
        let r = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(r),
            Op::LoadMem(LoadMem {
                addr: addr_ref,
                size,
            }),
        ));
        (r, used)
    };

    let result = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(result),
        Op::CmpFlags(CmpFlags { lhs, rhs, size }),
    ));
    Ok(1 + used)
}

fn decode_cmp_r_rm_with_size(
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
    size: OpSize,
) -> Result<usize, crate::DecodeError> {
    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;
    let lhs_reg = map_reg(modrm.reg, &prefixes.rex, true);
    let lhs = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(lhs),
        Op::LoadReg(LoadReg { reg: lhs_reg, size }),
    ));

    let (rhs, used) = if modrm.mod_ == 3 {
        let rhs_reg = map_reg(modrm.rm, &prefixes.rex, false);
        let r = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(r),
            Op::LoadReg(LoadReg { reg: rhs_reg, size }),
        ));
        (r, 1)
    } else {
        let (addr_ref, used) = emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
        let r = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(r),
            Op::LoadMem(LoadMem {
                addr: addr_ref,
                size,
            }),
        ));
        (r, used)
    };

    let result = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(result),
        Op::CmpFlags(CmpFlags { lhs, rhs, size }),
    ));
    Ok(1 + used)
}

fn sign_extend_imm8_to_size(imm: u8, size: OpSize) -> u64 {
    match size {
        OpSize::I64 => i64::from(imm as i8).cast_unsigned(),
        OpSize::I32 => u64::from((i64::from(imm as i8) as i32).cast_unsigned()),
        OpSize::I16 => u64::from((i64::from(imm as i8) as i16).cast_unsigned()),
        OpSize::I8 => imm as u64,
    }
}

fn decode_imul_imm(
    prefixes: &PrefixSet,
    opcode: u8,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let size = operand_size(prefixes, true);
    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;
    let modrm_bytes = bytes_for_modrm(&modrm, bytes, cursor + 1)?;
    let imm_bytes = if opcode == 0x6B {
        1
    } else if size == OpSize::I16 {
        2
    } else {
        4
    };
    let imm_offset = cursor + 1 + modrm_bytes;
    let mut imm = read_imm(bytes, imm_offset, imm_bytes)?;
    if opcode == 0x6B {
        imm = sign_extend_imm8_to_size(imm as u8, size);
    } else if size == OpSize::I64 {
        imm = i64::from(imm as i32).cast_unsigned();
    }

    let dst_reg = map_reg(modrm.reg, &prefixes.rex, true);
    let lhs = if modrm.mod_ == 3 {
        let src_reg = map_reg(modrm.rm, &prefixes.rex, false);
        let r = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(r),
            Op::LoadReg(LoadReg { reg: src_reg, size }),
        ));
        r
    } else {
        let (addr_ref, used) = emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
        debug_assert_eq!(used, modrm_bytes);
        let r = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(r),
            Op::LoadMem(LoadMem {
                addr: addr_ref,
                size,
            }),
        ));
        r
    };

    let rhs = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(rhs),
        Op::Constant(Constant { value: imm, size }),
    ));
    let result = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(result),
        Op::BinOp(BinOp {
            op: BinOpKind::Mul,
            lhs,
            rhs,
            size,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: dst_reg,
            value: result,
            size,
        }),
    ));
    Ok(1 + modrm_bytes + imm_bytes)
}

fn decode_test_rm_r(
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let size = operand_size(prefixes, true);
    decode_test_rm_r_with_size(prefixes, bytes, cursor, stmts, size)
}

fn decode_test_rm_r_with_size(
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
    size: OpSize,
) -> Result<usize, crate::DecodeError> {
    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;
    let rhs_reg = map_reg(modrm.reg, &prefixes.rex, true);
    let rhs = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(rhs),
        Op::LoadReg(LoadReg { reg: rhs_reg, size }),
    ));

    let (lhs, used) = if modrm.mod_ == 3 {
        let lhs_reg = map_reg(modrm.rm, &prefixes.rex, false);
        let r = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(r),
            Op::LoadReg(LoadReg { reg: lhs_reg, size }),
        ));
        (r, 1)
    } else {
        let (addr_ref, used) = emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
        let r = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(r),
            Op::LoadMem(LoadMem {
                addr: addr_ref,
                size,
            }),
        ));
        (r, used)
    };

    stmts.push(Stmt::new(
        None,
        Op::AluFlags(AluFlags {
            op: BinOpKind::And,
            lhs,
            rhs,
            size,
        }),
    ));
    Ok(1 + used)
}

fn decode_xchg(
    prefixes: &PrefixSet,
    opcode: u8,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let size = if opcode == 0x86 {
        OpSize::I8
    } else {
        operand_size(prefixes, true)
    };
    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;
    let reg_a = map_reg(modrm.reg, &prefixes.rex, true);

    let val_a = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(val_a),
        Op::LoadReg(LoadReg { reg: reg_a, size }),
    ));

    if modrm.mod_ == 3 {
        let reg_b = map_reg(modrm.rm, &prefixes.rex, false);
        let val_b = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(val_b),
            Op::LoadReg(LoadReg { reg: reg_b, size }),
        ));
        stmts.push(Stmt::new(
            None,
            Op::StoreReg(StoreReg {
                reg: reg_a,
                value: val_b,
                size,
            }),
        ));
        stmts.push(Stmt::new(
            None,
            Op::StoreReg(StoreReg {
                reg: reg_b,
                value: val_a,
                size,
            }),
        ));
        Ok(2)
    } else {
        let (addr, used) = emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
        let val_mem = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(val_mem),
            Op::LoadMem(LoadMem { addr, size }),
        ));
        stmts.push(Stmt::new(
            None,
            Op::StoreReg(StoreReg {
                reg: reg_a,
                value: val_mem,
                size,
            }),
        ));
        stmts.push(Stmt::new(
            None,
            Op::StoreMem(StoreMem {
                addr,
                value: val_a,
                size,
            }),
        ));
        Ok(1 + used)
    }
}

fn decode_xchg_acc(prefixes: &PrefixSet, opcode: u8, stmts: &mut Vec<Stmt>) -> usize {
    let size = operand_size(prefixes, true);
    let other = map_reg_raw(opcode - 0x90, &prefixes.rex);
    let acc_val = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(acc_val),
        Op::LoadReg(LoadReg {
            reg: Gpr::Rax,
            size,
        }),
    ));
    let other_val = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(other_val),
        Op::LoadReg(LoadReg { reg: other, size }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: Gpr::Rax,
            value: other_val,
            size,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: other,
            value: acc_val,
            size,
        }),
    ));
    1
}

fn decode_xlat(prefixes: &PrefixSet, stmts: &mut Vec<Stmt>) -> Result<usize, crate::DecodeError> {
    if prefixes.addr_override
        || prefixes.lock
        || prefixes.segment.is_some()
        || prefixes.rep.is_some()
    {
        return Err(crate::DecodeError::UnsupportedOpcode(0xD7));
    }

    let al = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(al),
        Op::LoadReg(LoadReg {
            reg: Gpr::Rax,
            size: OpSize::I8,
        }),
    ));
    let index = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(index),
        Op::Extend(Extend {
            value: al,
            from_size: OpSize::I8,
            to_size: OpSize::I64,
            is_signed: false,
        }),
    ));
    let base = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(base),
        Op::LoadReg(LoadReg {
            reg: Gpr::Rbx,
            size: OpSize::I64,
        }),
    ));
    let addr = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(addr),
        Op::BinOp(BinOp {
            op: BinOpKind::Add,
            lhs: base,
            rhs: index,
            size: OpSize::I64,
        }),
    ));
    let value = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(value),
        Op::LoadMem(LoadMem {
            addr,
            size: OpSize::I8,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: Gpr::Rax,
            value,
            size: OpSize::I8,
        }),
    ));
    Ok(1)
}

fn decode_x87_d9(
    prefixes: &PrefixSet,
    opcode: u8,
    bytes: &[u8],
    cursor: usize,
) -> Result<usize, crate::DecodeError> {
    if prefixes.rex.present
        || prefixes.lock
        || prefixes.segment.is_some()
        || prefixes.rep.is_some()
        || prefixes.operand_override
        || prefixes.addr_override
    {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }
    let (modrm, _) = modrm::parse_modrm(bytes, cursor + 1)?;
    if modrm.mod_ == 3 && modrm.reg == 2 && modrm.rm == 0 {
        Ok(2)
    } else {
        Err(crate::DecodeError::UnsupportedOpcode(opcode))
    }
}

fn decode_x87_db(
    prefixes: &PrefixSet,
    opcode: u8,
    bytes: &[u8],
    cursor: usize,
) -> Result<usize, crate::DecodeError> {
    if prefixes.rex.present
        || prefixes.lock
        || prefixes.segment.is_some()
        || prefixes.rep.is_some()
        || prefixes.operand_override
        || prefixes.addr_override
    {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }
    let (modrm, _) = modrm::parse_modrm(bytes, cursor + 1)?;
    if modrm.mod_ == 3 && modrm.reg == 4 && matches!(modrm.rm, 2 | 3) {
        Ok(2)
    } else {
        Err(crate::DecodeError::UnsupportedOpcode(opcode))
    }
}

fn string_op_size(prefixes: &PrefixSet, opcode: u8) -> OpSize {
    if matches!(opcode, 0xA4 | 0xA6 | 0xAA | 0xAC | 0xAE) {
        OpSize::I8
    } else {
        operand_size(prefixes, true)
    }
}

fn string_step(size: OpSize) -> u64 {
    match size {
        OpSize::I8 => 1,
        OpSize::I16 => 2,
        OpSize::I32 => 4,
        OpSize::I64 => 8,
    }
}

fn decode_lods(
    prefixes: &PrefixSet,
    opcode: u8,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.addr_override
        || prefixes.lock
        || prefixes.segment.is_some()
        || prefixes.rep.is_some()
    {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }

    let size = string_op_size(prefixes, opcode);
    let rsi = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(rsi),
        Op::LoadReg(LoadReg {
            reg: Gpr::Rsi,
            size: OpSize::I64,
        }),
    ));
    let value = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(value),
        Op::LoadMem(LoadMem { addr: rsi, size }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: Gpr::Rax,
            value,
            size,
        }),
    ));
    let one = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(one),
        Op::Constant(Constant {
            value: string_step(size),
            size: OpSize::I64,
        }),
    ));
    let next = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(next),
        Op::BinOp(BinOp {
            op: BinOpKind::Add,
            lhs: rsi,
            rhs: one,
            size: OpSize::I64,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: Gpr::Rsi,
            value: next,
            size: OpSize::I64,
        }),
    ));
    Ok(1)
}

fn decode_stos(
    prefixes: &PrefixSet,
    opcode: u8,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.addr_override
        || prefixes.lock
        || prefixes.segment.is_some()
        || prefixes.rep.is_some()
    {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }

    let size = string_op_size(prefixes, opcode);
    let rdi = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(rdi),
        Op::LoadReg(LoadReg {
            reg: Gpr::Rdi,
            size: OpSize::I64,
        }),
    ));
    let value = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(value),
        Op::LoadReg(LoadReg {
            reg: Gpr::Rax,
            size,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreMem(StoreMem {
            addr: rdi,
            value,
            size,
        }),
    ));
    let one = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(one),
        Op::Constant(Constant {
            value: string_step(size),
            size: OpSize::I64,
        }),
    ));
    let next = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(next),
        Op::BinOp(BinOp {
            op: BinOpKind::Add,
            lhs: rdi,
            rhs: one,
            size: OpSize::I64,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: Gpr::Rdi,
            value: next,
            size: OpSize::I64,
        }),
    ));
    Ok(1)
}

fn decode_scas(
    prefixes: &PrefixSet,
    opcode: u8,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.addr_override
        || prefixes.lock
        || prefixes.segment.is_some()
        || prefixes.rep.is_some()
    {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }

    let size = string_op_size(prefixes, opcode);
    let rdi = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(rdi),
        Op::LoadReg(LoadReg {
            reg: Gpr::Rdi,
            size: OpSize::I64,
        }),
    ));
    let lhs = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(lhs),
        Op::LoadReg(LoadReg {
            reg: Gpr::Rax,
            size,
        }),
    ));
    let rhs = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(rhs),
        Op::LoadMem(LoadMem { addr: rdi, size }),
    ));
    let flags = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(flags),
        Op::CmpFlags(CmpFlags { lhs, rhs, size }),
    ));
    let one = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(one),
        Op::Constant(Constant {
            value: string_step(size),
            size: OpSize::I64,
        }),
    ));
    let next = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(next),
        Op::BinOp(BinOp {
            op: BinOpKind::Add,
            lhs: rdi,
            rhs: one,
            size: OpSize::I64,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: Gpr::Rdi,
            value: next,
            size: OpSize::I64,
        }),
    ));
    Ok(1)
}

fn decode_cmps(
    prefixes: &PrefixSet,
    opcode: u8,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.addr_override
        || prefixes.lock
        || prefixes.segment.is_some()
        || prefixes.rep.is_some()
    {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }

    let size = string_op_size(prefixes, opcode);
    let rsi = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(rsi),
        Op::LoadReg(LoadReg {
            reg: Gpr::Rsi,
            size: OpSize::I64,
        }),
    ));
    let rdi = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(rdi),
        Op::LoadReg(LoadReg {
            reg: Gpr::Rdi,
            size: OpSize::I64,
        }),
    ));
    let lhs = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(lhs),
        Op::LoadMem(LoadMem { addr: rsi, size }),
    ));
    let rhs = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(rhs),
        Op::LoadMem(LoadMem { addr: rdi, size }),
    ));
    let flags = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(flags),
        Op::CmpFlags(CmpFlags { lhs, rhs, size }),
    ));
    let one = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(one),
        Op::Constant(Constant {
            value: string_step(size),
            size: OpSize::I64,
        }),
    ));
    let next_rsi = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(next_rsi),
        Op::BinOp(BinOp {
            op: BinOpKind::Add,
            lhs: rsi,
            rhs: one,
            size: OpSize::I64,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: Gpr::Rsi,
            value: next_rsi,
            size: OpSize::I64,
        }),
    ));
    let next_rdi = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(next_rdi),
        Op::BinOp(BinOp {
            op: BinOpKind::Add,
            lhs: rdi,
            rhs: one,
            size: OpSize::I64,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: Gpr::Rdi,
            value: next_rdi,
            size: OpSize::I64,
        }),
    ));
    Ok(1)
}

fn decode_movs(
    prefixes: &PrefixSet,
    opcode: u8,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.addr_override
        || prefixes.lock
        || prefixes.segment.is_some()
        || prefixes.rep.is_some()
    {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }

    let size = string_op_size(prefixes, opcode);
    let rsi = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(rsi),
        Op::LoadReg(LoadReg {
            reg: Gpr::Rsi,
            size: OpSize::I64,
        }),
    ));
    let rdi = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(rdi),
        Op::LoadReg(LoadReg {
            reg: Gpr::Rdi,
            size: OpSize::I64,
        }),
    ));
    let value = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(value),
        Op::LoadMem(LoadMem { addr: rsi, size }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreMem(StoreMem {
            addr: rdi,
            value,
            size,
        }),
    ));
    let one = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(one),
        Op::Constant(Constant {
            value: string_step(size),
            size: OpSize::I64,
        }),
    ));
    let next_rsi = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(next_rsi),
        Op::BinOp(BinOp {
            op: BinOpKind::Add,
            lhs: rsi,
            rhs: one,
            size: OpSize::I64,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: Gpr::Rsi,
            value: next_rsi,
            size: OpSize::I64,
        }),
    ));
    let next_rdi = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(next_rdi),
        Op::BinOp(BinOp {
            op: BinOpKind::Add,
            lhs: rdi,
            rhs: one,
            size: OpSize::I64,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: Gpr::Rdi,
            value: next_rdi,
            size: OpSize::I64,
        }),
    ));
    Ok(1)
}

fn decode_sign_extend_acc(prefixes: &PrefixSet, stmts: &mut Vec<Stmt>) -> usize {
    let (from_size, to_size) = if prefixes.rex.w {
        (OpSize::I32, OpSize::I64)
    } else if prefixes.operand_override {
        (OpSize::I8, OpSize::I16)
    } else {
        (OpSize::I16, OpSize::I32)
    };
    let value = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(value),
        Op::LoadReg(LoadReg {
            reg: Gpr::Rax,
            size: from_size,
        }),
    ));
    let extended = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(extended),
        Op::Extend(Extend {
            value,
            from_size,
            to_size,
            is_signed: true,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: Gpr::Rax,
            value: extended,
            size: to_size,
        }),
    ));
    1
}

fn decode_sign_extend_acc_to_dx(prefixes: &PrefixSet, stmts: &mut Vec<Stmt>) -> usize {
    let size = if prefixes.rex.w {
        OpSize::I64
    } else if prefixes.operand_override {
        OpSize::I16
    } else {
        OpSize::I32
    };
    let shift_bits = match size {
        OpSize::I64 => 63,
        OpSize::I32 => 31,
        OpSize::I16 => 15,
        OpSize::I8 => 7,
    };
    let value = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(value),
        Op::LoadReg(LoadReg {
            reg: Gpr::Rax,
            size,
        }),
    ));
    let shift = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(shift),
        Op::Constant(Constant {
            value: shift_bits,
            size,
        }),
    ));
    let high = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(high),
        Op::BinOp(BinOp {
            op: BinOpKind::Sar,
            lhs: value,
            rhs: shift,
            size,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: Gpr::Rdx,
            value: high,
            size,
        }),
    ));
    1
}

fn decode_push_r(prefixes: &PrefixSet, opcode: u8, stmts: &mut Vec<Stmt>) -> usize {
    let reg_idx = opcode - 0x50;
    let reg = map_reg_raw(reg_idx, &prefixes.rex);
    let val = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(val),
        Op::LoadReg(LoadReg {
            reg,
            size: OpSize::I64,
        }),
    ));
    // RSP adjustment via RspAdjust
    stmts.push(Stmt::new(
        None,
        Op::RspAdjust(RspAdjust { delta_bytes: -8 }),
    ));
    let rsp = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(rsp),
        Op::LoadReg(LoadReg {
            reg: Gpr::Rsp,
            size: OpSize::I64,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreMem(StoreMem {
            addr: rsp,
            value: val,
            size: OpSize::I64,
        }),
    ));
    1
}

fn decode_push_imm(
    prefixes: &PrefixSet,
    opcode: u8,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.rex.present || prefixes.lock || prefixes.segment.is_some() {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }

    let (imm, consumed) = if opcode == 0x6A {
        let imm = i64::from(*bytes.get(cursor + 1).ok_or(crate::DecodeError::Truncated)? as i8)
            .cast_unsigned();
        (imm, 2)
    } else {
        let imm_bytes = if prefixes.operand_override { 2 } else { 4 };
        let mut imm = read_imm(bytes, cursor + 1, imm_bytes)?;
        if imm_bytes == 4 {
            imm = i64::from(imm as i32).cast_unsigned();
        }
        (imm, 1 + imm_bytes)
    };

    let val = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(val),
        Op::Constant(Constant {
            value: imm,
            size: OpSize::I64,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::RspAdjust(RspAdjust { delta_bytes: -8 }),
    ));
    let rsp = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(rsp),
        Op::LoadReg(LoadReg {
            reg: Gpr::Rsp,
            size: OpSize::I64,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreMem(StoreMem {
            addr: rsp,
            value: val,
            size: OpSize::I64,
        }),
    ));
    Ok(consumed)
}

fn decode_pop_r(prefixes: &PrefixSet, opcode: u8, stmts: &mut Vec<Stmt>) -> usize {
    let reg_idx = opcode - 0x58;
    let reg = map_reg_raw(reg_idx, &prefixes.rex);
    let rsp = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(rsp),
        Op::LoadReg(LoadReg {
            reg: Gpr::Rsp,
            size: OpSize::I64,
        }),
    ));
    let val = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(val),
        Op::LoadMem(LoadMem {
            addr: rsp,
            size: OpSize::I64,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg,
            value: val,
            size: OpSize::I64,
        }),
    ));
    stmts.push(Stmt::new(None, Op::RspAdjust(RspAdjust { delta_bytes: 8 })));
    1
}

fn decode_pop_rm(
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;
    if modrm.reg != 0 {
        return Err(crate::DecodeError::UnsupportedOpcode(0x8F));
    }

    let rsp = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(rsp),
        Op::LoadReg(LoadReg {
            reg: Gpr::Rsp,
            size: OpSize::I64,
        }),
    ));
    let value = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(value),
        Op::LoadMem(LoadMem {
            addr: rsp,
            size: OpSize::I64,
        }),
    ));

    if modrm.mod_ == 3 {
        let dst_reg = map_reg(modrm.rm, &prefixes.rex, false);
        stmts.push(Stmt::new(
            None,
            Op::StoreReg(StoreReg {
                reg: dst_reg,
                value,
                size: OpSize::I64,
            }),
        ));
        stmts.push(Stmt::new(None, Op::RspAdjust(RspAdjust { delta_bytes: 8 })));
        Ok(2)
    } else {
        let (addr, used) = emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
        stmts.push(Stmt::new(
            None,
            Op::StoreMem(StoreMem {
                addr,
                value,
                size: OpSize::I64,
            }),
        ));
        stmts.push(Stmt::new(None, Op::RspAdjust(RspAdjust { delta_bytes: 8 })));
        Ok(1 + used)
    }
}

fn decode_movzx(
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
    from: OpSize,
) -> Result<usize, crate::DecodeError> {
    let size = operand_size(prefixes, true);
    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;
    let dst_reg = map_reg(modrm.reg, &prefixes.rex, true);

    let (src, used) = if modrm.mod_ == 3 {
        let src_reg = map_reg(modrm.rm, &prefixes.rex, false);
        let r = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(r),
            Op::LoadReg(LoadReg {
                reg: src_reg,
                size: from,
            }),
        ));
        (r, 1)
    } else {
        let (addr_ref, used) = emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
        let r = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(r),
            Op::LoadMem(LoadMem {
                addr: addr_ref,
                size: from,
            }),
        ));
        (r, used)
    };

    let extended = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(extended),
        Op::Extend(Extend {
            value: src,
            from_size: from,
            to_size: size,
            is_signed: false,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: dst_reg,
            value: extended,
            size,
        }),
    ));
    Ok(1 + used)
}

fn decode_movsx(
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
    from: OpSize,
) -> Result<usize, crate::DecodeError> {
    let size = operand_size(prefixes, true);
    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;
    let dst_reg = map_reg(modrm.reg, &prefixes.rex, true);

    let (src, used) = if modrm.mod_ == 3 {
        let src_reg = map_reg(modrm.rm, &prefixes.rex, false);
        let r = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(r),
            Op::LoadReg(LoadReg {
                reg: src_reg,
                size: from,
            }),
        ));
        (r, 1)
    } else {
        let (addr_ref, used) = emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
        let r = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(r),
            Op::LoadMem(LoadMem {
                addr: addr_ref,
                size: from,
            }),
        ));
        (r, used)
    };

    let extended = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(extended),
        Op::Extend(Extend {
            value: src,
            from_size: from,
            to_size: size,
            is_signed: true,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: dst_reg,
            value: extended,
            size,
        }),
    ));
    Ok(1 + used)
}

fn decode_movsxd(
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.operand_override {
        if prefixes.rep.is_some()
            || prefixes.lock
            || prefixes.segment.is_some()
            || prefixes.addr_override
            || prefixes.rex.present
        {
            return Err(crate::DecodeError::UnsupportedOpcode(0x63));
        }
        let (modrm, _) = modrm::parse_modrm(bytes, cursor + 1)?;
        let modrm_bytes = bytes_for_modrm(&modrm, bytes, cursor + 1)?;
        stmts.push(Stmt::new(
            None,
            Op::Trap(Trap {
                kind: TrapKind::Sigill,
            }),
        ));
        return Ok(1 + modrm_bytes);
    }

    if prefixes.rep.is_some()
        || prefixes.lock
        || prefixes.segment.is_some()
        || prefixes.addr_override
    {
        return Err(crate::DecodeError::UnsupportedOpcode(0x63));
    }

    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;
    let dst_reg = map_reg(modrm.reg, &prefixes.rex, true);
    let (src, used) = if modrm.mod_ == 3 {
        let src_reg = map_reg(modrm.rm, &prefixes.rex, false);
        let r = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(r),
            Op::LoadReg(LoadReg {
                reg: src_reg,
                size: OpSize::I32,
            }),
        ));
        (r, 1)
    } else {
        let (addr_ref, used) = emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
        let r = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(r),
            Op::LoadMem(LoadMem {
                addr: addr_ref,
                size: OpSize::I32,
            }),
        ));
        (r, used)
    };

    if prefixes.rex.w {
        let extended = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(extended),
            Op::Extend(Extend {
                value: src,
                from_size: OpSize::I32,
                to_size: OpSize::I64,
                is_signed: true,
            }),
        ));
        stmts.push(Stmt::new(
            None,
            Op::StoreReg(StoreReg {
                reg: dst_reg,
                value: extended,
                size: OpSize::I64,
            }),
        ));
    } else {
        stmts.push(Stmt::new(
            None,
            Op::StoreReg(StoreReg {
                reg: dst_reg,
                value: src,
                size: OpSize::I32,
            }),
        ));
    }
    Ok(1 + used)
}

fn decode_cmov(
    prefixes: &PrefixSet,
    opcode: u8,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.rep.is_some() || prefixes.lock || prefixes.segment.is_some() {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }

    let cc = jcc_condition(opcode).ok_or(crate::DecodeError::UnsupportedOpcode(opcode))?;
    let size = operand_size(prefixes, true);
    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;
    let dst_reg = map_reg(modrm.reg, &prefixes.rex, true);

    let false_value = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(false_value),
        Op::LoadReg(LoadReg { reg: dst_reg, size }),
    ));

    let (true_value, used) = if modrm.mod_ == 3 {
        let src_reg = map_reg(modrm.rm, &prefixes.rex, false);
        let r = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(r),
            Op::LoadReg(LoadReg { reg: src_reg, size }),
        ));
        (r, 1)
    } else {
        let (addr_ref, used) = emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
        let r = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(r),
            Op::LoadMem(LoadMem {
                addr: addr_ref,
                size,
            }),
        ));
        (r, used)
    };

    let selected = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(selected),
        Op::Select(prisma_ir::Select {
            cc,
            true_value,
            false_value,
            size,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: dst_reg,
            value: selected,
            size,
        }),
    ));
    Ok(1 + used)
}

fn decode_setcc(
    prefixes: &PrefixSet,
    opcode: u8,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.rep.is_some() || prefixes.lock || prefixes.segment.is_some() {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }

    let cc = jcc_condition(opcode).ok_or(crate::DecodeError::UnsupportedOpcode(opcode))?;
    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;

    let one = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(one),
        Op::Constant(Constant {
            value: 1,
            size: OpSize::I8,
        }),
    ));
    let zero = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(zero),
        Op::Constant(Constant {
            value: 0,
            size: OpSize::I8,
        }),
    ));
    let selected = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(selected),
        Op::Select(prisma_ir::Select {
            cc,
            true_value: one,
            false_value: zero,
            size: OpSize::I8,
        }),
    ));

    if modrm.mod_ == 3 {
        let reg = map_reg(modrm.rm, &prefixes.rex, false);
        stmts.push(Stmt::new(
            None,
            Op::StoreReg(StoreReg {
                reg,
                value: selected,
                size: OpSize::I8,
            }),
        ));
        Ok(2)
    } else {
        let (addr, used) = emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
        stmts.push(Stmt::new(
            None,
            Op::StoreMem(StoreMem {
                addr,
                value: selected,
                size: OpSize::I8,
            }),
        ));
        Ok(1 + used)
    }
}

fn decode_popcnt(
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let size = operand_size(prefixes, true);
    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;
    let dst_reg = map_reg(modrm.reg, &prefixes.rex, true);

    let (src, used) = if modrm.mod_ == 3 {
        let src_reg = map_reg(modrm.rm, &prefixes.rex, false);
        let r = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(r),
            Op::LoadReg(LoadReg { reg: src_reg, size }),
        ));
        (r, 1)
    } else {
        let (addr_ref, used) = emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
        let r = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(r),
            Op::LoadMem(LoadMem {
                addr: addr_ref,
                size,
            }),
        ));
        (r, used)
    };

    let result = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(result),
        Op::Popcnt(Popcnt { value: src, size }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: dst_reg,
            value: result,
            size,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::WriteFlagsPopcnt(prisma_ir::WriteFlagsPopcnt { src, size }),
    ));
    Ok(1 + used)
}

fn decode_lzcnt(
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let size = operand_size(prefixes, true);
    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;
    let dst_reg = map_reg(modrm.reg, &prefixes.rex, true);

    let (src, used) = if modrm.mod_ == 3 {
        let src_reg = map_reg(modrm.rm, &prefixes.rex, false);
        let r = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(r),
            Op::LoadReg(LoadReg { reg: src_reg, size }),
        ));
        (r, 1)
    } else {
        let (addr_ref, used) = emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
        let r = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(r),
            Op::LoadMem(LoadMem {
                addr: addr_ref,
                size,
            }),
        ));
        (r, used)
    };

    let result = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(result),
        Op::Lzcnt(Lzcnt { value: src, size }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: dst_reg,
            value: result,
            size,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::WriteFlagsCountZero(prisma_ir::WriteFlagsCountZero { src, result, size }),
    ));
    Ok(1 + used)
}

fn decode_tzcnt(
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let size = operand_size(prefixes, true);
    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;
    let dst_reg = map_reg(modrm.reg, &prefixes.rex, true);

    let (src, used) = if modrm.mod_ == 3 {
        let src_reg = map_reg(modrm.rm, &prefixes.rex, false);
        let r = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(r),
            Op::LoadReg(LoadReg { reg: src_reg, size }),
        ));
        (r, 1)
    } else {
        let (addr_ref, used) = emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
        let r = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(r),
            Op::LoadMem(LoadMem {
                addr: addr_ref,
                size,
            }),
        ));
        (r, used)
    };

    let result = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(result),
        Op::Tzcnt(Tzcnt { value: src, size }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: dst_reg,
            value: result,
            size,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::WriteFlagsCountZero(prisma_ir::WriteFlagsCountZero { src, result, size }),
    ));
    Ok(1 + used)
}

// BSF/BSR (bare 0F BC/BD), register-direct only — mirrors the C++ reference
// `decode_bsf_bsr`. BSF = trailing-zero count; BSR = the high set-bit *index*
// = (bit_width - 1) - lzcnt (NOT lzcnt itself). x86 sets ZF=1 and leaves the
// destination unchanged when the source is zero, so a CmpFlags{src,0} sets ZF
// and a Select keeps the old destination on the zero case.
fn decode_bsf_bsr(
    prefixes: &PrefixSet,
    op2: u8,
    bytes: &[u8],
    cursor: usize,
    is_bsr: bool,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let size = operand_size(prefixes, true);
    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;
    if modrm.mod_ != 3 {
        // Register-direct only, matching the C++ MVP.
        return Err(crate::DecodeError::UnsupportedOpcode(op2));
    }
    let dst_reg = map_reg(modrm.reg, &prefixes.rex, true);
    let src_reg = map_reg(modrm.rm, &prefixes.rex, false);

    let src = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(src),
        Op::LoadReg(LoadReg { reg: src_reg, size }),
    ));
    let old = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(old),
        Op::LoadReg(LoadReg { reg: dst_reg, size }),
    ));

    let count = if is_bsr {
        let lz = alloc_ref(stmts);
        stmts.push(Stmt::new(Some(lz), Op::Lzcnt(Lzcnt { value: src, size })));
        let width = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(width),
            Op::Constant(Constant {
                value: u64::from(size.bit_width() - 1),
                size,
            }),
        ));
        let idx = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(idx),
            Op::BinOp(BinOp {
                op: BinOpKind::Sub,
                lhs: width,
                rhs: lz,
                size,
            }),
        ));
        idx
    } else {
        let tz = alloc_ref(stmts);
        stmts.push(Stmt::new(Some(tz), Op::Tzcnt(Tzcnt { value: src, size })));
        tz
    };

    let zero = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(zero),
        Op::Constant(Constant { value: 0, size }),
    ));
    // CmpFlags needs a result ref (the flags value); the Select then reads NZCV
    // by condition code. Sets ZF iff src == 0.
    let flags = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(flags),
        Op::CmpFlags(CmpFlags {
            lhs: src,
            rhs: zero,
            size,
        }),
    ));
    let result = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(result),
        Op::Select(Select {
            cc: CondCode::Eq,
            true_value: old,
            false_value: count,
            size,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: dst_reg,
            value: result,
            size,
        }),
    ));
    Ok(2)
}

fn decode_bswap(
    prefixes: &PrefixSet,
    opcode: u8,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.rep.is_some() || prefixes.lock || prefixes.segment.is_some() {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }

    let size = operand_size(prefixes, true);
    if size == OpSize::I16 {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }

    let reg = map_reg(opcode & 0x07, &prefixes.rex, false);
    let src = alloc_ref(stmts);
    stmts.push(Stmt::new(Some(src), Op::LoadReg(LoadReg { reg, size })));

    let result = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(result),
        Op::Bswap(prisma_ir::Bswap { value: src, size }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg,
            value: result,
            size,
        }),
    ));
    Ok(1)
}

fn decode_three_byte_0f38(
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let opcode = *bytes.get(cursor + 1).ok_or(crate::DecodeError::Truncated)?;
    match opcode {
        0xF0 | 0xF1 if prefixes.rep == Some(0xF2) => {
            decode_crc32c(prefixes, opcode, bytes, cursor + 1, stmts)
        }
        0xF0 | 0xF1 => decode_movbe(prefixes, opcode, bytes, cursor + 1, stmts),
        _ => Err(crate::DecodeError::UnsupportedOpcode(opcode)),
    }
}

fn decode_movbe(
    prefixes: &PrefixSet,
    opcode: u8,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.rep.is_some() || prefixes.lock || prefixes.segment.is_some() {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }

    let size = operand_size(prefixes, true);
    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;
    if modrm.mod_ == 3 {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }

    let reg = map_reg(modrm.reg, &prefixes.rex, true);
    let (addr, used) = emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;

    if opcode == 0xF0 {
        let loaded = alloc_ref(stmts);
        stmts.push(Stmt::new(Some(loaded), Op::LoadMem(LoadMem { addr, size })));
        let swapped = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(swapped),
            Op::Bswap(prisma_ir::Bswap {
                value: loaded,
                size,
            }),
        ));
        stmts.push(Stmt::new(
            None,
            Op::StoreReg(StoreReg {
                reg,
                value: swapped,
                size,
            }),
        ));
    } else {
        let loaded = alloc_ref(stmts);
        stmts.push(Stmt::new(Some(loaded), Op::LoadReg(LoadReg { reg, size })));
        let swapped = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(swapped),
            Op::Bswap(prisma_ir::Bswap {
                value: loaded,
                size,
            }),
        ));
        stmts.push(Stmt::new(
            None,
            Op::StoreMem(StoreMem {
                addr,
                value: swapped,
                size,
            }),
        ));
    }

    Ok(2 + used)
}

fn decode_crc32c(
    prefixes: &PrefixSet,
    opcode: u8,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    if prefixes.rep != Some(0xF2) || prefixes.lock || prefixes.segment.is_some() {
        return Err(crate::DecodeError::UnsupportedOpcode(opcode));
    }

    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;
    let mut dst_size = OpSize::I32;
    let src_size = if opcode == 0xF0 {
        OpSize::I8
    } else if prefixes.operand_override {
        OpSize::I16
    } else if prefixes.rex.w {
        dst_size = OpSize::I64;
        OpSize::I64
    } else {
        OpSize::I32
    };

    let dst_reg = map_reg(modrm.reg, &prefixes.rex, true);
    let crc = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(crc),
        Op::LoadReg(LoadReg {
            reg: dst_reg,
            size: dst_size,
        }),
    ));

    let (data, used) = if modrm.mod_ == 3 {
        let src_reg = map_reg(modrm.rm, &prefixes.rex, false);
        let r = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(r),
            Op::LoadReg(LoadReg {
                reg: src_reg,
                size: src_size,
            }),
        ));
        (r, 1)
    } else {
        let (addr_ref, used) = emit_addr_with_size(modrm, prefixes, bytes, cursor + 1, stmts)?;
        let r = alloc_ref(stmts);
        stmts.push(Stmt::new(
            Some(r),
            Op::LoadMem(LoadMem {
                addr: addr_ref,
                size: src_size,
            }),
        ));
        (r, used)
    };

    let result = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(result),
        Op::Crc32c(prisma_ir::Crc32c {
            crc,
            data,
            data_size: src_size,
        }),
    ));
    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: dst_reg,
            value: result,
            size: dst_size,
        }),
    ));
    Ok(2 + used)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Register map from a 3-bit register index + REX extension.
fn map_reg_raw(idx: u8, rex: &RexPrefix) -> Gpr {
    let idx = idx & 0x7;
    match idx {
        0 => {
            if rex.b {
                Gpr::R8
            } else {
                Gpr::Rax
            }
        }
        1 => {
            if rex.b {
                Gpr::R9
            } else {
                Gpr::Rcx
            }
        }
        2 => {
            if rex.b {
                Gpr::R10
            } else {
                Gpr::Rdx
            }
        }
        3 => {
            if rex.b {
                Gpr::R11
            } else {
                Gpr::Rbx
            }
        }
        4 => {
            if rex.b {
                Gpr::R12
            } else {
                Gpr::Rsp
            }
        }
        5 => {
            if rex.b {
                Gpr::R13
            } else {
                Gpr::Rbp
            }
        }
        6 => {
            if rex.b {
                Gpr::R14
            } else {
                Gpr::Rsi
            }
        }
        7 => {
            if rex.b {
                Gpr::R15
            } else {
                Gpr::Rdi
            }
        }
        _ => unreachable!(),
    }
}

fn map_reg(idx: u8, rex: &RexPrefix, is_reg_field: bool) -> Gpr {
    let ext = if is_reg_field { rex.r } else { rex.b };
    let idx = idx & 0x7;
    match (idx, ext) {
        (0, false) => Gpr::Rax,
        (0, true) => Gpr::R8,
        (1, false) => Gpr::Rcx,
        (1, true) => Gpr::R9,
        (2, false) => Gpr::Rdx,
        (2, true) => Gpr::R10,
        (3, false) => Gpr::Rbx,
        (3, true) => Gpr::R11,
        (4, false) => Gpr::Rsp,
        (4, true) => Gpr::R12,
        (5, false) => Gpr::Rbp,
        (5, true) => Gpr::R13,
        (6, false) => Gpr::Rsi,
        (6, true) => Gpr::R14,
        (7, false) => Gpr::Rdi,
        (7, true) => Gpr::R15,
        _ => unreachable!(),
    }
}

fn bytes_for_modrm(
    modrm: &ModRm,
    bytes: &[u8],
    cursor: usize,
) -> Result<usize, crate::DecodeError> {
    let mut total = 1;
    if modrm.rm == 4 {
        total += 1;
        let sib_byte = *bytes.get(cursor + 1).ok_or(crate::DecodeError::Truncated)?;
        let base = sib_byte & 0x7;
        match modrm.mod_ {
            0 if base == 5 => total += 4,
            1 => total += 1,
            2 => total += 4,
            _ => {}
        }
    } else if modrm.mod_ == 0 && modrm.rm == 5 {
        total += 4;
    } else {
        match modrm.mod_ {
            1 => total += 1,
            2 => total += 4,
            _ => {}
        }
    }
    Ok(total)
}

fn emit_addr_with_size(
    modrm: ModRm,
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<(Ref, usize), crate::DecodeError> {
    emit_addr_with_size_at(modrm, prefixes, bytes, cursor, 0, stmts)
}

fn emit_addr_with_size_at(
    modrm: ModRm,
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    rip_after: u64,
    stmts: &mut Vec<Stmt>,
) -> Result<(Ref, usize), crate::DecodeError> {
    let n = bytes_for_modrm(&modrm, bytes, cursor)?;
    let sib = if modrm.mod_ != 3 && modrm.rm == 4 {
        Some(modrm::parse_sib(bytes, cursor + 1)?.0)
    } else {
        None
    };
    let disp_offset = cursor + 1 + usize::from(sib.is_some());
    let (addr, _next) = modrm::decode_addr(&modrm, sib.as_ref(), prefixes, bytes, disp_offset)?;
    let addr_ref = emit_addr_mode_at(&addr, rip_after, stmts);
    Ok((addr_ref, n))
}

fn emit_addr_mode(addr: &modrm::AddrMode, stmts: &mut Vec<Stmt>) -> Ref {
    emit_addr_mode_at(addr, 0, stmts)
}

fn emit_addr_mode_at(addr: &modrm::AddrMode, rip_after: u64, stmts: &mut Vec<Stmt>) -> Ref {
    match addr {
        modrm::AddrMode::BaseDisp { base, disp } => {
            let base_ref = emit_load_addr_reg(stmts, *base);
            emit_add_disp(stmts, base_ref, *disp)
        }
        modrm::AddrMode::Indexed {
            base,
            index,
            scale,
            disp,
        } => {
            let index_ref = emit_load_addr_reg(stmts, *index);
            let scaled_index = if *scale == 1 {
                index_ref
            } else {
                let shift_ref = emit_addr_const(stmts, u64::from(scale.trailing_zeros()));
                emit_addr_binop(stmts, BinOpKind::Shl, index_ref, shift_ref)
            };
            let base_plus_index = base.map_or(scaled_index, |base| {
                let base_ref = emit_load_addr_reg(stmts, base);
                emit_addr_binop(stmts, BinOpKind::Add, base_ref, scaled_index)
            });
            emit_add_disp(stmts, base_plus_index, *disp)
        }
        modrm::AddrMode::RipRelative { disp } => {
            emit_addr_const(stmts, add_signed_disp(rip_after, *disp))
        }
        modrm::AddrMode::Register(reg) => emit_load_addr_reg(stmts, *reg),
    }
}

fn add_signed_disp(base: u64, disp: i64) -> u64 {
    if disp >= 0 {
        base.wrapping_add(disp.cast_unsigned())
    } else {
        base.wrapping_sub(disp.unsigned_abs())
    }
}

fn emit_load_addr_reg(stmts: &mut Vec<Stmt>, reg: Gpr) -> Ref {
    let r = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(r),
        Op::LoadReg(LoadReg {
            reg,
            size: OpSize::I64,
        }),
    ));
    r
}

fn emit_addr_const(stmts: &mut Vec<Stmt>, value: u64) -> Ref {
    let r = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(r),
        Op::Constant(Constant {
            value,
            size: OpSize::I64,
        }),
    ));
    r
}

fn emit_addr_binop(stmts: &mut Vec<Stmt>, op: BinOpKind, lhs: Ref, rhs: Ref) -> Ref {
    let r = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(r),
        Op::BinOp(BinOp {
            op,
            lhs,
            rhs,
            size: OpSize::I64,
        }),
    ));
    r
}

fn emit_add_disp(stmts: &mut Vec<Stmt>, base_ref: Ref, disp: i64) -> Ref {
    if disp == 0 {
        return base_ref;
    }
    let disp_ref = emit_addr_const(stmts, disp.cast_unsigned());
    emit_addr_binop(stmts, BinOpKind::Add, base_ref, disp_ref)
}

fn alloc_ref(stmts: &mut Vec<Stmt>) -> Ref {
    stmts.len() as Ref
}

fn read_imm(bytes: &[u8], offset: usize, n: usize) -> Result<u64, crate::DecodeError> {
    let slice = bytes
        .get(offset..offset + n)
        .ok_or(crate::DecodeError::Truncated)?;
    let mut buf = [0u8; 8];
    for (i, &b) in slice.iter().enumerate() {
        buf[i] = b;
    }
    Ok(u64::from_le_bytes(buf))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use prisma_ir::{
        AluFlags, BinOp, BinOpKind, Bswap, CallReg, CallRel, CondCode, CondJumpRel, Constant,
        Cpuid, Extend, Fence, FenceKind, Gpr, JumpReg, JumpRel, LoadMem, LoadReg, Op, OpSize,
        Popcnt, Rdtsc, RetAdjusted, Return, RspAdjust, Select, Stmt, StoreMem, StoreReg, Syscall,
        Trap, TrapKind, Xgetbv,
    };

    #[test]
    fn decode_nop() {
        let d = decode_one(b"\x90", 0).unwrap();
        assert_eq!(d.stmts.len(), 0);
        assert_eq!(d.bytes_consumed, 1);

        let pause = decode_one(b"\xF3\x90", 0).unwrap();
        assert_eq!(pause.bytes_consumed, 2);
        assert!(pause.stmts.is_empty());

        assert_eq!(
            decode_one(b"\xF2\x90", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x90))
        );
    }

    #[test]
    fn decode_fwait_as_noop() {
        let d = decode_one(b"\x9B", 0).unwrap();
        assert_eq!(d.bytes_consumed, 1);
        assert!(d.stmts.is_empty());

        assert_eq!(
            decode_one(b"\xF3\x9B", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x9B))
        );
    }

    #[test]
    fn decode_cld_as_forward_string_noop() {
        let d = decode_one(b"\xFC", 0).unwrap();
        assert_eq!(d.bytes_consumed, 1);
        assert!(d.stmts.is_empty());

        assert_eq!(
            decode_one(b"\xF3\xFC", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xFC))
        );
    }

    #[test]
    fn decode_multibyte_nop_rm() {
        let d = decode_one(b"\x0F\x1F\x00", 0).unwrap();
        assert_eq!(d.bytes_consumed, 3);
        assert!(d.stmts.is_empty());

        let d = decode_one(b"\x0F\x1F\x44\x00\x00", 0).unwrap();
        assert_eq!(d.bytes_consumed, 5);
        assert!(d.stmts.is_empty());

        assert_eq!(
            decode_one(b"\x0F\x1F\xC8", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x1F))
        );
    }

    #[test]
    fn decode_prefetch_consumes_memory_operand_as_noop() {
        let simple = decode_one(b"\x0F\x18\x08", 0).unwrap();
        assert_eq!(simple.bytes_consumed, 3);
        assert!(simple.stmts.is_empty());

        let sib_disp = decode_one(b"\x0F\x18\x94\x88\x78\x56\x34\x12", 0).unwrap();
        assert_eq!(sib_disp.bytes_consumed, 8);
        assert!(sib_disp.stmts.is_empty());

        assert_eq!(
            decode_one(b"\x0F\x18\x20", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x18))
        );
        assert_eq!(
            decode_one(b"\x0F\x18\xC0", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x18))
        );
    }

    #[test]
    fn decode_xgetbv_writes_eax_edx_from_xcr_value() {
        let d = decode_one(b"\x0F\x01\xD0", 0).unwrap();
        assert_eq!(d.bytes_consumed, 3);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(Some(0), Op::Xgetbv(Xgetbv)),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 0,
                        size: OpSize::I32,
                    }),
                ),
                Stmt::new(
                    Some(2),
                    Op::Constant(Constant {
                        value: 32,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(3),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Shr,
                        lhs: 0,
                        rhs: 2,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rdx,
                        value: 3,
                        size: OpSize::I32,
                    }),
                ),
            ]
        );

        assert_eq!(
            decode_one(b"\x0F\x01\xD2", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x01))
        );
        assert_eq!(
            decode_one(b"\x66\x0F\x01\xD0", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x01))
        );
    }

    #[test]
    fn decode_syscall_emits_syscall_op() {
        let d = decode_one(b"\x0F\x05", 0).unwrap();
        assert_eq!(d.bytes_consumed, 2);
        assert_eq!(d.stmts, vec![Stmt::new(None, Op::Syscall(Syscall))]);

        assert_eq!(
            decode_one(b"\xF3\x0F\x05", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x05))
        );
    }

    #[test]
    fn decode_ud2_emits_sigill_trap() {
        let d = decode_one(b"\x0F\x0B", 0).unwrap();
        assert_eq!(d.bytes_consumed, 2);
        assert_eq!(
            d.stmts,
            vec![Stmt::new(
                None,
                Op::Trap(Trap {
                    kind: TrapKind::Sigill,
                }),
            )]
        );

        assert_eq!(
            decode_one(b"\xF3\x0F\x0B", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x0B))
        );
    }

    #[test]
    fn decode_invd_wbinvd_emit_sigill_traps() {
        for bytes in [
            b"\x0F\x08".as_slice(),
            b"\x0F\x09".as_slice(),
            b"\xF3\x0F\x09".as_slice(),
        ] {
            let d = decode_one(bytes, 0).unwrap();
            assert_eq!(d.bytes_consumed, bytes.len());
            assert_eq!(
                d.stmts,
                vec![Stmt::new(
                    None,
                    Op::Trap(Trap {
                        kind: TrapKind::Sigill,
                    }),
                )]
            );
        }

        assert_eq!(
            decode_one(b"\xF3\x0F\x08", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x08))
        );
    }

    #[test]
    fn decode_int3_and_hlt_emit_traps() {
        let int3 = decode_one(b"\xCC", 0).unwrap();
        assert_eq!(int3.bytes_consumed, 1);
        assert_eq!(
            int3.stmts,
            vec![Stmt::new(
                None,
                Op::Trap(Trap {
                    kind: TrapKind::Sigtrap,
                }),
            )]
        );

        let hlt = decode_one(b"\xF4", 0).unwrap();
        assert_eq!(hlt.bytes_consumed, 1);
        assert_eq!(
            hlt.stmts,
            vec![Stmt::new(
                None,
                Op::Trap(Trap {
                    kind: TrapKind::Sigill,
                }),
            )]
        );

        assert_eq!(
            decode_one(b"\xF3\xCC", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xCC))
        );
    }

    #[test]
    fn decode_icebp_emits_sigtrap() {
        let d = decode_one(b"\xF1", 0).unwrap();
        assert_eq!(d.bytes_consumed, 1);
        assert_eq!(
            d.stmts,
            vec![Stmt::new(
                None,
                Op::Trap(Trap {
                    kind: TrapKind::Sigtrap,
                }),
            )]
        );

        assert_eq!(
            decode_one(b"\xF3\xF1", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xF1))
        );
    }

    #[test]
    fn decode_cli_sti_emit_sigill_traps() {
        for bytes in [b"\xFA".as_slice(), b"\xFB".as_slice()] {
            let d = decode_one(bytes, 0).unwrap();
            assert_eq!(d.bytes_consumed, 1);
            assert_eq!(
                d.stmts,
                vec![Stmt::new(
                    None,
                    Op::Trap(Trap {
                        kind: TrapKind::Sigill,
                    }),
                )]
            );
        }

        assert_eq!(
            decode_one(b"\xF3\xFA", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xFA))
        );
    }

    #[test]
    fn decode_int_imm8_emits_sigtrap_and_consumes_vector() {
        let d = decode_one(b"\xCD\x80", 0).unwrap();
        assert_eq!(d.bytes_consumed, 2);
        assert_eq!(
            d.stmts,
            vec![Stmt::new(
                None,
                Op::Trap(Trap {
                    kind: TrapKind::Sigtrap,
                }),
            )]
        );

        assert_eq!(decode_one(b"\xCD", 0), Err(crate::DecodeError::Truncated));
        assert_eq!(
            decode_one(b"\xF3\xCD\x80", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xCD))
        );
    }

    #[test]
    fn decode_legacy_long_mode_invalids_emit_sigill_traps() {
        for bytes in [
            b"\x06".as_slice(),
            b"\x07".as_slice(),
            b"\x0E".as_slice(),
            b"\x16".as_slice(),
            b"\x17".as_slice(),
            b"\x1E".as_slice(),
            b"\x1F".as_slice(),
            b"\x27".as_slice(),
            b"\x2F".as_slice(),
            b"\x37".as_slice(),
            b"\x3F".as_slice(),
            b"\x60".as_slice(),
            b"\x61".as_slice(),
            b"\x9A".as_slice(),
            b"\xCE".as_slice(),
            b"\xD6".as_slice(),
            b"\xEA".as_slice(),
            b"\xD4\x0A".as_slice(),
            b"\xD5\x0A".as_slice(),
        ] {
            let d = decode_one(bytes, 0).unwrap();
            assert_eq!(d.bytes_consumed, bytes.len());
            assert_eq!(
                d.stmts,
                vec![Stmt::new(
                    None,
                    Op::Trap(Trap {
                        kind: TrapKind::Sigill,
                    }),
                )]
            );
        }

        assert_eq!(decode_one(b"\xD4", 0), Err(crate::DecodeError::Truncated));
        assert_eq!(
            decode_one(b"\x66\x27", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x27))
        );
        assert_eq!(
            decode_one(b"\xF3\xD5\x0A", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xD5))
        );
    }

    #[test]
    fn decode_io_instructions_emit_sigill_traps() {
        for bytes in [
            b"\xE4\x20".as_slice(),
            b"\xE5\x20".as_slice(),
            b"\xE6\x20".as_slice(),
            b"\xE7\x20".as_slice(),
        ] {
            let d = decode_one(bytes, 0).unwrap();
            assert_eq!(d.bytes_consumed, 2);
            assert_eq!(
                d.stmts,
                vec![Stmt::new(
                    None,
                    Op::Trap(Trap {
                        kind: TrapKind::Sigill,
                    }),
                )]
            );
        }

        for bytes in [
            b"\xEC".as_slice(),
            b"\xED".as_slice(),
            b"\xEE".as_slice(),
            b"\xEF".as_slice(),
        ] {
            let d = decode_one(bytes, 0).unwrap();
            assert_eq!(d.bytes_consumed, 1);
            assert_eq!(
                d.stmts,
                vec![Stmt::new(
                    None,
                    Op::Trap(Trap {
                        kind: TrapKind::Sigill,
                    }),
                )]
            );
        }

        assert_eq!(decode_one(b"\xE4", 0), Err(crate::DecodeError::Truncated));
        assert_eq!(
            decode_one(b"\xF3\xEC", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xEC))
        );
    }

    #[test]
    fn decode_cpuid_emits_cpuid_op() {
        let d = decode_one(b"\x0F\xA2", 0).unwrap();
        assert_eq!(d.bytes_consumed, 2);
        assert_eq!(d.stmts, vec![Stmt::new(None, Op::Cpuid(Cpuid))]);

        assert_eq!(
            decode_one(b"\xF3\x0F\xA2", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xA2))
        );
    }

    #[test]
    fn decode_prefetchw_consumes_memory_operand_as_noop() {
        let simple = decode_one(b"\x0F\x0D\x08", 0).unwrap();
        assert_eq!(simple.bytes_consumed, 3);
        assert!(simple.stmts.is_empty());

        let sib_disp = decode_one(b"\x0F\x0D\x84\x88\x78\x56\x34\x12", 0).unwrap();
        assert_eq!(sib_disp.bytes_consumed, 8);
        assert!(sib_disp.stmts.is_empty());

        assert_eq!(
            decode_one(b"\x0F\x0D\x10", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x0D))
        );
        assert_eq!(
            decode_one(b"\x0F\x0D\xC8", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x0D))
        );
    }

    #[test]
    fn decode_emms_and_femms_as_noops() {
        let emms = decode_one(b"\x0F\x77", 0).unwrap();
        assert_eq!(emms.bytes_consumed, 2);
        assert!(emms.stmts.is_empty());

        let femms = decode_one(b"\x0F\x0E", 0).unwrap();
        assert_eq!(femms.bytes_consumed, 2);
        assert!(femms.stmts.is_empty());

        assert_eq!(
            decode_one(b"\x66\x0F\x77", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x77))
        );
        assert_eq!(
            decode_one(b"\xF3\x0F\x0E", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x0E))
        );
    }

    #[test]
    fn decode_endbr_instructions_as_noops() {
        let endbr64 = decode_one(b"\xF3\x0F\x1E\xFA", 0).unwrap();
        assert_eq!(endbr64.bytes_consumed, 4);
        assert!(endbr64.stmts.is_empty());

        let endbr32 = decode_one(b"\xF3\x0F\x1E\xFB", 0).unwrap();
        assert_eq!(endbr32.bytes_consumed, 4);
        assert!(endbr32.stmts.is_empty());

        assert_eq!(
            decode_one(b"\x0F\x1E\xFA", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x1E))
        );
        assert_eq!(
            decode_one(b"\xF3\x0F\x1E\xFC", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x1E))
        );
        assert_eq!(
            decode_one(b"\xF3\x0F\x1E", 0),
            Err(crate::DecodeError::Truncated)
        );
    }

    #[test]
    fn decode_fence_instructions_emit_fence_ops() {
        for (bytes, kind) in [
            (b"\x0F\xAE\xE8".as_slice(), FenceKind::Lfence),
            (b"\x0F\xAE\xF0".as_slice(), FenceKind::Mfence),
            (b"\x0F\xAE\xF8".as_slice(), FenceKind::Sfence),
        ] {
            let d = decode_one(bytes, 0).unwrap();
            assert_eq!(d.bytes_consumed, 3);
            assert_eq!(d.stmts, vec![Stmt::new(None, Op::Fence(Fence { kind }))]);
        }

        assert_eq!(
            decode_one(b"\x0F\xAE\xE9", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xAE))
        );
        assert_eq!(
            decode_one(b"\x0F\xAE\x20", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xAE))
        );
    }

    #[test]
    fn decode_rdtsc_writes_eax_edx_from_counter() {
        let d = decode_one(b"\x0F\x31", 0).unwrap();
        assert_eq!(d.bytes_consumed, 2);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(Some(0), Op::Rdtsc(Rdtsc)),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 0,
                        size: OpSize::I32,
                    }),
                ),
                Stmt::new(
                    Some(2),
                    Op::Constant(Constant {
                        value: 32,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(3),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Shr,
                        lhs: 0,
                        rhs: 2,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rdx,
                        value: 3,
                        size: OpSize::I32,
                    }),
                ),
            ]
        );

        assert_eq!(
            decode_one(b"\xF3\x0F\x31", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x31))
        );
    }

    #[test]
    fn decode_clflush_consumes_memory_operand_as_noop() {
        let simple = decode_one(b"\x0F\xAE\x38", 0).unwrap();
        assert_eq!(simple.bytes_consumed, 3);
        assert!(simple.stmts.is_empty());

        let sib_disp = decode_one(b"\x0F\xAE\xBC\x88\x78\x56\x34\x12", 0).unwrap();
        assert_eq!(sib_disp.bytes_consumed, 8);
        assert!(sib_disp.stmts.is_empty());

        let clwb = decode_one(b"\x66\x0F\xAE\x30", 0).unwrap();
        assert_eq!(clwb.bytes_consumed, 4);
        assert!(clwb.stmts.is_empty());

        let clflushopt = decode_one(b"\x66\x0F\xAE\x38", 0).unwrap();
        assert_eq!(clflushopt.bytes_consumed, 4);
        assert!(clflushopt.stmts.is_empty());

        assert_eq!(
            decode_one(b"\x0F\xAE\x30", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xAE))
        );
        assert_eq!(
            decode_one(b"\xF3\x0F\xAE\x38", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xAE))
        );
    }

    #[test]
    fn decode_mov_ri_rax() {
        // mov rax, 0x42 (48 B8 42 00 00 00 00 00 00 00)
        let d = decode_one(b"\x48\xB8\x42\x00\x00\x00\x00\x00\x00\x00", 0).unwrap();
        assert_eq!(d.bytes_consumed, 10);
        assert_eq!(
            d.stmts,
            vec![
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
                        reg: Gpr::Rax,
                        value: 0,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_mov_r8_i8() {
        let d = decode_one(b"\xB1\x7F", 0).unwrap();
        assert_eq!(d.bytes_consumed, 2);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::Constant(Constant {
                        value: 0x7F,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rcx,
                        value: 0,
                        size: OpSize::I8,
                    }),
                ),
            ]
        );

        let d = decode_one(b"\x41\xB1\x80", 0).unwrap();
        assert_eq!(d.bytes_consumed, 3);
        assert!(matches!(
            d.stmts.last().unwrap().op,
            Op::StoreReg(StoreReg {
                reg: Gpr::R9,
                size: OpSize::I8,
                ..
            })
        ));
    }

    #[test]
    fn decode_mov_ri_operand_override_uses_imm16() {
        let d = decode_one(b"\x66\xB8\x34\x12", 0).unwrap();
        assert_eq!(d.bytes_consumed, 4);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::Constant(Constant {
                        value: 0x1234,
                        size: OpSize::I16,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 0,
                        size: OpSize::I16,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_mov_ri_rex_b_extends_opcode_register() {
        let d = decode_one(b"\x49\xB8\x78\x56\x34\x12\x00\x00\x00\x00", 0).unwrap();
        assert_eq!(d.bytes_consumed, 10);
        assert_eq!(
            d.stmts[1],
            Stmt::new(
                None,
                Op::StoreReg(StoreReg {
                    reg: Gpr::R8,
                    value: 0,
                    size: OpSize::I64,
                }),
            )
        );
    }

    #[test]
    fn decode_mov_ri_imm64_covers_all_sixteen_registers() {
        // B8+rd with REX.W (and REX.B for the high eight) must reach every GPR.
        const REGS: [Gpr; 16] = [
            Gpr::Rax,
            Gpr::Rcx,
            Gpr::Rdx,
            Gpr::Rbx,
            Gpr::Rsp,
            Gpr::Rbp,
            Gpr::Rsi,
            Gpr::Rdi,
            Gpr::R8,
            Gpr::R9,
            Gpr::R10,
            Gpr::R11,
            Gpr::R12,
            Gpr::R13,
            Gpr::R14,
            Gpr::R15,
        ];
        for (idx, expected) in REGS.into_iter().enumerate() {
            // REX prefix: 0x48 (W) for low 8, 0x49 (W|B) for high 8.
            let rex = if idx < 8 { 0x48u8 } else { 0x49u8 };
            let opcode = 0xB8u8 + (idx as u8 & 0x7);
            let bytes = [rex, opcode, 0x42, 0, 0, 0, 0, 0, 0, 0];
            let d = decode_one(&bytes, 0).unwrap();
            assert_eq!(d.bytes_consumed, 10, "reg idx {idx}");
            assert_eq!(
                d.stmts.last().unwrap().op,
                Op::StoreReg(StoreReg {
                    reg: expected,
                    value: 0,
                    size: OpSize::I64,
                }),
                "reg idx {idx}",
            );
        }
    }

    #[test]
    fn decode_mov_ri_imm32_without_rex_w_uses_i32_store() {
        // B8+rd, no REX.W, no 0x66 => MOV r32, imm32 (zero-extends to 64).
        let d = decode_one(b"\xB8\xEF\xBE\xAD\xDE", 0).unwrap();
        assert_eq!(d.bytes_consumed, 5);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::Constant(Constant {
                        value: 0xDEAD_BEEF,
                        size: OpSize::I32,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 0,
                        size: OpSize::I32,
                    }),
                ),
            ]
        );

        // REX.B selects r13d; still an I32 store.
        let d = decode_one(b"\x41\xBD\x01\x00\x00\x00", 0).unwrap();
        assert_eq!(d.bytes_consumed, 6);
        assert_eq!(
            d.stmts.last().unwrap().op,
            Op::StoreReg(StoreReg {
                reg: Gpr::R13,
                value: 0,
                size: OpSize::I32,
            }),
        );
    }

    #[test]
    fn decode_mov_rm32_imm32_register_form() {
        // C7 /0 without REX.W => MOV r/m32, imm32 (no sign-extension to 64).
        let d = decode_one(b"\xC7\xC1\x78\x56\x34\x12", 0).unwrap();
        assert_eq!(d.bytes_consumed, 6);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::Constant(Constant {
                        value: 0x1234_5678,
                        size: OpSize::I32,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rcx,
                        value: 0,
                        size: OpSize::I32,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_mov_rm_imm_register_sign_extends_i64_imm32() {
        let d = decode_one(b"\x48\xC7\xC0\xFF\xFF\xFF\xFF", 0).unwrap();
        assert_eq!(d.bytes_consumed, 7);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::Constant(Constant {
                        value: u64::MAX,
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
            ]
        );
    }

    #[test]
    fn decode_mov_rm_imm_memory_byte_form() {
        let d = decode_one(b"\xC6\x00\x7F", 0).unwrap();
        assert_eq!(d.bytes_consumed, 3);
        assert!(d.stmts.iter().any(|stmt| matches!(
            stmt.op,
            Op::Constant(Constant {
                value: 0x7F,
                size: OpSize::I8
            })
        )));
        assert!(matches!(
            d.stmts.last().unwrap().op,
            Op::StoreMem(StoreMem {
                size: OpSize::I8,
                ..
            })
        ));
    }

    #[test]
    fn decode_mov_rm_imm_rejects_nonzero_group_subop() {
        let r = decode_one(b"\xC7\xC8\x01\x00\x00\x00", 0);
        assert_eq!(r.unwrap_err(), crate::DecodeError::UnsupportedOpcode(0xC7));
    }

    #[test]
    fn decode_accumulator_add_immediate_stores_result() {
        let d = decode_one(b"\x48\x05\x34\x12\x00\x00", 0).unwrap();
        assert_eq!(d.bytes_consumed, 6);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
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
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 2,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_accumulator_cmp_immediate_sets_flags_only() {
        let d = decode_one(b"\x3D\x78\x56\x34\x12", 0).unwrap();
        assert_eq!(d.bytes_consumed, 5);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I32,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::Constant(Constant {
                        value: 0x1234_5678,
                        size: OpSize::I32,
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
            ]
        );
    }

    #[test]
    fn decode_accumulator_test_immediate_sets_alu_flags() {
        let d = decode_one(b"\xA8\x80", 0).unwrap();
        assert_eq!(d.bytes_consumed, 2);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::Constant(Constant {
                        value: 0x80,
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
            ]
        );
    }

    #[test]
    fn decode_mov_rm_r_reg() {
        // mov rax, rcx (48 89 C8)
        let d = decode_one(b"\x48\x89\xC8", 0).unwrap();
        assert_eq!(d.bytes_consumed, 3);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rcx,
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
            ]
        );
    }

    #[test]
    fn decode_mov_rm8_r8_reg() {
        let d = decode_one(b"\x88\xC8", 0).unwrap();
        assert_eq!(d.bytes_consumed, 2);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rcx,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 0,
                        size: OpSize::I8,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_mov_moffs_to_acc_and_acc_to_moffs() {
        let load = decode_one(b"\x48\xA1\x78\x56\x34\x12\x00\x00\x00\x00", 0).unwrap();
        assert_eq!(load.bytes_consumed, 10);
        assert_eq!(
            load.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::Constant(Constant {
                        value: 0x1234_5678,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadMem(LoadMem {
                        addr: 0,
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
            ]
        );

        let store = decode_one(b"\x67\xA2\x78\x56\x34\x12", 0).unwrap();
        assert_eq!(store.bytes_consumed, 6);
        assert_eq!(
            store.stmts[0],
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x1234_5678,
                    size: OpSize::I64,
                }),
            )
        );
        assert!(matches!(
            store.stmts.last().unwrap().op,
            Op::StoreMem(StoreMem {
                addr: 0,
                value: 1,
                size: OpSize::I8,
            })
        ));
    }

    #[test]
    fn decode_xchg_accumulator_opcode() {
        let d = decode_one(b"\x48\x91", 0).unwrap();
        assert_eq!(d.bytes_consumed, 2);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rcx,
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
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rcx,
                        value: 0,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );

        let d = decode_one(b"\x41\x91", 0).unwrap();
        assert!(matches!(
            d.stmts[1].op,
            Op::LoadReg(LoadReg { reg: Gpr::R9, .. })
        ));
    }

    #[test]
    fn decode_xchg_byte_register_form() {
        let d = decode_one(b"\x86\xC8", 0).unwrap();
        assert_eq!(d.bytes_consumed, 2);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rcx,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rcx,
                        value: 1,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 0,
                        size: OpSize::I8,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_xchg_memory_form() {
        let d = decode_one(b"\x48\x87\x08", 0).unwrap();
        assert_eq!(d.bytes_consumed, 3);
        assert!(matches!(
            d.stmts.last().unwrap().op,
            Op::StoreMem(StoreMem {
                addr: 1,
                value: 0,
                size: OpSize::I64,
            })
        ));
    }

    #[test]
    fn decode_xlat_loads_byte_from_rbx_plus_al_into_al() {
        let d = decode_one(b"\xD7", 0).unwrap();
        assert_eq!(d.bytes_consumed, 1);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::Extend(Extend {
                        value: 0,
                        from_size: OpSize::I8,
                        to_size: OpSize::I64,
                        is_signed: false,
                    }),
                ),
                Stmt::new(
                    Some(2),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rbx,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(3),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Add,
                        lhs: 2,
                        rhs: 1,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(4),
                    Op::LoadMem(LoadMem {
                        addr: 3,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 4,
                        size: OpSize::I8,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_fnop_as_noop() {
        let d = decode_one(b"\xD9\xD0", 0).unwrap();
        assert_eq!(d.bytes_consumed, 2);
        assert!(d.stmts.is_empty());

        assert_eq!(decode_one(b"\xD9", 0), Err(crate::DecodeError::Truncated));
        assert_eq!(
            decode_one(b"\xD9\xD1", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xD9))
        );
        assert_eq!(
            decode_one(b"\x66\xD9\xD0", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xD9))
        );
    }

    #[test]
    fn decode_fnclex_and_fninit_as_noops() {
        let fnclex = decode_one(b"\xDB\xE2", 0).unwrap();
        assert_eq!(fnclex.bytes_consumed, 2);
        assert!(fnclex.stmts.is_empty());

        let fninit = decode_one(b"\xDB\xE3", 0).unwrap();
        assert_eq!(fninit.bytes_consumed, 2);
        assert!(fninit.stmts.is_empty());

        assert_eq!(decode_one(b"\xDB", 0), Err(crate::DecodeError::Truncated));
        assert_eq!(
            decode_one(b"\xDB\xE4", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xDB))
        );
        assert_eq!(
            decode_one(b"\x66\xDB\xE3", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xDB))
        );
    }

    #[test]
    fn decode_lodsb_loads_byte_from_rsi_and_advances_rsi() {
        let d = decode_one(b"\xAC", 0).unwrap();
        assert_eq!(d.bytes_consumed, 1);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rsi,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadMem(LoadMem {
                        addr: 0,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 1,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    Some(3),
                    Op::Constant(Constant {
                        value: 1,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(4),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Add,
                        lhs: 0,
                        rhs: 3,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rsi,
                        value: 4,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_stosb_stores_al_to_rdi_and_advances_rdi() {
        let d = decode_one(b"\xAA", 0).unwrap();
        assert_eq!(d.bytes_consumed, 1);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rdi,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I8,
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
                    Some(3),
                    Op::Constant(Constant {
                        value: 1,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(4),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Add,
                        lhs: 0,
                        rhs: 3,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rdi,
                        value: 4,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_scasb_compares_al_with_rdi_byte_and_advances_rdi() {
        let d = decode_one(b"\xAE", 0).unwrap();
        assert_eq!(d.bytes_consumed, 1);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rdi,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
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
                    Some(3),
                    Op::CmpFlags(CmpFlags {
                        lhs: 1,
                        rhs: 2,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    Some(4),
                    Op::Constant(Constant {
                        value: 1,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(5),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Add,
                        lhs: 0,
                        rhs: 4,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rdi,
                        value: 5,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_cmpsb_compares_rsi_byte_with_rdi_byte_and_advances_both() {
        let d = decode_one(b"\xA6", 0).unwrap();
        assert_eq!(d.bytes_consumed, 1);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rsi,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rdi,
                        size: OpSize::I64,
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
                    Some(3),
                    Op::LoadMem(LoadMem {
                        addr: 1,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    Some(4),
                    Op::CmpFlags(CmpFlags {
                        lhs: 2,
                        rhs: 3,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    Some(5),
                    Op::Constant(Constant {
                        value: 1,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(6),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Add,
                        lhs: 0,
                        rhs: 5,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rsi,
                        value: 6,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(8),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Add,
                        lhs: 1,
                        rhs: 5,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rdi,
                        value: 8,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_movsb_copies_rsi_byte_to_rdi_byte_and_advances_both() {
        let d = decode_one(b"\xA4", 0).unwrap();
        assert_eq!(d.bytes_consumed, 1);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rsi,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rdi,
                        size: OpSize::I64,
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
                        addr: 1,
                        value: 2,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    Some(4),
                    Op::Constant(Constant {
                        value: 1,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(5),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Add,
                        lhs: 0,
                        rhs: 4,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rsi,
                        value: 5,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(7),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Add,
                        lhs: 1,
                        rhs: 4,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rdi,
                        value: 7,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_string_full_width_forms_use_operand_size_and_step() {
        let movsq = decode_one(b"\x48\xA5", 0).unwrap();
        assert_eq!(movsq.bytes_consumed, 2);
        assert!(matches!(
            movsq.stmts[2].op,
            Op::LoadMem(LoadMem {
                size: OpSize::I64,
                ..
            })
        ));
        assert!(matches!(
            movsq.stmts[4].op,
            Op::Constant(Constant {
                value: 8,
                size: OpSize::I64,
            })
        ));

        let lodsw = decode_one(b"\x66\xAD", 0).unwrap();
        assert_eq!(lodsw.bytes_consumed, 2);
        assert!(matches!(
            lodsw.stmts[1].op,
            Op::LoadMem(LoadMem {
                size: OpSize::I16,
                ..
            })
        ));
        assert!(matches!(
            lodsw.stmts[3].op,
            Op::Constant(Constant {
                value: 2,
                size: OpSize::I64,
            })
        ));

        let stosq = decode_one(b"\x48\xAB", 0).unwrap();
        assert_eq!(stosq.bytes_consumed, 2);
        assert!(matches!(
            stosq.stmts[2].op,
            Op::StoreMem(StoreMem {
                size: OpSize::I64,
                ..
            })
        ));

        let scasw = decode_one(b"\x66\xAF", 0).unwrap();
        assert_eq!(scasw.bytes_consumed, 2);
        assert!(matches!(
            scasw.stmts[3].op,
            Op::CmpFlags(CmpFlags {
                size: OpSize::I16,
                ..
            })
        ));

        let cmpsd = decode_one(b"\xA7", 0).unwrap();
        assert_eq!(cmpsd.bytes_consumed, 1);
        assert!(matches!(
            cmpsd.stmts[4].op,
            Op::CmpFlags(CmpFlags {
                size: OpSize::I32,
                ..
            })
        ));
    }

    #[test]
    fn decode_sign_extend_accumulator_opcode() {
        let cdqe = decode_one(b"\x48\x98", 0).unwrap();
        assert_eq!(cdqe.bytes_consumed, 2);
        assert_eq!(
            cdqe.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I32,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::Extend(Extend {
                        value: 0,
                        from_size: OpSize::I32,
                        to_size: OpSize::I64,
                        is_signed: true,
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
            ]
        );

        let cwde = decode_one(b"\x98", 0).unwrap();
        assert!(matches!(
            cwde.stmts[1].op,
            Op::Extend(Extend {
                from_size: OpSize::I16,
                to_size: OpSize::I32,
                ..
            })
        ));

        let cbw = decode_one(b"\x66\x98", 0).unwrap();
        assert!(matches!(
            cbw.stmts[1].op,
            Op::Extend(Extend {
                from_size: OpSize::I8,
                to_size: OpSize::I16,
                ..
            })
        ));
    }

    #[test]
    fn decode_sign_extend_accumulator_to_dx_opcode() {
        let cqo = decode_one(b"\x48\x99", 0).unwrap();
        assert_eq!(cqo.bytes_consumed, 2);
        assert_eq!(
            cqo.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::Constant(Constant {
                        value: 63,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(2),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Sar,
                        lhs: 0,
                        rhs: 1,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rdx,
                        value: 2,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );

        let cdq = decode_one(b"\x99", 0).unwrap();
        assert!(matches!(
            cdq.stmts[2].op,
            Op::BinOp(BinOp {
                op: BinOpKind::Sar,
                size: OpSize::I32,
                ..
            })
        ));

        let cwd = decode_one(b"\x66\x99", 0).unwrap();
        assert!(matches!(
            cwd.stmts[2].op,
            Op::BinOp(BinOp {
                op: BinOpKind::Sar,
                size: OpSize::I16,
                ..
            })
        ));
    }

    #[test]
    fn decode_mov_r_rm_reg_honors_rex_extensions() {
        // mov r9, r8 (4D 8B C8): REX.W + REX.R extends reg, REX.B extends r/m.
        let d = decode_one(b"\x4D\x8B\xC8", 0).unwrap();
        assert_eq!(d.bytes_consumed, 3);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::R8,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::R9,
                        value: 0,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_mov_r8_rm8_reg_and_memory() {
        let reg = decode_one(b"\x8A\xC1", 0).unwrap();
        assert_eq!(reg.bytes_consumed, 2);
        assert_eq!(
            reg.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rcx,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 0,
                        size: OpSize::I8,
                    }),
                ),
            ]
        );

        let mem = decode_one(b"\x8A\x08", 0).unwrap();
        assert_eq!(mem.bytes_consumed, 2);
        assert!(mem.stmts.iter().any(|stmt| matches!(
            stmt.op,
            Op::LoadMem(LoadMem {
                size: OpSize::I8,
                ..
            })
        )));
        assert!(matches!(
            mem.stmts.last().unwrap().op,
            Op::StoreReg(StoreReg {
                reg: Gpr::Rcx,
                size: OpSize::I8,
                ..
            })
        ));
    }

    #[test]
    fn decode_lea_stores_effective_address_without_loading_memory() {
        let d = decode_one(b"\x48\x8D\x41\x08", 0).unwrap();
        assert_eq!(d.bytes_consumed, 4);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rcx,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::Constant(Constant {
                        value: 8,
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
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 2,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_lea_supports_default_32_bit_destination() {
        let d = decode_one(b"\x8D\x41\x08", 0).unwrap();
        assert_eq!(d.bytes_consumed, 3);
        assert_eq!(
            d.stmts.last().unwrap(),
            &Stmt::new(
                None,
                Op::StoreReg(StoreReg {
                    reg: Gpr::Rax,
                    value: 2,
                    size: OpSize::I32,
                }),
            )
        );
    }

    #[test]
    fn decode_lea_rejects_16_bit_destination_and_register_form() {
        let operand_override = decode_one(b"\x66\x8D\x41\x08", 0);
        assert_eq!(
            operand_override.unwrap_err(),
            crate::DecodeError::UnsupportedOpcode(0x8D)
        );
        let reg_form = decode_one(b"\x48\x8D\xC1", 0);
        assert_eq!(
            reg_form.unwrap_err(),
            crate::DecodeError::UnsupportedOpcode(0x8D)
        );
    }

    #[test]
    fn decode_add_r_rm() {
        // add rax, rcx (48 01 C8)
        let d = decode_one(b"\x48\x01\xC8", 0).unwrap();
        assert_eq!(d.bytes_consumed, 3);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rcx,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
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
                        reg: Gpr::Rax,
                        value: 2,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_alu_byte_register_forms() {
        let rm_r = decode_one(b"\x00\xC8", 0).unwrap();
        assert_eq!(rm_r.bytes_consumed, 2);
        assert_eq!(
            rm_r.stmts[2],
            Stmt::new(
                Some(2),
                Op::BinOp(BinOp {
                    op: BinOpKind::Add,
                    lhs: 1,
                    rhs: 0,
                    size: OpSize::I8,
                }),
            )
        );
        assert!(matches!(
            rm_r.stmts.last().unwrap().op,
            Op::StoreReg(StoreReg {
                reg: Gpr::Rax,
                size: OpSize::I8,
                ..
            })
        ));

        let r_rm = decode_one(b"\x02\xC1", 0).unwrap();
        assert_eq!(r_rm.bytes_consumed, 2);
        assert_eq!(
            r_rm.stmts[2],
            Stmt::new(
                Some(2),
                Op::BinOp(BinOp {
                    op: BinOpKind::Add,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I8,
                }),
            )
        );
        assert!(matches!(
            r_rm.stmts.last().unwrap().op,
            Op::StoreReg(StoreReg {
                reg: Gpr::Rax,
                size: OpSize::I8,
                ..
            })
        ));
    }

    #[test]
    fn decode_logical_and_sub_byte_forms() {
        let cases = [
            (b"\x08\xC8".as_slice(), BinOpKind::Or),
            (b"\x0A\xC1".as_slice(), BinOpKind::Or),
            (b"\x20\xC8".as_slice(), BinOpKind::And),
            (b"\x22\xC1".as_slice(), BinOpKind::And),
            (b"\x28\xC8".as_slice(), BinOpKind::Sub),
            (b"\x2A\xC1".as_slice(), BinOpKind::Sub),
            (b"\x30\xC8".as_slice(), BinOpKind::Xor),
            (b"\x32\xC1".as_slice(), BinOpKind::Xor),
        ];

        for (bytes, op) in cases {
            let d = decode_one(bytes, 0).unwrap();
            assert_eq!(d.bytes_consumed, 2);
            assert!(d.stmts.iter().any(|stmt| matches!(
                stmt.op,
                Op::BinOp(BinOp {
                    op: actual,
                    size: OpSize::I8,
                    ..
                }) if actual == op
            )));
        }
    }

    #[test]
    fn decode_logical_and_cmp_r_rm_full_width_forms() {
        let cases = [
            (b"\x48\x0B\xC1".as_slice(), BinOpKind::Or),
            (b"\x48\x23\xC1".as_slice(), BinOpKind::And),
            (b"\x48\x33\xC1".as_slice(), BinOpKind::Xor),
        ];

        for (bytes, op) in cases {
            let d = decode_one(bytes, 0).unwrap();
            assert_eq!(d.bytes_consumed, 3);
            assert_eq!(
                d.stmts[2],
                Stmt::new(
                    Some(2),
                    Op::BinOp(BinOp {
                        op,
                        lhs: 0,
                        rhs: 1,
                        size: OpSize::I64,
                    }),
                )
            );
            assert!(matches!(
                d.stmts.last().unwrap().op,
                Op::StoreReg(StoreReg {
                    reg: Gpr::Rax,
                    size: OpSize::I64,
                    ..
                })
            ));
        }

        let cmp = decode_one(b"\x48\x3B\xC1", 0).unwrap();
        assert_eq!(cmp.bytes_consumed, 3);
        assert!(matches!(
            cmp.stmts.last().unwrap().op,
            Op::CmpFlags(CmpFlags {
                lhs: 0,
                rhs: 1,
                size: OpSize::I64,
            })
        ));
    }

    #[test]
    fn decode_cmp_byte_forms() {
        let rm_r = decode_one(b"\x38\xC8", 0).unwrap();
        assert_eq!(rm_r.bytes_consumed, 2);
        assert!(matches!(
            rm_r.stmts.last().unwrap().op,
            Op::CmpFlags(CmpFlags {
                lhs: 1,
                rhs: 0,
                size: OpSize::I8,
            })
        ));

        let r_rm = decode_one(b"\x3A\xC1", 0).unwrap();
        assert_eq!(r_rm.bytes_consumed, 2);
        assert!(matches!(
            r_rm.stmts.last().unwrap().op,
            Op::CmpFlags(CmpFlags {
                lhs: 0,
                rhs: 1,
                size: OpSize::I8,
            })
        ));
    }

    #[test]
    fn decode_add_rm_imm8_sign_extended() {
        // add rax, 0x10 (48 83 C0 10)
        let d = decode_one(b"\x48\x83\xC0\x10", 0).unwrap();
        assert_eq!(d.bytes_consumed, 4);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::Constant(Constant {
                        value: 0x10,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
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
                        reg: Gpr::Rax,
                        value: 2,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_sub_rm_imm8() {
        // sub rax, 0x10 (48 83 E8 10)
        let d = decode_one(b"\x48\x83\xE8\x10", 0).unwrap();
        assert_eq!(d.bytes_consumed, 4);
        assert_eq!(
            d.stmts[2],
            Stmt::new(
                Some(2),
                Op::BinOp(BinOp {
                    op: BinOpKind::Sub,
                    lhs: 1,
                    rhs: 0,
                    size: OpSize::I64,
                }),
            )
        );
    }

    #[test]
    fn decode_logical_rm_imm8() {
        let cases = [
            (b"\x48\x83\xC8\x10".as_slice(), BinOpKind::Or),
            (b"\x48\x83\xE0\x10".as_slice(), BinOpKind::And),
            (b"\x48\x83\xF0\x10".as_slice(), BinOpKind::Xor),
        ];

        for (bytes, op) in cases {
            let d = decode_one(bytes, 0).unwrap();
            assert_eq!(d.bytes_consumed, 4);
            assert_eq!(
                d.stmts[2],
                Stmt::new(
                    Some(2),
                    Op::BinOp(BinOp {
                        op,
                        lhs: 1,
                        rhs: 0,
                        size: OpSize::I64,
                    }),
                )
            );
        }
    }

    #[test]
    fn decode_adc_sbb_rm_imm8_placeholders() {
        let cases = [
            (b"\x48\x83\xD0\x10".as_slice(), BinOpKind::Add),
            (b"\x48\x83\xD8\x10".as_slice(), BinOpKind::Sub),
        ];

        for (bytes, op) in cases {
            let d = decode_one(bytes, 0).unwrap();
            assert_eq!(d.bytes_consumed, 4);
            assert_eq!(
                d.stmts[2],
                Stmt::new(
                    Some(2),
                    Op::BinOp(BinOp {
                        op,
                        lhs: 1,
                        rhs: 0,
                        size: OpSize::I64,
                    }),
                )
            );
        }
    }

    #[test]
    fn decode_adc_sbb_accumulator_immediate_placeholders() {
        let cases = [
            (b"\x14\x10".as_slice(), BinOpKind::Add, OpSize::I8, 2),
            (
                b"\x48\x15\x10\x00\x00\x00".as_slice(),
                BinOpKind::Add,
                OpSize::I64,
                6,
            ),
            (b"\x1C\x10".as_slice(), BinOpKind::Sub, OpSize::I8, 2),
            (
                b"\x48\x1D\x10\x00\x00\x00".as_slice(),
                BinOpKind::Sub,
                OpSize::I64,
                6,
            ),
        ];

        for (bytes, op, size, consumed) in cases {
            let d = decode_one(bytes, 0).unwrap();
            assert_eq!(d.bytes_consumed, consumed);
            assert_eq!(
                d.stmts[2],
                Stmt::new(
                    Some(2),
                    Op::BinOp(BinOp {
                        op,
                        lhs: 0,
                        rhs: 1,
                        size,
                    }),
                )
            );
            assert!(matches!(
                d.stmts.last().unwrap().op,
                Op::StoreReg(StoreReg {
                    reg: Gpr::Rax,
                    value: 2,
                    size: actual,
                }) if actual == size
            ));
        }
    }

    #[test]
    fn decode_adc_sbb_register_forms_placeholders() {
        let cases = [
            (b"\x10\xC8".as_slice(), BinOpKind::Add, OpSize::I8),
            (b"\x48\x11\xC8".as_slice(), BinOpKind::Add, OpSize::I64),
            (b"\x12\xC1".as_slice(), BinOpKind::Add, OpSize::I8),
            (b"\x48\x13\xC1".as_slice(), BinOpKind::Add, OpSize::I64),
            (b"\x18\xC8".as_slice(), BinOpKind::Sub, OpSize::I8),
            (b"\x48\x19\xC8".as_slice(), BinOpKind::Sub, OpSize::I64),
            (b"\x1A\xC1".as_slice(), BinOpKind::Sub, OpSize::I8),
            (b"\x48\x1B\xC1".as_slice(), BinOpKind::Sub, OpSize::I64),
        ];

        for (bytes, op, size) in cases {
            let d = decode_one(bytes, 0).unwrap();
            assert_eq!(d.bytes_consumed, bytes.len());
            assert!(matches!(
                d.stmts.iter().find_map(|stmt| match &stmt.op {
                    Op::BinOp(binop) => Some(binop),
                    _ => None,
                }),
                Some(BinOp {
                    op: actual,
                    size: actual_size,
                    ..
                }) if *actual == op && *actual_size == size
            ));
        }
    }

    #[test]
    fn decode_cmp_rm_imm8_sets_flags_without_store() {
        let d = decode_one(b"\x48\x83\xF8\x10", 0).unwrap();
        assert_eq!(d.bytes_consumed, 4);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::Constant(Constant {
                        value: 0x10,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(2),
                    Op::CmpFlags(CmpFlags {
                        lhs: 1,
                        rhs: 0,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_cmp_rm_r_register_form_uses_cmp_flags() {
        let d = decode_one(b"\x48\x39\xC8", 0).unwrap();
        assert_eq!(d.bytes_consumed, 3);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rcx,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(2),
                    Op::CmpFlags(CmpFlags {
                        lhs: 1,
                        rhs: 0,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_group1_imm8_honors_rm_register_and_rex_b() {
        let add = decode_one(b"\x48\x83\xC3\x10", 0).unwrap();
        assert_eq!(add.bytes_consumed, 4);
        assert_eq!(
            add.stmts[1],
            Stmt::new(
                Some(1),
                Op::LoadReg(LoadReg {
                    reg: Gpr::Rbx,
                    size: OpSize::I64,
                }),
            )
        );
        assert_eq!(
            add.stmts[3],
            Stmt::new(
                None,
                Op::StoreReg(StoreReg {
                    reg: Gpr::Rbx,
                    value: 2,
                    size: OpSize::I64,
                }),
            )
        );

        let cmp = decode_one(b"\x49\x83\xFB\x10", 0).unwrap();
        assert_eq!(cmp.bytes_consumed, 4);
        assert_eq!(
            cmp.stmts[1],
            Stmt::new(
                Some(1),
                Op::LoadReg(LoadReg {
                    reg: Gpr::R11,
                    size: OpSize::I64,
                }),
            )
        );
        assert_eq!(
            cmp.stmts[2],
            Stmt::new(
                Some(2),
                Op::CmpFlags(CmpFlags {
                    lhs: 1,
                    rhs: 0,
                    size: OpSize::I64,
                }),
            )
        );
    }

    #[test]
    fn decode_group1_80_byte_immediate_register() {
        let d = decode_one(b"\x80\xC0\x7F", 0).unwrap();
        assert_eq!(d.bytes_consumed, 3);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::Constant(Constant {
                        value: 0x7F,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    Some(2),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Add,
                        lhs: 1,
                        rhs: 0,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 2,
                        size: OpSize::I8,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_group1_81_sign_extends_i64_immediate() {
        let d = decode_one(b"\x48\x81\xC0\xFF\xFF\xFF\xFF", 0).unwrap();
        assert_eq!(d.bytes_consumed, 7);
        assert_eq!(
            d.stmts[0],
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: u64::MAX,
                    size: OpSize::I64,
                }),
            )
        );
    }

    #[test]
    fn decode_group1_81_cmp_memory_full_immediate() {
        let d = decode_one(b"\x81\x38\x78\x56\x34\x12", 0).unwrap();
        assert_eq!(d.bytes_consumed, 6);
        assert!(d.stmts.iter().any(|stmt| matches!(
            stmt.op,
            Op::CmpFlags(CmpFlags {
                size: OpSize::I32,
                ..
            })
        )));
    }

    #[test]
    fn decode_group1_81_register_direct_matrix() {
        // 81 /digit id over a register: mirrors the 0x83 group but with a full
        // imm32. /2 ADC and /3 SBB lower to ADD/SUB placeholders, matching the
        // C++ reference's carry/borrow stance.
        let cases: [(u8, BinOpKind); 7] = [
            (0, BinOpKind::Add), // ADD
            (1, BinOpKind::Or),  // OR
            (2, BinOpKind::Add), // ADC -> ADD placeholder
            (3, BinOpKind::Sub), // SBB -> SUB placeholder
            (4, BinOpKind::And), // AND
            (5, BinOpKind::Sub), // SUB
            (6, BinOpKind::Xor), // XOR
        ];
        for (digit, op) in cases {
            // REX.W, 0x81, modrm = 11 <digit> 000 (rax), imm32 = 0x1234.
            let modrm = 0xC0 | (digit << 3);
            let bytes = [0x48, 0x81, modrm, 0x34, 0x12, 0x00, 0x00];
            let d = decode_one(&bytes, 0).unwrap();
            assert_eq!(d.bytes_consumed, 7, "digit {digit}");
            // Emission order: imm Constant (ref 0), dst LoadReg (ref 1),
            // BinOp{lhs: dst, rhs: imm} (ref 2), StoreReg.
            assert_eq!(
                d.stmts[0],
                Stmt::new(
                    Some(0),
                    Op::Constant(Constant {
                        value: 0x1234,
                        size: OpSize::I64,
                    }),
                ),
                "digit {digit} imm32"
            );
            assert_eq!(
                d.stmts[1],
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I64,
                    }),
                ),
                "digit {digit} loads dst"
            );
            assert_eq!(
                d.stmts[2],
                Stmt::new(
                    Some(2),
                    Op::BinOp(BinOp {
                        op,
                        lhs: 1,
                        rhs: 0,
                        size: OpSize::I64,
                    }),
                ),
                "digit {digit} binop"
            );
            assert_eq!(
                d.stmts[3],
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 2,
                        size: OpSize::I64,
                    }),
                ),
                "digit {digit} stores result"
            );
        }
    }

    #[test]
    fn decode_group1_81_cmp_register_sets_flags_only() {
        // cmp rbx, 0x1234  (48 81 FB 34 12 00 00 -> /7 over rbx)
        let d = decode_one(b"\x48\x81\xFB\x34\x12\x00\x00", 0).unwrap();
        assert_eq!(d.bytes_consumed, 7);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::Constant(Constant {
                        value: 0x1234,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rbx,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(2),
                    Op::CmpFlags(CmpFlags {
                        lhs: 1,
                        rhs: 0,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_accumulator_immediate_opcode_matrix() {
        // opcode -> (is_byte, BinOpKind) for the ALU accumulator-immediate set,
        // excluding CMP (0x3C/0x3D) and TEST (0xA8/0xA9) which take other paths.
        let cases: [(u8, bool, BinOpKind); 12] = [
            (0x04, true, BinOpKind::Add),  // ADD al, imm8
            (0x05, false, BinOpKind::Add), // ADD eax, imm32
            (0x0C, true, BinOpKind::Or),   // OR  al, imm8
            (0x0D, false, BinOpKind::Or),  // OR  eax, imm32
            (0x14, true, BinOpKind::Add),  // ADC al  -> ADD placeholder
            (0x15, false, BinOpKind::Add), // ADC eax -> ADD placeholder
            (0x1C, true, BinOpKind::Sub),  // SBB al  -> SUB placeholder
            (0x1D, false, BinOpKind::Sub), // SBB eax -> SUB placeholder
            (0x24, true, BinOpKind::And),  // AND al, imm8
            (0x2C, true, BinOpKind::Sub),  // SUB al, imm8
            (0x34, true, BinOpKind::Xor),  // XOR al, imm8
            (0x35, false, BinOpKind::Xor), // XOR eax, imm32
        ];
        for (opcode, is_byte, op) in cases {
            let (bytes, consumed, size): (Vec<u8>, usize, OpSize) = if is_byte {
                (vec![opcode, 0x10], 2, OpSize::I8)
            } else {
                (vec![opcode, 0x34, 0x12, 0x00, 0x00], 5, OpSize::I32)
            };
            let d = decode_one(&bytes, 0).unwrap();
            assert_eq!(d.bytes_consumed, consumed, "opcode {opcode:#x}");
            assert_eq!(
                d.stmts[0],
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size
                    })
                ),
                "opcode {opcode:#x} loads rax"
            );
            assert_eq!(
                d.stmts[2],
                Stmt::new(
                    Some(2),
                    Op::BinOp(BinOp {
                        op,
                        lhs: 0,
                        rhs: 1,
                        size,
                    }),
                ),
                "opcode {opcode:#x} binop"
            );
            assert_eq!(
                d.stmts.last().unwrap(),
                &Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 2,
                        size,
                    }),
                ),
                "opcode {opcode:#x} stores rax"
            );
        }
    }

    #[test]
    fn decode_group2_imm8_shift_register() {
        let d = decode_one(b"\x48\xC1\xE0\x03", 0).unwrap();
        assert_eq!(d.bytes_consumed, 4);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::Constant(Constant {
                        value: 3,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(2),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Shl,
                        lhs: 1,
                        rhs: 0,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 2,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_group2_byte_imm8_shift_register() {
        let d = decode_one(b"\xC0\xE0\x03", 0).unwrap();
        assert_eq!(d.bytes_consumed, 3);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::Constant(Constant {
                        value: 3,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    Some(2),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Shl,
                        lhs: 1,
                        rhs: 0,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 2,
                        size: OpSize::I8,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_group2_imm8_memory_ror() {
        let d = decode_one(b"\x48\xC1\x0B\x04", 0).unwrap();
        assert_eq!(d.bytes_consumed, 4);
        assert!(d.stmts.iter().any(|stmt| matches!(
            stmt.op,
            Op::BinOp(BinOp {
                op: BinOpKind::Ror,
                ..
            })
        )));
        assert!(matches!(d.stmts.last().unwrap().op, Op::StoreMem(_)));
    }

    #[test]
    fn decode_group2_imm8_rejects_unsupported_subop() {
        let r = decode_one(b"\xC1\xD0\x01", 0);
        assert_eq!(r.unwrap_err(), crate::DecodeError::UnsupportedOpcode(0xC1));
    }

    #[test]
    fn decode_group2_one_count_shift_register() {
        let d = decode_one(b"\x48\xD1\xE8", 0).unwrap();
        assert_eq!(d.bytes_consumed, 3);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::Constant(Constant {
                        value: 1,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(2),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Shr,
                        lhs: 1,
                        rhs: 0,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 2,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_group2_cl_count_shift_register() {
        let d = decode_one(b"\x48\xD3\xE0", 0).unwrap();
        assert_eq!(d.bytes_consumed, 3);
        assert_eq!(
            d.stmts[0],
            Stmt::new(
                Some(0),
                Op::LoadReg(LoadReg {
                    reg: Gpr::Rcx,
                    size: OpSize::I8,
                }),
            )
        );
        assert!(matches!(
            d.stmts[2].op,
            Op::BinOp(BinOp {
                op: BinOpKind::Shl,
                rhs: 0,
                ..
            })
        ));
    }

    #[test]
    fn decode_group1_imm8_supports_memory_operands() {
        let add = decode_one(b"\x48\x83\x03\x10", 0).unwrap();
        assert_eq!(add.bytes_consumed, 4);
        assert_eq!(
            add.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::Constant(Constant {
                        value: 0x10,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rbx,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(2),
                    Op::LoadMem(LoadMem {
                        addr: 1,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(3),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Add,
                        lhs: 2,
                        rhs: 0,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreMem(StoreMem {
                        addr: 1,
                        value: 3,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );

        let cmp = decode_one(b"\x48\x83\x3B\x10", 0).unwrap();
        assert_eq!(cmp.bytes_consumed, 4);
        assert_eq!(
            cmp.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::Constant(Constant {
                        value: 0x10,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rbx,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(2),
                    Op::LoadMem(LoadMem {
                        addr: 1,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(3),
                    Op::CmpFlags(CmpFlags {
                        lhs: 2,
                        rhs: 0,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_group1_imm8_supports_logical_memory_operands() {
        for (bytes, op) in [
            (&b"\x48\x83\x0B\x10"[..], BinOpKind::Or),
            (&b"\x48\x83\x23\x10"[..], BinOpKind::And),
            (&b"\x48\x83\x33\x10"[..], BinOpKind::Xor),
        ] {
            let d = decode_one(bytes, 0).unwrap();
            assert_eq!(d.bytes_consumed, 4);
            assert_eq!(
                d.stmts,
                vec![
                    Stmt::new(
                        Some(0),
                        Op::Constant(Constant {
                            value: 0x10,
                            size: OpSize::I64,
                        }),
                    ),
                    Stmt::new(
                        Some(1),
                        Op::LoadReg(LoadReg {
                            reg: Gpr::Rbx,
                            size: OpSize::I64,
                        }),
                    ),
                    Stmt::new(
                        Some(2),
                        Op::LoadMem(LoadMem {
                            addr: 1,
                            size: OpSize::I64,
                        }),
                    ),
                    Stmt::new(
                        Some(3),
                        Op::BinOp(BinOp {
                            op,
                            lhs: 2,
                            rhs: 0,
                            size: OpSize::I64,
                        }),
                    ),
                    Stmt::new(
                        None,
                        Op::StoreMem(StoreMem {
                            addr: 1,
                            value: 3,
                            size: OpSize::I64,
                        }),
                    ),
                ]
            );
        }
    }

    #[test]
    fn decode_group1_imm8_sign_extends_memory_disp8_and_imm8() {
        let add = decode_one(b"\x48\x83\x43\xF8\xF0", 0).unwrap();
        assert_eq!(add.bytes_consumed, 5);
        assert_eq!(
            add.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::Constant(Constant {
                        value: 0xffff_ffff_ffff_fff0,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rbx,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(2),
                    Op::Constant(Constant {
                        value: 0xffff_ffff_ffff_fff8,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(3),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Add,
                        lhs: 1,
                        rhs: 2,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(4),
                    Op::LoadMem(LoadMem {
                        addr: 3,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(5),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Add,
                        lhs: 4,
                        rhs: 0,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreMem(StoreMem {
                        addr: 3,
                        value: 5,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_group1_imm8_supports_sib_disp_memory_operands() {
        let add = decode_one(b"\x48\x83\x44\x88\x7F\x10", 0).unwrap();
        assert_eq!(add.bytes_consumed, 6);
        assert_eq!(
            add.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::Constant(Constant {
                        value: 0x10,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rcx,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(2),
                    Op::Constant(Constant {
                        value: 2,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(3),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Shl,
                        lhs: 1,
                        rhs: 2,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(4),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(5),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Add,
                        lhs: 4,
                        rhs: 3,
                        size: OpSize::I64,
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
                    Op::BinOp(BinOp {
                        op: BinOpKind::Add,
                        lhs: 5,
                        rhs: 6,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(8),
                    Op::LoadMem(LoadMem {
                        addr: 7,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(9),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Add,
                        lhs: 8,
                        rhs: 0,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreMem(StoreMem {
                        addr: 7,
                        value: 9,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_group1_imm8_supports_sib_cmp_memory_operands() {
        let cmp = decode_one(b"\x48\x83\x7C\x88\x7F\x10", 0).unwrap();
        assert_eq!(cmp.bytes_consumed, 6);
        assert_eq!(cmp.stmts.len(), 10);
        assert_eq!(
            cmp.stmts[8],
            Stmt::new(
                Some(8),
                Op::LoadMem(LoadMem {
                    addr: 7,
                    size: OpSize::I64,
                }),
            )
        );
        assert_eq!(
            cmp.stmts[9],
            Stmt::new(
                Some(9),
                Op::CmpFlags(CmpFlags {
                    lhs: 8,
                    rhs: 0,
                    size: OpSize::I64,
                }),
            )
        );
    }

    #[test]
    fn decode_group1_imm8_supports_disp32_memory_operands() {
        let add = decode_one(b"\x48\x83\x83\x20\x00\x00\x00\x10", 0).unwrap();
        assert_eq!(add.bytes_consumed, 8);
        assert_eq!(
            add.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::Constant(Constant {
                        value: 0x10,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rbx,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(2),
                    Op::Constant(Constant {
                        value: 0x20,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(3),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Add,
                        lhs: 1,
                        rhs: 2,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(4),
                    Op::LoadMem(LoadMem {
                        addr: 3,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(5),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Add,
                        lhs: 4,
                        rhs: 0,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreMem(StoreMem {
                        addr: 3,
                        value: 5,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_group1_imm8_supports_rex_x_b_sib_memory_operands() {
        let add = decode_one(b"\x4B\x83\x44\x88\x20\x10", 0).unwrap();
        assert_eq!(add.bytes_consumed, 6);
        assert_eq!(
            add.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::Constant(Constant {
                        value: 0x10,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::R9,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(2),
                    Op::Constant(Constant {
                        value: 2,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(3),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Shl,
                        lhs: 1,
                        rhs: 2,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(4),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::R8,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(5),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Add,
                        lhs: 4,
                        rhs: 3,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(6),
                    Op::Constant(Constant {
                        value: 0x20,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(7),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Add,
                        lhs: 5,
                        rhs: 6,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(8),
                    Op::LoadMem(LoadMem {
                        addr: 7,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(9),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Add,
                        lhs: 8,
                        rhs: 0,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreMem(StoreMem {
                        addr: 7,
                        value: 9,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_group1_imm8_supports_rip_relative_memory_operands() {
        let add = decode_one_at(b"\x48\x83\x05\x34\x12\x00\x00\x10", 0, 0x1000).unwrap();
        assert_eq!(add.bytes_consumed, 8);
        assert_eq!(
            add.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::Constant(Constant {
                        value: 0x10,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::Constant(Constant {
                        value: 0x223c,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(2),
                    Op::LoadMem(LoadMem {
                        addr: 1,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(3),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Add,
                        lhs: 2,
                        rhs: 0,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreMem(StoreMem {
                        addr: 1,
                        value: 3,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );

        for (bytes, op) in [
            (
                &b"\x48\x83\x0D\x34\x12\x00\x00\x10"[..],
                Some(BinOpKind::Or),
            ),
            (
                &b"\x48\x83\x25\x34\x12\x00\x00\x10"[..],
                Some(BinOpKind::And),
            ),
            (
                &b"\x48\x83\x35\x34\x12\x00\x00\x10"[..],
                Some(BinOpKind::Xor),
            ),
            (&b"\x48\x83\x3D\x34\x12\x00\x00\x10"[..], None),
        ] {
            let d = decode_one_at(bytes, 0, 0x1000).unwrap();
            assert_eq!(d.bytes_consumed, 8);
            assert_eq!(
                d.stmts[1],
                Stmt::new(
                    Some(1),
                    Op::Constant(Constant {
                        value: 0x223c,
                        size: OpSize::I64,
                    }),
                )
            );
            if let Some(op) = op {
                assert_eq!(
                    d.stmts[3],
                    Stmt::new(
                        Some(3),
                        Op::BinOp(BinOp {
                            op,
                            lhs: 2,
                            rhs: 0,
                            size: OpSize::I64,
                        }),
                    )
                );
            } else {
                assert_eq!(
                    d.stmts[3],
                    Stmt::new(
                        Some(3),
                        Op::CmpFlags(CmpFlags {
                            lhs: 2,
                            rhs: 0,
                            size: OpSize::I64,
                        }),
                    )
                );
            }
        }
    }

    #[test]
    fn decode_add_r_rm_sib_disp8_consumes_full_modrm_path() {
        // add rax, qword [rax + rcx*4 + 0x7f]
        let d = decode_one(b"\x48\x03\x44\x88\x7F", 0).unwrap();
        assert_eq!(d.bytes_consumed, 5);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rcx,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(2),
                    Op::Constant(Constant {
                        value: 2,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(3),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Shl,
                        lhs: 1,
                        rhs: 2,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(4),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(5),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Add,
                        lhs: 4,
                        rhs: 3,
                        size: OpSize::I64,
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
                    Op::BinOp(BinOp {
                        op: BinOpKind::Add,
                        lhs: 5,
                        rhs: 6,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(8),
                    Op::LoadMem(LoadMem {
                        addr: 7,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(9),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Add,
                        lhs: 0,
                        rhs: 8,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 9,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_push_pop() {
        // push rax (50), pop rbx (5B)
        let push = decode_one(b"\x50", 0).unwrap();
        let pop = decode_one(b"\x5B", 0).unwrap();
        assert_eq!(push.bytes_consumed, 1);
        assert_eq!(pop.bytes_consumed, 1);
        assert_eq!(
            push.stmts[0],
            Stmt::new(
                Some(0),
                Op::LoadReg(LoadReg {
                    reg: Gpr::Rax,
                    size: OpSize::I64,
                }),
            )
        );
        assert_eq!(
            push.stmts[1],
            Stmt::new(None, Op::RspAdjust(RspAdjust { delta_bytes: -8 }))
        );
        assert_eq!(
            pop.stmts[2],
            Stmt::new(
                None,
                Op::StoreReg(StoreReg {
                    reg: Gpr::Rbx,
                    value: 1,
                    size: OpSize::I64,
                }),
            )
        );
    }

    #[test]
    fn decode_pop_rm() {
        let reg = decode_one(b"\x8F\xC0", 0).unwrap();
        assert_eq!(reg.bytes_consumed, 2);
        assert_eq!(
            reg.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rsp,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadMem(LoadMem {
                        addr: 0,
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
                Stmt::new(None, Op::RspAdjust(RspAdjust { delta_bytes: 8 })),
            ]
        );

        let mem = decode_one(b"\x8F\x00", 0).unwrap();
        assert_eq!(mem.bytes_consumed, 2);
        assert!(matches!(
            mem.stmts[mem.stmts.len() - 2].op,
            Op::StoreMem(StoreMem {
                value: 1,
                size: OpSize::I64,
                ..
            })
        ));
        assert_eq!(
            decode_one(b"\x8F\xC8", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x8F))
        );
    }

    #[test]
    fn decode_push_pop_honor_rex_b() {
        let push = decode_one(b"\x41\x50", 0).unwrap();
        let pop = decode_one(b"\x41\x58", 0).unwrap();
        assert_eq!(push.bytes_consumed, 2);
        assert_eq!(pop.bytes_consumed, 2);
        assert_eq!(
            push.stmts[0],
            Stmt::new(
                Some(0),
                Op::LoadReg(LoadReg {
                    reg: Gpr::R8,
                    size: OpSize::I64,
                }),
            )
        );
        assert_eq!(
            pop.stmts[2],
            Stmt::new(
                None,
                Op::StoreReg(StoreReg {
                    reg: Gpr::R8,
                    value: 1,
                    size: OpSize::I64,
                }),
            )
        );
    }

    #[test]
    fn decode_push_imm8_sign_extends_and_stores_on_stack() {
        let d = decode_one(b"\x6A\xFF", 0).unwrap();
        assert_eq!(d.bytes_consumed, 2);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::Constant(Constant {
                        value: u64::MAX,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(None, Op::RspAdjust(RspAdjust { delta_bytes: -8 })),
                Stmt::new(
                    Some(2),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rsp,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreMem(StoreMem {
                        addr: 2,
                        value: 0,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_push_imm32_sign_extends_and_stores_on_stack() {
        let d = decode_one(b"\x68\x34\x12\x00\x00", 0).unwrap();
        assert_eq!(d.bytes_consumed, 5);
        assert_eq!(
            d.stmts[0],
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 0x1234,
                    size: OpSize::I64,
                }),
            )
        );
        assert!(matches!(d.stmts.last().unwrap().op, Op::StoreMem(_)));
    }

    #[test]
    fn decode_movsx() {
        // movsx rax, byte [rcx] (48 0F BE 01)
        let d = decode_one(b"\x48\x0F\xBE\x01", 0).unwrap();
        assert_eq!(d.bytes_consumed, 4);
        assert_eq!(
            d.stmts[1],
            Stmt::new(
                Some(1),
                Op::LoadMem(LoadMem {
                    addr: 0,
                    size: OpSize::I8,
                }),
            )
        );
        assert_eq!(
            d.stmts[2],
            Stmt::new(
                Some(2),
                Op::Extend(Extend {
                    value: 1,
                    from_size: OpSize::I8,
                    to_size: OpSize::I64,
                    is_signed: true,
                }),
            )
        );
    }

    #[test]
    fn decode_movzx_without_rex_w_extends_to_i32() {
        let d = decode_one(b"\x0F\xB6\xC1", 0).unwrap();
        assert_eq!(d.bytes_consumed, 3);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rcx,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::Extend(Extend {
                        value: 0,
                        from_size: OpSize::I8,
                        to_size: OpSize::I32,
                        is_signed: false,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 1,
                        size: OpSize::I32,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_movsx_without_rex_w_extends_to_i32() {
        let d = decode_one(b"\x0F\xBE\xC1", 0).unwrap();
        assert_eq!(d.bytes_consumed, 3);
        assert_eq!(
            d.stmts[1],
            Stmt::new(
                Some(1),
                Op::Extend(Extend {
                    value: 0,
                    from_size: OpSize::I8,
                    to_size: OpSize::I32,
                    is_signed: true,
                }),
            )
        );
        assert_eq!(
            d.stmts[2],
            Stmt::new(
                None,
                Op::StoreReg(StoreReg {
                    reg: Gpr::Rax,
                    value: 1,
                    size: OpSize::I32,
                }),
            )
        );
    }

    #[test]
    fn decode_imul_r_rm_register_form() {
        let d = decode_one(b"\x48\x0F\xAF\xC1", 0).unwrap();
        assert_eq!(d.bytes_consumed, 4);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rcx,
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
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 2,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_xadd_register_form_placeholder() {
        let d = decode_one(b"\x48\x0F\xC1\xC8", 0).unwrap();
        assert_eq!(d.bytes_consumed, 4);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rcx,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
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
                        reg: Gpr::Rcx,
                        value: 1,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 2,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_movnti_stores_register_to_memory() {
        let d = decode_one(b"\x48\x0F\xC3\x08", 0).unwrap();
        assert_eq!(d.bytes_consumed, 4);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rcx,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreMem(StoreMem {
                        addr: 1,
                        value: 0,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );

        let d = decode_one(b"\x0F\xC3\x08", 0).unwrap();
        assert_eq!(d.bytes_consumed, 3);
        assert!(matches!(
            d.stmts.last().unwrap().op,
            Op::StoreMem(StoreMem {
                size: OpSize::I32,
                ..
            })
        ));

        assert_eq!(
            decode_one(b"\x0F\xC3\xC8", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xC3))
        );
    }

    #[test]
    fn decode_imul_immediate_forms() {
        let imm8 = decode_one(b"\x48\x6B\xC1\xF0", 0).unwrap();
        assert_eq!(imm8.bytes_consumed, 4);
        assert_eq!(
            imm8.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rcx,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::Constant(Constant {
                        value: 0xFFFF_FFFF_FFFF_FFF0,
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
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 2,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );

        let imm32 = decode_one(b"\x48\x69\xC1\x34\x12\x00\x00", 0).unwrap();
        assert_eq!(imm32.bytes_consumed, 7);
        assert_eq!(
            imm32.stmts[1],
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 0x1234,
                    size: OpSize::I64,
                }),
            )
        );
    }

    #[test]
    fn decode_movsx_mem_disp32_consumes_modrm_displacement() {
        let d = decode_one(b"\x48\x0F\xBE\x85\x78\x56\x34\x12", 0).unwrap();
        assert_eq!(d.bytes_consumed, 8);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rbp,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::Constant(Constant {
                        value: 0x1234_5678,
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
                Stmt::new(
                    Some(3),
                    Op::LoadMem(LoadMem {
                        addr: 2,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    Some(4),
                    Op::Extend(Extend {
                        value: 3,
                        from_size: OpSize::I8,
                        to_size: OpSize::I64,
                        is_signed: true,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 4,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_movsxd_register_form_sign_extends_i32_to_i64() {
        let d = decode_one(b"\x48\x63\xC1", 0).unwrap();
        assert_eq!(d.bytes_consumed, 3);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rcx,
                        size: OpSize::I32,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::Extend(Extend {
                        value: 0,
                        from_size: OpSize::I32,
                        to_size: OpSize::I64,
                        is_signed: true,
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
            ]
        );
    }

    #[test]
    fn decode_movsxd_without_rex_w_copies_i32_destination() {
        let d = decode_one(b"\x63\xC1", 0).unwrap();
        assert_eq!(d.bytes_consumed, 2);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rcx,
                        size: OpSize::I32,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 0,
                        size: OpSize::I32,
                    }),
                ),
            ]
        );

        let arpl = decode_one(b"\x66\x63\xC1", 0).unwrap();
        assert_eq!(arpl.bytes_consumed, 3);
        assert_eq!(
            arpl.stmts,
            vec![Stmt::new(
                None,
                Op::Trap(Trap {
                    kind: TrapKind::Sigill,
                }),
            )]
        );
    }

    #[test]
    fn decode_cmov_register_form_selects_between_source_and_old_dest() {
        let d = decode_one(b"\x48\x0F\x44\xC1", 0).unwrap();
        assert_eq!(d.bytes_consumed, 4);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rcx,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(2),
                    Op::Select(Select {
                        cc: CondCode::Eq,
                        true_value: 1,
                        false_value: 0,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 2,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_cmov_memory_form_uses_operand_size() {
        let d = decode_one(b"\x66\x0F\x45\x08", 0).unwrap();
        assert_eq!(d.bytes_consumed, 4);
        assert!(d.stmts.iter().any(|stmt| matches!(
            stmt.op,
            Op::LoadMem(LoadMem {
                size: OpSize::I16,
                ..
            })
        )));
        assert!(d.stmts.iter().any(|stmt| matches!(
            stmt.op,
            Op::Select(Select {
                cc: CondCode::Ne,
                size: OpSize::I16,
                ..
            })
        )));
    }

    #[test]
    fn decode_setcc_register_form_stores_selected_byte() {
        let d = decode_one(b"\x0F\x94\xC0", 0).unwrap();
        assert_eq!(d.bytes_consumed, 3);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::Constant(Constant {
                        value: 1,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::Constant(Constant {
                        value: 0,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    Some(2),
                    Op::Select(Select {
                        cc: CondCode::Eq,
                        true_value: 0,
                        false_value: 1,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 2,
                        size: OpSize::I8,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_setcc_memory_form_stores_selected_byte() {
        let d = decode_one(b"\x0F\x95\x08", 0).unwrap();
        assert_eq!(d.bytes_consumed, 3);
        assert!(d.stmts.iter().any(|stmt| matches!(
            stmt.op,
            Op::Select(Select {
                cc: CondCode::Ne,
                size: OpSize::I8,
                ..
            })
        )));
        assert!(matches!(
            d.stmts.last().unwrap().op,
            Op::StoreMem(StoreMem {
                size: OpSize::I8,
                ..
            })
        ));
    }

    #[test]
    fn decode_popcnt_requires_f3_prefix() {
        let r = decode_one(b"\x48\x0F\xB8\xC1", 0);
        assert_eq!(r.unwrap_err(), crate::DecodeError::UnsupportedOpcode(0xB8));
    }

    #[test]
    fn decode_popcnt_with_f3_prefix() {
        let d = decode_one(b"\xF3\x48\x0F\xB8\xC1", 0).unwrap();
        assert_eq!(d.bytes_consumed, 5);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rcx,
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
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 1,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::WriteFlagsPopcnt(prisma_ir::WriteFlagsPopcnt {
                        src: 0,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_lzcnt_tzcnt_write_exact_count_zero_flags() {
        let lzcnt = decode_one(b"\xF3\x48\x0F\xBD\xC1", 0).unwrap();
        assert_eq!(lzcnt.bytes_consumed, 5);
        assert!(matches!(lzcnt.stmts[1].op, Op::Lzcnt(_)));
        assert_eq!(
            lzcnt.stmts.last().unwrap(),
            &Stmt::new(
                None,
                Op::WriteFlagsCountZero(prisma_ir::WriteFlagsCountZero {
                    src: 0,
                    result: 1,
                    size: OpSize::I64,
                }),
            )
        );

        let tzcnt = decode_one(b"\xF3\x48\x0F\xBC\xC1", 0).unwrap();
        assert_eq!(tzcnt.bytes_consumed, 5);
        assert!(matches!(tzcnt.stmts[1].op, Op::Tzcnt(_)));
        assert_eq!(
            tzcnt.stmts.last().unwrap(),
            &Stmt::new(
                None,
                Op::WriteFlagsCountZero(prisma_ir::WriteFlagsCountZero {
                    src: 0,
                    result: 1,
                    size: OpSize::I64,
                }),
            )
        );
    }

    #[test]
    fn decode_bsf_bare_is_tzcnt_with_zero_select() {
        // bsf rax, rcx  (48 0F BC C1) — bare (no F3) => BSF, not TZCNT.
        let d = decode_one(b"\x48\x0F\xBC\xC1", 0).unwrap();
        assert_eq!(d.bytes_consumed, 4);
        // src load, old-dst load, tzcnt, zero const, cmpflags, select, store.
        assert_eq!(
            d.stmts[0],
            Stmt::new(
                Some(0),
                Op::LoadReg(LoadReg {
                    reg: Gpr::Rcx,
                    size: OpSize::I64,
                }),
            )
        );
        assert!(matches!(d.stmts[2].op, Op::Tzcnt(Tzcnt { value: 0, .. })));
        assert!(matches!(
            d.stmts.iter().find_map(|s| match &s.op {
                Op::Select(sel) => Some(sel.cc),
                _ => None,
            }),
            Some(CondCode::Eq)
        ));
        assert!(matches!(
            d.stmts.last().unwrap().op,
            Op::StoreReg(StoreReg { reg: Gpr::Rax, .. })
        ));
    }

    #[test]
    fn decode_bsr_bare_is_width_minus_lzcnt() {
        // bsr rax, rcx  (48 0F BD C1) — bare => BSR = (width-1) - lzcnt.
        let d = decode_one(b"\x48\x0F\xBD\xC1", 0).unwrap();
        assert_eq!(d.bytes_consumed, 4);
        assert!(matches!(d.stmts[2].op, Op::Lzcnt(Lzcnt { value: 0, .. })));
        assert_eq!(
            d.stmts[3],
            Stmt::new(
                Some(3),
                Op::Constant(Constant {
                    value: 63,
                    size: OpSize::I64,
                }),
            )
        );
        assert_eq!(
            d.stmts[4],
            Stmt::new(
                Some(4),
                Op::BinOp(BinOp {
                    op: BinOpKind::Sub,
                    lhs: 3,
                    rhs: 2,
                    size: OpSize::I64,
                }),
            )
        );
    }

    #[test]
    fn decode_bsf_memory_form_deferred() {
        // bsf rax, [rcx] (48 0F BC 01, mod=00): register-direct only.
        assert_eq!(
            decode_one(b"\x48\x0F\xBC\x01", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xBC))
        );
    }

    #[test]
    fn decode_bswap_registers() {
        let d = decode_one(b"\x48\x0F\xC8", 0).unwrap();
        assert_eq!(d.bytes_consumed, 3);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
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
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 1,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );

        let d = decode_one(b"\x41\x0F\xC8", 0).unwrap();
        assert_eq!(d.bytes_consumed, 3);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::R8,
                        size: OpSize::I32,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::Bswap(Bswap {
                        value: 0,
                        size: OpSize::I32,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::R8,
                        value: 1,
                        size: OpSize::I32,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_bswap_rejects_16_bit_operand_size() {
        let r = decode_one(b"\x66\x0F\xC8", 0);
        assert_eq!(r.unwrap_err(), crate::DecodeError::UnsupportedOpcode(0xC8));
    }

    #[test]
    fn decode_cmpxchg_rm_r_register_direct_emission_order() {
        // cmpxchg rcx, rdx  (REX.W 0F B1 /r, modrm 0xD1 = 11 010 001):
        //   reg=rdx (src), rm=rcx (dst). I64 operands.
        let d = decode_one(b"\x48\x0F\xB1\xD1", 0).unwrap();
        assert_eq!(d.bytes_consumed, 4);
        assert_eq!(
            d.stmts,
            vec![
                // ref_acc = RAX (operand size)
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I64,
                    }),
                ),
                // ref_rax_full = RAX (I64) for the accumulator writeback width
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I64,
                    }),
                ),
                // ref_src = ModRM.reg (rdx)
                Stmt::new(
                    Some(2),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rdx,
                        size: OpSize::I64,
                    }),
                ),
                // ref_dst = ModRM.rm (rcx)
                Stmt::new(
                    Some(3),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rcx,
                        size: OpSize::I64,
                    }),
                ),
                // ZF = (acc == dst); result ref required by the Rust lowerer.
                Stmt::new(
                    Some(4),
                    Op::CmpFlags(CmpFlags {
                        lhs: 0,
                        rhs: 3,
                        size: OpSize::I64,
                    }),
                ),
                // new_dst: equal -> src, else dst unchanged.
                Stmt::new(
                    Some(5),
                    Op::Select(Select {
                        cc: CondCode::Eq,
                        true_value: 2,
                        false_value: 3,
                        size: OpSize::I64,
                    }),
                ),
                // new_rax: equal -> full RAX (unchanged), else dst (failure).
                Stmt::new(
                    Some(6),
                    Op::Select(Select {
                        cc: CondCode::Eq,
                        true_value: 1,
                        false_value: 3,
                        size: OpSize::I64,
                    }),
                ),
                // Accumulator writeback FIRST.
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 6,
                        size: OpSize::I64,
                    }),
                ),
                // Then DEST.
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rcx,
                        value: 5,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_cmpxchg_rm8_r8_byte_form() {
        // cmpxchg cl, dl  (0F B0 /r, modrm 0xD1): reg=dl (src), rm=cl (dst).
        // I8 operands; rax_full still loaded (size != I16) for the writeback.
        let d = decode_one(b"\x0F\xB0\xD1", 0).unwrap();
        assert_eq!(d.bytes_consumed, 3);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(2),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rdx,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    Some(3),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rcx,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    Some(4),
                    Op::CmpFlags(CmpFlags {
                        lhs: 0,
                        rhs: 3,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    Some(5),
                    Op::Select(Select {
                        cc: CondCode::Eq,
                        true_value: 2,
                        false_value: 3,
                        size: OpSize::I8,
                    }),
                ),
                // Failure writeback selects into the full RAX (I64).
                Stmt::new(
                    Some(6),
                    Op::Select(Select {
                        cc: CondCode::Eq,
                        true_value: 1,
                        false_value: 3,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 6,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rcx,
                        value: 5,
                        size: OpSize::I8,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_cmpxchg_i16_failure_writes_ax_only() {
        // cmpxchg cx, dx  (66 0F B1 /r, modrm 0xD1): I16 operands. The failure
        // writeback selects into AX (I16), preserving the rest of RAX.
        let d = decode_one(b"\x66\x0F\xB1\xD1", 0).unwrap();
        assert_eq!(d.bytes_consumed, 4);
        // No ref_rax_full load for I16: acc, src, dst, cmp, new_dst, new_rax.
        assert_eq!(d.stmts.len(), 8);
        assert_eq!(
            d.stmts[0],
            Stmt::new(
                Some(0),
                Op::LoadReg(LoadReg {
                    reg: Gpr::Rax,
                    size: OpSize::I16,
                }),
            )
        );
        // ref_src is ref 1 (no full-RAX load was inserted).
        assert_eq!(
            d.stmts[1],
            Stmt::new(
                Some(1),
                Op::LoadReg(LoadReg {
                    reg: Gpr::Rdx,
                    size: OpSize::I16,
                }),
            )
        );
        // new_rax selects acc (success keeps AX) vs dst (failure), at I16.
        assert_eq!(
            d.stmts[5],
            Stmt::new(
                Some(5),
                Op::Select(Select {
                    cc: CondCode::Eq,
                    true_value: 0,
                    false_value: 2,
                    size: OpSize::I16,
                }),
            )
        );
        // Accumulator store is I16 (AX only) and comes before the DEST store.
        assert_eq!(
            d.stmts[6],
            Stmt::new(
                None,
                Op::StoreReg(StoreReg {
                    reg: Gpr::Rax,
                    value: 5,
                    size: OpSize::I16,
                }),
            )
        );
        assert!(matches!(
            d.stmts[7].op,
            Op::StoreReg(StoreReg {
                reg: Gpr::Rcx,
                size: OpSize::I16,
                ..
            })
        ));
    }

    #[test]
    fn decode_cmpxchg_memory_form_deferred() {
        // cmpxchg [rcx], rdx  (0F B1 /r, modrm 0x11 = 00 010 001): mod != 3.
        let r = decode_one(b"\x0F\xB1\x11", 0);
        assert_eq!(r.unwrap_err(), crate::DecodeError::UnsupportedOpcode(0xB1));
    }

    #[test]
    fn decode_crc32c_register_forms() {
        let d = decode_one(b"\xF2\x0F\x38\xF0\xC1", 0).unwrap();
        assert_eq!(d.bytes_consumed, 5);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I32,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rcx,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    Some(2),
                    Op::Crc32c(prisma_ir::Crc32c {
                        crc: 0,
                        data: 1,
                        data_size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 2,
                        size: OpSize::I32,
                    }),
                ),
            ]
        );

        let d = decode_one(b"\xF2\x48\x0F\x38\xF1\xC1", 0).unwrap();
        assert_eq!(d.bytes_consumed, 6);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rcx,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(2),
                    Op::Crc32c(prisma_ir::Crc32c {
                        crc: 0,
                        data: 1,
                        data_size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 2,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_movbe_memory_forms() {
        let d = decode_one(b"\x0F\x38\xF0\x08", 0).unwrap();
        assert_eq!(d.bytes_consumed, 4);
        assert!(d.stmts.iter().any(|stmt| matches!(
            stmt.op,
            Op::Bswap(Bswap {
                size: OpSize::I32,
                ..
            })
        )));
        assert!(matches!(
            d.stmts.last().unwrap().op,
            Op::StoreReg(StoreReg {
                reg: Gpr::Rcx,
                size: OpSize::I32,
                ..
            })
        ));

        let d = decode_one(b"\x66\x0F\x38\xF1\x08", 0).unwrap();
        assert_eq!(d.bytes_consumed, 5);
        assert!(d.stmts.iter().any(|stmt| matches!(
            stmt.op,
            Op::Bswap(Bswap {
                size: OpSize::I16,
                ..
            })
        )));
        assert!(matches!(
            d.stmts.last().unwrap().op,
            Op::StoreMem(StoreMem {
                size: OpSize::I16,
                ..
            })
        ));
    }

    #[test]
    fn decode_movbe_rejects_register_rm_form() {
        let r = decode_one(b"\x0F\x38\xF0\xC1", 0);
        assert_eq!(r.unwrap_err(), crate::DecodeError::UnsupportedOpcode(0xF0));
    }

    #[test]
    fn decode_crc32c_requires_f2_prefix() {
        let r = decode_one(b"\x0F\x38\xF1\xC1", 0);
        assert_eq!(r.unwrap_err(), crate::DecodeError::UnsupportedOpcode(0xF1));
    }

    #[test]
    fn decode_cond_jump_rel8() {
        let d = decode_one_at(b"\x74\xFE", 0, 0x1000).unwrap();
        assert_eq!(d.bytes_consumed, 2);
        assert_eq!(
            d.stmts,
            vec![Stmt::new(
                None,
                Op::CondJumpRel(CondJumpRel {
                    cc: CondCode::Eq,
                    target_guest_pc: 0x1000,
                    fallthrough_guest_pc: 0x1002,
                }),
            )]
        );
    }

    #[test]
    fn decode_cond_jump_rel8_covers_supported_conditions() {
        let supported: &[(u8, CondCode)] = &[
            (0x70, CondCode::Ov),
            (0x71, CondCode::NoOv),
            (0x72, CondCode::Nc),
            (0x73, CondCode::Cc),
            (0x74, CondCode::Eq),
            (0x75, CondCode::Ne),
            (0x76, CondCode::Ule),
            (0x77, CondCode::Ugt),
            (0x78, CondCode::Mi),
            (0x79, CondCode::Pl),
            (0x7C, CondCode::Slt),
            (0x7D, CondCode::Sge),
            (0x7E, CondCode::Sle),
            (0x7F, CondCode::Sgt),
        ];

        for &(opcode, cc) in supported {
            let d = decode_one_at(&[opcode, 0x02], 0, 0x1000).unwrap();
            assert_eq!(d.bytes_consumed, 2);
            assert_eq!(
                d.stmts,
                vec![Stmt::new(
                    None,
                    Op::CondJumpRel(CondJumpRel {
                        cc,
                        target_guest_pc: 0x1004,
                        fallthrough_guest_pc: 0x1002,
                    }),
                )]
            );
        }
    }

    #[test]
    fn decode_cond_jump_rel8_requires_no_rex_and_no_operand_override() {
        let d = decode_one(b"\x48\x74\x00", 0);
        assert_eq!(d.unwrap_err(), crate::DecodeError::UnsupportedOpcode(0x74));
        let d = decode_one(b"\x66\x74\x00", 0);
        assert_eq!(d.unwrap_err(), crate::DecodeError::UnsupportedOpcode(0x74));
    }

    #[test]
    fn decode_cond_jump_rel8_rejects_pf_and_npf() {
        assert_eq!(
            decode_one(b"\x7A\x00", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x7A))
        );
        assert_eq!(
            decode_one(b"\x7B\x00", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x7B))
        );
    }

    #[test]
    fn decode_cond_jump_rel32() {
        // 0F 80 F0 FF FF FF => je rel32(-16) with 0x2000 PC
        let d = decode_one_at(b"\x0F\x80\xF0\xFF\xFF\xFF", 0, 0x2000).unwrap();
        assert_eq!(d.bytes_consumed, 6);
        assert_eq!(
            d.stmts,
            vec![Stmt::new(
                None,
                Op::CondJumpRel(CondJumpRel {
                    cc: CondCode::Ov,
                    target_guest_pc: 0x1FF6,
                    fallthrough_guest_pc: 0x2006,
                }),
            )]
        );
    }

    #[test]
    fn decode_call_rel32() {
        let d = decode_one_at(b"\xE8\xFE\xFF\xFF\xFF", 0, 0x1000).unwrap();
        assert_eq!(d.bytes_consumed, 5);
        assert_eq!(
            d.stmts,
            vec![Stmt::new(
                None,
                Op::CallRel(CallRel {
                    target_guest_pc: 0x1003,
                    return_guest_pc: 0x1005,
                }),
            )]
        );
    }

    #[test]
    fn decode_cond_jump_rel32_covers_supported_conditions() {
        let supported: &[(u8, CondCode)] = &[
            (0x80, CondCode::Ov),
            (0x81, CondCode::NoOv),
            (0x82, CondCode::Nc),
            (0x83, CondCode::Cc),
            (0x84, CondCode::Eq),
            (0x85, CondCode::Ne),
            (0x86, CondCode::Ule),
            (0x87, CondCode::Ugt),
            (0x88, CondCode::Mi),
            (0x89, CondCode::Pl),
            (0x8C, CondCode::Slt),
            (0x8D, CondCode::Sge),
            (0x8E, CondCode::Sle),
            (0x8F, CondCode::Sgt),
        ];

        for &(opcode, cc) in supported {
            let d = decode_one_at(&[0x0F, opcode, 0x02, 0x00, 0x00, 0x00], 0, 0x1000).unwrap();
            assert_eq!(d.bytes_consumed, 6);
            assert_eq!(
                d.stmts,
                vec![Stmt::new(
                    None,
                    Op::CondJumpRel(CondJumpRel {
                        cc,
                        target_guest_pc: 0x1008,
                        fallthrough_guest_pc: 0x1006,
                    }),
                )]
            );
        }
    }

    #[test]
    fn decode_call_rel32_rejects_prefixes() {
        assert_eq!(
            decode_one(b"\x66\xE8\x00\x00\x00\x00", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xE8))
        );
        assert_eq!(
            decode_one(b"\x2E\xE8\x00\x00\x00\x00", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xE8))
        );
        assert_eq!(
            decode_one(b"\x48\xE8\x00\x00\x00\x00", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xE8))
        );
    }

    #[test]
    fn decode_call_rel32_truncates_when_missing_imm() {
        assert_eq!(decode_one(b"\xE8", 0), Err(crate::DecodeError::Truncated));
        assert_eq!(
            decode_one(b"\xE8\x01", 0),
            Err(crate::DecodeError::Truncated)
        );
    }

    #[test]
    fn decode_jump_rel8() {
        let d = decode_one_at(b"\xEB\xFE", 0, 0x1000).unwrap();
        assert_eq!(d.bytes_consumed, 2);
        assert_eq!(
            d.stmts,
            vec![Stmt::new(
                None,
                Op::JumpRel(JumpRel {
                    target_guest_pc: 0x1000,
                }),
            )]
        );
    }

    #[test]
    fn decode_jrcxz_rel8() {
        let d = decode_one_at(b"\xE3\x02", 0, 0x1000).unwrap();
        assert_eq!(d.bytes_consumed, 2);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rcx,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::Constant(Constant {
                        value: 0,
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
                    Op::CondJumpRel(CondJumpRel {
                        cc: CondCode::Eq,
                        target_guest_pc: 0x1004,
                        fallthrough_guest_pc: 0x1002,
                    }),
                ),
            ]
        );

        let d = decode_one_at(b"\x67\xE3\xFE", 0, 0x1000).unwrap();
        assert_eq!(d.bytes_consumed, 3);
        assert!(matches!(
            d.stmts[0].op,
            Op::LoadReg(LoadReg {
                reg: Gpr::Rcx,
                size: OpSize::I32,
            })
        ));
    }

    #[test]
    fn decode_loop_rel8_decrements_counter_and_branches_on_nonzero() {
        let d = decode_one_at(b"\xE2\xFE", 0, 0x9000).unwrap();
        assert_eq!(d.bytes_consumed, 2);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rcx,
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
                    Op::BinOp(BinOp {
                        op: BinOpKind::Sub,
                        lhs: 0,
                        rhs: 1,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rcx,
                        value: 2,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(4),
                    Op::Constant(Constant {
                        value: 0,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(5),
                    Op::CmpFlags(CmpFlags {
                        lhs: 2,
                        rhs: 4,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::CondJumpRel(CondJumpRel {
                        cc: CondCode::Ne,
                        target_guest_pc: 0x9000,
                        fallthrough_guest_pc: 0x9002,
                    }),
                ),
            ]
        );

        let d = decode_one_at(b"\x67\xE2\x00", 0, 0x9000).unwrap();
        assert_eq!(d.bytes_consumed, 3);
        assert!(matches!(
            d.stmts[0].op,
            Op::LoadReg(LoadReg {
                reg: Gpr::Rcx,
                size: OpSize::I32,
            })
        ));
    }

    #[test]
    fn decode_jump_rel32() {
        let d = decode_one_at(b"\xE9\xFC\xFF\xFF\xFF", 0, 0x2000).unwrap();
        assert_eq!(d.bytes_consumed, 5);
        assert_eq!(
            d.stmts,
            vec![Stmt::new(
                None,
                Op::JumpRel(JumpRel {
                    target_guest_pc: 0x2001,
                }),
            )]
        );
    }

    #[test]
    fn decode_jump_rel32_truncates_when_missing_imm() {
        assert_eq!(decode_one(b"\xE9", 0), Err(crate::DecodeError::Truncated));
        assert_eq!(
            decode_one(b"\xE9\x01\x00\x00", 0),
            Err(crate::DecodeError::Truncated)
        );
    }

    #[test]
    fn decode_jump_relative_rejects_prefixes() {
        assert_eq!(
            decode_one(b"\x66\xEB\x00", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xEB))
        );
        assert_eq!(
            decode_one(b"\x2E\xEB\x00", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xEB))
        );
        assert_eq!(
            decode_one(b"\x48\xEB\x00", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xEB))
        );
        assert_eq!(
            decode_one(b"\x66\xE9\x00\x00\x00\x00", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xE9))
        );
        assert_eq!(
            decode_one(b"\x2E\xE9\x00\x00\x00\x00", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xE9))
        );
        assert_eq!(
            decode_one(b"\x48\xE9\x00\x00\x00\x00", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xE9))
        );
    }

    #[test]
    fn decode_ret_ops() {
        let d = decode_one(b"\xC3", 0).unwrap();
        assert_eq!(d.bytes_consumed, 1);
        assert_eq!(d.stmts, vec![Stmt::new(None, Op::Return(Return))]);

        let d = decode_one_at(b"\xC2\x34\x12", 0, 0x1000).unwrap();
        assert_eq!(d.bytes_consumed, 3);
        assert_eq!(
            d.stmts,
            vec![Stmt::new(
                None,
                Op::RetAdjusted(RetAdjusted { pop_bytes: 0x123c }),
            )]
        );
    }

    #[test]
    fn decode_leave() {
        let d = decode_one(b"\xC9", 0).unwrap();
        assert_eq!(d.bytes_consumed, 1);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rbp,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rsp,
                        value: 0,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(2),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rsp,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(3),
                    Op::LoadMem(LoadMem {
                        addr: 2,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rbp,
                        value: 3,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(None, Op::RspAdjust(RspAdjust { delta_bytes: 8 })),
            ]
        );
    }

    #[test]
    fn decode_enter_level_zero_sets_up_stack_frame() {
        let d = decode_one(b"\xC8\x20\x00\x00", 0).unwrap();
        assert_eq!(d.bytes_consumed, 4);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rbp,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(None, Op::RspAdjust(RspAdjust { delta_bytes: -8 })),
                Stmt::new(
                    Some(2),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rsp,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreMem(StoreMem {
                        addr: 2,
                        value: 0,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rbp,
                        value: 2,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(None, Op::RspAdjust(RspAdjust { delta_bytes: -0x20 })),
            ]
        );

        assert_eq!(
            decode_one(b"\xC8\x20\x00", 0),
            Err(crate::DecodeError::Truncated)
        );
        assert_eq!(
            decode_one(b"\xC8\x20\x00\x01", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xC8))
        );
    }

    #[test]
    fn decode_ret_imm16_truncates_when_missing_imm() {
        assert_eq!(decode_one(b"\xC2", 0), Err(crate::DecodeError::Truncated));
        assert_eq!(
            decode_one(b"\xC2\x12", 0),
            Err(crate::DecodeError::Truncated)
        );
    }

    #[test]
    fn decode_ret_rejects_prefixes() {
        assert_eq!(
            decode_one(b"\x66\xC3", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xC3))
        );
        assert_eq!(
            decode_one(b"\x2E\xC2\x34\x12", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xC2))
        );
        assert_eq!(
            decode_one(b"\xF0\xC2\x34\x12", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xC2))
        );
        assert_eq!(
            decode_one(b"\x66\xC2\x34\x12", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xC2))
        );
        assert_eq!(
            decode_one(b"\xF0\xC3", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xC3))
        );
        assert_eq!(
            decode_one(b"\x2E\xC3", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xC3))
        );
    }

    #[test]
    fn decode_iret_emits_sigill_trap() {
        let d = decode_one(b"\xCF", 0).unwrap();
        assert_eq!(d.bytes_consumed, 1);
        assert_eq!(
            d.stmts,
            vec![Stmt::new(
                None,
                Op::Trap(Trap {
                    kind: TrapKind::Sigill,
                }),
            )]
        );

        assert_eq!(
            decode_one(b"\x66\xCF", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xCF))
        );
    }

    #[test]
    fn decode_control_and_debug_register_moves_emit_sigill_traps() {
        for bytes in [
            b"\x0F\x20\xC0".as_slice(),
            b"\x0F\x21\xC0".as_slice(),
            b"\x0F\x22\xC0".as_slice(),
            b"\x0F\x23\xC0".as_slice(),
        ] {
            let d = decode_one(bytes, 0).unwrap();
            assert_eq!(d.bytes_consumed, 3);
            assert_eq!(
                d.stmts,
                vec![Stmt::new(
                    None,
                    Op::Trap(Trap {
                        kind: TrapKind::Sigill,
                    }),
                )]
            );
        }

        assert_eq!(
            decode_one(b"\x0F\x20\x00", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x20))
        );
        assert_eq!(
            decode_one(b"\x66\x0F\x22\xC0", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x22))
        );
    }

    #[test]
    fn decode_vmx_and_smx_privileged_instructions_emit_sigill_traps() {
        let getsec = decode_one(b"\x0F\x37", 0).unwrap();
        assert_eq!(getsec.bytes_consumed, 2);
        assert_eq!(
            getsec.stmts,
            vec![Stmt::new(
                None,
                Op::Trap(Trap {
                    kind: TrapKind::Sigill,
                }),
            )]
        );

        for (bytes, consumed) in [
            (b"\x0F\x78\xC0".as_slice(), 3),
            (b"\x0F\x79\xC0".as_slice(), 3),
            (b"\x0F\x78\x40\x10".as_slice(), 4),
            (b"\x0F\x79\x84\x88\x20\x00\x00\x00".as_slice(), 8),
        ] {
            let d = decode_one(bytes, 0).unwrap();
            assert_eq!(d.bytes_consumed, consumed);
            assert_eq!(
                d.stmts,
                vec![Stmt::new(
                    None,
                    Op::Trap(Trap {
                        kind: TrapKind::Sigill,
                    }),
                )]
            );
        }

        assert_eq!(
            decode_one(b"\x66\x0F\x78\xC0", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x78))
        );
    }

    #[test]
    fn decode_ud1_ud0_emit_sigill_and_consume_modrm_operands() {
        for (bytes, consumed) in [
            (b"\x0F\xB9\xC0".as_slice(), 3),
            (b"\x0F\xB9\x40\x10".as_slice(), 4),
            (b"\x0F\xFF\xC0".as_slice(), 3),
            (b"\x0F\xFF\x84\x88\x20\x00\x00\x00".as_slice(), 8),
        ] {
            let d = decode_one(bytes, 0).unwrap();
            assert_eq!(d.bytes_consumed, consumed);
            assert_eq!(
                d.stmts,
                vec![Stmt::new(
                    None,
                    Op::Trap(Trap {
                        kind: TrapKind::Sigill,
                    }),
                )]
            );
        }

        assert_eq!(
            decode_one(b"\x66\x0F\xB9\xC0", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xB9))
        );
    }

    #[test]
    fn decode_group3_not_neg() {
        let d = decode_one(b"\xF7\xD0", 0).unwrap();
        assert_eq!(d.bytes_consumed, 2);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I32,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::Constant(Constant {
                        value: 0xFFFF_FFFF,
                        size: OpSize::I32,
                    }),
                ),
                Stmt::new(
                    Some(2),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Xor,
                        lhs: 0,
                        rhs: 1,
                        size: OpSize::I32,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 2,
                        size: OpSize::I32,
                    }),
                ),
            ]
        );

        let d = decode_one(b"\x48\xF7\x18\x10", 0).unwrap();
        assert!(d.bytes_consumed >= 2);
        assert!(matches!(d.stmts.last().unwrap().op, Op::StoreMem(_)));

        let d = decode_one(b"\xF7\x18\x10", 0).unwrap();
        assert!(matches!(d.stmts.last().unwrap().op, Op::StoreMem(_)));
    }

    #[test]
    fn decode_test_rm_r_sets_alu_flags_without_result_store() {
        let d = decode_one(b"\x48\x85\xC8", 0).unwrap();
        assert_eq!(d.bytes_consumed, 3);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rcx,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::AluFlags(AluFlags {
                        op: BinOpKind::And,
                        lhs: 1,
                        rhs: 0,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_test_rm8_r8_sets_alu_flags_without_result_store() {
        let d = decode_one(b"\x84\xC8", 0).unwrap();
        assert_eq!(d.bytes_consumed, 2);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rcx,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::AluFlags(AluFlags {
                        op: BinOpKind::And,
                        lhs: 1,
                        rhs: 0,
                        size: OpSize::I8,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_group3_test_immediate_sets_alu_flags() {
        let d = decode_one(b"\xF6\xC0\x80", 0).unwrap();
        assert_eq!(d.bytes_consumed, 3);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::Constant(Constant {
                        value: 0x80,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::AluFlags(AluFlags {
                        op: BinOpKind::And,
                        lhs: 1,
                        rhs: 0,
                        size: OpSize::I8,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_group3_neg() {
        let d = decode_one(b"\xF7\xD8", 0).unwrap();
        assert_eq!(d.bytes_consumed, 2);
        assert_eq!(
            d.stmts[2],
            Stmt::new(
                Some(2),
                Op::BinOp(BinOp {
                    op: BinOpKind::Sub,
                    lhs: 1,
                    rhs: 0,
                    size: OpSize::I32,
                }),
            )
        );
    }

    #[test]
    fn decode_group3_not_byte() {
        // F6 /2: NOT r/m8 -> dst = dst XOR 0xFF
        let d = decode_one(b"\xF6\xD3", 0).unwrap();
        assert_eq!(d.bytes_consumed, 2);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rbx,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::Constant(Constant {
                        value: 0xFF,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    Some(2),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Xor,
                        lhs: 0,
                        rhs: 1,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rbx,
                        value: 2,
                        size: OpSize::I8,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_group3_neg_byte() {
        // F6 /3: NEG r/m8 -> dst = 0 - dst
        let d = decode_one(b"\xF6\xD8", 0).unwrap();
        assert_eq!(d.bytes_consumed, 2);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::Constant(Constant {
                        value: 0,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    Some(2),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Sub,
                        lhs: 1,
                        rhs: 0,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 2,
                        size: OpSize::I8,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_group4_inc_dec_byte_memory() {
        // FE /0: INC byte ptr [rax]
        let inc = decode_one(b"\xFE\x00", 0).unwrap();
        assert!(inc.bytes_consumed >= 2);
        assert!(matches!(
            inc.stmts.last().unwrap().op,
            Op::StoreMem(StoreMem {
                size: OpSize::I8,
                ..
            })
        ));
        assert!(inc.stmts.iter().any(|s| matches!(
            s.op,
            Op::BinOp(BinOp {
                op: BinOpKind::Add,
                size: OpSize::I8,
                ..
            })
        )));

        // FE /1: DEC byte ptr [rax]
        let dec = decode_one(b"\xFE\x08", 0).unwrap();
        assert!(dec.bytes_consumed >= 2);
        assert!(matches!(
            dec.stmts.last().unwrap().op,
            Op::StoreMem(StoreMem {
                size: OpSize::I8,
                ..
            })
        ));
        assert!(dec.stmts.iter().any(|s| matches!(
            s.op,
            Op::BinOp(BinOp {
                op: BinOpKind::Sub,
                size: OpSize::I8,
                ..
            })
        )));
    }

    #[test]
    fn decode_group5_inc_dec_rexw_register() {
        // 48 FF C0: INC rax (64-bit)
        let inc = decode_one(b"\x48\xFF\xC0", 0).unwrap();
        assert_eq!(inc.bytes_consumed, 3);
        assert_eq!(
            inc.stmts.last().unwrap(),
            &Stmt::new(
                None,
                Op::StoreReg(StoreReg {
                    reg: Gpr::Rax,
                    value: 2,
                    size: OpSize::I64,
                }),
            )
        );
        assert!(inc.stmts.iter().any(|s| matches!(
            s.op,
            Op::BinOp(BinOp {
                op: BinOpKind::Add,
                size: OpSize::I64,
                ..
            })
        )));

        // 48 FF C8: DEC rax (64-bit)
        let dec = decode_one(b"\x48\xFF\xC8", 0).unwrap();
        assert_eq!(dec.bytes_consumed, 3);
        assert!(matches!(
            dec.stmts.last().unwrap().op,
            Op::StoreReg(StoreReg {
                reg: Gpr::Rax,
                size: OpSize::I64,
                ..
            })
        ));
        assert!(dec.stmts.iter().any(|s| matches!(
            s.op,
            Op::BinOp(BinOp {
                op: BinOpKind::Sub,
                size: OpSize::I64,
                ..
            })
        )));
    }

    #[test]
    fn decode_group3_unsupported_subop() {
        // /1 has no decode (only /0 is TEST here).
        assert_eq!(
            decode_one(b"\xF7\xC8", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xF7))
        );
    }

    #[test]
    fn decode_group3_muldiv_memory_form_deferred() {
        // mul dword [rax] (F7 /4, mod=00): register-direct only, like the C++
        // MVP — the memory form is still unsupported.
        assert_eq!(
            decode_one(b"\xF7\x20", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xF7))
        );
    }

    #[test]
    fn decode_group3_mul_imul_div_idiv_register_direct() {
        // (reg digit, opcode byte) -> (low kind, high kind). All over rcx
        // (REX.W F7 /digit, modrm = 11 <digit> 001).
        let cases = [
            (4u8, BinOpKind::Mul, BinOpKind::UMulHi), // mul rcx
            (5u8, BinOpKind::Mul, BinOpKind::SMulHi), // imul rcx
            (6u8, BinOpKind::UDiv, BinOpKind::UMod),  // div rcx
            (7u8, BinOpKind::SDiv, BinOpKind::SMod),  // idiv rcx
        ];
        for (digit, lo, hi) in cases {
            let modrm = 0xC0 | (digit << 3) | 0x01; // 11 <digit> 001 (rcx)
            let d = decode_one(&[0x48, 0xF7, modrm], 0).unwrap();
            assert_eq!(d.bytes_consumed, 3, "digit {digit}");
            // LoadReg Rax, LoadReg Rcx, BinOp lo, BinOp hi, StoreReg Rax, StoreReg Rdx.
            assert_eq!(
                d.stmts[0],
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I64,
                    }),
                ),
                "digit {digit}"
            );
            assert_eq!(
                d.stmts[2],
                Stmt::new(
                    Some(2),
                    Op::BinOp(BinOp {
                        op: lo,
                        lhs: 0,
                        rhs: 1,
                        size: OpSize::I64,
                    }),
                ),
                "digit {digit}"
            );
            assert_eq!(
                d.stmts[3],
                Stmt::new(
                    Some(3),
                    Op::BinOp(BinOp {
                        op: hi,
                        lhs: 0,
                        rhs: 1,
                        size: OpSize::I64,
                    }),
                ),
                "digit {digit}"
            );
            assert_eq!(
                d.stmts[4],
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 2,
                        size: OpSize::I64,
                    }),
                ),
                "digit {digit}"
            );
            assert_eq!(
                d.stmts[5],
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rdx,
                        value: 3,
                        size: OpSize::I64,
                    }),
                ),
                "digit {digit}"
            );
        }
    }

    #[test]
    fn decode_group3_not_neg_rexw() {
        let d = decode_one(b"\x48\xF7\xD0", 0).unwrap();
        assert_eq!(d.bytes_consumed, 3);
        assert!(matches!(
            d.stmts.last().unwrap().op,
            Op::StoreReg(StoreReg {
                size: OpSize::I64,
                ..
            })
        ));
        assert!(matches!(
            d.stmts[0].op,
            Op::LoadReg(LoadReg {
                reg: Gpr::Rax,
                size: OpSize::I64,
                ..
            })
        ));
    }

    #[test]
    fn decode_group3_neg_rexw() {
        let d = decode_one(b"\x48\xF7\xD8", 0).unwrap();
        assert_eq!(d.bytes_consumed, 3);
        assert!(matches!(
            d.stmts.last().unwrap().op,
            Op::StoreReg(StoreReg {
                size: OpSize::I64,
                ..
            })
        ));
        assert!(matches!(
            d.stmts[2].op,
            Op::BinOp(BinOp {
                size: OpSize::I64,
                ..
            })
        ));
    }

    #[test]
    fn decode_group4_inc_dec_byte_register_placeholders() {
        let inc = decode_one(b"\xFE\xC0", 0).unwrap();
        assert_eq!(inc.bytes_consumed, 2);
        assert_eq!(
            inc.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::Constant(Constant {
                        value: 1,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    Some(1),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    Some(2),
                    Op::BinOp(BinOp {
                        op: BinOpKind::Add,
                        lhs: 1,
                        rhs: 0,
                        size: OpSize::I8,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 2,
                        size: OpSize::I8,
                    }),
                ),
            ]
        );

        let dec = decode_one(b"\xFE\xC8", 0).unwrap();
        assert_eq!(dec.bytes_consumed, 2);
        assert!(matches!(
            dec.stmts[2].op,
            Op::BinOp(BinOp {
                op: BinOpKind::Sub,
                size: OpSize::I8,
                ..
            })
        ));
    }

    #[test]
    fn decode_group4_rejects_unsupported_subop() {
        assert_eq!(
            decode_one(b"\xFE\xD0", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xFE))
        );
    }

    #[test]
    fn decode_group5_inc_dec_call_jmp() {
        let inc = decode_one(b"\xFF\xC0", 0).unwrap();
        assert_eq!(inc.bytes_consumed, 2);
        assert_eq!(
            inc.stmts[3],
            Stmt::new(
                None,
                Op::StoreReg(StoreReg {
                    reg: Gpr::Rax,
                    value: 2,
                    size: OpSize::I32,
                }),
            )
        );

        let dec = decode_one(b"\xFF\xCB", 0).unwrap();
        assert_eq!(dec.bytes_consumed, 2);
        assert_eq!(
            dec.stmts[3],
            Stmt::new(
                None,
                Op::StoreReg(StoreReg {
                    reg: Gpr::Rbx,
                    value: 2,
                    size: OpSize::I32,
                }),
            )
        );

        let call = decode_one_at(b"\xFF\xD0", 0, 0x4000).unwrap();
        assert_eq!(call.bytes_consumed, 2);
        assert_eq!(
            call.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I32,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::CallReg(CallReg {
                        target: 0,
                        return_guest_pc: 0x4002,
                    }),
                ),
            ]
        );

        let jmp = decode_one(b"\xFF\xE0", 0).unwrap();
        assert_eq!(jmp.bytes_consumed, 2);
        assert_eq!(
            jmp.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I32,
                    }),
                ),
                Stmt::new(None, Op::JumpReg(JumpReg { target: 0 }),),
            ]
        );
    }

    #[test]
    fn decode_group5_memory_operands_rexw() {
        let inc = decode_one_at(b"\x48\xFF\x00", 0, 0x3000).unwrap();
        assert_eq!(inc.bytes_consumed, 3);
        assert!(matches!(inc.stmts.last().unwrap().op, Op::StoreMem(_)));

        let dec = decode_one_at(b"\x48\xFF\x08", 0, 0x3000).unwrap();
        assert_eq!(dec.bytes_consumed, 3);
        assert!(matches!(dec.stmts.last().unwrap().op, Op::StoreMem(_)));

        let call = decode_one_at(b"\x48\xFF\x10", 0, 0x3000).unwrap();
        assert_eq!(call.bytes_consumed, 3);
        match call.stmts.last().unwrap() {
            Stmt {
                op:
                    Op::CallReg(CallReg {
                        target,
                        return_guest_pc,
                    }),
                ..
            } => {
                assert_eq!(*target, 0);
                assert_eq!(*return_guest_pc, 0x3003);
            }
            other => panic!("unexpected stmt: {other:?}"),
        }

        let jmp = decode_one_at(b"\x48\xFF\x20", 0, 0x3000).unwrap();
        assert_eq!(jmp.bytes_consumed, 3);
        match jmp.stmts.last().unwrap() {
            Stmt {
                op: Op::JumpReg(JumpReg { target }),
                ..
            } => assert_eq!(*target, 0),
            other => panic!("unexpected stmt: {other:?}"),
        }

        let push = decode_one_at(b"\x48\xFF\x30", 0, 0x3000).unwrap();
        assert_eq!(push.bytes_consumed, 3);
        match push.stmts.last().unwrap() {
            Stmt {
                op:
                    Op::StoreMem(StoreMem {
                        value,
                        size: OpSize::I64,
                        ..
                    }),
                ..
            } => assert_eq!(*value, 1),
            other => panic!("unexpected stmt: {other:?}"),
        }
    }

    #[test]
    fn decode_group5_memory_operands() {
        let inc = decode_one(b"\xFF\x00", 0).unwrap();
        assert_eq!(inc.bytes_consumed, 2);
        assert!(matches!(inc.stmts.last().unwrap().op, Op::StoreMem(_)));

        let dec = decode_one(b"\xFF\x08", 0).unwrap();
        assert_eq!(dec.bytes_consumed, 2);
        assert!(matches!(dec.stmts.last().unwrap().op, Op::StoreMem(_)));

        let call = decode_one_at(b"\xFF\x10", 0, 0x3000).unwrap();
        assert_eq!(call.bytes_consumed, 2);
        match call.stmts.last().unwrap() {
            Stmt {
                op:
                    Op::CallReg(CallReg {
                        target,
                        return_guest_pc,
                    }),
                ..
            } => {
                assert_eq!(*target, 0);
                assert_eq!(*return_guest_pc, 0x3002);
            }
            other => panic!("unexpected stmt: {other:?}"),
        }

        let jmp = decode_one(b"\xFF\x20", 0).unwrap();
        assert_eq!(jmp.bytes_consumed, 2);
        match jmp.stmts.last().unwrap() {
            Stmt {
                op: Op::JumpReg(JumpReg { target }),
                ..
            } => assert_eq!(*target, 0),
            other => panic!("unexpected stmt: {other:?}"),
        }
    }

    #[test]
    fn decode_group5_push_reg() {
        let d = decode_one(b"\xFF\xF0", 0).unwrap();
        assert_eq!(d.bytes_consumed, 2);
        assert_eq!(
            d.stmts,
            vec![
                Stmt::new(
                    Some(0),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rax,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(None, Op::RspAdjust(RspAdjust { delta_bytes: -8 })),
                Stmt::new(
                    Some(2),
                    Op::LoadReg(LoadReg {
                        reg: Gpr::Rsp,
                        size: OpSize::I64,
                    }),
                ),
                Stmt::new(
                    None,
                    Op::StoreMem(StoreMem {
                        addr: 2,
                        value: 0,
                        size: OpSize::I64,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn decode_group5_unsupported_subop() {
        assert_eq!(
            decode_one(b"\xFF\xF8", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0xFF))
        );
    }

    #[test]
    fn decode_cond_jump_rel32_requires_restrictions() {
        assert_eq!(
            decode_one(b"\xF3\x0F\x80\x00\x00\x00\x00", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x80))
        );
        assert_eq!(
            decode_one(b"\x66\x0F\x80\x00\x00\x00\x00", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x80))
        );
        assert_eq!(
            decode_one(b"\x4F\x0F\x80\x00\x00\x00\x00", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x80))
        );
        assert_eq!(
            decode_one(b"\x0F\x8A\x00\x00\x00\x00", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x8A))
        );
        assert_eq!(
            decode_one(b"\x0F\x8B\x00\x00\x00\x00", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x8B))
        );
    }

    #[test]
    fn decode_fixed_two_byte_system_traps() {
        for &[opcode, expected_len] in &[
            [0x06, 2],
            [0x07, 2],
            [0x30, 2],
            [0x32, 2],
            [0x33, 2],
            [0x34, 2],
            [0x35, 2],
            [0xAA, 2],
        ] {
            let bytes = [0x0F, opcode];
            let d = decode_one(&bytes, 0).unwrap();
            assert_eq!(d.bytes_consumed, expected_len as usize);
            assert_eq!(
                d.stmts,
                vec![Stmt::new(
                    None,
                    Op::Trap(Trap {
                        kind: TrapKind::Sigill,
                    }),
                )]
            );
        }

        assert_eq!(
            decode_one(b"\x66\x0F\x32", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x32))
        );
    }

    #[test]
    fn decode_descriptor_group_privileged_traps_and_consumes_operands() {
        for bytes in [
            b"\x0F\x00\x10".as_slice(),
            b"\x0F\x00\x18".as_slice(),
            b"\x0F\x00\x50\x10".as_slice(),
        ] {
            let d = decode_one(bytes, 0).unwrap();
            assert_eq!(
                d.stmts,
                vec![Stmt::new(
                    None,
                    Op::Trap(Trap {
                        kind: TrapKind::Sigill,
                    }),
                )]
            );
        }

        assert_eq!(
            decode_one(b"\x0F\x00\x00", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x00))
        );
        assert_eq!(
            decode_one(b"\x66\x0F\x00\x10", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x00))
        );
    }

    #[test]
    fn decode_system_group_invlpg_traps_and_consumes_memory_operand() {
        let d = decode_one(b"\x0F\x01\x78\x10", 0).unwrap();
        assert_eq!(d.bytes_consumed, 4);
        assert_eq!(
            d.stmts,
            vec![Stmt::new(
                None,
                Op::Trap(Trap {
                    kind: TrapKind::Sigill,
                }),
            )]
        );

        for bytes in [
            b"\x0F\x01\x10".as_slice(),
            b"\x0F\x01\x18".as_slice(),
            b"\x0F\x01\xF0".as_slice(),
            b"\x0F\x01\xC1".as_slice(),
            b"\x0F\x01\xC2".as_slice(),
            b"\x0F\x01\xC3".as_slice(),
            b"\x0F\x01\xC4".as_slice(),
            b"\x0F\x01\xC8".as_slice(),
            b"\x0F\x01\xC9".as_slice(),
            b"\x0F\x01\xCA".as_slice(),
            b"\x0F\x01\xCB".as_slice(),
            b"\x0F\x01\xCF".as_slice(),
            b"\x0F\x01\xD1".as_slice(),
            b"\x0F\x01\xD4".as_slice(),
            b"\x0F\x01\xD8".as_slice(),
            b"\x0F\x01\xDF".as_slice(),
            b"\x0F\x01\xF8".as_slice(),
        ] {
            let d = decode_one(bytes, 0).unwrap();
            assert_eq!(
                d.stmts,
                vec![Stmt::new(
                    None,
                    Op::Trap(Trap {
                        kind: TrapKind::Sigill,
                    }),
                )]
            );
        }

        assert_eq!(
            decode_one(b"\x0F\x01\xF9", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x01))
        );
        assert_eq!(
            decode_one(b"\x66\x0F\x01\x38", 0),
            Err(crate::DecodeError::UnsupportedOpcode(0x01))
        );
    }

    #[test]
    fn unsupported_opcode() {
        let r = decode_one(b"\x0F\x00", 0);
        assert!(r.is_err());
    }
}
