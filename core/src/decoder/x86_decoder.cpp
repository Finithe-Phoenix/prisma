// core/src/decoder/x86_decoder.cpp — implementation of the MVP x86_64
// decoder declared in prisma/decoder.hpp.
//
// Format conventions (kept tight because the encoding reference matters):
//
//   REX prefix:   0100 WRXB  (0x40..0x4F)
//     W = operand size 64, R = ext reg, X = ext SIB.idx, B = ext rm/op-reg.
//
//   ModR/M byte:  mm rrr bbb
//     mm = 00  → [rm]            (except rm=100 SIB, rm=101 disp32 abs)
//     mm = 01  → [rm + disp8]    (except rm=100 SIB)
//     mm = 10  → [rm + disp32]   (except rm=100 SIB)
//     mm = 11  → register direct
//     rrr = reg field (extended by REX.R; we reject REX.R for now).
//     bbb = rm field  (extended by REX.B; we reject REX.B for now).

#include "prisma/decoder.hpp"

#include <cstdint>
#include <cstring>
#include <optional>
#include <variant>

namespace prisma::decoder {

namespace {

using Byte = std::uint8_t;

struct RexPrefix {
    bool present{false};
    bool w{false};
    bool r{false};
    bool x{false};
    bool b{false};
};

// Parsed ModR/M operand. If `mod == 11` it's a register direct reference
// (`base` is the register, displacement unused). Otherwise it describes a
// memory access with base register + signed displacement.
struct ModRmOperand {
    unsigned mod;          // 0..3
    unsigned reg;          // 0..7, the `reg` field
    ir::Gpr  base;         // the `rm` field interpreted as a GPR
    std::int32_t disp;     // 0 for mod=00, signed for mod=01/10
};

constexpr ir::Gpr gpr_from_index(unsigned idx) noexcept {
    return static_cast<ir::Gpr>(idx);
}

// Read `N` bytes at `off`, little-endian, as a uint64_t zero-extended.
template <std::size_t N>
std::optional<std::uint64_t> read_le(std::span<const Byte> bytes, std::size_t off) {
    static_assert(N >= 1 && N <= 8, "read_le: N must be 1..8");
    if (off + N > bytes.size()) return std::nullopt;
    std::uint64_t v = 0;
    for (std::size_t i = 0; i < N; ++i) {
        v |= static_cast<std::uint64_t>(bytes[off + i]) << (8u * i);
    }
    return v;
}

// Sign-extend an unsigned N-byte value to int32_t (N <= 4).
template <std::size_t N>
std::int32_t sign_extend_i32(std::uint64_t v) {
    static_assert(N == 1 || N == 2 || N == 4, "sign_extend_i32: N in {1,2,4}");
    if constexpr (N == 1) {
        return static_cast<std::int32_t>(static_cast<std::int8_t>(v));
    } else if constexpr (N == 2) {
        return static_cast<std::int32_t>(static_cast<std::int16_t>(v));
    } else {
        return static_cast<std::int32_t>(static_cast<std::uint32_t>(v));
    }
}

// Parse ModR/M (and any disp8 / disp32 that follows) starting at `cursor`.
// On success, advances `cursor` past all consumed bytes and returns the
// parsed operand. Rejects encodings that require SIB (rm=100 in modes
// 00/01/10) or disp32 absolute (mod=00 rm=101) — MVP scope.
std::variant<ModRmOperand, DecodeError> parse_modrm(
    std::span<const Byte> bytes,
    std::size_t& cursor) {
    if (cursor >= bytes.size()) return DecodeError::TruncatedInput;
    const Byte modrm = bytes[cursor++];

    ModRmOperand m;
    m.mod = (modrm >> 6) & 0x3u;
    m.reg = (modrm >> 3) & 0x7u;
    const unsigned rm = modrm & 0x7u;
    m.base = gpr_from_index(rm);
    m.disp = 0;

    if (m.mod == 0b11u) {
        // Register direct; no further bytes.
        return m;
    }

    // Reject SIB-required encodings (rm=100 in any memory mode).
    if (rm == 0b100u) return DecodeError::UnsupportedEncoding;

    // Reject disp32-absolute (mod=00 rm=101).
    if (m.mod == 0b00u && rm == 0b101u) return DecodeError::UnsupportedEncoding;

    // mod=00: no displacement; rm gives base register directly.
    if (m.mod == 0b00u) return m;

    // mod=01: disp8 follows.
    if (m.mod == 0b01u) {
        auto d = read_le<1>(bytes, cursor);
        if (!d) return DecodeError::TruncatedInput;
        cursor += 1;
        m.disp = sign_extend_i32<1>(*d);
        return m;
    }

    // mod=10: disp32 follows.
    {
        auto d = read_le<4>(bytes, cursor);
        if (!d) return DecodeError::TruncatedInput;
        cursor += 4;
        m.disp = sign_extend_i32<4>(*d);
        return m;
    }
}

// Emit the IR that computes a memory effective address and returns the
// Ref holding the final 64-bit address. Assumes `op.mod != 11`.
// Always emits:
//   %base = loadreg base
//   (if disp != 0): %disp_c = const disp; %addr = add %base, %disp_c
//   (if disp == 0): just returns %base ref
ir::Ref emit_address(
    std::vector<ir::Stmt>& stmts,
    const ModRmOperand& op,
    ir::Ref& next_ref) {
    const ir::Ref ref_base = next_ref++;
    stmts.push_back({ref_base, ir::LoadReg{op.base, ir::OpSize::I64}});
    if (op.disp == 0) return ref_base;
    const ir::Ref ref_disp = next_ref++;
    const ir::Ref ref_addr = next_ref++;
    // Sign-extend disp to 64-bit (the host IR is 64-bit; Constant value is u64).
    const std::uint64_t disp_u64 =
        static_cast<std::uint64_t>(static_cast<std::int64_t>(op.disp));
    stmts.push_back({ref_disp, ir::Constant{disp_u64, ir::OpSize::I64}});
    stmts.push_back({ref_addr,
                     ir::BinOp{ir::BinOpKind::Add, ref_base, ref_disp, ir::OpSize::I64}});
    return ref_addr;
}

// Common shape for ALU register-register ops (mod=11 only for MVP).
// Memory destination forms of ADD/OR/AND/SUB/XOR (mod != 11) are future
// work — they require reading the memory operand, doing the op, and
// writing it back atomically. Out of scope for this session.
std::variant<Decoded, DecodeError> decode_alu_rm_r(
    std::span<const Byte> bytes,
    std::size_t& cursor_in_out,
    ir::BinOpKind op,
    ir::Ref& next_ref) {
    auto modrm = parse_modrm(bytes, cursor_in_out);
    if (std::holds_alternative<DecodeError>(modrm)) {
        return std::get<DecodeError>(modrm);
    }
    const auto& m = std::get<ModRmOperand>(modrm);
    if (m.mod != 0b11u) return DecodeError::UnsupportedEncoding;

    Decoded d;
    const ir::Ref ref_dst = next_ref++;
    const ir::Ref ref_src = next_ref++;
    const ir::Ref ref_res = next_ref++;
    d.stmts.push_back({ref_dst, ir::LoadReg{m.base, ir::OpSize::I64}});
    d.stmts.push_back({ref_src, ir::LoadReg{gpr_from_index(m.reg), ir::OpSize::I64}});
    d.stmts.push_back({ref_res, ir::BinOp{op, ref_dst, ref_src, ir::OpSize::I64}});
    d.stmts.push_back({std::nullopt,
                       ir::StoreReg{m.base, ref_res, ir::OpSize::I64}});
    d.bytes_consumed = cursor_in_out;
    return d;
}

// MOV r/m<size>, r<size>  (0x89 /r) — stores REG into R/M.
// In register direct (mod=11): just StoreReg(rm, LoadReg(reg)).
// In memory form: StoreMemTSO(addr(rm), LoadReg(reg)).
std::variant<Decoded, DecodeError> decode_mov_rm_r(
    std::span<const Byte> bytes,
    std::size_t& cursor_in_out,
    ir::Ref& next_ref,
    ir::OpSize size) {
    auto modrm = parse_modrm(bytes, cursor_in_out);
    if (std::holds_alternative<DecodeError>(modrm)) {
        return std::get<DecodeError>(modrm);
    }
    const auto& m = std::get<ModRmOperand>(modrm);

    Decoded d;
    if (m.mod == 0b11u) {
        const ir::Ref ref_src = next_ref++;
        d.stmts.push_back({ref_src,
                           ir::LoadReg{gpr_from_index(m.reg), size}});
        d.stmts.push_back({std::nullopt,
                           ir::StoreReg{m.base, ref_src, size}});
    } else {
        const ir::Ref ref_src = next_ref++;
        d.stmts.push_back({ref_src,
                           ir::LoadReg{gpr_from_index(m.reg), size}});
        const ir::Ref ref_addr = emit_address(d.stmts, m, next_ref);
        d.stmts.push_back({std::nullopt,
                           ir::StoreMemTSO{ref_addr, ref_src, size}});
    }
    d.bytes_consumed = cursor_in_out;
    return d;
}

// MOV r<size>, r/m<size>  (0x8B /r) — loads from R/M into REG.
// In register direct (mod=11): same as 0x89 with roles swapped.
// In memory form: LoadMemTSO(addr(rm)) → StoreReg(reg).
std::variant<Decoded, DecodeError> decode_mov_r_rm(
    std::span<const Byte> bytes,
    std::size_t& cursor_in_out,
    ir::Ref& next_ref,
    ir::OpSize size) {
    auto modrm = parse_modrm(bytes, cursor_in_out);
    if (std::holds_alternative<DecodeError>(modrm)) {
        return std::get<DecodeError>(modrm);
    }
    const auto& m = std::get<ModRmOperand>(modrm);

    Decoded d;
    if (m.mod == 0b11u) {
        const ir::Ref ref_src = next_ref++;
        d.stmts.push_back({ref_src, ir::LoadReg{m.base, size}});
        d.stmts.push_back({std::nullopt,
                           ir::StoreReg{gpr_from_index(m.reg), ref_src, size}});
    } else {
        const ir::Ref ref_addr = emit_address(d.stmts, m, next_ref);
        const ir::Ref ref_loaded = next_ref++;
        d.stmts.push_back({ref_loaded,
                           ir::LoadMemTSO{ref_addr, size}});
        d.stmts.push_back({std::nullopt,
                           ir::StoreReg{gpr_from_index(m.reg), ref_loaded, size}});
    }
    d.bytes_consumed = cursor_in_out;
    return d;
}

}  // namespace

std::variant<Decoded, DecodeError> decode_one(
    std::span<const Byte> bytes,
    ir::Ref& next_ref,
    std::uint64_t instruction_guest_pc) {

    std::size_t cursor = 0;
    if (bytes.empty()) return DecodeError::TruncatedInput;

    // 1. Optional prefixes.
    //
    // We currently only support:
    //   * 0x66 (operand-size override),
    //   * one REX prefix.
    // More prefix combinations can be added later.
    bool has_operand_size_override = false;
    RexPrefix rex;
    bool seen_rex = false;
    bool seen_66 = false;
    while (cursor < bytes.size()) {
        const Byte b = bytes[cursor];
        if (b == 0x66u) {
            if (seen_66) return DecodeError::UnsupportedEncoding;
            has_operand_size_override = true;
            seen_66 = true;
            ++cursor;
            continue;
        }

        if ((b & 0xF0u) == 0x40u) {
            if (seen_rex) return DecodeError::UnsupportedEncoding;
            rex.present = true;
            rex.w = (b & 0x08u) != 0;
            rex.r = (b & 0x04u) != 0;
            rex.x = (b & 0x02u) != 0;
            rex.b = (b & 0x01u) != 0;
            seen_rex = true;
            ++cursor;
            continue;
        }
        break;
    }
    if (cursor >= bytes.size()) return DecodeError::TruncatedInput;

    // Reject extended registers for the MVP.
    if (rex.r || rex.x || rex.b) return DecodeError::UnsupportedEncoding;

    // 2. Opcode byte.
    const Byte opcode = bytes[cursor];
    ++cursor;

    // --- NOP (0x90) --------------------------------------------------------
    if (opcode == 0x90u) {
        if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
        Decoded d;
        d.bytes_consumed = cursor;
        return d;
    }

    // --- RET (0xC3) --------------------------------------------------------
    if (opcode == 0xC3u) {
        if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
        if (rex.present) return DecodeError::UnsupportedEncoding;
        Decoded d;
        d.stmts.push_back({std::nullopt, ir::Return{}});
        d.bytes_consumed = cursor;
        return d;
    }

    // --- JMP rel8 (EB cb) --------------------------------------------------
    if (opcode == 0xEBu) {
        if (rex.present) return DecodeError::UnsupportedEncoding;
        if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
        auto imm = read_le<1>(bytes, cursor);
        if (!imm) return DecodeError::TruncatedInput;
        cursor += 1;
        const std::int32_t rel = sign_extend_i32<1>(*imm);
        const std::uint64_t target =
            instruction_guest_pc + cursor + static_cast<std::uint64_t>(static_cast<std::int64_t>(rel));
        Decoded d;
        d.stmts.push_back({std::nullopt, ir::JumpRel{target}});
        d.bytes_consumed = cursor;
        return d;
    }

    // --- JMP rel32 (E9 cd) -------------------------------------------------
    if (opcode == 0xE9u) {
        if (rex.present) return DecodeError::UnsupportedEncoding;
        if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
        auto imm = read_le<4>(bytes, cursor);
        if (!imm) return DecodeError::TruncatedInput;
        cursor += 4;
        const std::int32_t rel = sign_extend_i32<4>(*imm);
        const std::uint64_t target =
            instruction_guest_pc + cursor + static_cast<std::uint64_t>(static_cast<std::int64_t>(rel));
        Decoded d;
        d.stmts.push_back({std::nullopt, ir::JumpRel{target}});
        d.bytes_consumed = cursor;
        return d;
    }

    // --- Jcc rel8 (74/75 for JE/JNE; more conditions arrive later) --------
    if (opcode == 0x74u || opcode == 0x75u) {
        if (rex.present) return DecodeError::UnsupportedEncoding;
        if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
        auto imm = read_le<1>(bytes, cursor);
        if (!imm) return DecodeError::TruncatedInput;
        cursor += 1;
        const std::int32_t rel = sign_extend_i32<1>(*imm);
        const std::uint64_t target =
            instruction_guest_pc + cursor + static_cast<std::uint64_t>(static_cast<std::int64_t>(rel));
        const std::uint64_t fallthrough = instruction_guest_pc + cursor;
        const ir::CondCode cc =
            (opcode == 0x74u) ? ir::CondCode::Eq : ir::CondCode::Ne;
        Decoded d;
        d.stmts.push_back({std::nullopt, ir::CondJumpRel{cc, target, fallthrough}});
        d.bytes_consumed = cursor;
        return d;
    }

    // --- CMP r/m64, r64 (48 39 /r, mod=11) --------------------------------
    //
    // mod != 11 (memory) left for a future session — the lowering doesn't
    // have to change, but decoder-level it reuses the memory-address
    // helper. Skipped to keep this commit tight.
    if (opcode == 0x39u) {
        if (!rex.w) return DecodeError::UnsupportedEncoding;
        if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
        auto modrm = parse_modrm(bytes, cursor);
        if (std::holds_alternative<DecodeError>(modrm)) {
            return std::get<DecodeError>(modrm);
        }
        const auto& m = std::get<ModRmOperand>(modrm);
        if (m.mod != 0b11u) return DecodeError::UnsupportedEncoding;

        Decoded d;
        const ir::Ref ref_lhs = next_ref++;
        const ir::Ref ref_rhs = next_ref++;
        d.stmts.push_back({ref_lhs, ir::LoadReg{m.base, ir::OpSize::I64}});
        d.stmts.push_back({ref_rhs, ir::LoadReg{gpr_from_index(m.reg), ir::OpSize::I64}});
        d.stmts.push_back({std::nullopt,
                           ir::CmpFlags{ref_lhs, ref_rhs, ir::OpSize::I64}});
        d.bytes_consumed = cursor;
        return d;
    }

    // --- MOV r<size>, imm<size> (B8+rd) ------------------------------------
    //
    // REX.W = 1 => I64, imm64
    // REX.W = 0, no 0x66 => I32, imm32 (zero-extend)
    // REX.W = 0, 0x66 => I16, imm16
    if (opcode >= 0xB8u && opcode <= 0xBFu) {
        const unsigned reg_idx = opcode - 0xB8u;
        const ir::OpSize size = rex.w ? ir::OpSize::I64
                                     : (has_operand_size_override ? ir::OpSize::I16
                                                                 : ir::OpSize::I32);

        if (rex.w && has_operand_size_override) return DecodeError::UnsupportedEncoding;

        std::size_t imm_size_bytes = 0;
        if (size == ir::OpSize::I16) imm_size_bytes = 2;
        else if (size == ir::OpSize::I32) imm_size_bytes = 4;
        else imm_size_bytes = 8;

        std::uint64_t imm = 0;
        if (imm_size_bytes == 2) {
            auto x = read_le<2>(bytes, cursor);
            if (!x) return DecodeError::TruncatedInput;
            cursor += 2;
            imm = *x;
        } else if (imm_size_bytes == 4) {
            auto x = read_le<4>(bytes, cursor);
            if (!x) return DecodeError::TruncatedInput;
            cursor += 4;
            imm = *x;
        } else {
            auto x = read_le<8>(bytes, cursor);
            if (!x) return DecodeError::TruncatedInput;
            cursor += 8;
            imm = *x;
        }

        Decoded d;
        const ir::Ref ref = next_ref++;
        d.stmts.push_back({ref, ir::Constant{imm, size}});
        d.stmts.push_back({std::nullopt,
                           ir::StoreReg{gpr_from_index(reg_idx), ref, size}});
        d.bytes_consumed = cursor;
        return d;
    }

    // --- MOV r/m<size>, r<size> (0x89 /r) --------------------------------
    if (opcode == 0x89u) {
        const ir::OpSize size = rex.w ? ir::OpSize::I64
                                      : (has_operand_size_override ? ir::OpSize::I16
                                                                  : ir::OpSize::I32);
        if (size == ir::OpSize::I64 && has_operand_size_override) {
            return DecodeError::UnsupportedEncoding;
        }
        return decode_mov_rm_r(bytes, cursor, next_ref, size);
    }

    // --- MOV r<size>, r/m<size> (0x8A /r) --------------------------------
    if (opcode == 0x8Au) {
        if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
        return decode_mov_r_rm(bytes, cursor, next_ref, ir::OpSize::I8);
    }

    // --- MOV r/m<size>, r<size> (0x88 /r) --------------------------------
    if (opcode == 0x88u) {
        if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
        return decode_mov_rm_r(bytes, cursor, next_ref, ir::OpSize::I8);
    }

    // --- MOV r<size>, r/m<size> (0x8B /r) --------------------------------
    if (opcode == 0x8Bu) {
        const ir::OpSize size = rex.w ? ir::OpSize::I64
                                      : (has_operand_size_override ? ir::OpSize::I16
                                                                  : ir::OpSize::I32);
        if (size == ir::OpSize::I64 && has_operand_size_override) {
            return DecodeError::UnsupportedEncoding;
        }
        return decode_mov_r_rm(bytes, cursor, next_ref, size);
    }

    // --- MOV r<size>, imm<size> (B0+rd for I8, B8+rd for I16/I32/I64) ----
    if (opcode >= 0xB0u && opcode <= 0xB7u) {
        if (has_operand_size_override) return DecodeError::UnsupportedEncoding;

        const unsigned reg_idx = opcode - 0xB0u;
        auto imm = read_le<1>(bytes, cursor);
        if (!imm) return DecodeError::TruncatedInput;
        cursor += 1;

        Decoded d;
        const ir::Ref ref = next_ref++;
        d.stmts.push_back({ref, ir::Constant{*imm, ir::OpSize::I8}});
        d.stmts.push_back({std::nullopt,
                           ir::StoreReg{gpr_from_index(reg_idx), ref, ir::OpSize::I8}});
        d.bytes_consumed = cursor;
        return d;
    }

    // --- ALU r/m64, r64 (shared shape, register-direct only for MVP) ------
    switch (opcode) {
        case 0x01u:
            if (!rex.w) return DecodeError::UnsupportedEncoding;
            return decode_alu_rm_r(bytes, cursor, ir::BinOpKind::Add, next_ref);
        case 0x09u:
            if (!rex.w) return DecodeError::UnsupportedEncoding;
            return decode_alu_rm_r(bytes, cursor, ir::BinOpKind::Or, next_ref);
        case 0x21u:
            if (!rex.w) return DecodeError::UnsupportedEncoding;
            return decode_alu_rm_r(bytes, cursor, ir::BinOpKind::And, next_ref);
        case 0x29u:
            if (!rex.w) return DecodeError::UnsupportedEncoding;
            return decode_alu_rm_r(bytes, cursor, ir::BinOpKind::Sub, next_ref);
        case 0x31u:
            if (!rex.w) return DecodeError::UnsupportedEncoding;
            return decode_alu_rm_r(bytes, cursor, ir::BinOpKind::Xor, next_ref);
        default:
            break;
    }

    return DecodeError::UnknownOpcode;
}

}  // namespace prisma::decoder
