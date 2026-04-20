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

std::variant<ModRmOperand, DecodeError> parse_modrm(
    std::span<const Byte> bytes,
    std::size_t& cursor);
ir::Ref emit_address(
    std::vector<ir::Stmt>& stmts,
    const ModRmOperand& op,
    ir::Ref& next_ref);
std::variant<Decoded, DecodeError> decode_cmpxchg_r64_rm64(
    std::span<const Byte> bytes,
    std::size_t& cursor,
    ir::Ref& next_ref);
std::variant<Decoded, DecodeError> decode_cmpxchg16b_m128(
    std::span<const Byte> bytes,
    std::size_t& cursor,
    ir::Ref& next_ref);
std::variant<Decoded, DecodeError> decode_xadd_r64_rm64(
    std::span<const Byte> bytes,
    std::size_t& cursor,
    ir::Ref& next_ref);

constexpr ir::Gpr gpr_from_index(unsigned idx) noexcept {
    return static_cast<ir::Gpr>(idx);
}

// Map Jcc encodings (70–7F, 0F80–8F) into IR CondCode.
// PF / NPF branches are currently unsupported due the lack of parity in IR.
std::optional<ir::CondCode> jcc_condition(Byte opcode_or_subop) noexcept {
    switch (opcode_or_subop & 0x0Fu) {
        case 0x0u: return ir::CondCode::Ov;   // jo/jno? see opcode suffix below
        case 0x1u: return ir::CondCode::NoOv;
        case 0x2u: return ir::CondCode::Nc;   // carry set
        case 0x3u: return ir::CondCode::Cc;   // carry clear
        case 0x4u: return ir::CondCode::Eq;
        case 0x5u: return ir::CondCode::Ne;
        case 0x6u: return ir::CondCode::Ule;
        case 0x7u: return ir::CondCode::Ugt;
        case 0x8u: return ir::CondCode::Mi;
        case 0x9u: return ir::CondCode::Pl;
        case 0xAu: return std::nullopt;  // PF / NPF unsupported
        case 0xBu: return std::nullopt;
        case 0xCu: return ir::CondCode::Slt;
        case 0xDu: return ir::CondCode::Sge;
        case 0xEu: return ir::CondCode::Sle;
        case 0xFu: return ir::CondCode::Sgt;
        default: return std::nullopt;
    }
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

// Emit sign- or zero-extension from a small integer width into 64-bit.
//
// We build this from two shifts because the MVP IR has no explicit cast
// op:
//   sign-extend:  (src << k) >>_s k
//   zero-extend:  (src << k) >>_u k
// where k = 64 - source_width_bits.
// The emitted size is always I64 because the destination register is I64
// for all movx opcodes in this MVP slice.
ir::Ref emit_movx_cast(
    std::vector<ir::Stmt>& stmts,
    ir::Ref value,
    ir::OpSize src_size,
    bool is_signed,
    ir::Ref& next_ref) {
    const std::uint64_t k = src_size == ir::OpSize::I8
                                ? 56u
                                : (src_size == ir::OpSize::I16 ? 48u : 32u);
    const ir::Ref ref_k = next_ref++;
    const ir::Ref ref_shifted = next_ref++;
    const ir::Ref ref_extended = next_ref++;
    stmts.push_back({ref_k, ir::Constant{k, ir::OpSize::I64}});
    stmts.push_back(
        {ref_shifted, ir::BinOp{ir::BinOpKind::Shl, value, ref_k, ir::OpSize::I64}});
    stmts.push_back({ref_extended,
                     ir::BinOp{is_signed ? ir::BinOpKind::Sar : ir::BinOpKind::Shr,
                               ref_shifted, ref_k, ir::OpSize::I64}});
    return ref_extended;
}

// Decode CBW/CWDE/CDQE (98) as sign extension through RAX.
//
// The destination width is derived from operand-size flags:
//   * no prefix, no REX.W -> CWDE (I16 -> I32)
//   * 0x66 prefix, no REX.W -> CBW (I8 -> I16)
//   * REX.W -> CDQE (I32 -> I64)
std::variant<Decoded, DecodeError> decode_cbw_cwde_cdqe(
    std::size_t bytes_consumed,
    ir::Ref& next_ref,
    ir::OpSize src_size,
    ir::OpSize dst_size) {
    Decoded d;
    const ir::Ref ref_src = next_ref++;
    d.stmts.push_back({ref_src, ir::LoadReg{ir::Gpr::Rax, src_size}});

    const ir::Ref ref_extended = emit_movx_cast(d.stmts, ref_src, src_size, true, next_ref);
    d.stmts.push_back({std::nullopt, ir::StoreReg{ir::Gpr::Rax, ref_extended, dst_size}});
    d.bytes_consumed = bytes_consumed;
    return d;
}

// Decode CWD/CDQ/CQO (99) as sign replication across DX:AX / EDX:EAX / RDX:RAX.
//
// The value in RAX of width `src_size` is copied to the low half (AX/EAX/RAX),
// and its sign bit is replicated into the high half (DX/EDX/RDX).
std::variant<Decoded, DecodeError> decode_cwd_cdq_cqo(
    std::size_t bytes_consumed,
    ir::Ref& next_ref,
    ir::OpSize src_size) {
    Decoded d;
    const ir::Ref ref_src = next_ref++;
    d.stmts.push_back({ref_src, ir::LoadReg{ir::Gpr::Rax, src_size}});

    const std::uint64_t shift_amount =
        src_size == ir::OpSize::I16 ? 15u : (src_size == ir::OpSize::I32 ? 31u : 63u);
    const ir::Ref ref_shift = next_ref++;
    const ir::Ref ref_high = next_ref++;
    d.stmts.push_back({ref_shift, ir::Constant{shift_amount, ir::OpSize::I64}});
    d.stmts.push_back(
        {ref_high, ir::BinOp{ir::BinOpKind::Sar, ref_src, ref_shift, src_size}});
    d.stmts.push_back({std::nullopt, ir::StoreReg{ir::Gpr::Rax, ref_src, src_size}});
    d.stmts.push_back({std::nullopt, ir::StoreReg{ir::Gpr::Rdx, ref_high, src_size}});
    d.bytes_consumed = bytes_consumed;
    return d;
}

// MOVSX/MOVZX r64, r/m8/16/32 (0F BE / BF, 63, 0F B6 / B7).
//
// Supported forms:
//   * 0F BE /r: sign-extend i8 to r64
//   * 0F BF /r: sign-extend i16 to r64
//   * 63 /r:    sign-extend i32 to r64 (MOVSXD)
//   * 0F B6 /r: zero-extend i8 to r64
//   * 0F B7 /r: zero-extend i16 to r64
//
// Decode shape:
//   dst = reg field
//   src = mem/register at rm
//   dst = movx(src)
std::variant<Decoded, DecodeError> decode_movx_r64_rm(
    std::span<const Byte> bytes,
    std::size_t& cursor,
    ir::Ref& next_ref,
    ir::OpSize src_size,
    bool sign_extend) {
    auto modrm = parse_modrm(bytes, cursor);
    if (std::holds_alternative<DecodeError>(modrm)) {
        return std::get<DecodeError>(modrm);
    }
    const auto& m = std::get<ModRmOperand>(modrm);

    Decoded d;
    // Allocate refs in statement-result order so the SSA stream reads
    // linearly (test expectations rely on this).
    ir::Ref ref_src;
    if (m.mod == 0b11u) {
        ref_src = next_ref++;
        d.stmts.push_back({ref_src, ir::LoadReg{m.base, src_size}});
    } else {
        const ir::Ref ref_addr = emit_address(d.stmts, m, next_ref);
        ref_src = next_ref++;
        d.stmts.push_back({ref_src, ir::LoadMemTSO{ref_addr, src_size}});
    }

    const ir::Ref ref_extended = emit_movx_cast(d.stmts, ref_src, src_size, sign_extend, next_ref);
    d.stmts.push_back(
        {std::nullopt, ir::StoreReg{gpr_from_index(m.reg), ref_extended, ir::OpSize::I64}});
    d.bytes_consumed = cursor;
    return d;
}

// CMOVcc r64, r/m64 (0F 40–4F /r).
// Encodes: dst = cond ? src : dst.
std::variant<Decoded, DecodeError> decode_cmovcc_r64_rm64(
    ir::CondCode cc,
    std::span<const Byte> bytes,
    std::size_t& cursor,
    ir::Ref& next_ref) {
    auto modrm = parse_modrm(bytes, cursor);
    if (std::holds_alternative<DecodeError>(modrm)) {
        return std::get<DecodeError>(modrm);
    }
    const auto& m = std::get<ModRmOperand>(modrm);

    Decoded d;
    const ir::Ref ref_dst = next_ref++;
    d.stmts.push_back({ref_dst, ir::LoadReg{gpr_from_index(m.reg), ir::OpSize::I64}});

    ir::Ref ref_src;
    if (m.mod == 0b11u) {
        ref_src = next_ref++;
        d.stmts.push_back({ref_src, ir::LoadReg{m.base, ir::OpSize::I64}});
    } else {
        const ir::Ref ref_addr = emit_address(d.stmts, m, next_ref);
        ref_src = next_ref++;
        d.stmts.push_back({ref_src, ir::LoadMemTSO{ref_addr, ir::OpSize::I64}});
    }

    const ir::Ref ref_selected = next_ref++;
    d.stmts.push_back({ref_selected, ir::Select{cc, ref_src, ref_dst, ir::OpSize::I64}});
    d.stmts.push_back(
        {std::nullopt, ir::StoreReg{gpr_from_index(m.reg), ref_selected, ir::OpSize::I64}});
    d.bytes_consumed = cursor;
    return d;
}

// XCHG r64, r/m64 (87 /r).
// Encodes: swap(dst, src).
std::variant<Decoded, DecodeError> decode_xchg_r64_rm64(
    std::span<const Byte> bytes,
    std::size_t& cursor,
    ir::Ref& next_ref) {
    auto modrm = parse_modrm(bytes, cursor);
    if (std::holds_alternative<DecodeError>(modrm)) {
        return std::get<DecodeError>(modrm);
    }
    const auto& m = std::get<ModRmOperand>(modrm);

    Decoded d;
    const ir::Ref ref_reg = next_ref++;
    d.stmts.push_back({ref_reg, ir::LoadReg{gpr_from_index(m.reg), ir::OpSize::I64}});

    if (m.mod == 0b11u) {
        const ir::Ref ref_mem_or_reg = next_ref++;
        d.stmts.push_back({ref_mem_or_reg, ir::LoadReg{m.base, ir::OpSize::I64}});
        d.stmts.push_back(
            {std::nullopt, ir::StoreReg{m.base, ref_reg, ir::OpSize::I64}});
        d.stmts.push_back(
            {std::nullopt,
             ir::StoreReg{gpr_from_index(m.reg), ref_mem_or_reg, ir::OpSize::I64}});
    } else {
        const ir::Ref ref_addr = emit_address(d.stmts, m, next_ref);
        const ir::Ref ref_mem = next_ref++;
        d.stmts.push_back({ref_mem, ir::LoadMemTSO{ref_addr, ir::OpSize::I64}});
        d.stmts.push_back(
            {std::nullopt, ir::StoreReg{gpr_from_index(m.reg), ref_mem, ir::OpSize::I64}});
        d.stmts.push_back({std::nullopt, ir::StoreMemTSO{ref_addr, ref_reg, ir::OpSize::I64}});
    }

    d.bytes_consumed = cursor;
    return d;
}

// CMPXCHG r/m64, r64 (0F B1 /r).
// Encodes:
//   if RAX == dst: dst = src else RAX = dst
//   ZF = (RAX == dst)
std::variant<Decoded, DecodeError> decode_cmpxchg_r64_rm64(
    std::span<const Byte> bytes,
    std::size_t& cursor,
    ir::Ref& next_ref) {
    auto modrm = parse_modrm(bytes, cursor);
    if (std::holds_alternative<DecodeError>(modrm)) {
        return std::get<DecodeError>(modrm);
    }
    const auto& m = std::get<ModRmOperand>(modrm);

    Decoded d;
    const ir::Ref ref_rax = next_ref++;
    d.stmts.push_back({ref_rax, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});

    const ir::Ref ref_src = next_ref++;
    d.stmts.push_back({ref_src, ir::LoadReg{gpr_from_index(m.reg), ir::OpSize::I64}});

    ir::Ref ref_addr = ir::kInvalidRef;
    ir::Ref ref_dst;
    if (m.mod == 0b11u) {
        ref_dst = next_ref++;
        d.stmts.push_back({ref_dst, ir::LoadReg{m.base, ir::OpSize::I64}});
    } else {
        ref_addr = emit_address(d.stmts, m, next_ref);
        ref_dst = next_ref++;
        d.stmts.push_back({ref_dst, ir::LoadMemTSO{ref_addr, ir::OpSize::I64}});
    }

    d.stmts.push_back({std::nullopt, ir::CmpFlags{ref_rax, ref_dst, ir::OpSize::I64}});
    const ir::Ref ref_new_dst = next_ref++;
    const ir::Ref ref_new_rax = next_ref++;
    d.stmts.push_back({ref_new_dst, ir::Select{ir::CondCode::Eq, ref_src, ref_dst, ir::OpSize::I64}});
    d.stmts.push_back({ref_new_rax, ir::Select{ir::CondCode::Eq, ref_rax, ref_dst, ir::OpSize::I64}});

    if (m.mod == 0b11u) {
        d.stmts.push_back(
            {std::nullopt, ir::StoreReg{m.base, ref_new_dst, ir::OpSize::I64}});
    } else {
        d.stmts.push_back({std::nullopt, ir::StoreMemTSO{ref_addr, ref_new_dst, ir::OpSize::I64}});
    }
    d.stmts.push_back({std::nullopt, ir::StoreReg{ir::Gpr::Rax, ref_new_rax, ir::OpSize::I64}});
    d.bytes_consumed = cursor;
    return d;
}

// CMPXCHG16B m128 (48 0F C7 /1), MVP placeholder.
//
// Encodes:
//   if RDX:RAX == [mem:128]:
//     [mem:128] = RCX:RBX
//     ZF = 1
//   else:
//     RAX:RDX = [mem:128]
//     ZF = 0
// Register direct forms are intentionally unsupported in this stage.
std::variant<Decoded, DecodeError> decode_cmpxchg16b_m128(
    std::span<const Byte> bytes,
    std::size_t& cursor,
    ir::Ref& next_ref) {
    auto modrm = parse_modrm(bytes, cursor);
    if (std::holds_alternative<DecodeError>(modrm)) {
        return std::get<DecodeError>(modrm);
    }
    const auto& m = std::get<ModRmOperand>(modrm);
    if (m.reg != 1u) return DecodeError::UnsupportedEncoding;
    if (m.mod == 0b11u) return DecodeError::UnsupportedEncoding;

    Decoded d;
    const ir::Ref ref_mem_addr = emit_address(d.stmts, m, next_ref);
    const ir::Ref ref_zero = next_ref++;
    d.stmts.push_back({ref_zero, ir::Constant{0u, ir::OpSize::I64}});
    const ir::Ref ref_rax = next_ref++;
    d.stmts.push_back({ref_rax, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    const ir::Ref ref_rdx = next_ref++;
    d.stmts.push_back({ref_rdx, ir::LoadReg{ir::Gpr::Rdx, ir::OpSize::I64}});
    const ir::Ref ref_rbx = next_ref++;
    d.stmts.push_back({ref_rbx, ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    const ir::Ref ref_rcx = next_ref++;
    d.stmts.push_back({ref_rcx, ir::LoadReg{ir::Gpr::Rcx, ir::OpSize::I64}});

    const ir::Ref ref_low_before = next_ref++;
    d.stmts.push_back({ref_low_before, ir::LoadMemTSO{ref_mem_addr, ir::OpSize::I64}});

    const ir::Ref ref_offset = next_ref++;
    d.stmts.push_back({ref_offset, ir::Constant{8u, ir::OpSize::I64}});
    const ir::Ref ref_high_addr = next_ref++;
    d.stmts.push_back(
        {ref_high_addr, ir::BinOp{ir::BinOpKind::Add, ref_mem_addr, ref_offset, ir::OpSize::I64}});
    const ir::Ref ref_high_before = next_ref++;
    d.stmts.push_back({ref_high_before, ir::LoadMemTSO{ref_high_addr, ir::OpSize::I64}});

    const ir::Ref ref_eq_low = next_ref++;
    d.stmts.push_back({ref_eq_low, ir::Compare{ir::CondCode::Eq, ref_low_before, ref_rax, ir::OpSize::I64}});
    const ir::Ref ref_eq_high = next_ref++;
    d.stmts.push_back({ref_eq_high, ir::Compare{ir::CondCode::Eq, ref_high_before, ref_rdx, ir::OpSize::I64}});
    const ir::Ref ref_eq_pair = next_ref++;
    d.stmts.push_back({ref_eq_pair, ir::BinOp{ir::BinOpKind::And, ref_eq_low, ref_eq_high, ir::OpSize::I64}});
    d.stmts.push_back({std::nullopt, ir::CmpFlags{ref_eq_pair, ref_zero, ir::OpSize::I64}});

    const ir::Ref ref_new_low = next_ref++;
    d.stmts.push_back(
        {ref_new_low, ir::Select{ir::CondCode::Ne, ref_rbx, ref_low_before, ir::OpSize::I64}});
    const ir::Ref ref_new_high = next_ref++;
    d.stmts.push_back(
        {ref_new_high, ir::Select{ir::CondCode::Ne, ref_rcx, ref_high_before, ir::OpSize::I64}});
    d.stmts.push_back({std::nullopt, ir::StoreMemTSO{ref_mem_addr, ref_new_low, ir::OpSize::I64}});
    d.stmts.push_back({std::nullopt, ir::StoreMemTSO{ref_high_addr, ref_new_high, ir::OpSize::I64}});

    const ir::Ref ref_new_rax = next_ref++;
    d.stmts.push_back(
        {ref_new_rax, ir::Select{ir::CondCode::Ne, ref_rax, ref_low_before, ir::OpSize::I64}});
    const ir::Ref ref_new_rdx = next_ref++;
    d.stmts.push_back(
        {ref_new_rdx, ir::Select{ir::CondCode::Ne, ref_rdx, ref_high_before, ir::OpSize::I64}});
    d.stmts.push_back({std::nullopt, ir::StoreReg{ir::Gpr::Rax, ref_new_rax, ir::OpSize::I64}});
    d.stmts.push_back({std::nullopt, ir::StoreReg{ir::Gpr::Rdx, ref_new_rdx, ir::OpSize::I64}});

    d.bytes_consumed = cursor;
    return d;
}

// XADD r/m64, r64 (0F C1 /r).
// Encodes:
//   tmp = r/m64
//   r/m64 = tmp + reg
//   reg = tmp
std::variant<Decoded, DecodeError> decode_xadd_r64_rm64(
    std::span<const Byte> bytes,
    std::size_t& cursor,
    ir::Ref& next_ref) {
    auto modrm = parse_modrm(bytes, cursor);
    if (std::holds_alternative<DecodeError>(modrm)) {
        return std::get<DecodeError>(modrm);
    }
    const auto& m = std::get<ModRmOperand>(modrm);

    Decoded d;
    const ir::Ref ref_src = next_ref++;
    d.stmts.push_back({ref_src, ir::LoadReg{gpr_from_index(m.reg), ir::OpSize::I64}});

    const ir::Ref ref_dst = next_ref++;
    if (m.mod == 0b11u) {
        d.stmts.push_back({ref_dst, ir::LoadReg{m.base, ir::OpSize::I64}});
        const ir::Ref ref_sum = next_ref++;
        d.stmts.push_back({ref_sum, ir::BinOp{ir::BinOpKind::Add, ref_dst, ref_src, ir::OpSize::I64}});
        d.stmts.push_back({std::nullopt, ir::StoreReg{m.base, ref_sum, ir::OpSize::I64}});
        d.stmts.push_back({std::nullopt, ir::StoreReg{gpr_from_index(m.reg), ref_dst, ir::OpSize::I64}});
    } else {
        const ir::Ref ref_addr = emit_address(d.stmts, m, next_ref);
        d.stmts.push_back({ref_dst, ir::LoadMemTSO{ref_addr, ir::OpSize::I64}});
        const ir::Ref ref_sum = next_ref++;
        d.stmts.push_back({ref_sum, ir::BinOp{ir::BinOpKind::Add, ref_dst, ref_src, ir::OpSize::I64}});
        d.stmts.push_back({std::nullopt, ir::StoreMemTSO{ref_addr, ref_sum, ir::OpSize::I64}});
        d.stmts.push_back(
            {std::nullopt, ir::StoreReg{gpr_from_index(m.reg), ref_dst, ir::OpSize::I64}});
    }
    d.bytes_consumed = cursor;
    return d;
}

// SETcc r/m8 (0F 90–9F /r).
// Encodes: dstbyte = cond ? 1 : 0.
std::variant<Decoded, DecodeError> decode_setcc_r8_rm8(
    ir::CondCode cc,
    std::span<const Byte> bytes,
    std::size_t& cursor,
    ir::Ref& next_ref) {
    auto modrm = parse_modrm(bytes, cursor);
    if (std::holds_alternative<DecodeError>(modrm)) {
        return std::get<DecodeError>(modrm);
    }
    const auto& m = std::get<ModRmOperand>(modrm);

    Decoded d;
    const ir::Ref ref_true = next_ref++;
    const ir::Ref ref_false = next_ref++;
    d.stmts.push_back({ref_true, ir::Constant{1u, ir::OpSize::I8}});
    d.stmts.push_back({ref_false, ir::Constant{0u, ir::OpSize::I8}});

    const ir::Ref ref_selected = next_ref++;
    d.stmts.push_back({ref_selected, ir::Select{cc, ref_true, ref_false, ir::OpSize::I8}});

    if (m.mod == 0b11u) {
        d.stmts.push_back(
            {std::nullopt, ir::StoreReg{m.base, ref_selected, ir::OpSize::I8}});
    } else {
        const ir::Ref ref_addr = emit_address(d.stmts, m, next_ref);
        d.stmts.push_back(
            {std::nullopt, ir::StoreMemTSO{ref_addr, ref_selected, ir::OpSize::I8}});
    }

    d.bytes_consumed = cursor;
    return d;
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
// Memory destination forms of ADD/OR/AND/SUB/XOR/ADC/SBB (mod != 11) are future
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

// TEST r/m64, r64 (48 85 /r) — decode and model as:
//   %lhs = loadreg(base)
//   %rhs = loadreg(reg)
//   %tmp = and(lhs, rhs)
//   %zero = const 0
//   CmpFlags(tmp, zero)
//
// This keeps the current decoder/lowering pipeline flowing while exact
// Carry/Overflow behavior for TEST is handled later.
std::variant<Decoded, DecodeError> decode_test_rm_r(
    std::span<const Byte> bytes,
    std::size_t& cursor_in_out,
    ir::Ref& next_ref) {
    auto modrm = parse_modrm(bytes, cursor_in_out);
    if (std::holds_alternative<DecodeError>(modrm)) {
        return std::get<DecodeError>(modrm);
    }
    const auto& m = std::get<ModRmOperand>(modrm);
    if (m.mod != 0b11u) return DecodeError::UnsupportedEncoding;

    Decoded d;
    const ir::Ref ref_lhs = next_ref++;
    const ir::Ref ref_rhs = next_ref++;
    const ir::Ref ref_tmp = next_ref++;
    const ir::Ref ref_zero = next_ref++;
    d.stmts.push_back({ref_lhs, ir::LoadReg{m.base, ir::OpSize::I64}});
    d.stmts.push_back({ref_rhs, ir::LoadReg{gpr_from_index(m.reg), ir::OpSize::I64}});
    d.stmts.push_back({ref_tmp,
                       ir::BinOp{ir::BinOpKind::And, ref_lhs, ref_rhs, ir::OpSize::I64}});
    d.stmts.push_back({ref_zero, ir::Constant{0u, ir::OpSize::I64}});
    d.stmts.push_back({std::nullopt, ir::CmpFlags{ref_tmp, ref_zero, ir::OpSize::I64}});
    d.bytes_consumed = cursor_in_out;
    return d;
}

std::variant<Decoded, DecodeError> decode_incdec_from_rm(
    const ModRmOperand& m,
    std::size_t bytes_consumed,
    ir::Ref& next_ref,
    ir::BinOpKind op) {
    if (m.mod != 0b11u) return DecodeError::UnsupportedEncoding;

    Decoded d;
    const ir::Ref ref_src = next_ref++;
    const ir::Ref ref_delta = next_ref++;
    const ir::Ref ref_dst = next_ref++;
    d.stmts.push_back({ref_src, ir::LoadReg{m.base, ir::OpSize::I64}});
    d.stmts.push_back({ref_delta, ir::Constant{1u, ir::OpSize::I64}});
    d.stmts.push_back({ref_dst, ir::BinOp{op, ref_src, ref_delta, ir::OpSize::I64}});
    d.stmts.push_back({std::nullopt, ir::StoreReg{m.base, ref_dst, ir::OpSize::I64}});
    d.bytes_consumed = bytes_consumed;
    return d;
}

// Group 3 operations for r/m64 (48 F7):
//   /2: NOT (bitwise NOT, x ^ -1)
//   /3: NEG (two's complement negation, 0 - x)
//   /4: MUL (placeholder) 64x64 -> low64 in RAX, clear RDX
//   /5: IMUL (placeholder) signed multiply, low64 in RAX, clear RDX
//   /6: DIV (placeholder) 128/64 -> quotient in RAX, remainder in RDX
// This keeps the decoder minimal while preserving existing lowering contracts.
std::variant<Decoded, DecodeError> decode_neg_not_from_rm(
    const ModRmOperand& m,
    std::size_t bytes_consumed,
    ir::Ref& next_ref,
    ir::BinOpKind op) {
    if (m.mod != 0b11u) return DecodeError::UnsupportedEncoding;

    Decoded d;
    const ir::Ref ref_src = next_ref++;
    const ir::Ref ref_rhs = next_ref++;
    const ir::Ref ref_dst = next_ref++;

    if (op == ir::BinOpKind::Xor) {
        d.stmts.push_back({ref_src, ir::LoadReg{m.base, ir::OpSize::I64}});
        d.stmts.push_back({ref_rhs, ir::Constant{~0ULL, ir::OpSize::I64}});
        d.stmts.push_back({ref_dst, ir::BinOp{op, ref_src, ref_rhs, ir::OpSize::I64}});
    } else {
        d.stmts.push_back({ref_src, ir::Constant{0u, ir::OpSize::I64}});
        d.stmts.push_back({ref_rhs, ir::LoadReg{m.base, ir::OpSize::I64}});
        d.stmts.push_back({ref_dst, ir::BinOp{op, ref_src, ref_rhs, ir::OpSize::I64}});
    }

    d.stmts.push_back({std::nullopt, ir::StoreReg{m.base, ref_dst, ir::OpSize::I64}});
    d.bytes_consumed = bytes_consumed;
    return d;
}

// DIV/IDIV r/m64 family (48 F7 /6 and /7) — MVP placeholder.
// Encodes:
//   placeholder quotient = 0 in RAX, placeholder remainder = 0 in RDX.
// Real signed/unsigned divide lowering (including divide-by-zero behavior and
// full RDX:RAX dividend handling) is deferred to lowering work (F1-BK-011).
std::variant<Decoded, DecodeError> decode_div_from_rm(
    const ModRmOperand& m,
    std::size_t bytes_consumed,
    ir::Ref& next_ref) {
    if (m.mod != 0b11u) return DecodeError::UnsupportedEncoding;

    Decoded d;
    const ir::Ref ref_dividend = next_ref++;
    const ir::Ref ref_divisor = next_ref++;
    const ir::Ref ref_zero = next_ref++;

    d.stmts.push_back({ref_dividend, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    d.stmts.push_back({ref_divisor, ir::LoadReg{m.base, ir::OpSize::I64}});
    d.stmts.push_back({ref_zero, ir::Constant{0u, ir::OpSize::I64}});
    d.stmts.push_back({std::nullopt,
                       ir::StoreReg{ir::Gpr::Rax, ref_zero, ir::OpSize::I64}});
    d.stmts.push_back({std::nullopt,
                       ir::StoreReg{ir::Gpr::Rdx, ref_zero, ir::OpSize::I64}});

    d.bytes_consumed = bytes_consumed;
    return d;
}

std::variant<Decoded, DecodeError> decode_mul_imul_from_rm(
    const ModRmOperand& m,
    std::size_t bytes_consumed,
    ir::Ref& next_ref,
    ir::BinOpKind op) {
    if (m.mod != 0b11u) return DecodeError::UnsupportedEncoding;

    Decoded d;
    const ir::Ref ref_rax = next_ref++;
    const ir::Ref ref_rhs = next_ref++;
    const ir::Ref ref_product = next_ref++;
    const ir::Ref ref_zero = next_ref++;

    d.stmts.push_back({ref_rax, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    d.stmts.push_back({ref_rhs, ir::LoadReg{m.base, ir::OpSize::I64}});
    d.stmts.push_back({ref_product,
                       ir::BinOp{op, ref_rax, ref_rhs, ir::OpSize::I64}});
    d.stmts.push_back({std::nullopt,
                       ir::StoreReg{ir::Gpr::Rax, ref_product, ir::OpSize::I64}});
    d.stmts.push_back({ref_zero, ir::Constant{0u, ir::OpSize::I64}});
    d.stmts.push_back({std::nullopt,
                       ir::StoreReg{ir::Gpr::Rdx, ref_zero, ir::OpSize::I64}});

    d.bytes_consumed = bytes_consumed;
    return d;
}

// IMUL r64, r/m64 (0F AF /r) — MVP register-direct only.
// Encodes:
//   dest = reg field, rhs = rm field, operation: dest = dest * rhs (signedness
//   deferred, lower as plain integer multiply for now).
std::variant<Decoded, DecodeError> decode_imul_r64_r_rm(
    std::span<const Byte> bytes,
    std::size_t& cursor,
    ir::Ref& next_ref) {
    auto modrm = parse_modrm(bytes, cursor);
    if (std::holds_alternative<DecodeError>(modrm)) {
        return std::get<DecodeError>(modrm);
    }
    const auto& m = std::get<ModRmOperand>(modrm);
    if (m.mod != 0b11u) return DecodeError::UnsupportedEncoding;

    Decoded d;
    const ir::Ref ref_dst_src = next_ref++;
    const ir::Ref ref_rhs = next_ref++;
    const ir::Ref ref_product = next_ref++;
    d.stmts.push_back({ref_dst_src, ir::LoadReg{gpr_from_index(m.reg), ir::OpSize::I64}});
    d.stmts.push_back({ref_rhs, ir::LoadReg{m.base, ir::OpSize::I64}});
    d.stmts.push_back(
        {ref_product, ir::BinOp{ir::BinOpKind::Mul, ref_dst_src, ref_rhs, ir::OpSize::I64}});
    d.stmts.push_back(
        {std::nullopt, ir::StoreReg{gpr_from_index(m.reg), ref_product, ir::OpSize::I64}});
    d.bytes_consumed = cursor;
    return d;
}

// IMUL r64, r/m64, imm32 (69 /r) — MVP register-direct only.
// Encodes:
//   dest = reg field, operation: dest = signext(imm32) * r/m64 (implemented as mul).
// The immediate is sign-extended to 64-bit and then multiplied as a placeholder.
std::variant<Decoded, DecodeError> decode_imul_r64_rm_imm32(
    std::span<const Byte> bytes,
    std::size_t& cursor,
    ir::Ref& next_ref) {
    auto modrm = parse_modrm(bytes, cursor);
    if (std::holds_alternative<DecodeError>(modrm)) {
        return std::get<DecodeError>(modrm);
    }
    const auto& m = std::get<ModRmOperand>(modrm);
    if (m.mod != 0b11u) return DecodeError::UnsupportedEncoding;

    auto imm = read_le<4>(bytes, cursor);
    if (!imm) return DecodeError::TruncatedInput;
    cursor += 4;
    const std::int32_t imm_i32 = sign_extend_i32<4>(*imm);
    const std::uint64_t imm_u64 = static_cast<std::uint64_t>(static_cast<std::int64_t>(imm_i32));

    Decoded d;
    const ir::Ref ref_rhs = next_ref++;
    const ir::Ref ref_imm = next_ref++;
    const ir::Ref ref_product = next_ref++;
    d.stmts.push_back({ref_rhs, ir::LoadReg{m.base, ir::OpSize::I64}});
    d.stmts.push_back({ref_imm, ir::Constant{imm_u64, ir::OpSize::I64}});
    d.stmts.push_back(
        {ref_product, ir::BinOp{ir::BinOpKind::Mul, ref_rhs, ref_imm, ir::OpSize::I64}});
    d.stmts.push_back(
        {std::nullopt, ir::StoreReg{gpr_from_index(m.reg), ref_product, ir::OpSize::I64}});
    d.bytes_consumed = cursor;
    return d;
}

// IMUL r64, r/m64, imm8 (6B /r) — MVP register-direct only.
// Encodes:
//   dest = reg field, operation: dest = signext(imm8) * r/m64 (implemented as mul).
// The immediate is sign-extended to 64-bit and then multiplied as a placeholder.
std::variant<Decoded, DecodeError> decode_imul_r64_rm_imm8(
    std::span<const Byte> bytes,
    std::size_t& cursor,
    ir::Ref& next_ref) {
    auto modrm = parse_modrm(bytes, cursor);
    if (std::holds_alternative<DecodeError>(modrm)) {
        return std::get<DecodeError>(modrm);
    }
    const auto& m = std::get<ModRmOperand>(modrm);
    if (m.mod != 0b11u) return DecodeError::UnsupportedEncoding;

    auto imm = read_le<1>(bytes, cursor);
    if (!imm) return DecodeError::TruncatedInput;
    cursor += 1;
    const std::int32_t imm_i8 = sign_extend_i32<1>(*imm);
    const std::uint64_t imm_u64 = static_cast<std::uint64_t>(static_cast<std::int64_t>(imm_i8));

    Decoded d;
    const ir::Ref ref_rhs = next_ref++;
    const ir::Ref ref_imm = next_ref++;
    const ir::Ref ref_product = next_ref++;
    d.stmts.push_back({ref_rhs, ir::LoadReg{m.base, ir::OpSize::I64}});
    d.stmts.push_back({ref_imm, ir::Constant{imm_u64, ir::OpSize::I64}});
    d.stmts.push_back(
        {ref_product, ir::BinOp{ir::BinOpKind::Mul, ref_rhs, ref_imm, ir::OpSize::I64}});
    d.stmts.push_back(
        {std::nullopt, ir::StoreReg{gpr_from_index(m.reg), ref_product, ir::OpSize::I64}});
    d.bytes_consumed = cursor;
    return d;
}

enum class BtSubOpcode : std::uint8_t {
    Bt  = 0u,
    Bts = 1u,
    Btr = 2u,
    Btc = 3u,
};

// BT/BTS/BTR/BTC r/m64, imm8 (0F BA /4,/5,/6,/7) — MVP register-direct only.
//
// These are decoded as:
//   BT:  CF = (src & (1 << imm)) ? 1 : 0
//   BTS: src = src | (1 << imm), CF = old bit
//   BTR: src = src & ~(1 << imm), CF = old bit
//   BTC: src = src ^ (1 << imm), CF = old bit
// Carry/flag materialization is represented via CmpFlags(old_bit, 0), where
// `old_bit` is either 0 or a single-bit mask.
std::variant<Decoded, DecodeError> decode_bt_r64_rm_imm8_from_rm(
    std::span<const Byte> bytes,
    const ModRmOperand& m,
    std::size_t& cursor,
    ir::Ref& next_ref,
    BtSubOpcode op) {
    if (m.mod != 0b11u) return DecodeError::UnsupportedEncoding;

    auto imm = read_le<1>(bytes, cursor);
    if (!imm) return DecodeError::TruncatedInput;
    cursor += 1;
    const std::uint64_t imm_u64 = *imm;

    Decoded d;
    const ir::Ref ref_src = next_ref++;
    const ir::Ref ref_shift = next_ref++;
    const ir::Ref ref_mask = next_ref++;
    const ir::Ref ref_one = next_ref++;
    const ir::Ref ref_oldbit = next_ref++;
    const ir::Ref ref_zero = next_ref++;

    d.stmts.push_back({ref_src, ir::LoadReg{m.base, ir::OpSize::I64}});
    d.stmts.push_back({ref_shift, ir::Constant{imm_u64, ir::OpSize::I64}});
    d.stmts.push_back({ref_one, ir::Constant{1u, ir::OpSize::I64}});
    d.stmts.push_back({ref_mask, ir::BinOp{ir::BinOpKind::Shl, ref_one, ref_shift, ir::OpSize::I64}});
    d.stmts.push_back({ref_oldbit, ir::BinOp{ir::BinOpKind::And, ref_src, ref_mask, ir::OpSize::I64}});

    if (op != BtSubOpcode::Bt) {
        ir::Ref ref_newval;
        switch (op) {
            case BtSubOpcode::Bts: {
                ref_newval = next_ref++;
                d.stmts.push_back({ref_newval,
                                   ir::BinOp{ir::BinOpKind::Or, ref_src, ref_mask, ir::OpSize::I64}});
                break;
            }
            case BtSubOpcode::Btr: {
                const ir::Ref ref_inv_mask = next_ref++;
                d.stmts.push_back(
                    {ref_inv_mask, ir::Constant{~0ULL, ir::OpSize::I64}});
                d.stmts.push_back({ref_inv_mask,
                                   ir::BinOp{ir::BinOpKind::Xor, ref_mask, ref_inv_mask, ir::OpSize::I64}});
                ref_newval = next_ref++;
                d.stmts.push_back({ref_newval,
                                   ir::BinOp{ir::BinOpKind::And, ref_src, ref_inv_mask, ir::OpSize::I64}});
                break;
            }
            case BtSubOpcode::Btc: {
                ref_newval = next_ref++;
                d.stmts.push_back({ref_newval,
                                   ir::BinOp{ir::BinOpKind::Xor, ref_src, ref_mask, ir::OpSize::I64}});
                break;
            }
            default:
                return {DecodeError::UnsupportedEncoding};
        }
        d.stmts.push_back({std::nullopt, ir::StoreReg{m.base, ref_newval, ir::OpSize::I64}});
    }

    d.stmts.push_back({ref_zero, ir::Constant{0u, ir::OpSize::I64}});
    d.stmts.push_back({std::nullopt, ir::CmpFlags{ref_oldbit, ref_zero, ir::OpSize::I64}});
    d.bytes_consumed = cursor;
    return d;
}

// BSF / BSR r64, r/m64 (0F BC /r, 0F BD /r) — MVP register-direct only.
//
// This is a placeholder decode for now:
//   * result register receives 0
//   * flags are computed as (src == 0) to preserve a flag-like effect
// A future lowering pass will replace this with a real bit-scan lowering.
std::variant<Decoded, DecodeError> decode_bsf_bsr_r64_r_rm(
    const ModRmOperand& m,
    std::size_t bytes_consumed,
    ir::Ref& next_ref) {
    if (m.mod != 0b11u) return DecodeError::UnsupportedEncoding;

    Decoded d;
    const ir::Ref ref_src = next_ref++;
    const ir::Ref ref_zero = next_ref++;
    d.stmts.push_back({ref_src, ir::LoadReg{m.base, ir::OpSize::I64}});
    d.stmts.push_back({ref_zero, ir::Constant{0u, ir::OpSize::I64}});
    d.stmts.push_back({std::nullopt, ir::StoreReg{gpr_from_index(m.reg), ref_zero, ir::OpSize::I64}});
    d.stmts.push_back({std::nullopt, ir::CmpFlags{ref_src, ref_zero, ir::OpSize::I64}});
    d.bytes_consumed = bytes_consumed;
    return d;
}

// LZCNT r64, r/m64 (F3 48 0F BD /r) — MVP register-direct only.
//
// This is a placeholder decode for now:
//   * result register receives 0
//   * flags are computed as (src == 0) to preserve a flag-like effect
//   * source and destination are both 64-bit GPRs
// A future lowering pass will replace this with a real leading-zero count lowering.
std::variant<Decoded, DecodeError> decode_lzcnt_r64_r_rm(
    const ModRmOperand& m,
    std::size_t bytes_consumed,
    ir::Ref& next_ref) {
    if (m.mod != 0b11u) return DecodeError::UnsupportedEncoding;

    Decoded d;
    const ir::Ref ref_src = next_ref++;
    const ir::Ref ref_zero = next_ref++;
    d.stmts.push_back({ref_src, ir::LoadReg{m.base, ir::OpSize::I64}});
    d.stmts.push_back({ref_zero, ir::Constant{0u, ir::OpSize::I64}});
    d.stmts.push_back({std::nullopt, ir::StoreReg{gpr_from_index(m.reg), ref_zero, ir::OpSize::I64}});
    d.stmts.push_back({std::nullopt, ir::CmpFlags{ref_src, ref_zero, ir::OpSize::I64}});
    d.bytes_consumed = bytes_consumed;
    return d;
}

// TZCNT r64, r/m64 (F3 48 0F BC /r) — MVP register-direct only.
//
// This is a placeholder decode for now:
//   * result register receives 0
//   * flags are computed as (src == 0) to preserve a flag-like effect
//   * source and destination are both 64-bit GPRs
// A future lowering pass will replace this with a real trailing-zero count lowering.
std::variant<Decoded, DecodeError> decode_tzcnt_r64_r_rm(
    const ModRmOperand& m,
    std::size_t bytes_consumed,
    ir::Ref& next_ref) {
    if (m.mod != 0b11u) return DecodeError::UnsupportedEncoding;

    Decoded d;
    const ir::Ref ref_src = next_ref++;
    const ir::Ref ref_zero = next_ref++;
    d.stmts.push_back({ref_src, ir::LoadReg{m.base, ir::OpSize::I64}});
    d.stmts.push_back({ref_zero, ir::Constant{0u, ir::OpSize::I64}});
    d.stmts.push_back({std::nullopt, ir::StoreReg{gpr_from_index(m.reg), ref_zero, ir::OpSize::I64}});
    d.stmts.push_back({std::nullopt, ir::CmpFlags{ref_src, ref_zero, ir::OpSize::I64}});
    d.bytes_consumed = bytes_consumed;
    return d;
}

// POPCNT r64, r/m64 (F3 48 0F B8 /r) — MVP register-direct only.
//
// This is a placeholder decode for now:
//   * result register receives 0
//   * flags are computed as (src == 0) to preserve a flag-like effect
//   * source and destination are both 64-bit GPRs
std::variant<Decoded, DecodeError> decode_popcnt_r64_r_rm(
    const ModRmOperand& m,
    std::size_t bytes_consumed,
    ir::Ref& next_ref) {
    if (m.mod != 0b11u) return DecodeError::UnsupportedEncoding;

    Decoded d;
    const ir::Ref ref_src = next_ref++;
    const ir::Ref ref_zero = next_ref++;
    d.stmts.push_back({ref_src, ir::LoadReg{m.base, ir::OpSize::I64}});
    d.stmts.push_back({ref_zero, ir::Constant{0u, ir::OpSize::I64}});
    d.stmts.push_back({std::nullopt, ir::StoreReg{gpr_from_index(m.reg), ref_zero, ir::OpSize::I64}});
    d.stmts.push_back({std::nullopt, ir::CmpFlags{ref_src, ref_zero, ir::OpSize::I64}});
    d.bytes_consumed = bytes_consumed;
    return d;
}

// PUSH r64 placeholder: `r` in 50+rd (register direct).
//
// Modelled as:
//   new_rsp = rsp - 8
//   store [new_rsp] = src
//   rsp = new_rsp
//
// This keeps stack-pointer mutation in the same instruction without using dedicated
// stack-specific IR operations.
std::variant<Decoded, DecodeError> decode_push_r64(
    ir::Gpr src,
    std::size_t bytes_consumed,
    ir::Ref& next_ref) {
    Decoded d;
    const ir::Ref ref_rsp = next_ref++;
    const ir::Ref ref_eight = next_ref++;
    const ir::Ref ref_new_rsp = next_ref++;
    const ir::Ref ref_src = next_ref++;
    d.stmts.push_back({ref_rsp, ir::LoadReg{ir::Gpr::Rsp, ir::OpSize::I64}});
    d.stmts.push_back({ref_eight, ir::Constant{8u, ir::OpSize::I64}});
    d.stmts.push_back(
        {ref_new_rsp, ir::BinOp{ir::BinOpKind::Sub, ref_rsp, ref_eight, ir::OpSize::I64}});
    d.stmts.push_back({ref_src, ir::LoadReg{src, ir::OpSize::I64}});
    d.stmts.push_back(
        {std::nullopt, ir::StoreMemTSO{ref_new_rsp, ref_src, ir::OpSize::I64}});
    d.stmts.push_back(
        {std::nullopt, ir::StoreReg{ir::Gpr::Rsp, ref_new_rsp, ir::OpSize::I64}});
    d.bytes_consumed = bytes_consumed;
    return d;
}

// POP r64 placeholder: `rd` in 58+rd (register direct).
//
// Modelled as:
//   dst = [rsp]
//   rsp = rsp + 8
std::variant<Decoded, DecodeError> decode_pop_r64(
    ir::Gpr dst,
    std::size_t bytes_consumed,
    ir::Ref& next_ref) {
    Decoded d;
    const ir::Ref ref_rsp = next_ref++;
    const ir::Ref ref_value = next_ref++;
    const ir::Ref ref_eight = next_ref++;
    const ir::Ref ref_new_rsp = next_ref++;
    d.stmts.push_back({ref_rsp, ir::LoadReg{ir::Gpr::Rsp, ir::OpSize::I64}});
    d.stmts.push_back({ref_value, ir::LoadMemTSO{ref_rsp, ir::OpSize::I64}});
    d.stmts.push_back({ref_eight, ir::Constant{8u, ir::OpSize::I64}});
    d.stmts.push_back(
        {ref_new_rsp, ir::BinOp{ir::BinOpKind::Add, ref_rsp, ref_eight, ir::OpSize::I64}});
    d.stmts.push_back(
        {std::nullopt, ir::StoreReg{dst, ref_value, ir::OpSize::I64}});
    d.stmts.push_back(
        {std::nullopt, ir::StoreReg{ir::Gpr::Rsp, ref_new_rsp, ir::OpSize::I64}});
    d.bytes_consumed = bytes_consumed;
    return d;
}

std::variant<Decoded, DecodeError> decode_push_imm8_r64(
    std::span<const Byte> bytes,
    std::size_t& cursor,
    ir::Ref& next_ref) {
    auto imm = read_le<1>(bytes, cursor);
    if (!imm) return DecodeError::TruncatedInput;
    cursor += 1;
    const std::int32_t imm_i8 = sign_extend_i32<1>(*imm);
    const std::uint64_t imm_u64 = static_cast<std::uint64_t>(static_cast<std::int64_t>(imm_i8));

    Decoded d;
    const ir::Ref ref_rsp = next_ref++;
    const ir::Ref ref_eight = next_ref++;
    const ir::Ref ref_new_rsp = next_ref++;
    const ir::Ref ref_imm = next_ref++;
    d.stmts.push_back({ref_rsp, ir::LoadReg{ir::Gpr::Rsp, ir::OpSize::I64}});
    d.stmts.push_back({ref_eight, ir::Constant{8u, ir::OpSize::I64}});
    d.stmts.push_back(
        {ref_new_rsp, ir::BinOp{ir::BinOpKind::Sub, ref_rsp, ref_eight, ir::OpSize::I64}});
    d.stmts.push_back({ref_imm, ir::Constant{imm_u64, ir::OpSize::I64}});
    d.stmts.push_back(
        {std::nullopt, ir::StoreMemTSO{ref_new_rsp, ref_imm, ir::OpSize::I64}});
    d.stmts.push_back(
        {std::nullopt, ir::StoreReg{ir::Gpr::Rsp, ref_new_rsp, ir::OpSize::I64}});
    d.bytes_consumed = cursor;
    return d;
}

std::variant<Decoded, DecodeError> decode_push_imm32_r64(
    std::span<const Byte> bytes,
    std::size_t& cursor,
    ir::Ref& next_ref) {
    auto imm = read_le<4>(bytes, cursor);
    if (!imm) return DecodeError::TruncatedInput;
    cursor += 4;
    const std::int32_t imm_i32 = sign_extend_i32<4>(*imm);
    const std::uint64_t imm_u64 = static_cast<std::uint64_t>(static_cast<std::int64_t>(imm_i32));

    Decoded d;
    const ir::Ref ref_rsp = next_ref++;
    const ir::Ref ref_eight = next_ref++;
    const ir::Ref ref_new_rsp = next_ref++;
    const ir::Ref ref_imm = next_ref++;
    d.stmts.push_back({ref_rsp, ir::LoadReg{ir::Gpr::Rsp, ir::OpSize::I64}});
    d.stmts.push_back({ref_eight, ir::Constant{8u, ir::OpSize::I64}});
    d.stmts.push_back(
        {ref_new_rsp, ir::BinOp{ir::BinOpKind::Sub, ref_rsp, ref_eight, ir::OpSize::I64}});
    d.stmts.push_back({ref_imm, ir::Constant{imm_u64, ir::OpSize::I64}});
    d.stmts.push_back(
        {std::nullopt, ir::StoreMemTSO{ref_new_rsp, ref_imm, ir::OpSize::I64}});
    d.stmts.push_back(
        {std::nullopt, ir::StoreReg{ir::Gpr::Rsp, ref_new_rsp, ir::OpSize::I64}});
    d.bytes_consumed = cursor;
    return d;
}

// PUSHFQ placeholder: opcode 9C.
//
// There is no explicit flags register in the current IR yet, so this is
// modelled as pushing a constant 0 to the current stack pointer.
std::variant<Decoded, DecodeError> decode_pushfq(
    std::size_t bytes_consumed,
    ir::Ref& next_ref) {
    Decoded d;
    const ir::Ref ref_rsp = next_ref++;
    const ir::Ref ref_eight = next_ref++;
    const ir::Ref ref_new_rsp = next_ref++;
    const ir::Ref ref_flags = next_ref++;
    d.stmts.push_back({ref_rsp, ir::LoadReg{ir::Gpr::Rsp, ir::OpSize::I64}});
    d.stmts.push_back({ref_eight, ir::Constant{8u, ir::OpSize::I64}});
    d.stmts.push_back(
        {ref_new_rsp, ir::BinOp{ir::BinOpKind::Sub, ref_rsp, ref_eight, ir::OpSize::I64}});
    d.stmts.push_back({ref_flags, ir::Constant{0u, ir::OpSize::I64}});
    d.stmts.push_back(
        {std::nullopt, ir::StoreMemTSO{ref_new_rsp, ref_flags, ir::OpSize::I64}});
    d.stmts.push_back(
        {std::nullopt, ir::StoreReg{ir::Gpr::Rsp, ref_new_rsp, ir::OpSize::I64}});
    d.bytes_consumed = bytes_consumed;
    return d;
}

// POPFQ placeholder: opcode 9D.
//
// There is no explicit flags bank in the current IR yet, so this is modelled
// as a stack pop into a temporary that is intentionally discarded.
std::variant<Decoded, DecodeError> decode_popfq(
    std::size_t bytes_consumed,
    ir::Ref& next_ref) {
    Decoded d;
    const ir::Ref ref_rsp = next_ref++;
    const ir::Ref ref_flags = next_ref++;
    const ir::Ref ref_eight = next_ref++;
    const ir::Ref ref_new_rsp = next_ref++;
    d.stmts.push_back({ref_rsp, ir::LoadReg{ir::Gpr::Rsp, ir::OpSize::I64}});
    d.stmts.push_back({ref_flags, ir::LoadMemTSO{ref_rsp, ir::OpSize::I64}});
    d.stmts.push_back({ref_eight, ir::Constant{8u, ir::OpSize::I64}});
    d.stmts.push_back(
        {ref_new_rsp, ir::BinOp{ir::BinOpKind::Add, ref_rsp, ref_eight, ir::OpSize::I64}});
    d.stmts.push_back(
        {std::nullopt, ir::StoreReg{ir::Gpr::Rsp, ref_new_rsp, ir::OpSize::I64}});
    d.bytes_consumed = bytes_consumed;
    return d;
}

// LEA r64, [mem] (48 8D /r) — MVP memory-only form.
//
// This is a placeholder decode for now:
//   * destination = reg field
//   * source = effective address of r/m operand
// Register-direct / memory forms beyond base+disp are deferred.
std::variant<Decoded, DecodeError> decode_lea_r64_mem(
    std::span<const Byte> bytes,
    std::size_t& cursor,
    ir::Ref& next_ref) {
    auto modrm = parse_modrm(bytes, cursor);
    if (std::holds_alternative<DecodeError>(modrm)) {
        return std::get<DecodeError>(modrm);
    }
    const auto& m = std::get<ModRmOperand>(modrm);
    if (m.mod == 0b11u) return DecodeError::UnsupportedEncoding;

    Decoded d;
    const ir::Ref ref_addr = emit_address(d.stmts, m, next_ref);
    d.stmts.push_back({std::nullopt, ir::StoreReg{gpr_from_index(m.reg), ref_addr, ir::OpSize::I64}});
    d.bytes_consumed = cursor;
    return d;
}

// SHL/SHR/SAR/ROL/ROR/RCL/RCR r/m64, imm8 (48 C1 /4|/5|/7|/0|/1|/2|/3) — MVP register-direct only.
// Encodes:
//   r/m64 <- reg-shift-operation(r/m64, imm8).
// The immediate is loaded as a zero-extended u64; lowering handles shift count
// masking rules.
std::variant<Decoded, DecodeError> decode_shift_r64_rm_imm8_from_rm(
    std::span<const Byte> bytes,
    const ModRmOperand& m,
    std::size_t& cursor,
    ir::Ref& next_ref,
    ir::BinOpKind op) {
    if (m.mod != 0b11u) return DecodeError::UnsupportedEncoding;

    auto imm = read_le<1>(bytes, cursor);
    if (!imm) return DecodeError::TruncatedInput;
    cursor += 1;

    Decoded d;
    const ir::Ref ref_dst = next_ref++;
    const ir::Ref ref_imm = next_ref++;
    const ir::Ref ref_res = next_ref++;
    d.stmts.push_back({ref_dst, ir::LoadReg{m.base, ir::OpSize::I64}});
    d.stmts.push_back({ref_imm, ir::Constant{*imm, ir::OpSize::I64}});
    d.stmts.push_back({ref_res, ir::BinOp{op, ref_dst, ref_imm, ir::OpSize::I64}});
    d.stmts.push_back({std::nullopt,
                       ir::StoreReg{m.base, ref_res, ir::OpSize::I64}});
    d.bytes_consumed = cursor;
    return d;
}

// SHL/SHR/SAR/ROL/ROR/RCL/RCR r/m64, CL (48 D3 /4|/5|/7|/0|/1|/2|/3) — MVP register-direct only.
// Encodes:
//   r/m64 <- reg-shift-operation(r/m64, cl).
// The shift amount is loaded from CL as I64.
std::variant<Decoded, DecodeError> decode_shift_r64_rm_cl_from_rm(
    const ModRmOperand& m,
    std::size_t bytes_consumed,
    ir::Ref& next_ref,
    ir::BinOpKind op) {
    if (m.mod != 0b11u) return DecodeError::UnsupportedEncoding;

    Decoded d;
    const ir::Ref ref_dst = next_ref++;
    const ir::Ref ref_shift = next_ref++;
    const ir::Ref ref_res = next_ref++;
    d.stmts.push_back({ref_dst, ir::LoadReg{m.base, ir::OpSize::I64}});
    d.stmts.push_back({ref_shift, ir::LoadReg{ir::Gpr::Rcx, ir::OpSize::I64}});
    d.stmts.push_back({ref_res, ir::BinOp{op, ref_dst, ref_shift, ir::OpSize::I64}});
    d.stmts.push_back(
        {std::nullopt, ir::StoreReg{m.base, ref_res, ir::OpSize::I64}});
    d.bytes_consumed = bytes_consumed;
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
    //   * 0xF3 (LZCNT/TZCNT group prefix),
    //   * one REX prefix.
    // More prefix combinations can be added later.
    bool has_operand_size_override = false;
    bool has_f3 = false;
    RexPrefix rex;
    bool seen_rex = false;
    bool seen_66 = false;
    bool seen_f3 = false;
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

        if (b == 0xF3u) {
            if (seen_f3) return DecodeError::UnsupportedEncoding;
            has_f3 = true;
            seen_f3 = true;
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

    // --- CBW/CWDE/CDQE (0x98) -------------------------------------------
    if (opcode == 0x98u) {
        if (has_f3) return DecodeError::UnsupportedEncoding;
        const ir::OpSize src_size = rex.w ? ir::OpSize::I32
                                          : (has_operand_size_override ? ir::OpSize::I8
                                                                       : ir::OpSize::I16);
        const ir::OpSize dst_size = rex.w ? ir::OpSize::I64
                                          : (has_operand_size_override ? ir::OpSize::I16
                                                                       : ir::OpSize::I32);
        return decode_cbw_cwde_cdqe(cursor, next_ref, src_size, dst_size);
    }

    // --- CWD/CDQ/CQO (0x99) ---------------------------------------------
    if (opcode == 0x99u) {
        if (has_f3) return DecodeError::UnsupportedEncoding;
        const ir::OpSize size = rex.w ? ir::OpSize::I64
                                      : (has_operand_size_override ? ir::OpSize::I16
                                                                   : ir::OpSize::I32);
        return decode_cwd_cdq_cqo(cursor, next_ref, size);
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

    // --- LEAVE (0xC9) -------------------------------------------------------
    //
    // MVP placeholder:
    //   RSP ← RBP
    //   RBP ← [RSP]
    // This ignores segment/far behavior and keeps it in plain GPR ops.
    if (opcode == 0xC9u) {
        if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
        if (rex.present) return DecodeError::UnsupportedEncoding;
        Decoded d;
        const ir::Ref ref_old_rbp = next_ref++;
        const ir::Ref ref_new_rbp = next_ref++;
        d.stmts.push_back({ref_old_rbp, ir::LoadReg{ir::Gpr::Rbp, ir::OpSize::I64}});
        d.stmts.push_back({std::nullopt, ir::StoreReg{ir::Gpr::Rsp, ref_old_rbp, ir::OpSize::I64}});
        d.stmts.push_back({ref_new_rbp, ir::LoadMemTSO{ref_old_rbp, ir::OpSize::I64}});
        d.stmts.push_back({std::nullopt, ir::StoreReg{ir::Gpr::Rbp, ref_new_rbp, ir::OpSize::I64}});
        d.bytes_consumed = cursor;
        return d;
    }

    // --- RET imm16 (0xC2) ---------------------------------------------------
    //
    // MVP placeholder: performs `RSP += imm16 + 8`, then emits Return.
    // Correct pop-into-return is not modelled yet; lowering still treats Return
    // as the block terminator with x0 = halt sentinel.
    if (opcode == 0xC2u) {
        if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
        if (rex.present) return DecodeError::UnsupportedEncoding;
        auto imm = read_le<2>(bytes, cursor);
        if (!imm) return DecodeError::TruncatedInput;
        cursor += 2;
        const std::uint64_t pop_bytes = *imm + 8u;
        Decoded d;
        const ir::Ref ref_rsp = next_ref++;
        const ir::Ref ref_pop = next_ref++;
        const ir::Ref ref_new_rsp = next_ref++;
        d.stmts.push_back({ref_rsp, ir::LoadReg{ir::Gpr::Rsp, ir::OpSize::I64}});
        d.stmts.push_back({ref_pop, ir::Constant{pop_bytes, ir::OpSize::I64}});
        d.stmts.push_back({ref_new_rsp,
                           ir::BinOp{ir::BinOpKind::Add, ref_rsp, ref_pop, ir::OpSize::I64}});
        d.stmts.push_back({std::nullopt, ir::StoreReg{ir::Gpr::Rsp, ref_new_rsp, ir::OpSize::I64}});
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

    // --- CALL rel32 (E8 cd) -------------------------------------------------
    //
    // MVP placeholder: treat CALL as an absolute direct jump. Proper return-address
    // handling is deferred to `F1-IR-008` and runtime stack-call support.
    if (opcode == 0xE8u) {
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

    // --- Jcc rel8 (70–7F) -----------------------------------------------
    if (opcode >= 0x70u && opcode <= 0x7Fu) {
        if (rex.present) return DecodeError::UnsupportedEncoding;
        if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
        auto cc = jcc_condition(opcode);
        if (!cc) return DecodeError::UnsupportedEncoding;
        auto imm = read_le<1>(bytes, cursor);
        if (!imm) return DecodeError::TruncatedInput;
        cursor += 1;
        const std::int32_t rel = sign_extend_i32<1>(*imm);
        const std::uint64_t target =
            instruction_guest_pc + cursor + static_cast<std::uint64_t>(static_cast<std::int64_t>(rel));
        const std::uint64_t fallthrough = instruction_guest_pc + cursor;
        Decoded d;
        d.stmts.push_back({std::nullopt, ir::CondJumpRel{*cc, target, fallthrough}});
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

    // --- LEA r64, [mem] (48 8D /r) --------------------------------------
    if (opcode == 0x8Du) {
        if (!rex.w) return DecodeError::UnsupportedEncoding;
        if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
        if (has_f3) return DecodeError::UnsupportedEncoding;
        return decode_lea_r64_mem(bytes, cursor, next_ref);
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

    // --- Two-byte opcodes (0x0F xx) --------------------------------------
    if (opcode == 0x0Fu) {
        auto sub = read_le<1>(bytes, cursor);
        if (!sub) return DecodeError::TruncatedInput;
        ++cursor;
        const Byte subop = static_cast<Byte>(*sub);

        // --- Jcc rel32 (0F 80–8F) --------------------------------------
        if (subop >= 0x80u && subop <= 0x8Fu) {
            if (rex.present) return DecodeError::UnsupportedEncoding;
            if (has_f3) return DecodeError::UnsupportedEncoding;
            if (has_operand_size_override) return DecodeError::UnsupportedEncoding;

            auto cc = jcc_condition(subop);
            if (!cc) return DecodeError::UnsupportedEncoding;
            auto imm = read_le<4>(bytes, cursor);
            if (!imm) return DecodeError::TruncatedInput;
            cursor += 4;

            const std::int32_t rel = sign_extend_i32<4>(*imm);
            const std::uint64_t target =
                instruction_guest_pc + cursor + static_cast<std::uint64_t>(static_cast<std::int64_t>(rel));
            const std::uint64_t fallthrough = instruction_guest_pc + cursor;
            Decoded d;
            d.stmts.push_back({std::nullopt, ir::CondJumpRel{*cc, target, fallthrough}});
            d.bytes_consumed = cursor;
            return d;
        }

        // --- CMOVcc r64, r/m64 (0F 40–4F) ---------------------------
        if (subop >= 0x40u && subop <= 0x4Fu) {
            if (!rex.w) return DecodeError::UnsupportedEncoding;
            if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
            if (has_f3) return DecodeError::UnsupportedEncoding;
            const auto cc = jcc_condition(subop);
            if (!cc) return DecodeError::UnsupportedEncoding;
            return decode_cmovcc_r64_rm64(*cc, bytes, cursor, next_ref);
        }

        // --- SETcc r/m8 (0F 90–9F) ----------------------------------
        if (subop >= 0x90u && subop <= 0x9Fu) {
            if (rex.present) return DecodeError::UnsupportedEncoding;
            if (has_f3) return DecodeError::UnsupportedEncoding;
            if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
            const auto cc = jcc_condition(subop);
            if (!cc) return DecodeError::UnsupportedEncoding;
            return decode_setcc_r8_rm8(*cc, bytes, cursor, next_ref);
        }

        if (subop == 0xBEu) {
            if (!rex.w) return DecodeError::UnsupportedEncoding;
            if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
            if (has_f3) return DecodeError::UnsupportedEncoding;
            return decode_movx_r64_rm(bytes, cursor, next_ref, ir::OpSize::I8, true);
        }
        if (subop == 0xBFu) {
            if (!rex.w) return DecodeError::UnsupportedEncoding;
            if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
            if (has_f3) return DecodeError::UnsupportedEncoding;
            return decode_movx_r64_rm(bytes, cursor, next_ref, ir::OpSize::I16, true);
        }
        if (subop == 0xB6u) {
            if (!rex.w) return DecodeError::UnsupportedEncoding;
            if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
            if (has_f3) return DecodeError::UnsupportedEncoding;
            return decode_movx_r64_rm(bytes, cursor, next_ref, ir::OpSize::I8, false);
        }
        if (subop == 0xB7u) {
            if (!rex.w) return DecodeError::UnsupportedEncoding;
            if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
            if (has_f3) return DecodeError::UnsupportedEncoding;
            return decode_movx_r64_rm(bytes, cursor, next_ref, ir::OpSize::I16, false);
        }
        if (subop == 0xAFu) {
            if (!rex.w) return DecodeError::UnsupportedEncoding;
            if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
            return decode_imul_r64_r_rm(bytes, cursor, next_ref);
        }
        if (subop == 0xBAu) {
            auto modrm = parse_modrm(bytes, cursor);
            if (std::holds_alternative<DecodeError>(modrm)) {
                return std::get<DecodeError>(modrm);
            }
            const auto& m = std::get<ModRmOperand>(modrm);
            if (m.mod != 0b11u) return DecodeError::UnsupportedEncoding;

            if (m.reg == 4u) {
                return decode_bt_r64_rm_imm8_from_rm(bytes, m, cursor, next_ref, BtSubOpcode::Bt);
            }
            if (m.reg == 5u) {
                return decode_bt_r64_rm_imm8_from_rm(bytes, m, cursor, next_ref, BtSubOpcode::Bts);
            }
            if (m.reg == 6u) {
                return decode_bt_r64_rm_imm8_from_rm(bytes, m, cursor, next_ref, BtSubOpcode::Btr);
            }
            if (m.reg == 7u) {
                return decode_bt_r64_rm_imm8_from_rm(bytes, m, cursor, next_ref, BtSubOpcode::Btc);
            }
            return DecodeError::UnsupportedEncoding;
        }
        if (subop == 0xB1u) {
            if (!rex.w) return DecodeError::UnsupportedEncoding;
            if (has_f3) return DecodeError::UnsupportedEncoding;
            if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
            return decode_cmpxchg_r64_rm64(bytes, cursor, next_ref);
        }
        if (subop == 0xC1u) {
            if (!rex.w) return DecodeError::UnsupportedEncoding;
            if (has_f3) return DecodeError::UnsupportedEncoding;
            if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
            return decode_xadd_r64_rm64(bytes, cursor, next_ref);
        }
        if (subop == 0xC7u) {
            if (!rex.w) return DecodeError::UnsupportedEncoding;
            if (has_f3) return DecodeError::UnsupportedEncoding;
            if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
            return decode_cmpxchg16b_m128(bytes, cursor, next_ref);
        }
        if (subop == 0xB8u) {
            if (!has_f3) return DecodeError::UnsupportedEncoding;
            if (!rex.w) return DecodeError::UnsupportedEncoding;
            if (has_operand_size_override) return DecodeError::UnsupportedEncoding;

            auto modrm = parse_modrm(bytes, cursor);
            if (std::holds_alternative<DecodeError>(modrm)) {
                return std::get<DecodeError>(modrm);
            }
            const auto& m = std::get<ModRmOperand>(modrm);
            return decode_popcnt_r64_r_rm(m, cursor, next_ref);
        }
        if (subop == 0xBCu || subop == 0xBDu) {
            if (!rex.w) return DecodeError::UnsupportedEncoding;
            if (has_operand_size_override) return DecodeError::UnsupportedEncoding;

            auto modrm = parse_modrm(bytes, cursor);
            if (std::holds_alternative<DecodeError>(modrm)) {
                return std::get<DecodeError>(modrm);
            }
            const auto& m = std::get<ModRmOperand>(modrm);
            if (has_f3) {
                if (subop == 0xBDu) {
                    return decode_lzcnt_r64_r_rm(m, cursor, next_ref);
                }
                if (subop == 0xBCu) {
                    return decode_tzcnt_r64_r_rm(m, cursor, next_ref);
                }
                return DecodeError::UnsupportedEncoding;
            }
            return decode_bsf_bsr_r64_r_rm(m, cursor, next_ref);
        }
        return DecodeError::UnknownOpcode;
    }

    // --- MOVSXD r64, r/m32 (63 /r) --------------------------------------
    if (opcode == 0x63u) {
        // MOVSXD r64, r/m32 (sign-extend 32-bit source to 64).
        if (!rex.w) return DecodeError::UnsupportedEncoding;
        if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
        if (has_f3) return DecodeError::UnsupportedEncoding;
        return decode_movx_r64_rm(bytes, cursor, next_ref, ir::OpSize::I32, true);
    }

    // --- ALU r/m64, r64 (shared shape, register-direct only for MVP) ----
    switch (opcode) {
        case 0x9Cu:
            if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
            if (has_f3) return DecodeError::UnsupportedEncoding;
            if (rex.present) return DecodeError::UnsupportedEncoding;
            return decode_pushfq(cursor, next_ref);
        case 0x9Du:
            if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
            if (has_f3) return DecodeError::UnsupportedEncoding;
            if (rex.present) return DecodeError::UnsupportedEncoding;
            return decode_popfq(cursor, next_ref);
        case 0x50u:
            if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
            if (has_f3) return DecodeError::UnsupportedEncoding;
            return decode_push_r64(gpr_from_index(opcode - 0x50u), cursor, next_ref);
        case 0x58u:
            if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
            if (has_f3) return DecodeError::UnsupportedEncoding;
            return decode_pop_r64(gpr_from_index(opcode - 0x58u), cursor, next_ref);
        case 0x6Au:
            if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
            if (has_f3) return DecodeError::UnsupportedEncoding;
            return decode_push_imm8_r64(bytes, cursor, next_ref);
        case 0x68u:
            if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
            if (has_f3) return DecodeError::UnsupportedEncoding;
            return decode_push_imm32_r64(bytes, cursor, next_ref);
        case 0x6Bu:
            if (!rex.w) return DecodeError::UnsupportedEncoding;
            if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
            return decode_imul_r64_rm_imm8(bytes, cursor, next_ref);
        case 0xC1u:
            if (!rex.w) return DecodeError::UnsupportedEncoding;
            if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
            {
                auto modrm = parse_modrm(bytes, cursor);
                if (std::holds_alternative<DecodeError>(modrm)) {
                    return std::get<DecodeError>(modrm);
                }
                const auto& m = std::get<ModRmOperand>(modrm);
                if (m.reg == 0u) {
                    return decode_shift_r64_rm_imm8_from_rm(bytes, m, cursor, next_ref, ir::BinOpKind::Rol);
                }
                if (m.reg == 1u) {
                    return decode_shift_r64_rm_imm8_from_rm(bytes, m, cursor, next_ref, ir::BinOpKind::Ror);
                }
                if (m.reg == 2u) {
                    return decode_shift_r64_rm_imm8_from_rm(bytes, m, cursor, next_ref, ir::BinOpKind::Rcl);
                }
                if (m.reg == 3u) {
                    return decode_shift_r64_rm_imm8_from_rm(bytes, m, cursor, next_ref, ir::BinOpKind::Rcr);
                }
                if (m.reg == 4u) {
                    return decode_shift_r64_rm_imm8_from_rm(bytes, m, cursor, next_ref, ir::BinOpKind::Shl);
                }
                if (m.reg == 5u) {
                    return decode_shift_r64_rm_imm8_from_rm(bytes, m, cursor, next_ref, ir::BinOpKind::Shr);
                }
                if (m.reg == 7u) {
                    return decode_shift_r64_rm_imm8_from_rm(bytes, m, cursor, next_ref, ir::BinOpKind::Sar);
                }
                return DecodeError::UnsupportedEncoding;
            }
        case 0xD3u:
            if (!rex.w) return DecodeError::UnsupportedEncoding;
            if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
            {
                auto modrm = parse_modrm(bytes, cursor);
                if (std::holds_alternative<DecodeError>(modrm)) {
                    return std::get<DecodeError>(modrm);
                }
                const auto& m = std::get<ModRmOperand>(modrm);
                if (m.mod != 0b11u) return DecodeError::UnsupportedEncoding;
                if (m.reg == 0u) {
                    return decode_shift_r64_rm_cl_from_rm(m, cursor, next_ref, ir::BinOpKind::Rol);
                }
                if (m.reg == 1u) {
                    return decode_shift_r64_rm_cl_from_rm(m, cursor, next_ref, ir::BinOpKind::Ror);
                }
                if (m.reg == 2u) {
                    return decode_shift_r64_rm_cl_from_rm(m, cursor, next_ref, ir::BinOpKind::Rcl);
                }
                if (m.reg == 3u) {
                    return decode_shift_r64_rm_cl_from_rm(m, cursor, next_ref, ir::BinOpKind::Rcr);
                }
                if (m.reg == 4u) {
                    return decode_shift_r64_rm_cl_from_rm(m, cursor, next_ref, ir::BinOpKind::Shl);
                }
                if (m.reg == 5u) {
                    return decode_shift_r64_rm_cl_from_rm(m, cursor, next_ref, ir::BinOpKind::Shr);
                }
                if (m.reg == 7u) {
                    return decode_shift_r64_rm_cl_from_rm(m, cursor, next_ref, ir::BinOpKind::Sar);
                }
                return DecodeError::UnsupportedEncoding;
            }
        case 0x69u:
            if (!rex.w) return DecodeError::UnsupportedEncoding;
            if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
            return decode_imul_r64_rm_imm32(bytes, cursor, next_ref);
        case 0x01u:
            if (!rex.w) return DecodeError::UnsupportedEncoding;
            return decode_alu_rm_r(bytes, cursor, ir::BinOpKind::Add, next_ref);
        case 0x11u:
            if (!rex.w) return DecodeError::UnsupportedEncoding;
            // NOTE: this emits plain add for now; carry-in is not yet modeled.
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
        case 0x19u:
            if (!rex.w) return DecodeError::UnsupportedEncoding;
            // NOTE: this emits plain sub for now; borrow-in is not yet modeled.
            return decode_alu_rm_r(bytes, cursor, ir::BinOpKind::Sub, next_ref);
        case 0x31u:
            if (!rex.w) return DecodeError::UnsupportedEncoding;
            return decode_alu_rm_r(bytes, cursor, ir::BinOpKind::Xor, next_ref);
        case 0x85u:
            if (!rex.w) return DecodeError::UnsupportedEncoding;
            if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
            return decode_test_rm_r(bytes, cursor, next_ref);
        case 0x87u:
            if (!rex.w) return DecodeError::UnsupportedEncoding;
            if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
            if (has_f3) return DecodeError::UnsupportedEncoding;
            return decode_xchg_r64_rm64(bytes, cursor, next_ref);
        case 0xFFu: {
            if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
            if (!rex.w) return DecodeError::UnsupportedEncoding;
            auto modrm = parse_modrm(bytes, cursor);
            if (std::holds_alternative<DecodeError>(modrm)) {
                return std::get<DecodeError>(modrm);
            }
            const auto& m = std::get<ModRmOperand>(modrm);
            if (m.reg == 0u) {
                return decode_incdec_from_rm(m, cursor, next_ref, ir::BinOpKind::Add);
            }
            if (m.reg == 1u) {
                return decode_incdec_from_rm(m, cursor, next_ref, ir::BinOpKind::Sub);
            }
            if (m.reg == 4u) {
                Decoded d;
                const ir::Ref ref_target = next_ref++;
                if (m.mod == 0b11u) {
                    d.stmts.push_back({ref_target, ir::LoadReg{m.base, ir::OpSize::I64}});
                } else {
                    const ir::Ref ref_addr = emit_address(d.stmts, m, next_ref);
                    d.stmts.push_back({ref_target, ir::LoadMemTSO{ref_addr, ir::OpSize::I64}});
                }
                d.stmts.push_back({std::nullopt, ir::JumpReg{ref_target}});
                d.bytes_consumed = cursor;
                return d;
            }
            if (m.reg == 2u) {
                Decoded d;
                const ir::Ref ref_target = next_ref++;
                if (m.mod == 0b11u) {
                    d.stmts.push_back({ref_target, ir::LoadReg{m.base, ir::OpSize::I64}});
                } else {
                    const ir::Ref ref_addr = emit_address(d.stmts, m, next_ref);
                    d.stmts.push_back(
                        {ref_target, ir::LoadMemTSO{ref_addr, ir::OpSize::I64}});
                }
                // MVP placeholder: decode CALL r/m64 like CALL-like transfer.
                // Returning into the guest stack is not yet modeled.
                d.stmts.push_back({std::nullopt, ir::JumpReg{ref_target}});
                d.bytes_consumed = cursor;
                return d;
            }
            return DecodeError::UnsupportedEncoding;
        }
        case 0xF7u: {
            if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
            if (!rex.w) return DecodeError::UnsupportedEncoding;
            auto modrm = parse_modrm(bytes, cursor);
            if (std::holds_alternative<DecodeError>(modrm)) {
                return std::get<DecodeError>(modrm);
            }
            const auto& m = std::get<ModRmOperand>(modrm);
            if (m.reg == 2u) {
                return decode_neg_not_from_rm(m, cursor, next_ref, ir::BinOpKind::Xor);
            }
            if (m.reg == 3u) {
                return decode_neg_not_from_rm(m, cursor, next_ref, ir::BinOpKind::Sub);
            }
            if (m.reg == 4u) {
                return decode_mul_imul_from_rm(m, cursor, next_ref, ir::BinOpKind::Mul);
            }
            if (m.reg == 5u) {
                return decode_mul_imul_from_rm(m, cursor, next_ref, ir::BinOpKind::Mul);
            }
            if (m.reg == 6u) {
                return decode_div_from_rm(m, cursor, next_ref);
            }
            if (m.reg == 7u) {
                return decode_div_from_rm(m, cursor, next_ref);
            }
            return DecodeError::UnsupportedEncoding;
        }
        default:
            break;
    }

    return DecodeError::UnknownOpcode;
}

}  // namespace prisma::decoder
