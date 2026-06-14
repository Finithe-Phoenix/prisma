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
        tables::OneByteOpcode::MovRmR => decode_mov_rm_r(&prefixes, bytes, cursor, &mut stmts)?,
        tables::OneByteOpcode::MovRRm => decode_mov_r_rm(&prefixes, bytes, cursor, &mut stmts)?,
        tables::OneByteOpcode::MovRI => {
            decode_mov_ri(&prefixes, opcode, bytes, cursor, &mut stmts)?
        }
        tables::OneByteOpcode::AddRmR => {
            decode_binop_rm_r(BinOpKind::Add, &prefixes, bytes, cursor, &mut stmts)?
        }
        tables::OneByteOpcode::AddRRm => {
            decode_binop_r_rm(BinOpKind::Add, &prefixes, bytes, cursor, &mut stmts)?
        }
        tables::OneByteOpcode::AluRmImm8 => {
            decode_alu_rm_imm8(&prefixes, bytes, cursor, opcode_guest_pc, &mut stmts)?
        }
        tables::OneByteOpcode::SubRmR => {
            decode_binop_rm_r(BinOpKind::Sub, &prefixes, bytes, cursor, &mut stmts)?
        }
        tables::OneByteOpcode::SubRRm => {
            decode_binop_r_rm(BinOpKind::Sub, &prefixes, bytes, cursor, &mut stmts)?
        }
        tables::OneByteOpcode::AndRmR => {
            decode_binop_rm_r(BinOpKind::And, &prefixes, bytes, cursor, &mut stmts)?
        }
        tables::OneByteOpcode::OrRmR => {
            decode_binop_rm_r(BinOpKind::Or, &prefixes, bytes, cursor, &mut stmts)?
        }
        tables::OneByteOpcode::XorRmR => {
            decode_binop_rm_r(BinOpKind::Xor, &prefixes, bytes, cursor, &mut stmts)?
        }
        tables::OneByteOpcode::CmpRmR => decode_cmp_rm_r(&prefixes, bytes, cursor, &mut stmts)?,
        tables::OneByteOpcode::TestRmR => decode_test_rm_r(&prefixes, bytes, cursor, &mut stmts)?,
        tables::OneByteOpcode::Nop => {
            if prefixes.rex.w {
                return Err(crate::DecodeError::UnsupportedOpcode(opcode));
            }
            1
        }
        tables::OneByteOpcode::CondJumpRel8 => {
            if prefixes.rex.present || prefixes.operand_override {
                return Err(crate::DecodeError::UnsupportedOpcode(opcode));
            }
            decode_cond_jump_rel8(&prefixes, bytes, cursor, opcode_guest_pc, &mut stmts)?
        }
        tables::OneByteOpcode::Xchg => decode_xchg(&prefixes, bytes, cursor, &mut stmts)?,
        tables::OneByteOpcode::PushReg => decode_push_r(&prefixes, opcode, &mut stmts),
        tables::OneByteOpcode::PopReg => decode_pop_r(&prefixes, opcode, &mut stmts),
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
        tables::TwoByteOpcode::MovzxI8 => decode_movzx(prefixes, bytes, start, stmts, OpSize::I8),
        tables::TwoByteOpcode::MovzxI16 => decode_movzx(prefixes, bytes, start, stmts, OpSize::I16),
        tables::TwoByteOpcode::MovsxI8 => decode_movsx(prefixes, bytes, start, stmts, OpSize::I8),
        tables::TwoByteOpcode::MovsxI16 => decode_movsx(prefixes, bytes, start, stmts, OpSize::I16),
        tables::TwoByteOpcode::Popcnt => {
            if prefixes.rep == Some(0xF3) {
                decode_popcnt(prefixes, bytes, start, stmts)
            } else {
                Err(crate::DecodeError::UnsupportedOpcode(op2))
            }
        }
        tables::TwoByteOpcode::Lzcnt => {
            if prefixes.rep == Some(0xF3) {
                decode_lzcnt(prefixes, bytes, start, stmts)
            } else {
                Err(crate::DecodeError::UnsupportedOpcode(op2))
            }
        }
        tables::TwoByteOpcode::Tzcnt => {
            if prefixes.rep == Some(0xF3) {
                decode_tzcnt(prefixes, bytes, start, stmts)
            } else {
                Err(crate::DecodeError::UnsupportedOpcode(op2))
            }
        }
        tables::TwoByteOpcode::CondJumpRel32 => {
            if prefixes.rex.present {
                Err(crate::DecodeError::UnsupportedOpcode(op2))
            } else if prefixes.rep == Some(0xF3) {
                Err(crate::DecodeError::UnsupportedOpcode(op2))
            } else if prefixes.operand_override {
                Err(crate::DecodeError::UnsupportedOpcode(op2))
            } else {
                decode_cond_jump_rel32(op2, bytes, cursor, opcode_guest_pc, stmts)
            }
        }
        tables::TwoByteOpcode::Unsupported => Err(crate::DecodeError::UnsupportedOpcode(op2)),
    }?;
    Ok(1 + consumed_after_escape)
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
    let rel = i64::from(
        i8::from_le_bytes([*bytes.get(cursor + 1).ok_or(crate::DecodeError::Truncated)?]),
    );
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
        rel_bytes.try_into().map_err(|_| crate::DecodeError::Truncated)?,
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

fn decode_mov_r_rm(
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

fn decode_binop_rm_r(
    kind: BinOpKind,
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let size = operand_size(prefixes, true);
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

#[allow(clippy::too_many_lines)]
fn decode_alu_rm_imm8(
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    opcode_guest_pc: u64,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let size = operand_size(prefixes, true);
    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;
    let kind = match modrm.reg {
        1 => Some(BinOpKind::Or),
        4 => Some(BinOpKind::And),
        6 => Some(BinOpKind::Xor),
        0 | 2 => Some(BinOpKind::Add),
        3 | 5 => Some(BinOpKind::Sub),
        7 => None,
        _ => return Err(crate::DecodeError::UnsupportedOpcode(0x83)),
    };
    let modrm_bytes = bytes_for_modrm(&modrm, bytes, cursor + 1)?;
    let imm_offset = cursor + 1 + modrm_bytes;
    let imm = i64::from(*bytes.get(imm_offset).ok_or(crate::DecodeError::Truncated)? as i8)
        .cast_unsigned();
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
            return Ok(1 + modrm_bytes + 1);
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
        let rip_after = opcode_guest_pc.wrapping_add(1 + modrm_bytes as u64 + 1);
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
            return Ok(1 + modrm_bytes + 1);
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
    Ok(1 + modrm_bytes + 1)
}

fn decode_cmp_rm_r(
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let size = operand_size(prefixes, true);
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

fn decode_test_rm_r(
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let size = operand_size(prefixes, true);
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
        Op::BinOp(BinOp {
            op: BinOpKind::And,
            lhs,
            rhs,
            size,
        }),
    ));
    // TEST doesn't write result to a register — it only sets flags
    // The result is unused, DCE will remove it
    Ok(1 + used)
}

fn decode_xchg(
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
) -> Result<usize, crate::DecodeError> {
    let size = operand_size(prefixes, true);
    let (modrm, _after) = modrm::parse_modrm(bytes, cursor + 1)?;
    let reg_a = map_reg(modrm.reg, &prefixes.rex, true);
    let reg_b = map_reg(modrm.rm, &prefixes.rex, false);

    let val_a = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(val_a),
        Op::LoadReg(LoadReg { reg: reg_a, size }),
    ));
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

fn decode_movzx(
    prefixes: &PrefixSet,
    bytes: &[u8],
    cursor: usize,
    stmts: &mut Vec<Stmt>,
    from: OpSize,
) -> Result<usize, crate::DecodeError> {
    let size = OpSize::I64;
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
    let size = OpSize::I64;
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
    Ok(1 + used)
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
        BinOp, BinOpKind, CondCode, CondJumpRel, Constant, Extend, Gpr, LoadMem, LoadReg, Op, OpSize,
        Popcnt, RspAdjust, Stmt, StoreReg,
    };

    #[test]
    fn decode_nop() {
        let d = decode_one(b"\x90", 0).unwrap();
        assert_eq!(d.stmts.len(), 0);
        assert_eq!(d.bytes_consumed, 1);
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
            ]
        );
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
    fn unsupported_opcode() {
        let r = decode_one(b"\x0F\x00", 0);
        assert!(r.is_err());
    }
}
