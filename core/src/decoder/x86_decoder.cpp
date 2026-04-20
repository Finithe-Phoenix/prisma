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

    // Reject extended registers for the MVP. When supported, reg_from_modrm()
    // will OR REX.R / REX.B into the index.
    if (rex.r || rex.x || rex.b) return DecodeError::UnsupportedEncoding;

    // 2. Opcode byte.
    const Byte opcode = bytes[cursor];
    ++cursor;

    // --- RET (C3) ----------------------------------------------------------
    if (opcode == 0xC3u) {
        if (rex.present) return DecodeError::UnsupportedEncoding;  // no REX on RET in MVP
        Decoded d;
        d.stmts.push_back({std::nullopt, ir::Return{}});
        d.bytes_consumed = cursor;
        return d;
    }

    // --- MOV r64, imm64 (B8+rd with REX.W) --------------------------------
    if (opcode >= 0xB8u && opcode <= 0xBFu) {
        if (!rex.w) return DecodeError::UnsupportedEncoding;  // MVP requires 64-bit
        const unsigned reg_idx = opcode - 0xB8u;
        auto imm = read_le<8>(bytes, cursor);
        if (!imm) return DecodeError::TruncatedInput;
        cursor += 8;

        Decoded d;
        const ir::Ref ref = next_ref++;
        d.stmts.push_back({ref, ir::Constant{*imm, ir::OpSize::I64}});
        d.stmts.push_back({std::nullopt, ir::StoreReg{gpr_from_index(reg_idx), ref, ir::OpSize::I64}});
        d.bytes_consumed = cursor;
        return d;
    }

    // --- ADD r/m64, r64 (01 /r) -------------------------------------------
    // --- SUB r/m64, r64 (29 /r) -------------------------------------------
    if (opcode == 0x01u || opcode == 0x29u) {
        if (!rex.w) return DecodeError::UnsupportedEncoding;
        if (cursor >= bytes.size()) return DecodeError::TruncatedInput;
        const Byte modrm = bytes[cursor++];
        const unsigned mod = (modrm >> 6) & 0x3u;
        const unsigned reg = (modrm >> 3) & 0x7u;   // src
        const unsigned rm  =  modrm       & 0x7u;   // dst (register-direct form)

        if (mod != 0b11u) return DecodeError::UnsupportedEncoding;

        const ir::BinOpKind k = (opcode == 0x01u) ? ir::BinOpKind::Add : ir::BinOpKind::Sub;

        Decoded d;
        const ir::Ref ref_dst = next_ref++;
        const ir::Ref ref_src = next_ref++;
        const ir::Ref ref_res = next_ref++;
        d.stmts.push_back({ref_dst, ir::LoadReg{gpr_from_index(rm), ir::OpSize::I64}});
        d.stmts.push_back({ref_src, ir::LoadReg{gpr_from_index(reg), ir::OpSize::I64}});
        d.stmts.push_back({ref_res, ir::BinOp{k, ref_dst, ref_src, ir::OpSize::I64}});
        d.stmts.push_back({std::nullopt, ir::StoreReg{gpr_from_index(rm), ref_res, ir::OpSize::I64}});
        d.bytes_consumed = cursor;
        return d;
    }

    return DecodeError::UnknownOpcode;
}

}  // namespace prisma::decoder
