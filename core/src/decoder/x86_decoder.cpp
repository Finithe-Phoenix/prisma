// core/src/decoder/x86_decoder.cpp — implementation of the MVP x86_64
// decoder declared in prisma/decoder.hpp.
//
// Format conventions (kept tight because the encoding reference matters):
//
//   REX prefix:   0100 WRXB  (0x40..0x4F)
//     W = operand size 64, R = ext reg, X = ext SIB.idx, B = ext rm/op-reg.
//
//   ModR/M byte:  mm rrr bbb
//     mm = 11 → both operands in registers (the only form this MVP accepts).
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

// Common shape for ALU register-register ops of the form
//   REX.W <opcode> <ModR/M mod=11, reg=src, rm=dst>
// that we lower to:
//   %dst0 = loadreg rm
//   %src  = loadreg reg
//   %res  = binop <op> %dst0, %src
//          storereg rm, %res
//
// `bytes` is the full stream. `cursor_in_out` points AFTER the opcode byte
// on entry and is advanced past the ModR/M byte on success.
std::variant<Decoded, DecodeError> decode_alu_rm_r(
    std::span<const Byte> bytes,
    std::size_t& cursor_in_out,
    ir::BinOpKind op,
    ir::Ref& next_ref) {
    if (cursor_in_out >= bytes.size()) return DecodeError::TruncatedInput;
    const Byte modrm = bytes[cursor_in_out++];
    const unsigned mod = (modrm >> 6) & 0x3u;
    const unsigned reg = (modrm >> 3) & 0x7u;   // src
    const unsigned rm  =  modrm       & 0x7u;   // dst

    if (mod != 0b11u) return DecodeError::UnsupportedEncoding;

    Decoded d;
    const ir::Ref ref_dst = next_ref++;
    const ir::Ref ref_src = next_ref++;
    const ir::Ref ref_res = next_ref++;
    d.stmts.push_back({ref_dst, ir::LoadReg{gpr_from_index(rm), ir::OpSize::I64}});
    d.stmts.push_back({ref_src, ir::LoadReg{gpr_from_index(reg), ir::OpSize::I64}});
    d.stmts.push_back({ref_res, ir::BinOp{op, ref_dst, ref_src, ir::OpSize::I64}});
    d.stmts.push_back({std::nullopt,
                       ir::StoreReg{gpr_from_index(rm), ref_res, ir::OpSize::I64}});
    d.bytes_consumed = cursor_in_out;
    return d;
}

// MOV r/m64, r64 (mod=11): no arithmetic, just StoreReg(dst, LoadReg(src)).
std::variant<Decoded, DecodeError> decode_mov_rm_r(
    std::span<const Byte> bytes,
    std::size_t& cursor_in_out,
    ir::Ref& next_ref) {
    if (cursor_in_out >= bytes.size()) return DecodeError::TruncatedInput;
    const Byte modrm = bytes[cursor_in_out++];
    const unsigned mod = (modrm >> 6) & 0x3u;
    const unsigned reg = (modrm >> 3) & 0x7u;   // src
    const unsigned rm  =  modrm       & 0x7u;   // dst

    if (mod != 0b11u) return DecodeError::UnsupportedEncoding;

    Decoded d;
    const ir::Ref ref_src = next_ref++;
    d.stmts.push_back({ref_src, ir::LoadReg{gpr_from_index(reg), ir::OpSize::I64}});
    d.stmts.push_back({std::nullopt,
                       ir::StoreReg{gpr_from_index(rm), ref_src, ir::OpSize::I64}});
    d.bytes_consumed = cursor_in_out;
    return d;
}

}  // namespace

std::variant<Decoded, DecodeError> decode_one(
    std::span<const Byte> bytes,
    ir::Ref& next_ref) {

    std::size_t cursor = 0;
    if (bytes.empty()) return DecodeError::TruncatedInput;

    // 1. Optional REX prefix.
    RexPrefix rex;
    if (Byte b = bytes[cursor]; (b & 0xF0u) == 0x40u) {
        rex.present = true;
        rex.w = (b & 0x08u) != 0;
        rex.r = (b & 0x04u) != 0;
        rex.x = (b & 0x02u) != 0;
        rex.b = (b & 0x01u) != 0;
        ++cursor;
        if (cursor >= bytes.size()) return DecodeError::TruncatedInput;
    }

    // Reject extended registers for the MVP.
    if (rex.r || rex.x || rex.b) return DecodeError::UnsupportedEncoding;

    // 2. Opcode byte.
    const Byte opcode = bytes[cursor];
    ++cursor;

    // --- NOP (0x90) --------------------------------------------------------
    // Strictly speaking NOP is an alias of XCHG EAX, EAX, but in the MVP we
    // consume it as a no-op. It is legal to have REX.W before NOP; we accept
    // that too (treating 48 90 as a 64-bit no-op).
    if (opcode == 0x90u) {
        Decoded d;
        d.bytes_consumed = cursor;
        return d;
    }

    // --- RET (0xC3) --------------------------------------------------------
    if (opcode == 0xC3u) {
        if (rex.present) return DecodeError::UnsupportedEncoding;  // no REX on RET in MVP
        Decoded d;
        d.stmts.push_back({std::nullopt, ir::Return{}});
        d.bytes_consumed = cursor;
        return d;
    }

    // --- MOV r64, imm64 (B8+rd with REX.W) --------------------------------
    if (opcode >= 0xB8u && opcode <= 0xBFu) {
        if (!rex.w) return DecodeError::UnsupportedEncoding;
        const unsigned reg_idx = opcode - 0xB8u;
        auto imm = read_le<8>(bytes, cursor);
        if (!imm) return DecodeError::TruncatedInput;
        cursor += 8;

        Decoded d;
        const ir::Ref ref = next_ref++;
        d.stmts.push_back({ref, ir::Constant{*imm, ir::OpSize::I64}});
        d.stmts.push_back({std::nullopt,
                           ir::StoreReg{gpr_from_index(reg_idx), ref, ir::OpSize::I64}});
        d.bytes_consumed = cursor;
        return d;
    }

    // --- MOV r/m64, r64 (0x89 /r) -----------------------------------------
    if (opcode == 0x89u) {
        if (!rex.w) return DecodeError::UnsupportedEncoding;
        return decode_mov_rm_r(bytes, cursor, next_ref);
    }

    // --- ALU r/m64, r64 (shared shape) ------------------------------------
    // OR  = 0x09  → Or
    // AND = 0x21  → And
    // SUB = 0x29  → Sub
    // XOR = 0x31  → Xor
    // ADD = 0x01  → Add
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
