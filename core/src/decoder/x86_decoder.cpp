// core/src/decoder/x86_decoder.cpp — implementation of the MVP x86_64
// decoder declared in prisma/decoder.hpp.
//
// Format conventions (kept tight because the encoding reference matters):
//
//   REX prefix:   0100 WRXB  (0x40..0x4F)
//     W = operand size 64, R = ext reg, X = ext SIB.idx, B = ext rm/op-reg.
//
//   ModR/M byte:  mm rrr bbb
//     mm = 00  → [rm]            (except rm=100 SIB, rm=101 RIP+disp32)
//     mm = 01  → [rm + disp8]    (except rm=100 SIB)
//     mm = 10  → [rm + disp32]   (except rm=100 SIB)
//     mm = 11  → register direct
//     rrr = reg field (extended by REX.R).
//     bbb = rm field  (extended by REX.B).

#include "prisma/decoder.hpp"

#include <array>
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

// F2-IR-048 — VEX-encoded ops (AVX-128). When `present` is true, the
// instruction was encoded with C4 / C5 and the existing prefix bits
// (`has_66/has_f2/has_f3` and `rex`) were synthesised from the VEX
// fields. `vvvv` is the third (non-destructive) source operand
// extracted from the VEX byte (always inverted on the wire). `L` ≠ 0
// means 256-bit (YMM) which we currently reject.
struct VexPrefix {
    bool         present{false};
    bool         L{false};       // 0 = 128-bit, 1 = 256-bit
    std::uint8_t vvvv{0};        // 4-bit second source register
    std::uint8_t mmmmm{1};       // implied escape (1=0F, 2=0F38, 3=0F3A)
    std::uint8_t pp{0};          // implied prefix (00=none, 01=66, 10=F3, 11=F2)
};

// Parsed ModR/M operand. If `mod == 11` it's a register direct reference
// (`base` is the register, displacement unused). Otherwise it describes a
// memory access with optional base + optional scaled index + signed
// displacement, including RIP-relative addressing.
struct ModRmOperand {
    unsigned mod;          // 0..3
    unsigned reg;          // 0..15, the `reg` field
    ir::Gpr  base;         // register-direct rm, or memory base if present
    bool     has_base;     // memory form: whether a base register exists
    bool     has_index;    // memory form: whether a scaled index exists
    ir::Gpr  index;        // memory form index register when has_index=true
    unsigned scale_shift;  // 0..3, scale = 1 << scale_shift
    bool     rip_relative; // memory form: base is RIP + disp32
    bool     address_size_32; // memory form: apply 32-bit address semantics
    std::int32_t disp;     // 0 for mod=00, signed for mod=01/10
};

std::variant<ModRmOperand, DecodeError> parse_modrm(
    std::span<const Byte> bytes,
    std::size_t& cursor,
    const RexPrefix& rex,
    bool address_size_override);
ir::Ref emit_address(
    std::vector<ir::Stmt>& stmts,
    const ModRmOperand& op,
    ir::Ref& next_ref,
    std::uint64_t rip_after);
std::variant<Decoded, DecodeError> decode_cmpxchg_r64_rm64(
    std::span<const Byte> bytes,
    std::size_t& cursor,
    const RexPrefix& rex,
    std::uint64_t instruction_guest_pc,
    bool address_size_override,
    ir::Ref& next_ref);
std::variant<Decoded, DecodeError> decode_cmpxchg16b_m128(
    std::span<const Byte> bytes,
    std::size_t& cursor,
    const RexPrefix& rex,
    std::uint64_t instruction_guest_pc,
    bool address_size_override,
    ir::Ref& next_ref);
std::variant<Decoded, DecodeError> decode_xadd_r64_rm64(
    std::span<const Byte> bytes,
    std::size_t& cursor,
    const RexPrefix& rex,
    std::uint64_t instruction_guest_pc,
    bool address_size_override,
    ir::Ref& next_ref);

constexpr ir::Gpr gpr_from_index(unsigned idx) noexcept {
    return static_cast<ir::Gpr>(idx);
}

constexpr unsigned opcode_reg_index(Byte opcode, Byte base_opcode, const RexPrefix& rex) noexcept {
    return static_cast<unsigned>(opcode - base_opcode) | (rex.b ? 0x8u : 0u);
}

constexpr std::uint64_t byte_width(ir::OpSize size) noexcept {
    return ir::bit_width(size) / 8u;
}

constexpr ir::OpSize operand_size_from_prefixes(
    const RexPrefix& rex,
    bool has_operand_size_override) noexcept {
    return rex.w ? ir::OpSize::I64
                 : (has_operand_size_override ? ir::OpSize::I16 : ir::OpSize::I32);
}

constexpr std::size_t kMaxInstructionBytes = 15;

enum class OneByteDispatchKind : std::uint8_t {
    None,
    Pushfq,
    Popfq,
    PushReg,
    PopReg,
    PushImm8,
    PushImm32,
    ImulImm8,
    ShiftImm8,
    ShiftCl,
    ImulImm32,
    AluRmR,
    TestRmR,
    XchgRmR,
    Group5,
    Group3,
};

struct OneByteDispatchEntry {
    OneByteDispatchKind kind{OneByteDispatchKind::None};
    ir::BinOpKind binop{ir::BinOpKind::Add};
};

constexpr auto build_one_byte_dispatch_table() noexcept {
    std::array<OneByteDispatchEntry, 256> table{};

    table[0x01u] = {OneByteDispatchKind::AluRmR, ir::BinOpKind::Add};
    table[0x09u] = {OneByteDispatchKind::AluRmR, ir::BinOpKind::Or};
    table[0x11u] = {OneByteDispatchKind::AluRmR, ir::BinOpKind::Add};
    table[0x19u] = {OneByteDispatchKind::AluRmR, ir::BinOpKind::Sub};
    table[0x21u] = {OneByteDispatchKind::AluRmR, ir::BinOpKind::And};
    table[0x29u] = {OneByteDispatchKind::AluRmR, ir::BinOpKind::Sub};
    table[0x31u] = {OneByteDispatchKind::AluRmR, ir::BinOpKind::Xor};
    table[0x68u] = {OneByteDispatchKind::PushImm32};
    table[0x69u] = {OneByteDispatchKind::ImulImm32};
    table[0x6Au] = {OneByteDispatchKind::PushImm8};
    table[0x6Bu] = {OneByteDispatchKind::ImulImm8};
    table[0x85u] = {OneByteDispatchKind::TestRmR};
    table[0x87u] = {OneByteDispatchKind::XchgRmR};
    table[0x9Cu] = {OneByteDispatchKind::Pushfq};
    table[0x9Du] = {OneByteDispatchKind::Popfq};
    table[0xC1u] = {OneByteDispatchKind::ShiftImm8};
    table[0xD3u] = {OneByteDispatchKind::ShiftCl};
    table[0xF7u] = {OneByteDispatchKind::Group3};
    table[0xFFu] = {OneByteDispatchKind::Group5};

    for (unsigned opcode = 0x50u; opcode <= 0x57u; ++opcode) {
        table[opcode] = {OneByteDispatchKind::PushReg};
    }
    for (unsigned opcode = 0x58u; opcode <= 0x5Fu; ++opcode) {
        table[opcode] = {OneByteDispatchKind::PopReg};
    }

    return table;
}

constexpr auto kOneByteDispatchTable = build_one_byte_dispatch_table();

enum class TwoByteDispatchKind : std::uint8_t {
    None,
    JccRel32,
    Cmovcc,
    Setcc,
    Rdtsc,
    Syscall,
    Group01,
    Cpuid,
    ImulRm,
    BtImmGroup,
    Popcnt,
    Movzx8,
    Movzx16,
    Cmpxchg,
    Movsx8,
    Movsx16,
    BitScanOrCount,
    Xadd,
    Cmpxchg16b,
};

struct TwoByteDispatchEntry {
    TwoByteDispatchKind kind{TwoByteDispatchKind::None};
};

constexpr auto build_two_byte_dispatch_table() noexcept {
    std::array<TwoByteDispatchEntry, 256> table{};

    table[0x01u] = {TwoByteDispatchKind::Group01};
    table[0x05u] = {TwoByteDispatchKind::Syscall};
    table[0x31u] = {TwoByteDispatchKind::Rdtsc};
    table[0xA2u] = {TwoByteDispatchKind::Cpuid};
    table[0xAFu] = {TwoByteDispatchKind::ImulRm};
    table[0xB1u] = {TwoByteDispatchKind::Cmpxchg};
    table[0xB6u] = {TwoByteDispatchKind::Movzx8};
    table[0xB7u] = {TwoByteDispatchKind::Movzx16};
    table[0xB8u] = {TwoByteDispatchKind::Popcnt};
    table[0xBAu] = {TwoByteDispatchKind::BtImmGroup};
    table[0xBCu] = {TwoByteDispatchKind::BitScanOrCount};
    table[0xBDu] = {TwoByteDispatchKind::BitScanOrCount};
    table[0xBEu] = {TwoByteDispatchKind::Movsx8};
    table[0xBFu] = {TwoByteDispatchKind::Movsx16};
    table[0xC1u] = {TwoByteDispatchKind::Xadd};
    table[0xC7u] = {TwoByteDispatchKind::Cmpxchg16b};

    for (unsigned opcode = 0x40u; opcode <= 0x4Fu; ++opcode) {
        table[opcode] = {TwoByteDispatchKind::Cmovcc};
    }
    for (unsigned opcode = 0x80u; opcode <= 0x8Fu; ++opcode) {
        table[opcode] = {TwoByteDispatchKind::JccRel32};
    }
    for (unsigned opcode = 0x90u; opcode <= 0x9Fu; ++opcode) {
        table[opcode] = {TwoByteDispatchKind::Setcc};
    }

    return table;
}

constexpr auto kTwoByteDispatchTable = build_two_byte_dispatch_table();

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

// Consume `N` bytes from the current instruction stream while enforcing the
// architectural 15-byte maximum x86 instruction length.
template <std::size_t N>
std::variant<std::uint64_t, DecodeError> consume_le(
    std::span<const Byte> bytes,
    std::size_t& cursor) {
    static_assert(N >= 1 && N <= 8, "consume_le: N must be 1..8");

    const std::size_t end = cursor + N;
    if (end > kMaxInstructionBytes) {
        return end <= bytes.size()
                   ? DecodeError::UnsupportedEncoding
                   : DecodeError::TruncatedInput;
    }

    auto v = read_le<N>(bytes, cursor);
    if (!v) return DecodeError::TruncatedInput;
    cursor = end;
    return *v;
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

ir::Ref emit_zero_extend_u32(std::vector<ir::Stmt>& stmts, ir::Ref value, ir::Ref& next_ref) {
    const ir::Ref ref_mask = next_ref++;
    const ir::Ref ref_masked = next_ref++;
    stmts.push_back({ref_mask, ir::Constant{0xFFFF'FFFFULL, ir::OpSize::I64}});
    stmts.push_back(
        {ref_masked, ir::BinOp{ir::BinOpKind::And, value, ref_mask, ir::OpSize::I64}});
    return ref_masked;
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
    const RexPrefix& rex,
    std::uint64_t instruction_guest_pc,
    bool address_size_override,
    ir::Ref& next_ref,
    ir::OpSize src_size,
    bool sign_extend) {
    auto modrm = parse_modrm(bytes, cursor, rex, address_size_override);
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
        const ir::Ref ref_addr = emit_address(d.stmts, m, next_ref, instruction_guest_pc + cursor);
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
    const RexPrefix& rex,
    std::uint64_t instruction_guest_pc,
    bool address_size_override,
    ir::Ref& next_ref) {
    auto modrm = parse_modrm(bytes, cursor, rex, address_size_override);
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
        const ir::Ref ref_addr = emit_address(d.stmts, m, next_ref, instruction_guest_pc + cursor);
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
    const RexPrefix& rex,
    std::uint64_t instruction_guest_pc,
    bool address_size_override,
    ir::Ref& next_ref,
    ir::OpSize size) {
    auto modrm = parse_modrm(bytes, cursor, rex, address_size_override);
    if (std::holds_alternative<DecodeError>(modrm)) {
        return std::get<DecodeError>(modrm);
    }
    const auto& m = std::get<ModRmOperand>(modrm);

    Decoded d;
    const ir::Ref ref_reg = next_ref++;
    d.stmts.push_back({ref_reg, ir::LoadReg{gpr_from_index(m.reg), size}});

    if (m.mod == 0b11u) {
        const ir::Ref ref_mem_or_reg = next_ref++;
        d.stmts.push_back({ref_mem_or_reg, ir::LoadReg{m.base, size}});
        d.stmts.push_back(
            {std::nullopt, ir::StoreReg{m.base, ref_reg, size}});
        d.stmts.push_back(
            {std::nullopt, ir::StoreReg{gpr_from_index(m.reg), ref_mem_or_reg, size}});
    } else {
        const ir::Ref ref_addr = emit_address(d.stmts, m, next_ref, instruction_guest_pc + cursor);
        const ir::Ref ref_mem = next_ref++;
        d.stmts.push_back({ref_mem, ir::LoadMemTSO{ref_addr, size}});
        d.stmts.push_back(
            {std::nullopt, ir::StoreReg{gpr_from_index(m.reg), ref_mem, size}});
        d.stmts.push_back({std::nullopt, ir::StoreMemTSO{ref_addr, ref_reg, size}});
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
    const RexPrefix& rex,
    std::uint64_t instruction_guest_pc,
    bool address_size_override,
    ir::Ref& next_ref) {
    auto modrm = parse_modrm(bytes, cursor, rex, address_size_override);
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
        ref_addr = emit_address(d.stmts, m, next_ref, instruction_guest_pc + cursor);
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
    const RexPrefix& rex,
    std::uint64_t instruction_guest_pc,
    bool address_size_override,
    ir::Ref& next_ref) {
    auto modrm = parse_modrm(bytes, cursor, rex, address_size_override);
    if (std::holds_alternative<DecodeError>(modrm)) {
        return std::get<DecodeError>(modrm);
    }
    const auto& m = std::get<ModRmOperand>(modrm);
    if (m.reg != 1u) return DecodeError::UnsupportedEncoding;
    if (m.mod == 0b11u) return DecodeError::UnsupportedEncoding;

    Decoded d;
    const ir::Ref ref_mem_addr = emit_address(d.stmts, m, next_ref, instruction_guest_pc + cursor);
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
    const RexPrefix& rex,
    std::uint64_t instruction_guest_pc,
    bool address_size_override,
    ir::Ref& next_ref) {
    auto modrm = parse_modrm(bytes, cursor, rex, address_size_override);
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
        const ir::Ref ref_addr = emit_address(d.stmts, m, next_ref, instruction_guest_pc + cursor);
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

// String ops currently assume DF=0 and therefore advance RSI/RDI forward.
// REP-family loop semantics are left for a later slice; these helpers model
// exactly one element.
std::variant<Decoded, DecodeError> decode_stos(ir::OpSize size,
                                               bool address_size_override,
                                               std::size_t bytes_consumed,
                                               ir::Ref& next_ref) {
    Decoded d;
    const ir::Ref ref_value = next_ref++;
    const ir::Ref ref_rdi = next_ref++;
    d.stmts.push_back({ref_value, ir::LoadReg{ir::Gpr::Rax, size}});
    d.stmts.push_back({ref_rdi, ir::LoadReg{ir::Gpr::Rdi, ir::OpSize::I64}});
    const ir::Ref ref_addr = address_size_override ? emit_zero_extend_u32(d.stmts, ref_rdi, next_ref)
                                                   : ref_rdi;
    d.stmts.push_back({std::nullopt, ir::StoreMemTSO{ref_addr, ref_value, size}});
    const ir::Ref ref_stride = next_ref++;
    const ir::Ref ref_next_rdi = next_ref++;
    d.stmts.push_back({ref_stride, ir::Constant{byte_width(size), ir::OpSize::I64}});
    d.stmts.push_back(
        {ref_next_rdi, ir::BinOp{ir::BinOpKind::Add, ref_rdi, ref_stride, ir::OpSize::I64}});
    const ir::Ref ref_wrapped_rdi =
        address_size_override ? emit_zero_extend_u32(d.stmts, ref_next_rdi, next_ref) : ref_next_rdi;
    d.stmts.push_back({std::nullopt, ir::StoreReg{ir::Gpr::Rdi, ref_wrapped_rdi, ir::OpSize::I64}});
    d.bytes_consumed = bytes_consumed;
    return d;
}

std::variant<Decoded, DecodeError> decode_movs(ir::OpSize size,
                                               bool address_size_override,
                                               std::size_t bytes_consumed,
                                               ir::Ref& next_ref) {
    Decoded d;
    const ir::Ref ref_rsi = next_ref++;
    d.stmts.push_back({ref_rsi, ir::LoadReg{ir::Gpr::Rsi, ir::OpSize::I64}});
    const ir::Ref ref_src_addr = address_size_override ? emit_zero_extend_u32(d.stmts, ref_rsi, next_ref)
                                                       : ref_rsi;
    const ir::Ref ref_value = next_ref++;
    d.stmts.push_back({ref_value, ir::LoadMemTSO{ref_src_addr, size}});
    const ir::Ref ref_rdi = next_ref++;
    d.stmts.push_back({ref_rdi, ir::LoadReg{ir::Gpr::Rdi, ir::OpSize::I64}});
    const ir::Ref ref_dst_addr = address_size_override ? emit_zero_extend_u32(d.stmts, ref_rdi, next_ref)
                                                       : ref_rdi;
    d.stmts.push_back({std::nullopt, ir::StoreMemTSO{ref_dst_addr, ref_value, size}});
    const ir::Ref ref_stride = next_ref++;
    const ir::Ref ref_next_rsi = next_ref++;
    const ir::Ref ref_next_rdi = next_ref++;
    d.stmts.push_back({ref_stride, ir::Constant{byte_width(size), ir::OpSize::I64}});
    d.stmts.push_back(
        {ref_next_rsi, ir::BinOp{ir::BinOpKind::Add, ref_rsi, ref_stride, ir::OpSize::I64}});
    d.stmts.push_back(
        {ref_next_rdi, ir::BinOp{ir::BinOpKind::Add, ref_rdi, ref_stride, ir::OpSize::I64}});
    const ir::Ref ref_wrapped_rsi =
        address_size_override ? emit_zero_extend_u32(d.stmts, ref_next_rsi, next_ref) : ref_next_rsi;
    const ir::Ref ref_wrapped_rdi =
        address_size_override ? emit_zero_extend_u32(d.stmts, ref_next_rdi, next_ref) : ref_next_rdi;
    d.stmts.push_back({std::nullopt, ir::StoreReg{ir::Gpr::Rsi, ref_wrapped_rsi, ir::OpSize::I64}});
    d.stmts.push_back({std::nullopt, ir::StoreReg{ir::Gpr::Rdi, ref_wrapped_rdi, ir::OpSize::I64}});
    d.bytes_consumed = bytes_consumed;
    return d;
}

std::variant<Decoded, DecodeError> decode_cmps(ir::OpSize size,
                                               bool address_size_override,
                                               std::size_t bytes_consumed,
                                               ir::Ref& next_ref) {
    Decoded d;
    const ir::Ref ref_rsi = next_ref++;
    d.stmts.push_back({ref_rsi, ir::LoadReg{ir::Gpr::Rsi, ir::OpSize::I64}});
    const ir::Ref ref_src_addr = address_size_override ? emit_zero_extend_u32(d.stmts, ref_rsi, next_ref)
                                                       : ref_rsi;
    const ir::Ref ref_lhs = next_ref++;
    d.stmts.push_back({ref_lhs, ir::LoadMemTSO{ref_src_addr, size}});
    const ir::Ref ref_rdi = next_ref++;
    d.stmts.push_back({ref_rdi, ir::LoadReg{ir::Gpr::Rdi, ir::OpSize::I64}});
    const ir::Ref ref_dst_addr = address_size_override ? emit_zero_extend_u32(d.stmts, ref_rdi, next_ref)
                                                       : ref_rdi;
    const ir::Ref ref_rhs = next_ref++;
    d.stmts.push_back({ref_rhs, ir::LoadMemTSO{ref_dst_addr, size}});
    d.stmts.push_back({std::nullopt, ir::CmpFlags{ref_lhs, ref_rhs, size}});
    const ir::Ref ref_stride = next_ref++;
    const ir::Ref ref_next_rsi = next_ref++;
    const ir::Ref ref_next_rdi = next_ref++;
    d.stmts.push_back({ref_stride, ir::Constant{byte_width(size), ir::OpSize::I64}});
    d.stmts.push_back(
        {ref_next_rsi, ir::BinOp{ir::BinOpKind::Add, ref_rsi, ref_stride, ir::OpSize::I64}});
    d.stmts.push_back(
        {ref_next_rdi, ir::BinOp{ir::BinOpKind::Add, ref_rdi, ref_stride, ir::OpSize::I64}});
    const ir::Ref ref_wrapped_rsi =
        address_size_override ? emit_zero_extend_u32(d.stmts, ref_next_rsi, next_ref) : ref_next_rsi;
    const ir::Ref ref_wrapped_rdi =
        address_size_override ? emit_zero_extend_u32(d.stmts, ref_next_rdi, next_ref) : ref_next_rdi;
    d.stmts.push_back({std::nullopt, ir::StoreReg{ir::Gpr::Rsi, ref_wrapped_rsi, ir::OpSize::I64}});
    d.stmts.push_back({std::nullopt, ir::StoreReg{ir::Gpr::Rdi, ref_wrapped_rdi, ir::OpSize::I64}});
    d.bytes_consumed = bytes_consumed;
    return d;
}

std::variant<Decoded, DecodeError> decode_scas(ir::OpSize size,
                                               bool address_size_override,
                                               std::size_t bytes_consumed,
                                               ir::Ref& next_ref) {
    Decoded d;
    const ir::Ref ref_acc = next_ref++;
    const ir::Ref ref_rdi = next_ref++;
    d.stmts.push_back({ref_acc, ir::LoadReg{ir::Gpr::Rax, size}});
    d.stmts.push_back({ref_rdi, ir::LoadReg{ir::Gpr::Rdi, ir::OpSize::I64}});
    const ir::Ref ref_addr = address_size_override ? emit_zero_extend_u32(d.stmts, ref_rdi, next_ref)
                                                   : ref_rdi;
    const ir::Ref ref_rhs = next_ref++;
    d.stmts.push_back({ref_rhs, ir::LoadMemTSO{ref_addr, size}});
    d.stmts.push_back({std::nullopt, ir::CmpFlags{ref_acc, ref_rhs, size}});
    const ir::Ref ref_stride = next_ref++;
    const ir::Ref ref_next_rdi = next_ref++;
    d.stmts.push_back({ref_stride, ir::Constant{byte_width(size), ir::OpSize::I64}});
    d.stmts.push_back(
        {ref_next_rdi, ir::BinOp{ir::BinOpKind::Add, ref_rdi, ref_stride, ir::OpSize::I64}});
    const ir::Ref ref_wrapped_rdi =
        address_size_override ? emit_zero_extend_u32(d.stmts, ref_next_rdi, next_ref) : ref_next_rdi;
    d.stmts.push_back({std::nullopt, ir::StoreReg{ir::Gpr::Rdi, ref_wrapped_rdi, ir::OpSize::I64}});
    d.bytes_consumed = bytes_consumed;
    return d;
}

// SETcc r/m8 (0F 90–9F /r).
// Encodes: dstbyte = cond ? 1 : 0.
std::variant<Decoded, DecodeError> decode_setcc_r8_rm8(
    ir::CondCode cc,
    std::span<const Byte> bytes,
    std::size_t& cursor,
    const RexPrefix& rex,
    std::uint64_t instruction_guest_pc,
    bool address_size_override,
    ir::Ref& next_ref) {
    auto modrm = parse_modrm(bytes, cursor, rex, address_size_override);
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
        const ir::Ref ref_addr = emit_address(d.stmts, m, next_ref, instruction_guest_pc + cursor);
        d.stmts.push_back(
            {std::nullopt, ir::StoreMemTSO{ref_addr, ref_selected, ir::OpSize::I8}});
    }

    d.bytes_consumed = cursor;
    return d;
}

// Parse ModR/M (and any SIB / disp8 / disp32 that follows) starting at
// `cursor`. On success, advances `cursor` past all consumed bytes and returns
// the parsed operand.
std::variant<ModRmOperand, DecodeError> parse_modrm(
    std::span<const Byte> bytes,
    std::size_t& cursor,
    const RexPrefix& rex,
    bool address_size_override) {
    auto modrm_byte = consume_le<1>(bytes, cursor);
    if (std::holds_alternative<DecodeError>(modrm_byte)) {
        return std::get<DecodeError>(modrm_byte);
    }
    const Byte modrm = static_cast<Byte>(std::get<std::uint64_t>(modrm_byte));
    const unsigned mod = (modrm >> 6) & 0x3u;
    const unsigned reg_lo = (modrm >> 3) & 0x7u;
    const unsigned rm_lo = modrm & 0x7u;

    ModRmOperand m;
    m.mod = mod;
    m.reg = reg_lo | (rex.r ? 0x8u : 0u);
    m.base = gpr_from_index(rm_lo | (rex.b ? 0x8u : 0u));
    m.has_base = false;
    m.has_index = false;
    m.index = ir::Gpr::Rax;
    m.scale_shift = 0;
    m.rip_relative = false;
    m.address_size_32 = address_size_override;
    m.disp = 0;

    if (m.mod == 0b11u) {
        // Register direct; no further bytes.
        return m;
    }

    bool needs_disp32 = false;
    if (rm_lo == 0b100u) {
        auto sib_byte = consume_le<1>(bytes, cursor);
        if (std::holds_alternative<DecodeError>(sib_byte)) {
            return std::get<DecodeError>(sib_byte);
        }
        const Byte sib = static_cast<Byte>(std::get<std::uint64_t>(sib_byte));
        m.scale_shift = (sib >> 6) & 0x3u;
        const unsigned index_lo = (sib >> 3) & 0x7u;
        const unsigned base_lo = sib & 0x7u;

        if (!(index_lo == 0b100u && !rex.x)) {
            m.has_index = true;
            m.index = gpr_from_index(index_lo | (rex.x ? 0x8u : 0u));
        }

        if (m.mod == 0b00u && base_lo == 0b101u && !rex.b) {
            needs_disp32 = true;
        } else {
            m.has_base = true;
            m.base = gpr_from_index(base_lo | (rex.b ? 0x8u : 0u));
        }
    } else if (m.mod == 0b00u && rm_lo == 0b101u && !rex.b) {
        if (address_size_override) {
            needs_disp32 = true;
        } else {
            m.rip_relative = true;
            needs_disp32 = true;
        }
    } else {
        m.has_base = true;
        m.base = gpr_from_index(rm_lo | (rex.b ? 0x8u : 0u));
    }

    if (m.mod == 0b00u) {
        if (!needs_disp32) return m;
    } else if (m.mod == 0b01u) {
        auto d = consume_le<1>(bytes, cursor);
        if (std::holds_alternative<DecodeError>(d)) {
            return std::get<DecodeError>(d);
        }
        m.disp = sign_extend_i32<1>(std::get<std::uint64_t>(d));
        return m;
    } else {
        needs_disp32 = true;
    }

    if (needs_disp32) {
        auto d = consume_le<4>(bytes, cursor);
        if (std::holds_alternative<DecodeError>(d)) {
            return std::get<DecodeError>(d);
        }
        m.disp = sign_extend_i32<4>(std::get<std::uint64_t>(d));
    }
    return m;
}

// Emit the IR that computes a memory effective address and returns the
// Ref holding the final 64-bit address. Assumes `op.mod != 11`.
ir::Ref emit_address(
    std::vector<ir::Stmt>& stmts,
    const ModRmOperand& op,
    ir::Ref& next_ref,
    std::uint64_t rip_after) {
    const auto finalize_addr = [&](ir::Ref ref) -> ir::Ref {
        if (!op.address_size_32) return ref;
        return emit_zero_extend_u32(stmts, ref, next_ref);
    };

    if (op.rip_relative) {
        const std::uint64_t abs =
            rip_after + static_cast<std::uint64_t>(static_cast<std::int64_t>(op.disp));
        const ir::Ref ref_abs = next_ref++;
        stmts.push_back({ref_abs, ir::Constant{abs, ir::OpSize::I64}});
        return finalize_addr(ref_abs);
    }

    std::optional<ir::Ref> ref_addr;
    if (op.has_base) {
        const ir::Ref ref_base = next_ref++;
        stmts.push_back({ref_base, ir::LoadReg{op.base, ir::OpSize::I64}});
        ref_addr = ref_base;
    }

    if (op.has_index) {
        const ir::Ref ref_index = next_ref++;
        stmts.push_back({ref_index, ir::LoadReg{op.index, ir::OpSize::I64}});
        ir::Ref ref_scaled = ref_index;
        if (op.scale_shift != 0u) {
            const ir::Ref ref_shift = next_ref++;
            ref_scaled = next_ref++;
            stmts.push_back({ref_shift, ir::Constant{op.scale_shift, ir::OpSize::I64}});
            stmts.push_back(
                {ref_scaled, ir::BinOp{ir::BinOpKind::Shl, ref_index, ref_shift, ir::OpSize::I64}});
        }
        if (ref_addr) {
            const ir::Ref ref_sum = next_ref++;
            stmts.push_back(
                {ref_sum, ir::BinOp{ir::BinOpKind::Add, *ref_addr, ref_scaled, ir::OpSize::I64}});
            ref_addr = ref_sum;
        } else {
            ref_addr = ref_scaled;
        }
    }

    if (!ref_addr) {
        const std::uint64_t disp_u64 =
            static_cast<std::uint64_t>(static_cast<std::int64_t>(op.disp));
        const ir::Ref ref_disp_only = next_ref++;
        stmts.push_back({ref_disp_only, ir::Constant{disp_u64, ir::OpSize::I64}});
        return finalize_addr(ref_disp_only);
    }

    if (op.disp != 0) {
        const ir::Ref ref_disp = next_ref++;
        const ir::Ref ref_with_disp = next_ref++;
        const std::uint64_t disp_u64 =
            static_cast<std::uint64_t>(static_cast<std::int64_t>(op.disp));
        stmts.push_back({ref_disp, ir::Constant{disp_u64, ir::OpSize::I64}});
        stmts.push_back(
            {ref_with_disp, ir::BinOp{ir::BinOpKind::Add, *ref_addr, ref_disp, ir::OpSize::I64}});
        return finalize_addr(ref_with_disp);
    }
    return finalize_addr(*ref_addr);
}

// Common shape for ALU register-register ops (mod=11 only for MVP).
// Memory destination forms of ADD/OR/AND/SUB/XOR/ADC/SBB (mod != 11) are future
// work — they require reading the memory operand, doing the op, and
// writing it back atomically. Out of scope for this session.
std::variant<Decoded, DecodeError> decode_alu_rm_r(
    std::span<const Byte> bytes,
    std::size_t& cursor_in_out,
    const RexPrefix& rex,
    bool address_size_override,
    std::uint64_t /*instruction_guest_pc*/,
    ir::BinOpKind op,
    ir::Ref& next_ref,
    ir::OpSize size) {
    auto modrm = parse_modrm(bytes, cursor_in_out, rex, address_size_override);
    if (std::holds_alternative<DecodeError>(modrm)) {
        return std::get<DecodeError>(modrm);
    }
    const auto& m = std::get<ModRmOperand>(modrm);
    if (m.mod != 0b11u) return DecodeError::UnsupportedEncoding;

    Decoded d;
    const ir::Ref ref_dst = next_ref++;
    const ir::Ref ref_src = next_ref++;
    const ir::Ref ref_res = next_ref++;
    d.stmts.push_back({ref_dst, ir::LoadReg{m.base, size}});
    d.stmts.push_back({ref_src, ir::LoadReg{gpr_from_index(m.reg), size}});
    d.stmts.push_back({ref_res, ir::BinOp{op, ref_dst, ref_src, size}});
    d.stmts.push_back({std::nullopt,
                       ir::StoreReg{m.base, ref_res, size}});
    d.bytes_consumed = cursor_in_out;
    return d;
}

// MOV r/m<size>, r<size>  (0x89 /r) — stores REG into R/M.
// In register direct (mod=11): just StoreReg(rm, LoadReg(reg)).
// In memory form: StoreMemTSO(addr(rm), LoadReg(reg)).
std::variant<Decoded, DecodeError> decode_mov_rm_r(
    std::span<const Byte> bytes,
    std::size_t& cursor_in_out,
    const RexPrefix& rex,
    std::uint64_t instruction_guest_pc,
    bool address_size_override,
    ir::Ref& next_ref,
    ir::OpSize size) {
    auto modrm = parse_modrm(bytes, cursor_in_out, rex, address_size_override);
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
        const ir::Ref ref_addr =
            emit_address(d.stmts, m, next_ref, instruction_guest_pc + cursor_in_out);
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
    const RexPrefix& rex,
    std::uint64_t instruction_guest_pc,
    bool address_size_override,
    ir::Ref& next_ref,
    ir::OpSize size) {
    auto modrm = parse_modrm(bytes, cursor_in_out, rex, address_size_override);
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
        const ir::Ref ref_addr =
            emit_address(d.stmts, m, next_ref, instruction_guest_pc + cursor_in_out);
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
    const RexPrefix& rex,
    bool address_size_override,
    ir::Ref& next_ref,
    ir::OpSize size) {
    auto modrm = parse_modrm(bytes, cursor_in_out, rex, address_size_override);
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
    d.stmts.push_back({ref_lhs, ir::LoadReg{m.base, size}});
    d.stmts.push_back({ref_rhs, ir::LoadReg{gpr_from_index(m.reg), size}});
    d.stmts.push_back({ref_tmp, ir::BinOp{ir::BinOpKind::And, ref_lhs, ref_rhs, size}});
    d.stmts.push_back({ref_zero, ir::Constant{0u, ir::OpSize::I64}});
    d.stmts.push_back({std::nullopt, ir::CmpFlags{ref_tmp, ref_zero, size}});
    d.bytes_consumed = cursor_in_out;
    return d;
}

std::variant<Decoded, DecodeError> decode_incdec_from_rm(
    const ModRmOperand& m,
    std::size_t bytes_consumed,
    ir::Ref& next_ref,
    ir::BinOpKind op,
    ir::OpSize size) {
    if (m.mod != 0b11u) return DecodeError::UnsupportedEncoding;

    Decoded d;
    const ir::Ref ref_src = next_ref++;
    const ir::Ref ref_delta = next_ref++;
    const ir::Ref ref_dst = next_ref++;
    d.stmts.push_back({ref_src, ir::LoadReg{m.base, size}});
    d.stmts.push_back({ref_delta, ir::Constant{1u, ir::OpSize::I64}});
    d.stmts.push_back({ref_dst, ir::BinOp{op, ref_src, ref_delta, size}});
    d.stmts.push_back({std::nullopt, ir::StoreReg{m.base, ref_dst, size}});
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
    ir::BinOpKind op,
    ir::OpSize size) {
    if (m.mod != 0b11u) return DecodeError::UnsupportedEncoding;

    Decoded d;
    const ir::Ref ref_src = next_ref++;
    const ir::Ref ref_rhs = next_ref++;
    const ir::Ref ref_dst = next_ref++;

    if (op == ir::BinOpKind::Xor) {
        d.stmts.push_back({ref_src, ir::LoadReg{m.base, size}});
        d.stmts.push_back({ref_rhs, ir::Constant{ir::mask_to_size(~0ULL, size), size}});
        d.stmts.push_back({ref_dst, ir::BinOp{op, ref_src, ref_rhs, size}});
    } else {
        d.stmts.push_back({ref_src, ir::Constant{0u, size}});
        d.stmts.push_back({ref_rhs, ir::LoadReg{m.base, size}});
        d.stmts.push_back({ref_dst, ir::BinOp{op, ref_src, ref_rhs, size}});
    }

    d.stmts.push_back({std::nullopt, ir::StoreReg{m.base, ref_dst, size}});
    d.bytes_consumed = bytes_consumed;
    return d;
}

// F2-BK-007 — DIV/IDIV r/m64. Treats the dividend as 64-bit (RAX
// only; RDX is assumed zero / sign-extended). The full 128-bit
// RDX:RAX dividend support is deferred — most compiler-generated code
// zero-extends or sign-extends RDX before DIV, so this covers the
// common case correctly. Divide-by-zero produces ARM64 UDIV/SDIV's
// well-defined behaviour (zero quotient) rather than the x86 #DE
// trap; the trap path is a follow-up.
std::variant<Decoded, DecodeError> decode_div_from_rm(
    const ModRmOperand& m,
    std::size_t bytes_consumed,
    ir::Ref& next_ref,
    ir::OpSize size,
    bool is_signed) {
    if (m.mod != 0b11u) return DecodeError::UnsupportedEncoding;

    Decoded d;
    const ir::Ref ref_dividend  = next_ref++;
    const ir::Ref ref_divisor   = next_ref++;
    const ir::Ref ref_quotient  = next_ref++;
    const ir::Ref ref_remainder = next_ref++;

    d.stmts.push_back({ref_dividend, ir::LoadReg{ir::Gpr::Rax, size}});
    d.stmts.push_back({ref_divisor,  ir::LoadReg{m.base,       size}});
    d.stmts.push_back({ref_quotient,
        ir::BinOp{is_signed ? ir::BinOpKind::SDiv : ir::BinOpKind::UDiv,
                  ref_dividend, ref_divisor, size}});
    d.stmts.push_back({ref_remainder,
        ir::BinOp{is_signed ? ir::BinOpKind::SMod : ir::BinOpKind::UMod,
                  ref_dividend, ref_divisor, size}});
    d.stmts.push_back({std::nullopt, ir::StoreReg{ir::Gpr::Rax, ref_quotient,  size}});
    d.stmts.push_back({std::nullopt, ir::StoreReg{ir::Gpr::Rdx, ref_remainder, size}});

    d.bytes_consumed = bytes_consumed;
    return d;
}

// F2-BK-007 — MUL/IMUL r/m64. Computes the full 128-bit product as
// two 64-bit halves: low → RAX (BinOp::Mul), high → RDX (UMulHi for
// MUL or SMulHi for IMUL). ARM64 MUL is sign-agnostic on the low
// half, so both forms share the BinOp::Mul kind.
std::variant<Decoded, DecodeError> decode_mul_imul_from_rm(
    const ModRmOperand& m,
    std::size_t bytes_consumed,
    ir::Ref& next_ref,
    ir::BinOpKind op,
    ir::OpSize size,
    bool is_signed = false) {
    if (m.mod != 0b11u) return DecodeError::UnsupportedEncoding;

    Decoded d;
    const ir::Ref ref_rax = next_ref++;
    const ir::Ref ref_rhs = next_ref++;
    const ir::Ref ref_lo  = next_ref++;
    const ir::Ref ref_hi  = next_ref++;
    const ir::BinOpKind hi_kind =
        is_signed ? ir::BinOpKind::SMulHi : ir::BinOpKind::UMulHi;

    d.stmts.push_back({ref_rax, ir::LoadReg{ir::Gpr::Rax, size}});
    d.stmts.push_back({ref_rhs, ir::LoadReg{m.base,       size}});
    d.stmts.push_back({ref_lo,  ir::BinOp{op,      ref_rax, ref_rhs, size}});
    d.stmts.push_back({ref_hi,  ir::BinOp{hi_kind, ref_rax, ref_rhs, size}});
    d.stmts.push_back({std::nullopt, ir::StoreReg{ir::Gpr::Rax, ref_lo, size}});
    d.stmts.push_back({std::nullopt, ir::StoreReg{ir::Gpr::Rdx, ref_hi, size}});

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
    const RexPrefix& rex,
    bool address_size_override,
    std::uint64_t /*instruction_guest_pc*/,
    ir::Ref& next_ref,
    ir::OpSize size) {
    auto modrm = parse_modrm(bytes, cursor, rex, address_size_override);
    if (std::holds_alternative<DecodeError>(modrm)) {
        return std::get<DecodeError>(modrm);
    }
    const auto& m = std::get<ModRmOperand>(modrm);
    if (m.mod != 0b11u) return DecodeError::UnsupportedEncoding;

    Decoded d;
    const ir::Ref ref_dst_src = next_ref++;
    const ir::Ref ref_rhs = next_ref++;
    const ir::Ref ref_product = next_ref++;
    d.stmts.push_back({ref_dst_src, ir::LoadReg{gpr_from_index(m.reg), size}});
    d.stmts.push_back({ref_rhs, ir::LoadReg{m.base, size}});
    d.stmts.push_back({ref_product, ir::BinOp{ir::BinOpKind::Mul, ref_dst_src, ref_rhs, size}});
    d.stmts.push_back({std::nullopt, ir::StoreReg{gpr_from_index(m.reg), ref_product, size}});
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
    const RexPrefix& rex,
    bool address_size_override,
    std::uint64_t /*instruction_guest_pc*/,
    ir::Ref& next_ref,
    ir::OpSize size) {
    auto modrm = parse_modrm(bytes, cursor, rex, address_size_override);
    if (std::holds_alternative<DecodeError>(modrm)) {
        return std::get<DecodeError>(modrm);
    }
    const auto& m = std::get<ModRmOperand>(modrm);
    if (m.mod != 0b11u) return DecodeError::UnsupportedEncoding;

    std::uint64_t imm_u64 = 0;
    if (size == ir::OpSize::I16) {
        auto imm = consume_le<2>(bytes, cursor);
        if (std::holds_alternative<DecodeError>(imm)) {
            return std::get<DecodeError>(imm);
        }
        const std::int32_t imm_i16 = sign_extend_i32<2>(std::get<std::uint64_t>(imm));
        imm_u64 = static_cast<std::uint64_t>(static_cast<std::int64_t>(imm_i16));
    } else {
        auto imm = consume_le<4>(bytes, cursor);
        if (std::holds_alternative<DecodeError>(imm)) {
            return std::get<DecodeError>(imm);
        }
        const std::int32_t imm_i32 = sign_extend_i32<4>(std::get<std::uint64_t>(imm));
        imm_u64 = static_cast<std::uint64_t>(static_cast<std::int64_t>(imm_i32));
    }

    Decoded d;
    const ir::Ref ref_rhs = next_ref++;
    const ir::Ref ref_imm = next_ref++;
    const ir::Ref ref_product = next_ref++;
    d.stmts.push_back({ref_rhs, ir::LoadReg{m.base, size}});
    d.stmts.push_back({ref_imm, ir::Constant{ir::mask_to_size(imm_u64, size), size}});
    d.stmts.push_back({ref_product, ir::BinOp{ir::BinOpKind::Mul, ref_rhs, ref_imm, size}});
    d.stmts.push_back({std::nullopt, ir::StoreReg{gpr_from_index(m.reg), ref_product, size}});
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
    const RexPrefix& rex,
    bool address_size_override,
    std::uint64_t /*instruction_guest_pc*/,
    ir::Ref& next_ref,
    ir::OpSize size) {
    auto modrm = parse_modrm(bytes, cursor, rex, address_size_override);
    if (std::holds_alternative<DecodeError>(modrm)) {
        return std::get<DecodeError>(modrm);
    }
    const auto& m = std::get<ModRmOperand>(modrm);
    if (m.mod != 0b11u) return DecodeError::UnsupportedEncoding;

    auto imm = consume_le<1>(bytes, cursor);
    if (std::holds_alternative<DecodeError>(imm)) {
        return std::get<DecodeError>(imm);
    }
    const std::int32_t imm_i8 = sign_extend_i32<1>(std::get<std::uint64_t>(imm));
    const std::uint64_t imm_u64 = static_cast<std::uint64_t>(static_cast<std::int64_t>(imm_i8));

    Decoded d;
    const ir::Ref ref_rhs = next_ref++;
    const ir::Ref ref_imm = next_ref++;
    const ir::Ref ref_product = next_ref++;
    d.stmts.push_back({ref_rhs, ir::LoadReg{m.base, size}});
    d.stmts.push_back({ref_imm, ir::Constant{ir::mask_to_size(imm_u64, size), size}});
    d.stmts.push_back({ref_product, ir::BinOp{ir::BinOpKind::Mul, ref_rhs, ref_imm, size}});
    d.stmts.push_back({std::nullopt, ir::StoreReg{gpr_from_index(m.reg), ref_product, size}});
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

    auto imm = consume_le<1>(bytes, cursor);
    if (std::holds_alternative<DecodeError>(imm)) {
        return std::get<DecodeError>(imm);
    }
    const std::uint64_t imm_u64 = std::get<std::uint64_t>(imm);

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

// RDTSC / RDTSCP placeholder.
//
// There is no time-source or serializing side-effect op in the IR yet, so
// for now we model both forms as zeroing the architecturally-written outputs:
//   * RDTSC  -> EDX:EAX = 0
//   * RDTSCP -> EDX:EAX = 0, ECX = 0
std::variant<Decoded, DecodeError> decode_rdtsc_placeholder(
    std::size_t bytes_consumed,
    ir::Ref& next_ref,
    bool with_aux) {
    Decoded d;
    const ir::Ref ref_zero = next_ref++;
    d.stmts.push_back({ref_zero, ir::Constant{0u, ir::OpSize::I32}});
    d.stmts.push_back({std::nullopt, ir::StoreReg{ir::Gpr::Rax, ref_zero, ir::OpSize::I32}});
    d.stmts.push_back({std::nullopt, ir::StoreReg{ir::Gpr::Rdx, ref_zero, ir::OpSize::I32}});
    if (with_aux) {
        d.stmts.push_back({std::nullopt, ir::StoreReg{ir::Gpr::Rcx, ref_zero, ir::OpSize::I32}});
    }
    d.bytes_consumed = bytes_consumed;
    return d;
}

std::variant<Decoded, DecodeError> decode_cpuid_placeholder(
    std::size_t bytes_consumed) {
    Decoded d;
    d.stmts.push_back({std::nullopt, ir::Cpuid{}});
    d.bytes_consumed = bytes_consumed;
    return d;
}

std::variant<Decoded, DecodeError> decode_syscall_placeholder(
    std::size_t bytes_consumed) {
    Decoded d;
    d.stmts.push_back({std::nullopt, ir::Syscall{}});
    d.bytes_consumed = bytes_consumed;
    return d;
}

std::variant<Decoded, DecodeError> decode_int3_sigtrap(
    std::size_t bytes_consumed) {
    Decoded d;
    d.stmts.push_back({std::nullopt, ir::Trap{ir::TrapKind::Sigtrap}});
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
    auto imm = consume_le<1>(bytes, cursor);
    if (std::holds_alternative<DecodeError>(imm)) {
        return std::get<DecodeError>(imm);
    }
    const std::int32_t imm_i8 = sign_extend_i32<1>(std::get<std::uint64_t>(imm));
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
    auto imm = consume_le<4>(bytes, cursor);
    if (std::holds_alternative<DecodeError>(imm)) {
        return std::get<DecodeError>(imm);
    }
    const std::int32_t imm_i32 = sign_extend_i32<4>(std::get<std::uint64_t>(imm));
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
// This decodes the full currently-supported addressing subset:
//   * destination = reg field
//   * source = effective address of r/m operand
//   * base+disp, SIB, RIP-relative, and extended-register fields via REX
// Register-direct forms remain unsupported because LEA requires memory syntax.
std::variant<Decoded, DecodeError> decode_lea_r64_mem(
    std::span<const Byte> bytes,
    std::size_t& cursor,
    const RexPrefix& rex,
    std::uint64_t instruction_guest_pc,
    bool address_size_override,
    ir::Ref& next_ref) {
    auto modrm = parse_modrm(bytes, cursor, rex, address_size_override);
    if (std::holds_alternative<DecodeError>(modrm)) {
        return std::get<DecodeError>(modrm);
    }
    const auto& m = std::get<ModRmOperand>(modrm);
    if (m.mod == 0b11u) return DecodeError::UnsupportedEncoding;

    Decoded d;
    const ir::Ref ref_addr = emit_address(d.stmts, m, next_ref, instruction_guest_pc + cursor);
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
    ir::BinOpKind op,
    ir::OpSize size) {
    if (m.mod != 0b11u) return DecodeError::UnsupportedEncoding;

    auto imm = consume_le<1>(bytes, cursor);
    if (std::holds_alternative<DecodeError>(imm)) {
        return std::get<DecodeError>(imm);
    }

    Decoded d;
    const ir::Ref ref_dst = next_ref++;
    const ir::Ref ref_imm = next_ref++;
    const ir::Ref ref_res = next_ref++;
    d.stmts.push_back({ref_dst, ir::LoadReg{m.base, size}});
    d.stmts.push_back({ref_imm, ir::Constant{std::get<std::uint64_t>(imm), ir::OpSize::I64}});
    d.stmts.push_back({ref_res, ir::BinOp{op, ref_dst, ref_imm, size}});
    d.stmts.push_back({std::nullopt, ir::StoreReg{m.base, ref_res, size}});
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
    ir::BinOpKind op,
    ir::OpSize size) {
    if (m.mod != 0b11u) return DecodeError::UnsupportedEncoding;

    Decoded d;
    const ir::Ref ref_dst = next_ref++;
    const ir::Ref ref_shift = next_ref++;
    const ir::Ref ref_res = next_ref++;
    d.stmts.push_back({ref_dst, ir::LoadReg{m.base, size}});
    d.stmts.push_back({ref_shift, ir::LoadReg{ir::Gpr::Rcx, ir::OpSize::I64}});
    d.stmts.push_back({ref_res, ir::BinOp{op, ref_dst, ref_shift, size}});
    d.stmts.push_back({std::nullopt, ir::StoreReg{m.base, ref_res, size}});
    d.bytes_consumed = bytes_consumed;
    return d;
}

using DispatchDecodeResult = std::variant<Decoded, DecodeError>;

std::optional<ir::OpSize> checked_operand_size(
    const RexPrefix& rex,
    bool has_operand_size_override) noexcept {
    const ir::OpSize size = operand_size_from_prefixes(rex, has_operand_size_override);
    if (size == ir::OpSize::I64 && has_operand_size_override) {
        return std::nullopt;
    }
    return size;
}

std::optional<ir::BinOpKind> shift_group_binop(unsigned reg) noexcept {
    switch (reg) {
        case 0u: return ir::BinOpKind::Rol;
        case 1u: return ir::BinOpKind::Ror;
        case 2u: return ir::BinOpKind::Rcl;
        case 3u: return ir::BinOpKind::Rcr;
        case 4u: return ir::BinOpKind::Shl;
        case 5u: return ir::BinOpKind::Shr;
        case 7u: return ir::BinOpKind::Sar;
        default: return std::nullopt;
    }
}

std::optional<BtSubOpcode> bt_subopcode_from_reg(unsigned reg) noexcept {
    switch (reg) {
        case 4u: return BtSubOpcode::Bt;
        case 5u: return BtSubOpcode::Bts;
        case 6u: return BtSubOpcode::Btr;
        case 7u: return BtSubOpcode::Btc;
        default: return std::nullopt;
    }
}

DispatchDecodeResult decode_shift_group_imm8_dispatch(
    std::span<const Byte> bytes,
    std::size_t& cursor,
    const RexPrefix& rex,
    bool has_operand_size_override,
    bool has_address_size_override,
    ir::Ref& next_ref) {
    const auto size = checked_operand_size(rex, has_operand_size_override);
    if (!size) return DecodeError::UnsupportedEncoding;

    auto modrm = parse_modrm(bytes, cursor, rex, has_address_size_override);
    if (std::holds_alternative<DecodeError>(modrm)) {
        return std::get<DecodeError>(modrm);
    }
    const auto& m = std::get<ModRmOperand>(modrm);
    const auto op = shift_group_binop(m.reg);
    if (!op) return DecodeError::UnsupportedEncoding;
    return decode_shift_r64_rm_imm8_from_rm(bytes, m, cursor, next_ref, *op, *size);
}

DispatchDecodeResult decode_shift_group_cl_dispatch(
    std::span<const Byte> bytes,
    std::size_t& cursor,
    const RexPrefix& rex,
    bool has_operand_size_override,
    bool has_address_size_override,
    ir::Ref& next_ref) {
    const auto size = checked_operand_size(rex, has_operand_size_override);
    if (!size) return DecodeError::UnsupportedEncoding;

    auto modrm = parse_modrm(bytes, cursor, rex, has_address_size_override);
    if (std::holds_alternative<DecodeError>(modrm)) {
        return std::get<DecodeError>(modrm);
    }
    const auto& m = std::get<ModRmOperand>(modrm);
    const auto op = shift_group_binop(m.reg);
    if (!op) return DecodeError::UnsupportedEncoding;
    return decode_shift_r64_rm_cl_from_rm(m, cursor, next_ref, *op, *size);
}

DispatchDecodeResult decode_group5_dispatch(
    std::span<const Byte> bytes,
    std::size_t& cursor,
    const RexPrefix& rex,
    bool has_operand_size_override,
    bool has_address_size_override,
    std::uint64_t instruction_guest_pc,
    ir::Ref& next_ref) {
    const auto size = checked_operand_size(rex, has_operand_size_override);
    if (!size) return DecodeError::UnsupportedEncoding;

    auto modrm = parse_modrm(bytes, cursor, rex, has_address_size_override);
    if (std::holds_alternative<DecodeError>(modrm)) {
        return std::get<DecodeError>(modrm);
    }
    const auto& m = std::get<ModRmOperand>(modrm);

    if (m.reg == 0u) {
        return decode_incdec_from_rm(m, cursor, next_ref, ir::BinOpKind::Add, *size);
    }
    if (m.reg == 1u) {
        return decode_incdec_from_rm(m, cursor, next_ref, ir::BinOpKind::Sub, *size);
    }
    if (*size != ir::OpSize::I64) return DecodeError::UnsupportedEncoding;

    if (m.reg == 4u || m.reg == 2u) {
        Decoded d;
        const ir::Ref ref_target = next_ref++;
        if (m.mod == 0b11u) {
            d.stmts.push_back({ref_target, ir::LoadReg{m.base, ir::OpSize::I64}});
        } else {
            const ir::Ref ref_addr =
                emit_address(d.stmts, m, next_ref, instruction_guest_pc + cursor);
            d.stmts.push_back({ref_target, ir::LoadMemTSO{ref_addr, ir::OpSize::I64}});
        }
        d.stmts.push_back({std::nullopt, ir::JumpReg{ref_target}});
        d.bytes_consumed = cursor;
        return d;
    }

    return DecodeError::UnsupportedEncoding;
}

DispatchDecodeResult decode_group3_dispatch(
    std::span<const Byte> bytes,
    std::size_t& cursor,
    const RexPrefix& rex,
    bool has_operand_size_override,
    bool has_address_size_override,
    ir::Ref& next_ref) {
    const auto size = checked_operand_size(rex, has_operand_size_override);
    if (!size) return DecodeError::UnsupportedEncoding;

    auto modrm = parse_modrm(bytes, cursor, rex, has_address_size_override);
    if (std::holds_alternative<DecodeError>(modrm)) {
        return std::get<DecodeError>(modrm);
    }
    const auto& m = std::get<ModRmOperand>(modrm);

    if (m.reg == 2u) {
        return decode_neg_not_from_rm(m, cursor, next_ref, ir::BinOpKind::Xor, *size);
    }
    if (m.reg == 3u) {
        return decode_neg_not_from_rm(m, cursor, next_ref, ir::BinOpKind::Sub, *size);
    }
    if (m.reg == 4u || m.reg == 5u) {
        // /4 = MUL (unsigned), /5 = IMUL (signed).
        return decode_mul_imul_from_rm(m, cursor, next_ref,
                                       ir::BinOpKind::Mul, *size,
                                       /*is_signed=*/m.reg == 5u);
    }
    if (m.reg == 6u || m.reg == 7u) {
        // /6 = DIV (unsigned), /7 = IDIV (signed).
        return decode_div_from_rm(m, cursor, next_ref, *size, /*is_signed=*/m.reg == 7u);
    }
    return DecodeError::UnsupportedEncoding;
}

DispatchDecodeResult decode_bt_group_imm8_dispatch(
    std::span<const Byte> bytes,
    std::size_t& cursor,
    const RexPrefix& rex,
    bool has_address_size_override,
    ir::Ref& next_ref) {
    auto modrm = parse_modrm(bytes, cursor, rex, has_address_size_override);
    if (std::holds_alternative<DecodeError>(modrm)) {
        return std::get<DecodeError>(modrm);
    }
    const auto& m = std::get<ModRmOperand>(modrm);
    if (m.mod != 0b11u) return DecodeError::UnsupportedEncoding;

    const auto op = bt_subopcode_from_reg(m.reg);
    if (!op) return DecodeError::UnsupportedEncoding;
    return decode_bt_r64_rm_imm8_from_rm(bytes, m, cursor, next_ref, *op);
}

[[maybe_unused]] std::optional<DispatchDecodeResult> try_decode_two_byte_dispatch(
    Byte subop,
    std::span<const Byte> bytes,
    std::size_t& cursor,
    const RexPrefix& rex,
    bool has_operand_size_override,
    bool has_address_size_override,
    bool has_f2,
    bool has_f3,
    bool has_lock,
    std::uint64_t instruction_guest_pc,
    ir::Ref& next_ref) {
    const auto entry = kTwoByteDispatchTable[subop];
    switch (entry.kind) {
        case TwoByteDispatchKind::None:
            return std::nullopt;
        case TwoByteDispatchKind::JccRel32: {
            if (rex.present) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            if (has_f3) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            if (has_operand_size_override) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};

            const auto cc = jcc_condition(subop);
            if (!cc) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            auto imm = consume_le<4>(bytes, cursor);
            if (std::holds_alternative<DecodeError>(imm)) {
                return DispatchDecodeResult{std::get<DecodeError>(imm)};
            }

            const std::int32_t rel = sign_extend_i32<4>(std::get<std::uint64_t>(imm));
            const std::uint64_t target =
                instruction_guest_pc + cursor + static_cast<std::uint64_t>(static_cast<std::int64_t>(rel));
            const std::uint64_t fallthrough = instruction_guest_pc + cursor;
            Decoded d;
            d.stmts.push_back({std::nullopt, ir::CondJumpRel{*cc, target, fallthrough}});
            d.bytes_consumed = cursor;
            return DispatchDecodeResult{std::move(d)};
        }
        case TwoByteDispatchKind::Cmovcc: {
            if (!rex.w) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            if (has_operand_size_override) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            if (has_f3) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            const auto cc = jcc_condition(subop);
            if (!cc) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            return DispatchDecodeResult{decode_cmovcc_r64_rm64(
                *cc, bytes, cursor, rex, instruction_guest_pc, has_address_size_override, next_ref)};
        }
        case TwoByteDispatchKind::Setcc: {
            if (rex.present) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            if (has_f3) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            if (has_operand_size_override) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            const auto cc = jcc_condition(subop);
            if (!cc) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            return DispatchDecodeResult{decode_setcc_r8_rm8(
                *cc, bytes, cursor, rex, instruction_guest_pc, has_address_size_override, next_ref)};
        }
        case TwoByteDispatchKind::Movsx8:
            if (!rex.w) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            if (has_operand_size_override) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            if (has_f3) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            return DispatchDecodeResult{decode_movx_r64_rm(
                bytes, cursor, rex, instruction_guest_pc, has_address_size_override, next_ref, ir::OpSize::I8, true)};
        case TwoByteDispatchKind::Movsx16:
            if (!rex.w) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            if (has_operand_size_override) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            if (has_f3) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            return DispatchDecodeResult{decode_movx_r64_rm(
                bytes, cursor, rex, instruction_guest_pc, has_address_size_override, next_ref, ir::OpSize::I16, true)};
        case TwoByteDispatchKind::Movzx8:
            if (!rex.w) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            if (has_operand_size_override) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            if (has_f3) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            return DispatchDecodeResult{decode_movx_r64_rm(
                bytes, cursor, rex, instruction_guest_pc, has_address_size_override, next_ref, ir::OpSize::I8, false)};
        case TwoByteDispatchKind::Movzx16:
            if (!rex.w) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            if (has_operand_size_override) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            if (has_f3) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            return DispatchDecodeResult{decode_movx_r64_rm(
                bytes, cursor, rex, instruction_guest_pc, has_address_size_override, next_ref, ir::OpSize::I16, false)};
        case TwoByteDispatchKind::ImulRm: {
            const auto size = checked_operand_size(rex, has_operand_size_override);
            if (!size) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            return DispatchDecodeResult{decode_imul_r64_r_rm(
                bytes, cursor, rex, has_address_size_override, instruction_guest_pc, next_ref, *size)};
        }
        case TwoByteDispatchKind::BtImmGroup:
            return DispatchDecodeResult{decode_bt_group_imm8_dispatch(
                bytes, cursor, rex, has_address_size_override, next_ref)};
        case TwoByteDispatchKind::Cmpxchg:
            if (!rex.w) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            if (has_operand_size_override) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            if ((has_f2 || has_f3) && !has_lock) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            return DispatchDecodeResult{decode_cmpxchg_r64_rm64(
                bytes, cursor, rex, instruction_guest_pc, has_address_size_override, next_ref)};
        case TwoByteDispatchKind::Xadd:
            if (!rex.w) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            if (has_operand_size_override) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            if ((has_f2 || has_f3) && !has_lock) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            return DispatchDecodeResult{decode_xadd_r64_rm64(
                bytes, cursor, rex, instruction_guest_pc, has_address_size_override, next_ref)};
        case TwoByteDispatchKind::Cmpxchg16b:
            if (!rex.w) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            if (has_operand_size_override) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            if ((has_f2 || has_f3) && !has_lock) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            return DispatchDecodeResult{decode_cmpxchg16b_m128(
                bytes, cursor, rex, instruction_guest_pc, has_address_size_override, next_ref)};
        case TwoByteDispatchKind::Popcnt: {
            if (!has_f3) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            if (!rex.w) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            if (has_operand_size_override) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};

            auto modrm = parse_modrm(bytes, cursor, rex, has_address_size_override);
            if (std::holds_alternative<DecodeError>(modrm)) {
                return DispatchDecodeResult{std::get<DecodeError>(modrm)};
            }
            const auto& m = std::get<ModRmOperand>(modrm);
            return DispatchDecodeResult{decode_popcnt_r64_r_rm(m, cursor, next_ref)};
        }
        case TwoByteDispatchKind::BitScanOrCount: {
            if (!rex.w) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            if (has_operand_size_override) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};

            auto modrm = parse_modrm(bytes, cursor, rex, has_address_size_override);
            if (std::holds_alternative<DecodeError>(modrm)) {
                return DispatchDecodeResult{std::get<DecodeError>(modrm)};
            }
            const auto& m = std::get<ModRmOperand>(modrm);
            if (has_f3) {
                return subop == 0xBDu
                           ? DispatchDecodeResult{decode_lzcnt_r64_r_rm(m, cursor, next_ref)}
                           : DispatchDecodeResult{decode_tzcnt_r64_r_rm(m, cursor, next_ref)};
            }
            return DispatchDecodeResult{decode_bsf_bsr_r64_r_rm(m, cursor, next_ref)};
        }
        case TwoByteDispatchKind::Rdtsc:
            if (rex.present || has_operand_size_override || has_address_size_override) {
                return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            }
            if (has_lock || has_f2 || has_f3) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            return DispatchDecodeResult{decode_rdtsc_placeholder(cursor, next_ref, false)};
        case TwoByteDispatchKind::Cpuid:
            if (rex.present || has_operand_size_override || has_address_size_override) {
                return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            }
            if (has_lock || has_f2 || has_f3) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            return DispatchDecodeResult{decode_cpuid_placeholder(cursor)};
        case TwoByteDispatchKind::Syscall:
            if (rex.present || has_operand_size_override || has_address_size_override) {
                return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            }
            if (has_lock || has_f2 || has_f3) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            return DispatchDecodeResult{decode_syscall_placeholder(cursor)};
        case TwoByteDispatchKind::Group01: {
            if (rex.present || has_operand_size_override || has_address_size_override) {
                return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            }
            if (has_lock || has_f2 || has_f3) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};

            auto third = consume_le<1>(bytes, cursor);
            if (std::holds_alternative<DecodeError>(third)) {
                return DispatchDecodeResult{std::get<DecodeError>(third)};
            }
            if (static_cast<Byte>(std::get<std::uint64_t>(third)) == 0xF9u) {
                return DispatchDecodeResult{decode_rdtsc_placeholder(cursor, next_ref, true)};
            }
            return DispatchDecodeResult{DecodeError::UnknownOpcode};
        }
    }

    return std::nullopt;
}

[[maybe_unused]] std::optional<DispatchDecodeResult> try_decode_one_byte_dispatch(
    Byte opcode,
    std::span<const Byte> bytes,
    std::size_t& cursor,
    const RexPrefix& rex,
    bool has_operand_size_override,
    bool has_address_size_override,
    bool has_f3,
    std::uint64_t instruction_guest_pc,
    ir::Ref& next_ref) {
    const auto entry = kOneByteDispatchTable[opcode];
    switch (entry.kind) {
        case OneByteDispatchKind::None:
            return std::nullopt;
        case OneByteDispatchKind::Pushfq:
            if (has_operand_size_override) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            if (has_f3) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            if (rex.present) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            return DispatchDecodeResult{decode_pushfq(cursor, next_ref)};
        case OneByteDispatchKind::Popfq:
            if (has_operand_size_override) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            if (has_f3) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            if (rex.present) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            return DispatchDecodeResult{decode_popfq(cursor, next_ref)};
        case OneByteDispatchKind::PushReg:
            if (has_operand_size_override) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            if (has_f3) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            return DispatchDecodeResult{
                decode_push_r64(gpr_from_index(opcode_reg_index(opcode, 0x50u, rex)), cursor, next_ref)};
        case OneByteDispatchKind::PopReg:
            if (has_operand_size_override) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            if (has_f3) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            return DispatchDecodeResult{
                decode_pop_r64(gpr_from_index(opcode_reg_index(opcode, 0x58u, rex)), cursor, next_ref)};
        case OneByteDispatchKind::PushImm8:
            if (has_operand_size_override) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            if (has_f3) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            return DispatchDecodeResult{decode_push_imm8_r64(bytes, cursor, next_ref)};
        case OneByteDispatchKind::PushImm32:
            if (has_operand_size_override) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            if (has_f3) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            return DispatchDecodeResult{decode_push_imm32_r64(bytes, cursor, next_ref)};
        case OneByteDispatchKind::ImulImm8: {
            const auto size = checked_operand_size(rex, has_operand_size_override);
            if (!size) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            return DispatchDecodeResult{decode_imul_r64_rm_imm8(
                bytes, cursor, rex, has_address_size_override, instruction_guest_pc, next_ref, *size)};
        }
        case OneByteDispatchKind::ImulImm32: {
            const auto size = checked_operand_size(rex, has_operand_size_override);
            if (!size) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            return DispatchDecodeResult{decode_imul_r64_rm_imm32(
                bytes, cursor, rex, has_address_size_override, instruction_guest_pc, next_ref, *size)};
        }
        case OneByteDispatchKind::ShiftImm8:
            return DispatchDecodeResult{decode_shift_group_imm8_dispatch(
                bytes, cursor, rex, has_operand_size_override, has_address_size_override, next_ref)};
        case OneByteDispatchKind::ShiftCl:
            return DispatchDecodeResult{decode_shift_group_cl_dispatch(
                bytes, cursor, rex, has_operand_size_override, has_address_size_override, next_ref)};
        case OneByteDispatchKind::AluRmR: {
            const auto size = checked_operand_size(rex, has_operand_size_override);
            if (!size) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            return DispatchDecodeResult{decode_alu_rm_r(
                bytes, cursor, rex, has_address_size_override, instruction_guest_pc, entry.binop, next_ref, *size)};
        }
        case OneByteDispatchKind::TestRmR: {
            const auto size = checked_operand_size(rex, has_operand_size_override);
            if (!size) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            return DispatchDecodeResult{decode_test_rm_r(
                bytes, cursor, rex, has_address_size_override, next_ref, *size)};
        }
        case OneByteDispatchKind::XchgRmR: {
            if (has_f3) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            const auto size = checked_operand_size(rex, has_operand_size_override);
            if (!size) return DispatchDecodeResult{DecodeError::UnsupportedEncoding};
            return DispatchDecodeResult{decode_xchg_r64_rm64(
                bytes, cursor, rex, instruction_guest_pc, has_address_size_override, next_ref, *size)};
        }
        case OneByteDispatchKind::Group5:
            return DispatchDecodeResult{decode_group5_dispatch(
                bytes, cursor, rex, has_operand_size_override, has_address_size_override, instruction_guest_pc, next_ref)};
        case OneByteDispatchKind::Group3:
            return DispatchDecodeResult{decode_group3_dispatch(
                bytes, cursor, rex, has_operand_size_override, has_address_size_override, next_ref)};
    }

    return std::nullopt;
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
    //   * 0x26 / 0x2E / 0x36 / 0x3E legacy segment overrides as ignored no-ops
    //     in long mode,
    //   * 0x66 (operand-size override),
    //   * 0x67 (address-size override to 32-bit addressing in long mode),
    //   * 0xF0 (LOCK) for selected atomic exchange-family opcodes,
    //   * 0xF2 / 0xF3 as HLE hints (XACQUIRE / XRELEASE) when paired with LOCK
    //     on selected atomic exchange-family opcodes,
    //   * 0xF3 (LZCNT/TZCNT group prefix),
    //   * one REX prefix.
    // More prefix combinations can be added later.
    bool has_operand_size_override = false;
    bool has_address_size_override = false;
    bool has_f2 = false;
    bool has_f3 = false;
    bool has_lock = false;
    RexPrefix rex;
    bool seen_rex = false;
    bool seen_66 = false;
    bool seen_67 = false;
    bool seen_f2 = false;
    bool seen_f3 = false;
    bool seen_lock = false;
    bool seen_segment_override = false;
    VexPrefix vex;
    while (cursor < bytes.size()) {
        if (cursor >= kMaxInstructionBytes) {
            return DecodeError::UnsupportedEncoding;
        }
        const Byte b = bytes[cursor];
        if (b == 0x26u || b == 0x2Eu || b == 0x36u || b == 0x3Eu) {
            if (seen_segment_override) return DecodeError::UnsupportedEncoding;
            seen_segment_override = true;
            ++cursor;
            continue;
        }

        if (b == 0x64u || b == 0x65u) {
            return DecodeError::UnsupportedEncoding;
        }

        if (b == 0x66u) {
            if (seen_66) return DecodeError::UnsupportedEncoding;
            has_operand_size_override = true;
            seen_66 = true;
            ++cursor;
            continue;
        }

        if (b == 0x67u) {
            if (seen_67) return DecodeError::UnsupportedEncoding;
            has_address_size_override = true;
            seen_67 = true;
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

        // F2-IR-048 — VEX prefix (C4 / C5). Must follow legacy prefixes;
        // appearing before a VEX prefix means the legacy ones are
        // illegal. Per Intel: VEX is incompatible with REX, LOCK, F0,
        // F2, F3, 66 prefixes appearing in the same instruction.
        if (b == 0xC5u || b == 0xC4u) {
            if (seen_rex || seen_66 || seen_f2 || seen_f3 || seen_lock) {
                return DecodeError::UnsupportedEncoding;
            }
            // Need at least one more byte for the VEX payload.
            if (b == 0xC5u) {
                if (cursor + 1 >= bytes.size()) return DecodeError::TruncatedInput;
                const Byte v1 = bytes[cursor + 1];
                vex.present = true;
                rex.present = true;
                rex.r = (v1 & 0x80u) == 0;            // R̅ (inverted)
                vex.vvvv = static_cast<std::uint8_t>(((~v1) >> 3) & 0x0Fu);
                vex.L    = (v1 & 0x04u) != 0;
                vex.pp   = static_cast<std::uint8_t>(v1 & 0x03u);
                vex.mmmmm = 1;                         // implied 0F
                cursor += 2;
            } else {
                if (cursor + 2 >= bytes.size()) return DecodeError::TruncatedInput;
                const Byte v1 = bytes[cursor + 1];
                const Byte v2 = bytes[cursor + 2];
                vex.present = true;
                rex.present = true;
                rex.r = (v1 & 0x80u) == 0;
                rex.x = (v1 & 0x40u) == 0;
                rex.b = (v1 & 0x20u) == 0;
                vex.mmmmm = static_cast<std::uint8_t>(v1 & 0x1Fu);
                rex.w = (v2 & 0x80u) != 0;
                vex.vvvv = static_cast<std::uint8_t>(((~v2) >> 3) & 0x0Fu);
                vex.L    = (v2 & 0x04u) != 0;
                vex.pp   = static_cast<std::uint8_t>(v2 & 0x03u);
                cursor += 3;
            }
            if (vex.mmmmm == 0 || vex.mmmmm > 3) return DecodeError::UnsupportedEncoding;
            // F2-IR-005 — AVX-256 allowlist. VEX.L=1 (256-bit YMM) is
            // accepted only for opcodes whose handlers know how to emit
            // the low+high-lane IR pair. Each subsequent commit grows
            // this list as a new opcode batch is wired up. Scalar FP
            // ops with L=1 are #UD per Intel SDM regardless.
            if (vex.L) {
                if (cursor >= bytes.size()) return DecodeError::TruncatedInput;
                const Byte avx256_op = bytes[cursor];
                const bool packed_fp_ps_pd =
                    vex.mmmmm == 1 && (vex.pp == 0 || vex.pp == 1) &&
                    (avx256_op == 0x58u || avx256_op == 0x59u ||
                     avx256_op == 0x5Cu || avx256_op == 0x5Eu ||
                     avx256_op == 0x5Du || avx256_op == 0x5Fu ||
                     avx256_op == 0x51u);
                // Bitwise float-domain logical ops (VANDPS/PD, VORPS/PD,
                // VXORPS/PD). 0F 54/56/57 with pp=00/01.
                const bool fp_bitwise =
                    vex.mmmmm == 1 && (vex.pp == 0 || vex.pp == 1) &&
                    (avx256_op == 0x54u || avx256_op == 0x56u ||
                     avx256_op == 0x57u);
                // Integer SIMD add/sub/bitwise (mmmmm=1, pp=01 → 66 0F).
                // PADDB/W/D/Q (FC/FD/FE/D4), PSUBB/W/D/Q (F8/F9/FA/FB),
                // PAND (DB), POR (EB), PXOR (EF).
                const bool int_simd_addsub_bitwise =
                    vex.mmmmm == 1 && vex.pp == 1 &&
                    (avx256_op == 0xFCu || avx256_op == 0xFDu ||
                     avx256_op == 0xFEu || avx256_op == 0xD4u ||
                     avx256_op == 0xF8u || avx256_op == 0xF9u ||
                     avx256_op == 0xFAu || avx256_op == 0xFBu ||
                     avx256_op == 0xDBu || avx256_op == 0xEBu ||
                     avx256_op == 0xEFu);
                // PCMPEQ B/W/D (74/75/76) + PCMPGT B/W/D (64/65/66),
                // mmmmm=1 pp=01.
                const bool int_cmp =
                    vex.mmmmm == 1 && vex.pp == 1 &&
                    (avx256_op == 0x74u || avx256_op == 0x75u ||
                     avx256_op == 0x76u || avx256_op == 0x64u ||
                     avx256_op == 0x65u || avx256_op == 0x66u);
                // UNPCKL/H FP: 0F 14/15, pp=00 (PS) or pp=01 (PD).
                const bool fp_unpck =
                    vex.mmmmm == 1 && (vex.pp == 0 || vex.pp == 1) &&
                    (avx256_op == 0x14u || avx256_op == 0x15u);
                // PUNPCKL/H BW/WD/DQ/QDQ: 66 0F 60/61/62/6C / 68/69/6A/6D.
                const bool int_unpck =
                    vex.mmmmm == 1 && vex.pp == 1 &&
                    (avx256_op == 0x60u || avx256_op == 0x61u ||
                     avx256_op == 0x62u || avx256_op == 0x6Cu ||
                     avx256_op == 0x68u || avx256_op == 0x69u ||
                     avx256_op == 0x6Au || avx256_op == 0x6Du);
                // CMPxxPS/PD (packed): 0F C2 with pp=00 (PS) or pp=01
                // (PD). Scalar variants (pp=02/03) are #UD with L=1.
                const bool fp_cmp_packed =
                    vex.mmmmm == 1 && (vex.pp == 0 || vex.pp == 1) &&
                    avx256_op == 0xC2u;
                // SHUFPS/PD (0F C6, pp=00 or pp=01).
                const bool fp_shuf =
                    vex.mmmmm == 1 && (vex.pp == 0 || vex.pp == 1) &&
                    avx256_op == 0xC6u;
                // HADDPS/PD (0F 7C, pp=01 → PD or pp=03 → PS).
                const bool fp_hadd =
                    vex.mmmmm == 1 && (vex.pp == 1 || vex.pp == 3) &&
                    avx256_op == 0x7Cu;
                // VBROADCASTSS/SD/F128 (mmmmm=2, pp=01).
                const bool avx_broadcast =
                    vex.mmmmm == 2 && vex.pp == 1 &&
                    (avx256_op == 0x18u || avx256_op == 0x19u ||
                     avx256_op == 0x1Au);
                // VINSERTF128 / VEXTRACTF128 / VPERM2F128 (mmmmm=3, pp=01).
                const bool avx_lane_xfer =
                    vex.mmmmm == 3 && vex.pp == 1 &&
                    (avx256_op == 0x06u || avx256_op == 0x18u ||
                     avx256_op == 0x19u);
                // FMA3 packed (mmmmm=2 → 0F 38, pp=01 → 66 prefix).
                // High nibble in {9,A,B} = ordering 132/213/231;
                // even low nibble = packed (PS if W=0, PD if W=1).
                // Odd low nibble = scalar SS/SD (only L=0 valid).
                const std::uint8_t op_lo = avx256_op & 0x0Fu;
                const bool fma3_ymm =
                    vex.mmmmm == 2 && vex.pp == 1 &&
                    (avx256_op >= 0x98u && avx256_op <= 0xBFu) &&
                    ((avx256_op & 0xF0u) == 0x90u ||
                     (avx256_op & 0xF0u) == 0xA0u ||
                     (avx256_op & 0xF0u) == 0xB0u) &&
                    (op_lo == 0x8u || op_lo == 0xAu ||
                     op_lo == 0xCu || op_lo == 0xEu);
                // VFMADDSUB / VFMSUBADD ymm: opcodes X6/X7 with X
                // in {9,A,B}. Always packed (no scalar form).
                const bool fma3_addsub_ymm =
                    vex.mmmmm == 2 && vex.pp == 1 &&
                    (avx256_op == 0x96u || avx256_op == 0x97u ||
                     avx256_op == 0xA6u || avx256_op == 0xA7u ||
                     avx256_op == 0xB6u || avx256_op == 0xB7u);
                if (!(packed_fp_ps_pd || fp_bitwise ||
                      int_simd_addsub_bitwise ||
                      int_cmp || fp_unpck || int_unpck ||
                      fp_cmp_packed || fp_shuf || fp_hadd ||
                      avx_broadcast || avx_lane_xfer ||
                      fma3_ymm || fma3_addsub_ymm)) {
                    return DecodeError::UnsupportedEncoding;
                }
            }
            // Synthesise the legacy mandatory prefixes the SSE branches
            // gate on, so the existing dispatch can be reused.
            switch (vex.pp) {
                case 1: has_operand_size_override = true; break;
                case 2: has_f3 = true; break;
                case 3: has_f2 = true; break;
            }
            break;  // VEX consumes the prefix slot; opcode follows.
        }

        if (b == 0xF0u) {
            if (seen_lock) return DecodeError::UnsupportedEncoding;
            has_lock = true;
            seen_lock = true;
            ++cursor;
            continue;
        }

        if (b == 0xF2u) {
            if (seen_f2 || seen_f3) return DecodeError::UnsupportedEncoding;
            has_f2 = true;
            seen_f2 = true;
            ++cursor;
            continue;
        }

        if (b == 0xF3u) {
            if (seen_f2 || seen_f3) return DecodeError::UnsupportedEncoding;
            has_f3 = true;
            seen_f3 = true;
            ++cursor;
            continue;
        }

        break;
    }
    if (cursor >= bytes.size()) return DecodeError::TruncatedInput;
    if (cursor >= kMaxInstructionBytes) return DecodeError::UnsupportedEncoding;

    // 2. Opcode byte. With VEX-encoded instructions, the escape level is
    // encoded in vex.mmmmm rather than 0F / 0F 38 / 0F 3A bytes in the
    // stream — synthesise opcode = 0x0F to enter the existing two-byte
    // dispatch and (for mmmmm=2/3) the inner sub-dispatch will inject
    // the right `subop` byte.
    Byte opcode;
    if (vex.present) {
        opcode = 0x0Fu;
    } else {
        opcode = bytes[cursor];
        ++cursor;
    }

    // LOCK currently only makes sense for the selected two-byte atomic
    // exchange-family opcodes handled below.
    if (has_lock && opcode != 0x0Fu) {
        return DecodeError::UnsupportedEncoding;
    }

    // Outside 0F-prefixed atomics and string instructions, F2 is not a
    // meaningful prefix in the current MVP slice.
    if (has_f2 &&
        opcode != 0x0Fu &&
        opcode != 0xA4u &&
        opcode != 0xA5u &&
        opcode != 0xA6u &&
        opcode != 0xA7u &&
        opcode != 0xAEu &&
        opcode != 0xAFu &&
        opcode != 0xAAu &&
        opcode != 0xABu) {
        return DecodeError::UnsupportedEncoding;
    }

    // --- INT3 (0xCC) -------------------------------------------------------
    if (opcode == 0xCCu) {
        if (rex.present || has_operand_size_override || has_address_size_override) {
            return DecodeError::UnsupportedEncoding;
        }
        if (has_lock || has_f2 || has_f3) return DecodeError::UnsupportedEncoding;
        return decode_int3_sigtrap(cursor);
    }

    // --- NOP (0x90) --------------------------------------------------------
    if (opcode == 0x90u) {
        if (has_operand_size_override || has_lock || has_f2 || has_f3) {
            return DecodeError::UnsupportedEncoding;
        }
        if (rex.b) return DecodeError::UnsupportedEncoding;
        Decoded d;
        d.bytes_consumed = cursor;
        return d;
    }

    // --- HLT (0xF4) --------------------------------------------------------
    // Privileged instruction: reject in user-mode decoder.
    if (opcode == 0xF4u) {
        return DecodeError::UnsupportedEncoding;
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
        if (has_operand_size_override || has_address_size_override) return DecodeError::UnsupportedEncoding;
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
        if (has_operand_size_override || has_address_size_override) return DecodeError::UnsupportedEncoding;
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
        if (has_operand_size_override || has_address_size_override) return DecodeError::UnsupportedEncoding;
        if (rex.present) return DecodeError::UnsupportedEncoding;
        auto imm = consume_le<2>(bytes, cursor);
        if (std::holds_alternative<DecodeError>(imm)) {
            return std::get<DecodeError>(imm);
        }
        const std::uint64_t pop_bytes = std::get<std::uint64_t>(imm) + 8u;
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
        if (has_operand_size_override || has_address_size_override) return DecodeError::UnsupportedEncoding;
        auto imm = consume_le<1>(bytes, cursor);
        if (std::holds_alternative<DecodeError>(imm)) {
            return std::get<DecodeError>(imm);
        }
        const std::int32_t rel = sign_extend_i32<1>(std::get<std::uint64_t>(imm));
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
        if (has_operand_size_override || has_address_size_override) return DecodeError::UnsupportedEncoding;
        auto imm = consume_le<4>(bytes, cursor);
        if (std::holds_alternative<DecodeError>(imm)) {
            return std::get<DecodeError>(imm);
        }
        const std::int32_t rel = sign_extend_i32<4>(std::get<std::uint64_t>(imm));
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
        auto imm = consume_le<4>(bytes, cursor);
        if (std::holds_alternative<DecodeError>(imm)) {
            return std::get<DecodeError>(imm);
        }
        const std::int32_t rel = sign_extend_i32<4>(std::get<std::uint64_t>(imm));
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
        auto imm = consume_le<1>(bytes, cursor);
        if (std::holds_alternative<DecodeError>(imm)) {
            return std::get<DecodeError>(imm);
        }
        const std::int32_t rel = sign_extend_i32<1>(std::get<std::uint64_t>(imm));
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
        const ir::OpSize size = operand_size_from_prefixes(rex, has_operand_size_override);
        if (size == ir::OpSize::I64 && has_operand_size_override) {
            return DecodeError::UnsupportedEncoding;
        }
        auto modrm = parse_modrm(bytes, cursor, rex, has_address_size_override);
        if (std::holds_alternative<DecodeError>(modrm)) {
            return std::get<DecodeError>(modrm);
        }
        const auto& m = std::get<ModRmOperand>(modrm);
        if (m.mod != 0b11u) return DecodeError::UnsupportedEncoding;

        Decoded d;
        const ir::Ref ref_lhs = next_ref++;
        const ir::Ref ref_rhs = next_ref++;
        d.stmts.push_back({ref_lhs, ir::LoadReg{m.base, size}});
        d.stmts.push_back({ref_rhs, ir::LoadReg{gpr_from_index(m.reg), size}});
        d.stmts.push_back({std::nullopt, ir::CmpFlags{ref_lhs, ref_rhs, size}});
        d.bytes_consumed = cursor;
        return d;
    }

    // F1-DC-066: REP / REPE / REPNE on string ops decode to an
    // InlineAsm{full original bytes} placeholder. The runtime
    // interprets via a software stub until the proper IR-level
    // loop expansion lands. Any (F2 || F3) prefix on a string-op
    // opcode lands in the helper below.
    auto rep_string_inline_asm = [&](std::size_t consumed)
        -> std::variant<Decoded, DecodeError> {
        Decoded d;
        std::vector<std::uint8_t> raw(bytes.begin(),
                                      bytes.begin() + static_cast<std::ptrdiff_t>(consumed));
        d.stmts.push_back({std::nullopt, ir::InlineAsm{std::move(raw)}});
        d.bytes_consumed = consumed;
        return d;
    };

    // --- STOSB/STOSW/STOSD/STOSQ (AA / AB) ----------------------------------
    if (opcode == 0xAAu || opcode == 0xABu) {
        if (has_lock) return DecodeError::UnsupportedEncoding;
        if (has_f2 || has_f3) {
            // REP / REPNE STOS — InlineAsm placeholder.
            return rep_string_inline_asm(cursor);
        }
        const ir::OpSize size =
            opcode == 0xAAu ? ir::OpSize::I8
                            : (rex.w ? ir::OpSize::I64
                                     : (has_operand_size_override ? ir::OpSize::I16
                                                                  : ir::OpSize::I32));
        if (opcode == 0xAAu && (rex.present || has_operand_size_override)) {
            return DecodeError::UnsupportedEncoding;
        }
        if (opcode == 0xABu && rex.w && has_operand_size_override) {
            return DecodeError::UnsupportedEncoding;
        }
        return decode_stos(size, has_address_size_override, cursor, next_ref);
    }

    // --- MOVSB/MOVSW/MOVSD/MOVSQ (A4 / A5) ----------------------------------
    if (opcode == 0xA4u || opcode == 0xA5u) {
        if (has_lock) return DecodeError::UnsupportedEncoding;
        if (has_f2 || has_f3) return rep_string_inline_asm(cursor);
        const ir::OpSize size =
            opcode == 0xA4u ? ir::OpSize::I8
                            : (rex.w ? ir::OpSize::I64
                                     : (has_operand_size_override ? ir::OpSize::I16
                                                                  : ir::OpSize::I32));
        if (opcode == 0xA4u && (rex.present || has_operand_size_override)) {
            return DecodeError::UnsupportedEncoding;
        }
        if (opcode == 0xA5u && rex.w && has_operand_size_override) {
            return DecodeError::UnsupportedEncoding;
        }
        return decode_movs(size, has_address_size_override, cursor, next_ref);
    }

    // --- CMPSB/CMPSW/CMPSD/CMPSQ (A6 / A7) ----------------------------------
    if (opcode == 0xA6u || opcode == 0xA7u) {
        if (has_lock) return DecodeError::UnsupportedEncoding;
        if (has_f2 || has_f3) return rep_string_inline_asm(cursor);
        const ir::OpSize size =
            opcode == 0xA6u ? ir::OpSize::I8
                            : (rex.w ? ir::OpSize::I64
                                     : (has_operand_size_override ? ir::OpSize::I16
                                                                  : ir::OpSize::I32));
        if (opcode == 0xA6u && (rex.present || has_operand_size_override)) {
            return DecodeError::UnsupportedEncoding;
        }
        if (opcode == 0xA7u && rex.w && has_operand_size_override) {
            return DecodeError::UnsupportedEncoding;
        }
        return decode_cmps(size, has_address_size_override, cursor, next_ref);
    }

    // --- SCASB/SCASW/SCASD/SCASQ (AE / AF) ----------------------------------
    if (opcode == 0xAEu || opcode == 0xAFu) {
        if (has_lock) return DecodeError::UnsupportedEncoding;
        if (has_f2 || has_f3) return rep_string_inline_asm(cursor);
        const ir::OpSize size =
            opcode == 0xAEu ? ir::OpSize::I8
                            : (rex.w ? ir::OpSize::I64
                                     : (has_operand_size_override ? ir::OpSize::I16
                                                                  : ir::OpSize::I32));
        if (opcode == 0xAEu && (rex.present || has_operand_size_override)) {
            return DecodeError::UnsupportedEncoding;
        }
        if (opcode == 0xAFu && rex.w && has_operand_size_override) {
            return DecodeError::UnsupportedEncoding;
        }
        return decode_scas(size, has_address_size_override, cursor, next_ref);
    }

    // --- MOV r<size>, imm<size> (B8+rd) ------------------------------------
    //
    // REX.W = 1 => I64, imm64
    // REX.W = 0, no 0x66 => I32, imm32 (zero-extend)
    // REX.W = 0, 0x66 => I16, imm16
    if (opcode >= 0xB8u && opcode <= 0xBFu) {
        const unsigned reg_idx = opcode_reg_index(opcode, 0xB8u, rex);
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
            auto x = consume_le<2>(bytes, cursor);
            if (std::holds_alternative<DecodeError>(x)) {
                return std::get<DecodeError>(x);
            }
            imm = std::get<std::uint64_t>(x);
        } else if (imm_size_bytes == 4) {
            auto x = consume_le<4>(bytes, cursor);
            if (std::holds_alternative<DecodeError>(x)) {
                return std::get<DecodeError>(x);
            }
            imm = std::get<std::uint64_t>(x);
        } else {
            auto x = consume_le<8>(bytes, cursor);
            if (std::holds_alternative<DecodeError>(x)) {
                return std::get<DecodeError>(x);
            }
            imm = std::get<std::uint64_t>(x);
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
        return decode_mov_rm_r(bytes, cursor, rex, instruction_guest_pc, has_address_size_override, next_ref, size);
    }

    // --- MOV r<size>, r/m<size> (0x8A /r) --------------------------------
    if (opcode == 0x8Au) {
        if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
        return decode_mov_r_rm(bytes, cursor, rex, instruction_guest_pc, has_address_size_override, next_ref, ir::OpSize::I8);
    }

    // --- MOV r/m<size>, r<size> (0x88 /r) --------------------------------
    if (opcode == 0x88u) {
        if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
        return decode_mov_rm_r(bytes, cursor, rex, instruction_guest_pc, has_address_size_override, next_ref, ir::OpSize::I8);
    }

    // --- MOV r<size>, r/m<size> (0x8B /r) --------------------------------
    if (opcode == 0x8Bu) {
        const ir::OpSize size = rex.w ? ir::OpSize::I64
                                      : (has_operand_size_override ? ir::OpSize::I16
                                                                  : ir::OpSize::I32);
        if (size == ir::OpSize::I64 && has_operand_size_override) {
            return DecodeError::UnsupportedEncoding;
        }
        return decode_mov_r_rm(bytes, cursor, rex, instruction_guest_pc, has_address_size_override, next_ref, size);
    }

    // --- LEA r64, [mem] (48 8D /r) --------------------------------------
    if (opcode == 0x8Du) {
        if (!rex.w) return DecodeError::UnsupportedEncoding;
        if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
        if (has_f3) return DecodeError::UnsupportedEncoding;
        return decode_lea_r64_mem(bytes, cursor, rex, instruction_guest_pc, has_address_size_override, next_ref);
    }

    // --- MOV r<size>, imm<size> (B0+rd for I8, B8+rd for I16/I32/I64) ----
    if (opcode >= 0xB0u && opcode <= 0xB7u) {
        if (has_operand_size_override) return DecodeError::UnsupportedEncoding;

        const unsigned reg_idx = opcode_reg_index(opcode, 0xB0u, rex);
        auto imm = consume_le<1>(bytes, cursor);
        if (std::holds_alternative<DecodeError>(imm)) {
            return std::get<DecodeError>(imm);
        }

        Decoded d;
        const ir::Ref ref = next_ref++;
        d.stmts.push_back({ref, ir::Constant{std::get<std::uint64_t>(imm), ir::OpSize::I8}});
        d.stmts.push_back({std::nullopt,
                           ir::StoreReg{gpr_from_index(reg_idx), ref, ir::OpSize::I8}});
        d.bytes_consumed = cursor;
        return d;
    }

    // --- Two-byte opcodes (0x0F xx) --------------------------------------
    if (opcode == 0x0Fu) {
        Byte subop;
        if (vex.present && vex.mmmmm == 2) {
            subop = 0x38u;  // route into the 0F 38 escape sub-dispatch
        } else if (vex.present && vex.mmmmm == 3) {
            subop = 0x3Au;  // route into the 0F 3A escape sub-dispatch
        } else {
            auto sub = consume_le<1>(bytes, cursor);
            if (std::holds_alternative<DecodeError>(sub)) {
                return std::get<DecodeError>(sub);
            }
            subop = static_cast<Byte>(std::get<std::uint64_t>(sub));
        }

        // F2-IR-038: 0F 3A escape — SSSE3 / SSE4.1 imm-bearing ops.
        if (subop == 0x3Au && has_operand_size_override && !has_lock &&
            !has_f2 && !has_f3) {
            auto third = consume_le<1>(bytes, cursor);
            if (std::holds_alternative<DecodeError>(third)) {
                return std::get<DecodeError>(third);
            }
            const Byte sub3 = static_cast<Byte>(std::get<std::uint64_t>(third));

            // F2-IR-005 follow-up — AVX lane-crossing 128-bit moves
            // (66 0F 3A, VEX.L=1 only):
            //   sub3 = 0x18 — VINSERTF128 ymm1, ymm2, xmm3/m128, imm8
            //   sub3 = 0x19 — VEXTRACTF128 xmm1/m128, ymm2, imm8
            //   sub3 = 0x06 — VPERM2F128   ymm1, ymm2, ymm3/m128, imm8
            // imm8 bit 0 (insert/extract) selects which 128-bit half:
            // 0 = low, 1 = high. VEX.vvvv supplies the other half for
            // VINSERTF128 (the unmodified source). VEXTRACTF128 has no
            // vvvv (must be 0b1111).
            if (vex.present && vex.L &&
                (sub3 == 0x18u || sub3 == 0x19u)) {
                auto modrm = parse_modrm(bytes, cursor, rex,
                                         has_address_size_override);
                if (std::holds_alternative<DecodeError>(modrm)) {
                    return std::get<DecodeError>(modrm);
                }
                const auto& m = std::get<ModRmOperand>(modrm);
                auto imm = consume_le<1>(bytes, cursor);
                if (std::holds_alternative<DecodeError>(imm)) {
                    return std::get<DecodeError>(imm);
                }
                const std::uint8_t ctrl =
                    static_cast<std::uint8_t>(std::get<std::uint64_t>(imm)) & 0x01u;
                Decoded d;
                if (sub3 == 0x18u) {
                    // VINSERTF128 ymm1 (m.reg), ymm2 (vvvv), xmm3/m128 (m.r/m)
                    const std::uint8_t r_dst = static_cast<std::uint8_t>(m.reg);
                    const std::uint8_t r_v   = static_cast<std::uint8_t>(vex.vvvv);
                    // Source 128-bit value (xmm3 or memory).
                    ir::Ref r_src;
                    if (m.mod == 0b11) {
                        r_src = next_ref++;
                        d.stmts.push_back({r_src,
                            ir::LoadVecReg{static_cast<std::uint8_t>(
                                static_cast<unsigned>(m.base))}});
                    } else {
                        const ir::Ref r_addr = emit_address(d.stmts, m, next_ref,
                                                            instruction_guest_pc + cursor);
                        r_src = next_ref++;
                        d.stmts.push_back({r_src, ir::LoadVec{r_addr}});
                    }
                    if (ctrl == 0u) {
                        // Insert into low half. Low ← src, High ← vvvv.hi.
                        const ir::Ref r_v_hi = next_ref++;
                        d.stmts.push_back({r_v_hi, ir::LoadVecRegHi{r_v}});
                        d.stmts.push_back({std::nullopt,
                            ir::StoreVecReg{r_dst, r_src}});
                        d.stmts.push_back({std::nullopt,
                            ir::StoreVecRegHi{r_dst, r_v_hi}});
                    } else {
                        // Insert into high half. Low ← vvvv.lo, High ← src.
                        const ir::Ref r_v_lo = next_ref++;
                        d.stmts.push_back({r_v_lo, ir::LoadVecReg{r_v}});
                        d.stmts.push_back({std::nullopt,
                            ir::StoreVecReg{r_dst, r_v_lo}});
                        d.stmts.push_back({std::nullopt,
                            ir::StoreVecRegHi{r_dst, r_src}});
                    }
                } else {
                    // VEXTRACTF128 xmm1/m128 (m.r/m), ymm2 (m.reg), imm8.
                    // In the VEX-encoded extract, ModRM.reg is the *source*
                    // ymm and ModRM.r/m is the destination xmm/m128. vvvv
                    // is reserved (encoded 0b1111 → stored 0); we don't
                    // enforce since other VEX handlers don't either.
                    const std::uint8_t r_src = static_cast<std::uint8_t>(m.reg);
                    const ir::Ref r_half = next_ref++;
                    if (ctrl == 0u) {
                        d.stmts.push_back({r_half, ir::LoadVecReg{r_src}});
                    } else {
                        d.stmts.push_back({r_half, ir::LoadVecRegHi{r_src}});
                    }
                    if (m.mod == 0b11) {
                        // Destination is xmm — write low half, zero upper.
                        const std::uint8_t r_dst =
                            static_cast<std::uint8_t>(static_cast<unsigned>(m.base));
                        const ir::Ref r_zero = next_ref++;
                        d.stmts.push_back({r_zero,
                            ir::VecConstant{0ULL, 0ULL}});
                        d.stmts.push_back({std::nullopt,
                            ir::StoreVecReg{r_dst, r_half}});
                        d.stmts.push_back({std::nullopt,
                            ir::StoreVecRegHi{r_dst, r_zero}});
                    } else {
                        const ir::Ref r_addr = emit_address(d.stmts, m, next_ref,
                                                            instruction_guest_pc + cursor);
                        d.stmts.push_back({std::nullopt,
                            ir::StoreVec{r_addr, r_half}});
                    }
                }
                d.bytes_consumed = cursor;
                return d;
            }

            // VPERM2F128 ymm1, ymm2, ymm3/m128, imm8 (66 0F 3A 06 /r ib).
            // imm8 bits [1:0] select source for low half:
            //   0b00 = ymm2.lo, 0b01 = ymm2.hi, 0b10 = src.lo, 0b11 = src.hi
            // imm8 bits [5:4] same encoding for high half.
            // imm8 bit 3 → zero the low half; bit 7 → zero the high half.
            if (vex.present && vex.L && sub3 == 0x06u) {
                auto modrm = parse_modrm(bytes, cursor, rex,
                                         has_address_size_override);
                if (std::holds_alternative<DecodeError>(modrm)) {
                    return std::get<DecodeError>(modrm);
                }
                const auto& m = std::get<ModRmOperand>(modrm);
                auto imm = consume_le<1>(bytes, cursor);
                if (std::holds_alternative<DecodeError>(imm)) {
                    return std::get<DecodeError>(imm);
                }
                const std::uint8_t imm8 =
                    static_cast<std::uint8_t>(std::get<std::uint64_t>(imm));
                const std::uint8_t r_dst = static_cast<std::uint8_t>(m.reg);
                const std::uint8_t r_v   = static_cast<std::uint8_t>(vex.vvvv);
                Decoded d;
                // Materialise the four candidate 128-bit halves: vvvv.lo,
                // vvvv.hi, src.lo, src.hi. Some may end up dead and DCE
                // will clean them up later.
                const ir::Ref r_v_lo  = next_ref++;
                const ir::Ref r_v_hi  = next_ref++;
                const ir::Ref r_s_lo  = next_ref++;
                const ir::Ref r_s_hi  = next_ref++;
                d.stmts.push_back({r_v_lo, ir::LoadVecReg{r_v}});
                d.stmts.push_back({r_v_hi, ir::LoadVecRegHi{r_v}});
                if (m.mod == 0b11) {
                    const std::uint8_t r_rm =
                        static_cast<std::uint8_t>(static_cast<unsigned>(m.base));
                    d.stmts.push_back({r_s_lo, ir::LoadVecReg{r_rm}});
                    d.stmts.push_back({r_s_hi, ir::LoadVecRegHi{r_rm}});
                } else {
                    const ir::Ref r_addr = emit_address(d.stmts, m, next_ref,
                                                        instruction_guest_pc + cursor);
                    const ir::Ref r_off16 = next_ref++;
                    const ir::Ref r_addr_hi = next_ref++;
                    d.stmts.push_back({r_off16,
                        ir::Constant{16ULL, ir::OpSize::I64}});
                    d.stmts.push_back({r_addr_hi,
                        ir::BinOp{ir::BinOpKind::Add, r_addr, r_off16,
                                  ir::OpSize::I64}});
                    d.stmts.push_back({r_s_lo, ir::LoadVec{r_addr}});
                    d.stmts.push_back({r_s_hi, ir::LoadVec{r_addr_hi}});
                }
                const ir::Ref r_zero = next_ref++;
                d.stmts.push_back({r_zero, ir::VecConstant{0ULL, 0ULL}});
                auto pick = [&](unsigned sel, bool zero_bit) -> ir::Ref {
                    if (zero_bit) return r_zero;
                    switch (sel & 0x3u) {
                        case 0: return r_v_lo;
                        case 1: return r_v_hi;
                        case 2: return r_s_lo;
                        case 3: return r_s_hi;
                    }
                    return r_zero;
                };
                const ir::Ref out_lo = pick(imm8 & 0x03u,
                                            (imm8 & 0x08u) != 0);
                const ir::Ref out_hi = pick((imm8 >> 4) & 0x03u,
                                            (imm8 & 0x80u) != 0);
                d.stmts.push_back({std::nullopt,
                    ir::StoreVecReg{r_dst, out_lo}});
                d.stmts.push_back({std::nullopt,
                    ir::StoreVecRegHi{r_dst, out_hi}});
                d.bytes_consumed = cursor;
                return d;
            }

            // SSE4.1 ROUNDPS/PD/SS/SD (66 0F 3A 08/09/0A/0B /r ib).
            if (sub3 == 0x08u || sub3 == 0x09u ||
                sub3 == 0x0Au || sub3 == 0x0Bu) {
                auto modrm = parse_modrm(bytes, cursor, rex,
                                         has_address_size_override);
                if (std::holds_alternative<DecodeError>(modrm)) {
                    return std::get<DecodeError>(modrm);
                }
                const auto& m = std::get<ModRmOperand>(modrm);
                auto imm = consume_le<1>(bytes, cursor);
                if (std::holds_alternative<DecodeError>(imm)) {
                    return std::get<DecodeError>(imm);
                }
                const std::uint8_t mode = static_cast<std::uint8_t>(
                    std::get<std::uint64_t>(imm)) & 0x07u;
                const bool is_packed = (sub3 == 0x08u || sub3 == 0x09u);
                const ir::FpSize fp_sz =
                    (sub3 == 0x08u || sub3 == 0x0Au) ? ir::FpSize::F32
                                                      : ir::FpSize::F64;
                Decoded d;
                const ir::Ref r_lhs = next_ref++;
                const ir::Ref r_src = next_ref++;
                const ir::Ref r_res = next_ref++;
                d.stmts.push_back({r_lhs,
                    ir::LoadVecReg{static_cast<std::uint8_t>(m.reg)}});
                if (m.mod == 0b11) {
                    d.stmts.push_back({r_src,
                        ir::LoadVecReg{static_cast<std::uint8_t>(
                            static_cast<unsigned>(m.base))}});
                } else {
                    const ir::Ref r_addr = emit_address(d.stmts, m, next_ref,
                                                        instruction_guest_pc + cursor);
                    d.stmts.push_back({r_src, ir::LoadVec{r_addr}});
                }
                d.stmts.push_back({r_res,
                    ir::VecFpRound{r_lhs, r_src, fp_sz, mode, is_packed}});
                d.stmts.push_back({std::nullopt,
                    ir::StoreVecReg{static_cast<std::uint8_t>(m.reg), r_res}});
                d.bytes_consumed = cursor;
                return d;
            }

            // SSE4.1 PEXTRB/D/Q + PINSRB/D/Q (S4/D2 selected by REX.W on dword/qword forms).
            //   66 0F 3A 14 /r ib — PEXTRB B16 lane → r/m32
            //   66 0F 3A 16 /r ib — PEXTRD/Q (REX.W=0 → PEXTRD S4 lane → r/m32;
            //                                  REX.W=1 → PEXTRQ D2 lane → r/m64)
            //   66 0F 3A 20 /r ib — PINSRB
            //   66 0F 3A 22 /r ib — PINSRD/Q
            if (sub3 == 0x14u || sub3 == 0x16u ||
                sub3 == 0x20u || sub3 == 0x22u) {
                auto modrm = parse_modrm(bytes, cursor, rex,
                                         has_address_size_override);
                if (std::holds_alternative<DecodeError>(modrm)) {
                    return std::get<DecodeError>(modrm);
                }
                const auto& m = std::get<ModRmOperand>(modrm);
                if (m.mod != 0b11) return DecodeError::UnsupportedEncoding;
                auto imm = consume_le<1>(bytes, cursor);
                if (std::holds_alternative<DecodeError>(imm)) {
                    return std::get<DecodeError>(imm);
                }
                const std::uint8_t imm_byte = static_cast<std::uint8_t>(
                    std::get<std::uint64_t>(imm));
                ir::VecLane lane;
                std::uint8_t lane_idx;
                ir::OpSize gpr_sz;
                switch (sub3) {
                    case 0x14u: lane = ir::VecLane::B16; lane_idx = imm_byte & 0x0Fu;
                                gpr_sz = ir::OpSize::I32; break;
                    case 0x20u: lane = ir::VecLane::B16; lane_idx = imm_byte & 0x0Fu;
                                gpr_sz = ir::OpSize::I32; break;
                    case 0x16u:  // PEXTRD/Q
                    case 0x22u:  // PINSRD/Q
                        if (rex.w) {
                            lane = ir::VecLane::D2; lane_idx = imm_byte & 0x01u;
                            gpr_sz = ir::OpSize::I64;
                        } else {
                            lane = ir::VecLane::S4; lane_idx = imm_byte & 0x03u;
                            gpr_sz = ir::OpSize::I32;
                        }
                        break;
                    default: lane = ir::VecLane::B16; lane_idx = 0; gpr_sz = ir::OpSize::I32;
                }
                const bool is_extract = (sub3 == 0x14u || sub3 == 0x16u);
                Decoded d;
                if (is_extract) {
                    // GPR dest = m.reg, XMM source = m.base.
                    const ir::Gpr dst_gpr = static_cast<ir::Gpr>(m.reg);
                    const unsigned src_xmm = static_cast<unsigned>(m.base);
                    const ir::Ref r_xmm = next_ref++;
                    const ir::Ref r_val = next_ref++;
                    d.stmts.push_back({r_xmm,
                        ir::LoadVecReg{static_cast<std::uint8_t>(src_xmm)}});
                    d.stmts.push_back({r_val,
                        ir::VecExtractLaneU{r_xmm, lane_idx, lane}});
                    d.stmts.push_back({std::nullopt,
                        ir::StoreReg{dst_gpr, r_val, gpr_sz}});
                } else {
                    // PINSR*: xmm dest = m.reg, GPR source = m.base.
                    const ir::Ref r_lhs = next_ref++;
                    const ir::Ref r_val = next_ref++;
                    const ir::Ref r_res = next_ref++;
                    d.stmts.push_back({r_lhs,
                        ir::LoadVecReg{static_cast<std::uint8_t>(m.reg)}});
                    d.stmts.push_back({r_val,
                        ir::LoadReg{m.base, gpr_sz}});
                    d.stmts.push_back({r_res,
                        ir::VecInsertLane{r_lhs, r_val, lane_idx, lane}});
                    d.stmts.push_back({std::nullopt,
                        ir::StoreVecReg{static_cast<std::uint8_t>(m.reg), r_res}});
                }
                d.bytes_consumed = cursor;
                return d;
            }
            if (sub3 == 0x0Fu) {  // PALIGNR
                auto modrm = parse_modrm(bytes, cursor, rex,
                                         has_address_size_override);
                if (std::holds_alternative<DecodeError>(modrm)) {
                    return std::get<DecodeError>(modrm);
                }
                const auto& m = std::get<ModRmOperand>(modrm);
                auto imm = consume_le<1>(bytes, cursor);
                if (std::holds_alternative<DecodeError>(imm)) {
                    return std::get<DecodeError>(imm);
                }
                const std::uint8_t count =
                    static_cast<std::uint8_t>(std::get<std::uint64_t>(imm));
                Decoded d;
                const ir::Ref r_lhs = next_ref++;
                const ir::Ref r_rhs = next_ref++;
                const ir::Ref r_res = next_ref++;
                const std::uint8_t lhs_xmm_palignr = vex.present
                    ? static_cast<std::uint8_t>(vex.vvvv)
                    : static_cast<std::uint8_t>(m.reg);
                d.stmts.push_back({r_lhs, ir::LoadVecReg{lhs_xmm_palignr}});
                if (m.mod == 0b11) {
                    d.stmts.push_back({r_rhs,
                        ir::LoadVecReg{static_cast<std::uint8_t>(
                            static_cast<unsigned>(m.base))}});
                } else {
                    const ir::Ref r_addr = emit_address(d.stmts, m, next_ref,
                                                        instruction_guest_pc + cursor);
                    d.stmts.push_back({r_rhs, ir::LoadVec{r_addr}});
                }
                d.stmts.push_back({r_res, ir::VecAlignr{r_lhs, r_rhs, count}});
                d.stmts.push_back({std::nullopt,
                    ir::StoreVecReg{static_cast<std::uint8_t>(m.reg), r_res}});
                d.bytes_consumed = cursor;
                return d;
            }
            return DecodeError::UnsupportedEncoding;
        }

        // F2-IR-036: 0F 38 escape — SSSE3 / SSE4.1.
        if (subop == 0x38u && has_operand_size_override && !has_lock &&
            !has_f2 && !has_f3) {
            auto third = consume_le<1>(bytes, cursor);
            if (std::holds_alternative<DecodeError>(third)) {
                return std::get<DecodeError>(third);
            }
            const Byte sub3 = static_cast<Byte>(std::get<std::uint64_t>(third));
            // SSE4.1 PTEST (66 0F 38 17 /r). Sets NZCV via WriteFlagsPtest.
            if (sub3 == 0x17u) {
                auto modrm = parse_modrm(bytes, cursor, rex,
                                         has_address_size_override);
                if (std::holds_alternative<DecodeError>(modrm)) {
                    return std::get<DecodeError>(modrm);
                }
                const auto& m = std::get<ModRmOperand>(modrm);
                Decoded d;
                const ir::Ref r_lhs = next_ref++;
                const ir::Ref r_rhs = next_ref++;
                const ir::Ref r_flags = next_ref++;
                d.stmts.push_back({r_lhs,
                    ir::LoadVecReg{static_cast<std::uint8_t>(m.reg)}});
                if (m.mod == 0b11) {
                    d.stmts.push_back({r_rhs,
                        ir::LoadVecReg{static_cast<std::uint8_t>(
                            static_cast<unsigned>(m.base))}});
                } else {
                    const ir::Ref r_addr = emit_address(d.stmts, m, next_ref,
                                                        instruction_guest_pc + cursor);
                    d.stmts.push_back({r_rhs, ir::LoadVec{r_addr}});
                }
                d.stmts.push_back({r_flags,
                    ir::WriteFlagsPtest{r_lhs, r_rhs}});
                d.bytes_consumed = cursor;
                return d;
            }

            // F2-IR-005 follow-up — AVX broadcasts (66 0F 38 18/19/1A).
            //   VBROADCASTSS xmm/ymm, xmm/m32  (sub3 = 0x18)
            //   VBROADCASTSD ymm,    xmm/m64   (sub3 = 0x19, ymm-only)
            //   VBROADCASTF128 ymm,  m128       (sub3 = 0x1A, mem-only)
            // VEX-encoded only. Lower-half result is built as a 128-bit
            // broadcast Ref; for VEX.L=1 we replicate it to ymm_hi as
            // well (per-lane semantics by repetition; no cross-lane data
            // movement needed even though the x86 encoding is "lane-
            // crossing" relative to AVX-128).
            if (vex.present &&
                (sub3 == 0x18u || sub3 == 0x19u || sub3 == 0x1Au)) {
                // VBROADCASTSD (0x19) requires VEX.L=1.
                if (sub3 == 0x19u && !vex.L) return DecodeError::UnsupportedEncoding;
                // VBROADCASTF128 (0x1A) requires VEX.L=1 and mem operand.
                if (sub3 == 0x1Au && !vex.L) return DecodeError::UnsupportedEncoding;
                auto modrm = parse_modrm(bytes, cursor, rex,
                                         has_address_size_override);
                if (std::holds_alternative<DecodeError>(modrm)) {
                    return std::get<DecodeError>(modrm);
                }
                const auto& m = std::get<ModRmOperand>(modrm);
                if (sub3 == 0x1Au && m.mod == 0b11) {
                    return DecodeError::UnsupportedEncoding;
                }
                Decoded d;
                const std::uint8_t r_dst = static_cast<std::uint8_t>(m.reg);
                const ir::Ref r_lo = next_ref++;
                if (sub3 == 0x18u) {
                    // VBROADCASTSS — broadcast 32-bit lane 0 → all 4 lanes.
                    if (m.mod == 0b11) {
                        const ir::Ref r_src = next_ref++;
                        d.stmts.push_back({r_src,
                            ir::LoadVecReg{static_cast<std::uint8_t>(
                                static_cast<unsigned>(m.base))}});
                        d.stmts.push_back({r_lo,
                            ir::VecShuffle32x4{r_src, 0x00u}});
                    } else {
                        const ir::Ref r_addr = emit_address(d.stmts, m, next_ref,
                                                            instruction_guest_pc + cursor);
                        const ir::Ref r_val32 = next_ref++;
                        const ir::Ref r_lane = next_ref++;
                        d.stmts.push_back({r_val32,
                            ir::LoadMem{r_addr, ir::OpSize::I32}});
                        d.stmts.push_back({r_lane,
                            ir::XmmFromGpr{r_val32, ir::OpSize::I32}});
                        d.stmts.push_back({r_lo,
                            ir::VecShuffle32x4{r_lane, 0x00u}});
                    }
                } else if (sub3 == 0x19u) {
                    // VBROADCASTSD ymm — broadcast 64-bit lane 0 → both
                    // 64-bit lanes (UNPCKLPD self-pair semantics).
                    ir::Ref r_src;
                    if (m.mod == 0b11) {
                        r_src = next_ref++;
                        d.stmts.push_back({r_src,
                            ir::LoadVecReg{static_cast<std::uint8_t>(
                                static_cast<unsigned>(m.base))}});
                    } else {
                        const ir::Ref r_addr = emit_address(d.stmts, m, next_ref,
                                                            instruction_guest_pc + cursor);
                        const ir::Ref r_val64 = next_ref++;
                        r_src = next_ref++;
                        d.stmts.push_back({r_val64,
                            ir::LoadMem{r_addr, ir::OpSize::I64}});
                        d.stmts.push_back({r_src,
                            ir::XmmFromGpr{r_val64, ir::OpSize::I64}});
                    }
                    d.stmts.push_back({r_lo,
                        ir::VecUnpack{/*is_high=*/false, r_src, r_src,
                                      ir::VecLane::D2}});
                } else {
                    // VBROADCASTF128 — load full 128 bits, replicate to
                    // both ymm halves.
                    const ir::Ref r_addr = emit_address(d.stmts, m, next_ref,
                                                        instruction_guest_pc + cursor);
                    d.stmts.push_back({r_lo, ir::LoadVec{r_addr}});
                }
                d.stmts.push_back({std::nullopt,
                    ir::StoreVecReg{r_dst, r_lo}});
                if (vex.L) {
                    d.stmts.push_back({std::nullopt,
                        ir::StoreVecRegHi{r_dst, r_lo}});
                }
                d.bytes_consumed = cursor;
                return d;
            }

            // F2-IR-006 — FMA3 (66 0F 38, VEX-only).
            // High-nibble selects ordering: 9x=132, Ax=213, Bx=231.
            // Low-nibble parity selects packed-vs-scalar:
            //   even (8/A/C/E) → packed (PS if W=0, PD if W=1)
            //   odd  (9/B/D/F) → scalar (SS if W=0, SD if W=1)  — DEFERRED
            // Family is the *pair* of low-nibbles (ignoring scalar/packed):
            //   8/9 → VFMADD   (neg_addend=F, neg_mul=F)
            //   A/B → VFMSUB   (neg_addend=T, neg_mul=F)
            //   C/D → VFNMADD  (neg_addend=F, neg_mul=T)
            //   E/F → VFNMSUB  (neg_addend=T, neg_mul=T)
            // Scalar (odd low-nibble) needs upper-lane preservation and a
            // dedicated IR op; deferred. MADDSUB/MSUBADD (96/97/A6/A7/B6/B7)
            // also deferred — they need a per-lane sign-blend lowering.
            // F2-IR-006 — VFMADDSUB / VFMSUBADD packed (66 0F 38 X6/X7,
            // X in {9,A,B}). Per-lane alternation between ADD and SUB:
            //   VFMADDSUB: even lanes = SUB (b*c - a), odd lanes = ADD (a + b*c)
            //   VFMSUBADD: even lanes = ADD,           odd lanes = SUB
            // Lowered as two packed VecFpFma + a VecBlend with an
            // alternating mask. PS uses S4 lane granularity; PD uses D2.
            // No scalar form per Intel SDM.
            if (vex.present &&
                (sub3 == 0x96u || sub3 == 0x97u ||
                 sub3 == 0xA6u || sub3 == 0xA7u ||
                 sub3 == 0xB6u || sub3 == 0xB7u)) {
                auto modrm = parse_modrm(bytes, cursor, rex,
                                         has_address_size_override);
                if (std::holds_alternative<DecodeError>(modrm)) {
                    return std::get<DecodeError>(modrm);
                }
                const auto& m = std::get<ModRmOperand>(modrm);
                const std::uint8_t r_dst = static_cast<std::uint8_t>(m.reg);
                const std::uint8_t r_v   = static_cast<std::uint8_t>(vex.vvvv);
                const ir::VecFpSize size = rex.w ? ir::VecFpSize::D2
                                                 : ir::VecFpSize::S4;
                const bool is_msubadd = (sub3 & 0x01u) != 0u;  // 0x97/A7/B7
                const std::uint8_t hi = sub3 & 0xF0u;
                Decoded d;
                std::optional<ir::Ref> r_addr_lo;
                if (m.mod != 0b11) {
                    r_addr_lo = emit_address(d.stmts, m, next_ref,
                                             instruction_guest_pc + cursor);
                }
                auto emit_pair = [&](bool hi_half) -> ir::Ref {
                    const ir::Ref r_dst_load = next_ref++;
                    const ir::Ref r_v_load   = next_ref++;
                    const ir::Ref r_rm_load  = next_ref++;
                    if (hi_half) {
                        d.stmts.push_back({r_dst_load, ir::LoadVecRegHi{r_dst}});
                        d.stmts.push_back({r_v_load,   ir::LoadVecRegHi{r_v}});
                        if (m.mod == 0b11) {
                            d.stmts.push_back({r_rm_load,
                                ir::LoadVecRegHi{static_cast<std::uint8_t>(
                                    static_cast<unsigned>(m.base))}});
                        } else {
                            const ir::Ref r_off16 = next_ref++;
                            const ir::Ref r_addr_hi = next_ref++;
                            d.stmts.push_back({r_off16,
                                ir::Constant{16ULL, ir::OpSize::I64}});
                            d.stmts.push_back({r_addr_hi,
                                ir::BinOp{ir::BinOpKind::Add,
                                          *r_addr_lo, r_off16, ir::OpSize::I64}});
                            d.stmts.push_back({r_rm_load,
                                ir::LoadVec{r_addr_hi}});
                        }
                    } else {
                        d.stmts.push_back({r_dst_load, ir::LoadVecReg{r_dst}});
                        d.stmts.push_back({r_v_load,   ir::LoadVecReg{r_v}});
                        if (m.mod == 0b11) {
                            d.stmts.push_back({r_rm_load,
                                ir::LoadVecReg{static_cast<std::uint8_t>(
                                    static_cast<unsigned>(m.base))}});
                        } else {
                            d.stmts.push_back({r_rm_load,
                                ir::LoadVec{*r_addr_lo}});
                        }
                    }
                    ir::Ref ra, rb, rc;
                    switch (hi) {
                        case 0x90u:  // 132
                            ra = r_v_load; rb = r_dst_load; rc = r_rm_load; break;
                        case 0xA0u:  // 213
                            ra = r_rm_load; rb = r_v_load; rc = r_dst_load; break;
                        case 0xB0u:  // 231
                            ra = r_dst_load; rb = r_v_load; rc = r_rm_load; break;
                        default: ra = rb = rc = 0u; break;
                    }
                    const ir::Ref r_add = next_ref++;
                    const ir::Ref r_sub = next_ref++;
                    d.stmts.push_back({r_add,
                        ir::VecFpFma{ra, rb, rc, /*neg_addend=*/false,
                                     /*neg_mul=*/false, size}});
                    d.stmts.push_back({r_sub,
                        ir::VecFpFma{ra, rb, rc, /*neg_addend=*/true,
                                     /*neg_mul=*/false, size}});
                    // Mask: alternating lanes. For VFMADDSUB (is_msubadd=F),
                    // mask MSB is set on ODD lanes (so VecBlend picks
                    // src=V_add at odd, dst=V_sub at even).
                    // For VFMSUBADD: mask MSB set on EVEN lanes.
                    std::uint64_t mask_lo = 0, mask_hi = 0;
                    if (size == ir::VecFpSize::S4) {
                        // 32-bit lanes: 4 lanes per half.
                        if (is_msubadd) {
                            // even-set: lanes 0, 2 → MSB=1; lanes 1, 3 → MSB=0
                            mask_lo = 0x00000000FFFFFFFFULL;
                            mask_hi = 0x00000000FFFFFFFFULL;
                        } else {
                            // odd-set: lanes 1, 3 → MSB=1; lanes 0, 2 → MSB=0
                            mask_lo = 0xFFFFFFFF00000000ULL;
                            mask_hi = 0xFFFFFFFF00000000ULL;
                        }
                    } else {
                        // 64-bit lanes: 2 lanes per half.
                        if (is_msubadd) {
                            // lane 0 → MSB=1, lane 1 → MSB=0
                            mask_lo = 0xFFFFFFFFFFFFFFFFULL;
                            mask_hi = 0x0000000000000000ULL;
                        } else {
                            // lane 0 → MSB=0, lane 1 → MSB=1
                            mask_lo = 0x0000000000000000ULL;
                            mask_hi = 0xFFFFFFFFFFFFFFFFULL;
                        }
                    }
                    const ir::Ref r_mask = next_ref++;
                    d.stmts.push_back({r_mask,
                        ir::VecConstant{mask_lo, mask_hi}});
                    const ir::Ref r_res = next_ref++;
                    // VecBlend: dst (mask MSB == 0) + src (mask MSB == 1).
                    // Lane granularity = the FP size (S4 for PS, D2 for PD).
                    const ir::VecLane blend_lane =
                        size == ir::VecFpSize::S4 ? ir::VecLane::S4
                                                  : ir::VecLane::D2;
                    d.stmts.push_back({r_res,
                        ir::VecBlend{r_sub, r_add, r_mask, blend_lane}});
                    return r_res;
                };
                const ir::Ref r_lo = emit_pair(false);
                d.stmts.push_back({std::nullopt,
                    ir::StoreVecReg{r_dst, r_lo}});
                if (vex.L) {
                    const ir::Ref r_hi = emit_pair(true);
                    d.stmts.push_back({std::nullopt,
                        ir::StoreVecRegHi{r_dst, r_hi}});
                }
                d.bytes_consumed = cursor;
                return d;
            }

            const std::uint8_t fma_hi = sub3 & 0xF0u;
            const std::uint8_t fma_lo = sub3 & 0x0Fu;
            const bool fma_packed_even =
                (fma_lo == 0x8u || fma_lo == 0xAu ||
                 fma_lo == 0xCu || fma_lo == 0xEu);
            const bool fma_scalar_odd =
                (fma_lo == 0x9u || fma_lo == 0xBu ||
                 fma_lo == 0xDu || fma_lo == 0xFu);
            const bool fma_high_ok =
                (fma_hi == 0x90u || fma_hi == 0xA0u || fma_hi == 0xB0u);
            // Scalar FMA: VEX.L=1 is illegal per Intel SDM (#UD).
            if (vex.present && fma_scalar_odd && fma_high_ok && !vex.L) {
                auto modrm = parse_modrm(bytes, cursor, rex,
                                         has_address_size_override);
                if (std::holds_alternative<DecodeError>(modrm)) {
                    return std::get<DecodeError>(modrm);
                }
                const auto& m = std::get<ModRmOperand>(modrm);
                const std::uint8_t r_dst = static_cast<std::uint8_t>(m.reg);
                const std::uint8_t r_v   = static_cast<std::uint8_t>(vex.vvvv);
                const ir::FpSize size = rex.w ? ir::FpSize::F64 : ir::FpSize::F32;
                const std::uint8_t family_lo = (fma_lo & 0xEu);
                bool neg_addend = false, neg_mul = false;
                switch (family_lo) {
                    case 0x8u: neg_addend = false; neg_mul = false; break;
                    case 0xAu: neg_addend = true;  neg_mul = false; break;
                    case 0xCu: neg_addend = false; neg_mul = true;  break;
                    case 0xEu: neg_addend = true;  neg_mul = true;  break;
                    default: return DecodeError::UnsupportedEncoding;
                }
                Decoded d;
                std::optional<ir::Ref> r_addr_lo;
                if (m.mod != 0b11) {
                    r_addr_lo = emit_address(d.stmts, m, next_ref,
                                             instruction_guest_pc + cursor);
                }
                const ir::Ref r_dst_load = next_ref++;
                const ir::Ref r_v_load   = next_ref++;
                const ir::Ref r_rm_load  = next_ref++;
                d.stmts.push_back({r_dst_load, ir::LoadVecReg{r_dst}});
                d.stmts.push_back({r_v_load,   ir::LoadVecReg{r_v}});
                if (m.mod == 0b11) {
                    d.stmts.push_back({r_rm_load,
                        ir::LoadVecReg{static_cast<std::uint8_t>(
                            static_cast<unsigned>(m.base))}});
                } else {
                    d.stmts.push_back({r_rm_load, ir::LoadVec{*r_addr_lo}});
                }
                ir::Ref ra, rb, rc;
                switch (fma_hi) {
                    case 0x90u:  // 132: b=r_dst, c=r_rm, a=r_v
                        ra = r_v_load; rb = r_dst_load; rc = r_rm_load; break;
                    case 0xA0u:  // 213: b=r_v, c=r_dst, a=r_rm
                        ra = r_rm_load; rb = r_v_load; rc = r_dst_load; break;
                    case 0xB0u:  // 231: b=r_v, c=r_rm, a=r_dst
                        ra = r_dst_load; rb = r_v_load; rc = r_rm_load; break;
                    default: return DecodeError::UnsupportedEncoding;
                }
                const ir::Ref r_res = next_ref++;
                d.stmts.push_back({r_res,
                    ir::VecFpScalarFma{ra, rb, rc, /*scalar_upper=*/r_dst_load,
                                       neg_addend, neg_mul, size}});
                d.stmts.push_back({std::nullopt,
                    ir::StoreVecReg{r_dst, r_res}});
                d.bytes_consumed = cursor;
                return d;
            }
            if (vex.present && fma_packed_even && fma_high_ok) {
                auto modrm = parse_modrm(bytes, cursor, rex,
                                         has_address_size_override);
                if (std::holds_alternative<DecodeError>(modrm)) {
                    return std::get<DecodeError>(modrm);
                }
                const auto& m = std::get<ModRmOperand>(modrm);
                const std::uint8_t r_dst = static_cast<std::uint8_t>(m.reg);
                const std::uint8_t r_v   = static_cast<std::uint8_t>(vex.vvvv);
                // VEX.W picks PS (W=0) vs PD (W=1) for the packed forms.
                const ir::VecFpSize size = rex.w ? ir::VecFpSize::D2
                                                 : ir::VecFpSize::S4;
                const std::uint8_t high_nibble = fma_hi;
                // family — keyed by the low-nibble pair (ignoring scalar
                // bit, which we already constrained to even above).
                const std::uint8_t family_lo = (fma_lo & 0xEu);
                bool neg_addend = false;
                bool neg_mul    = false;
                switch (family_lo) {
                    case 0x8u: /* MADD  */ neg_addend = false; neg_mul = false; break;
                    case 0xAu: /* MSUB  */ neg_addend = true;  neg_mul = false; break;
                    case 0xCu: /* NMADD */ neg_addend = false; neg_mul = true;  break;
                    case 0xEu: /* NMSUB */ neg_addend = true;  neg_mul = true;  break;
                    default: return DecodeError::UnsupportedEncoding;
                }
                Decoded d;
                std::optional<ir::Ref> r_addr_lo;
                if (m.mod != 0b11) {
                    r_addr_lo = emit_address(d.stmts, m, next_ref,
                                             instruction_guest_pc + cursor);
                }
                // Materialise the three operands into IR refs. The r/m
                // operand may be reg or memory; vvvv and reg are always
                // xmm registers.
                const ir::Ref r_a = next_ref++;
                const ir::Ref r_b = next_ref++;
                const ir::Ref r_c = next_ref++;
                // Loads: we need r_dst (always xmm), r_v (always xmm),
                // and r_rm (xmm or memory). Map them onto a/b/c per the
                // 132/213/231 permutation.
                ir::Ref r_dst_load = next_ref++;
                ir::Ref r_v_load   = next_ref++;
                ir::Ref r_rm_load  = next_ref++;
                d.stmts.push_back({r_dst_load, ir::LoadVecReg{r_dst}});
                d.stmts.push_back({r_v_load,   ir::LoadVecReg{r_v}});
                if (m.mod == 0b11) {
                    d.stmts.push_back({r_rm_load,
                        ir::LoadVecReg{static_cast<std::uint8_t>(
                            static_cast<unsigned>(m.base))}});
                } else {
                    d.stmts.push_back({r_rm_load, ir::LoadVec{*r_addr_lo}});
                }
                ir::Ref ra, rb, rc;
                switch (high_nibble) {
                    case 0x90u:  // 132: b=r_dst, c=r_rm, a=r_v
                        ra = r_v_load; rb = r_dst_load; rc = r_rm_load; break;
                    case 0xA0u:  // 213: b=r_v, c=r_dst, a=r_rm
                        ra = r_rm_load; rb = r_v_load; rc = r_dst_load; break;
                    case 0xB0u:  // 231: b=r_v, c=r_rm, a=r_dst
                        ra = r_dst_load; rb = r_v_load; rc = r_rm_load; break;
                    default: return DecodeError::UnsupportedEncoding;
                }
                (void)r_a; (void)r_b; (void)r_c;
                const ir::Ref r_res = next_ref++;
                d.stmts.push_back({r_res,
                    ir::VecFpFma{ra, rb, rc, neg_addend, neg_mul, size}});
                d.stmts.push_back({std::nullopt,
                    ir::StoreVecReg{r_dst, r_res}});
                // F2-IR-006 ymm extension — emit the high-lane pair.
                if (vex.L) {
                    const ir::Ref r_dst_hi = next_ref++;
                    const ir::Ref r_v_hi   = next_ref++;
                    const ir::Ref r_rm_hi  = next_ref++;
                    d.stmts.push_back({r_dst_hi, ir::LoadVecRegHi{r_dst}});
                    d.stmts.push_back({r_v_hi,   ir::LoadVecRegHi{r_v}});
                    if (m.mod == 0b11) {
                        d.stmts.push_back({r_rm_hi,
                            ir::LoadVecRegHi{static_cast<std::uint8_t>(
                                static_cast<unsigned>(m.base))}});
                    } else {
                        const ir::Ref r_off16 = next_ref++;
                        const ir::Ref r_addr_hi = next_ref++;
                        d.stmts.push_back({r_off16,
                            ir::Constant{16ULL, ir::OpSize::I64}});
                        d.stmts.push_back({r_addr_hi,
                            ir::BinOp{ir::BinOpKind::Add, *r_addr_lo, r_off16,
                                      ir::OpSize::I64}});
                        d.stmts.push_back({r_rm_hi, ir::LoadVec{r_addr_hi}});
                    }
                    ir::Ref ra_h, rb_h, rc_h;
                    switch (high_nibble) {
                        case 0x90u:
                            ra_h = r_v_hi; rb_h = r_dst_hi; rc_h = r_rm_hi; break;
                        case 0xA0u:
                            ra_h = r_rm_hi; rb_h = r_v_hi; rc_h = r_dst_hi; break;
                        case 0xB0u:
                            ra_h = r_dst_hi; rb_h = r_v_hi; rc_h = r_rm_hi; break;
                        default: return DecodeError::UnsupportedEncoding;
                    }
                    const ir::Ref r_res_hi = next_ref++;
                    d.stmts.push_back({r_res_hi,
                        ir::VecFpFma{ra_h, rb_h, rc_h, neg_addend, neg_mul, size}});
                    d.stmts.push_back({std::nullopt,
                        ir::StoreVecRegHi{r_dst, r_res_hi}});
                }
                d.bytes_consumed = cursor;
                return d;
            }

            // SSE4.1 BLEND* with implicit XMM0 mask:
            //   66 0F 38 10 — PBLENDVB  (B16 lane)
            //   66 0F 38 14 — BLENDVPS (S4 lane)
            //   66 0F 38 15 — BLENDVPD (D2 lane)
            if (sub3 == 0x10u || sub3 == 0x14u || sub3 == 0x15u) {
                auto modrm = parse_modrm(bytes, cursor, rex,
                                         has_address_size_override);
                if (std::holds_alternative<DecodeError>(modrm)) {
                    return std::get<DecodeError>(modrm);
                }
                const auto& m = std::get<ModRmOperand>(modrm);
                const ir::VecLane lane =
                    sub3 == 0x10u ? ir::VecLane::B16 :
                    sub3 == 0x14u ? ir::VecLane::S4  :
                                    ir::VecLane::D2;
                Decoded d;
                const ir::Ref r_dst  = next_ref++;
                const ir::Ref r_src  = next_ref++;
                const ir::Ref r_mask = next_ref++;
                const ir::Ref r_res  = next_ref++;
                d.stmts.push_back({r_dst,
                    ir::LoadVecReg{static_cast<std::uint8_t>(m.reg)}});
                if (m.mod == 0b11) {
                    d.stmts.push_back({r_src,
                        ir::LoadVecReg{static_cast<std::uint8_t>(
                            static_cast<unsigned>(m.base))}});
                } else {
                    const ir::Ref r_addr = emit_address(d.stmts, m, next_ref,
                                                        instruction_guest_pc + cursor);
                    d.stmts.push_back({r_src, ir::LoadVec{r_addr}});
                }
                d.stmts.push_back({r_mask, ir::LoadVecReg{0u}});  // implicit XMM0
                d.stmts.push_back({r_res,
                    ir::VecBlend{r_dst, r_src, r_mask, lane}});
                d.stmts.push_back({std::nullopt,
                    ir::StoreVecReg{static_cast<std::uint8_t>(m.reg), r_res}});
                d.bytes_consumed = cursor;
                return d;
            }

            // SSE4.1 PMOVZX/SX widening converts (66 0F 38 20..25, 30..35).
            if ((sub3 >= 0x20u && sub3 <= 0x25u) ||
                (sub3 >= 0x30u && sub3 <= 0x35u)) {
                auto modrm = parse_modrm(bytes, cursor, rex,
                                         has_address_size_override);
                if (std::holds_alternative<DecodeError>(modrm)) {
                    return std::get<DecodeError>(modrm);
                }
                const auto& m = std::get<ModRmOperand>(modrm);
                const bool is_signed = sub3 < 0x30u;
                const std::uint8_t low_nibble = sub3 & 0x0Fu;
                ir::VecLane narrow, wide;
                switch (low_nibble) {
                    case 0x0u: narrow = ir::VecLane::B16; wide = ir::VecLane::H8; break;
                    case 0x1u: narrow = ir::VecLane::B16; wide = ir::VecLane::S4; break;
                    case 0x2u: narrow = ir::VecLane::B16; wide = ir::VecLane::D2; break;
                    case 0x3u: narrow = ir::VecLane::H8;  wide = ir::VecLane::S4; break;
                    case 0x4u: narrow = ir::VecLane::H8;  wide = ir::VecLane::D2; break;
                    case 0x5u: narrow = ir::VecLane::S4;  wide = ir::VecLane::D2; break;
                    default:   narrow = ir::VecLane::B16; wide = ir::VecLane::H8;
                }
                Decoded d;
                const ir::Ref r_src = next_ref++;
                const ir::Ref r_res = next_ref++;
                if (m.mod == 0b11) {
                    d.stmts.push_back({r_src,
                        ir::LoadVecReg{static_cast<std::uint8_t>(
                            static_cast<unsigned>(m.base))}});
                } else {
                    const ir::Ref r_addr = emit_address(d.stmts, m, next_ref,
                                                        instruction_guest_pc + cursor);
                    d.stmts.push_back({r_src, ir::LoadVec{r_addr}});
                }
                d.stmts.push_back({r_res,
                    ir::VecExtend{r_src, narrow, wide, is_signed}});
                d.stmts.push_back({std::nullopt,
                    ir::StoreVecReg{static_cast<std::uint8_t>(m.reg), r_res}});
                d.bytes_consumed = cursor;
                return d;
            }

            // PSHUFB: 66 0F 38 00 /r
            // PABSB:  66 0F 38 1C /r   (B16)
            // PABSW:  66 0F 38 1D /r   (H8)
            // PABSD:  66 0F 38 1E /r   (S4)
            //
            // Also a quick SIMD-binop family routed through the existing
            // VecBinOp branch:
            //   PHADDW  66 0F 38 01  → PairAddInt H8
            //   PHADDD  66 0F 38 02  → PairAddInt S4
            //   PHSUBW  66 0F 38 05  → PairSubInt H8
            //   PHSUBD  66 0F 38 06  → PairSubInt S4
            //   PMULLD  66 0F 38 40  → Mul          S4   (SSE4.1)
            //   PCMPEQQ 66 0F 38 29  → VecCmp Eq    D2   (SSE4.1)
            if (sub3 == 0x01u || sub3 == 0x02u ||
                sub3 == 0x05u || sub3 == 0x06u ||
                sub3 == 0x40u || sub3 == 0x29u || sub3 == 0x37u ||
                (sub3 >= 0x38u && sub3 <= 0x3Fu)) {
                auto modrm = parse_modrm(bytes, cursor, rex,
                                         has_address_size_override);
                if (std::holds_alternative<DecodeError>(modrm)) {
                    return std::get<DecodeError>(modrm);
                }
                const auto& m = std::get<ModRmOperand>(modrm);
                Decoded d;
                const ir::Ref r_lhs = next_ref++;
                const ir::Ref r_rhs = next_ref++;
                const ir::Ref r_res = next_ref++;
                d.stmts.push_back({r_lhs,
                    ir::LoadVecReg{static_cast<std::uint8_t>(m.reg)}});
                if (m.mod == 0b11) {
                    d.stmts.push_back({r_rhs,
                        ir::LoadVecReg{static_cast<std::uint8_t>(
                            static_cast<unsigned>(m.base))}});
                } else {
                    const ir::Ref r_addr = emit_address(d.stmts, m, next_ref,
                                                        instruction_guest_pc + cursor);
                    d.stmts.push_back({r_rhs, ir::LoadVec{r_addr}});
                }
                if (sub3 == 0x29u) {
                    d.stmts.push_back({r_res,
                        ir::VecCmp{ir::VecCmpKind::Eq, r_lhs, r_rhs, ir::VecLane::D2}});
                } else if (sub3 == 0x37u) {
                    d.stmts.push_back({r_res,
                        ir::VecCmp{ir::VecCmpKind::Gt, r_lhs, r_rhs, ir::VecLane::D2}});
                } else if (sub3 >= 0x38u && sub3 <= 0x3Fu) {
                    ir::VecBinOpKind k;
                    ir::VecLane l;
                    switch (sub3) {
                        case 0x38u: k = ir::VecBinOpKind::SMin; l = ir::VecLane::B16; break;
                        case 0x39u: k = ir::VecBinOpKind::SMin; l = ir::VecLane::S4;  break;
                        case 0x3Au: k = ir::VecBinOpKind::UMin; l = ir::VecLane::H8;  break;
                        case 0x3Bu: k = ir::VecBinOpKind::UMin; l = ir::VecLane::S4;  break;
                        case 0x3Cu: k = ir::VecBinOpKind::SMax; l = ir::VecLane::B16; break;
                        case 0x3Du: k = ir::VecBinOpKind::SMax; l = ir::VecLane::S4;  break;
                        case 0x3Eu: k = ir::VecBinOpKind::UMax; l = ir::VecLane::H8;  break;
                        case 0x3Fu: k = ir::VecBinOpKind::UMax; l = ir::VecLane::S4;  break;
                        default:    k = ir::VecBinOpKind::SMin; l = ir::VecLane::B16;
                    }
                    d.stmts.push_back({r_res, ir::VecBinOp{k, r_lhs, r_rhs, l}});
                } else {
                    ir::VecBinOpKind k;
                    ir::VecLane l;
                    switch (sub3) {
                        case 0x01u: k = ir::VecBinOpKind::PairAddInt; l = ir::VecLane::H8; break;
                        case 0x02u: k = ir::VecBinOpKind::PairAddInt; l = ir::VecLane::S4; break;
                        case 0x05u: k = ir::VecBinOpKind::PairSubInt; l = ir::VecLane::H8; break;
                        case 0x06u: k = ir::VecBinOpKind::PairSubInt; l = ir::VecLane::S4; break;
                        default:    k = ir::VecBinOpKind::Mul;        l = ir::VecLane::S4; break;
                    }
                    d.stmts.push_back({r_res, ir::VecBinOp{k, r_lhs, r_rhs, l}});
                }
                d.stmts.push_back({std::nullopt,
                    ir::StoreVecReg{static_cast<std::uint8_t>(m.reg), r_res}});
                d.bytes_consumed = cursor;
                return d;
            }
            if (sub3 == 0x00u || sub3 == 0x1Cu || sub3 == 0x1Du || sub3 == 0x1Eu) {
                auto modrm = parse_modrm(bytes, cursor, rex,
                                         has_address_size_override);
                if (std::holds_alternative<DecodeError>(modrm)) {
                    return std::get<DecodeError>(modrm);
                }
                const auto& m = std::get<ModRmOperand>(modrm);
                Decoded d;
                if (sub3 == 0x00u) {
                    const ir::Ref r_dst_old = next_ref++;
                    const ir::Ref r_mask    = next_ref++;
                    const ir::Ref r_res     = next_ref++;
                    const std::uint8_t lhs_xmm_pshufb = vex.present
                        ? static_cast<std::uint8_t>(vex.vvvv)
                        : static_cast<std::uint8_t>(m.reg);
                    d.stmts.push_back({r_dst_old,
                        ir::LoadVecReg{lhs_xmm_pshufb}});
                    if (m.mod == 0b11) {
                        d.stmts.push_back({r_mask,
                            ir::LoadVecReg{static_cast<std::uint8_t>(
                                static_cast<unsigned>(m.base))}});
                    } else {
                        const ir::Ref r_addr = emit_address(d.stmts, m, next_ref,
                                                            instruction_guest_pc + cursor);
                        d.stmts.push_back({r_mask, ir::LoadVec{r_addr}});
                    }
                    d.stmts.push_back({r_res,
                        ir::VecPshufb{r_dst_old, r_mask}});
                    d.stmts.push_back({std::nullopt,
                        ir::StoreVecReg{static_cast<std::uint8_t>(m.reg), r_res}});
                } else {
                    const ir::VecLane lane =
                        sub3 == 0x1Cu ? ir::VecLane::B16 :
                        sub3 == 0x1Du ? ir::VecLane::H8  :
                                        ir::VecLane::S4;
                    const ir::Ref r_src = next_ref++;
                    const ir::Ref r_res = next_ref++;
                    if (m.mod == 0b11) {
                        d.stmts.push_back({r_src,
                            ir::LoadVecReg{static_cast<std::uint8_t>(
                                static_cast<unsigned>(m.base))}});
                    } else {
                        const ir::Ref r_addr = emit_address(d.stmts, m, next_ref,
                                                            instruction_guest_pc + cursor);
                        d.stmts.push_back({r_src, ir::LoadVec{r_addr}});
                    }
                    d.stmts.push_back({r_res, ir::VecAbs{r_src, lane}});
                    d.stmts.push_back({std::nullopt,
                        ir::StoreVecReg{static_cast<std::uint8_t>(m.reg), r_res}});
                }
                d.bytes_consumed = cursor;
                return d;
            }
            return DecodeError::UnsupportedEncoding;
        }

        // F2-IR-003: SSE2 integer SIMD register-direct.
        //   66 0F FC /r — PADDB xmm1, xmm2   (16 × i8)
        //   66 0F FD /r — PADDW xmm1, xmm2   ( 8 × i16)
        //   66 0F FE /r — PADDD xmm1, xmm2   ( 4 × i32)
        //   66 0F D4 /r — PADDQ xmm1, xmm2   ( 2 × i64)
        //   66 0F F8 /r — PSUBB xmm1, xmm2
        //   66 0F F9 /r — PSUBW xmm1, xmm2
        //   66 0F FA /r — PSUBD xmm1, xmm2
        //   66 0F FB /r — PSUBQ xmm1, xmm2
        //   66 0F EB /r — POR   xmm1, xmm2   (bitwise; lane-agnostic)
        //   66 0F DB /r — PAND  xmm1, xmm2
        //   66 0F EF /r — PXOR  xmm1, xmm2
        // Memory operands (mod != 11) are deferred — emit
        // UnsupportedEncoding for now so callers know to fall back.
        // Also: 0F 54 (ANDPS), 0F 56 (ORPS), 0F 57 (XORPS), no prefix.
        //       66 0F 54 (ANDPD), 66 0F 56 (ORPD), 66 0F 57 (XORPD).
        // These are bitwise — lane-agnostic — so we map them to the
        // same VecBinOp kinds as PAND/POR/PXOR with B16 lane.
        const bool is_andps_family =
            !has_lock && !has_f2 && !has_f3 &&
            (subop == 0x54u || subop == 0x56u || subop == 0x57u);
        const bool is_sat_arith =
            has_operand_size_override && !has_lock && !has_f2 && !has_f3 &&
            (subop == 0xECu || subop == 0xEDu ||  // PADDSB/W
             subop == 0xDCu || subop == 0xDDu ||  // PADDUSB/W
             subop == 0xE8u || subop == 0xE9u ||  // PSUBSB/W
             subop == 0xD8u || subop == 0xD9u);   // PSUBUSB/W
        const bool is_minmax =
            has_operand_size_override && !has_lock && !has_f2 && !has_f3 &&
            (subop == 0xDAu || subop == 0xDEu ||  // PMINUB / PMAXUB
             subop == 0xEAu || subop == 0xEEu);   // PMINSW / PMAXSW
        if ((has_operand_size_override && !has_lock && !has_f2 && !has_f3 &&
            (subop == 0xFCu || subop == 0xFDu || subop == 0xFEu ||
             subop == 0xD4u || subop == 0xF8u || subop == 0xF9u ||
             subop == 0xFAu || subop == 0xFBu || subop == 0xEBu ||
             subop == 0xDBu || subop == 0xEFu ||
             subop == 0xD5u || subop == 0xE4u || subop == 0xE5u ||
             subop == 0xF4u || subop == 0xF6u))
            || is_andps_family || is_sat_arith || is_minmax) {
            auto modrm = parse_modrm(bytes, cursor, rex,
                                     has_address_size_override);
            if (std::holds_alternative<DecodeError>(modrm)) {
                return std::get<DecodeError>(modrm);
            }
            const auto& m = std::get<ModRmOperand>(modrm);
            ir::VecBinOpKind vop = ir::VecBinOpKind::Add;
            ir::VecLane      lane = ir::VecLane::B16;
            switch (subop) {
                case 0xFCu: vop = ir::VecBinOpKind::Add; lane = ir::VecLane::B16; break;
                case 0xFDu: vop = ir::VecBinOpKind::Add; lane = ir::VecLane::H8;  break;
                case 0xFEu: vop = ir::VecBinOpKind::Add; lane = ir::VecLane::S4;  break;
                case 0xD4u: vop = ir::VecBinOpKind::Add; lane = ir::VecLane::D2;  break;
                case 0xF8u: vop = ir::VecBinOpKind::Sub; lane = ir::VecLane::B16; break;
                case 0xF9u: vop = ir::VecBinOpKind::Sub; lane = ir::VecLane::H8;  break;
                case 0xFAu: vop = ir::VecBinOpKind::Sub; lane = ir::VecLane::S4;  break;
                case 0xFBu: vop = ir::VecBinOpKind::Sub; lane = ir::VecLane::D2;  break;
                case 0xDBu: vop = ir::VecBinOpKind::And; break;
                case 0xEBu: vop = ir::VecBinOpKind::Or;  break;
                case 0xEFu: vop = ir::VecBinOpKind::Xor; break;
                case 0xD5u: vop = ir::VecBinOpKind::Mul; lane = ir::VecLane::H8; break;
                case 0x54u: vop = ir::VecBinOpKind::And; break;  // ANDPS/ANDPD
                case 0x56u: vop = ir::VecBinOpKind::Or;  break;  // ORPS/ORPD
                case 0x57u: vop = ir::VecBinOpKind::Xor; break;  // XORPS/XORPD
                case 0xECu: vop = ir::VecBinOpKind::SqAdd; lane = ir::VecLane::B16; break;
                case 0xEDu: vop = ir::VecBinOpKind::SqAdd; lane = ir::VecLane::H8;  break;
                case 0xDCu: vop = ir::VecBinOpKind::UqAdd; lane = ir::VecLane::B16; break;
                case 0xDDu: vop = ir::VecBinOpKind::UqAdd; lane = ir::VecLane::H8;  break;
                case 0xE8u: vop = ir::VecBinOpKind::SqSub; lane = ir::VecLane::B16; break;
                case 0xE9u: vop = ir::VecBinOpKind::SqSub; lane = ir::VecLane::H8;  break;
                case 0xD8u: vop = ir::VecBinOpKind::UqSub; lane = ir::VecLane::B16; break;
                case 0xD9u: vop = ir::VecBinOpKind::UqSub; lane = ir::VecLane::H8;  break;
                case 0xDAu: vop = ir::VecBinOpKind::UMin;  lane = ir::VecLane::B16; break;
                case 0xDEu: vop = ir::VecBinOpKind::UMax;  lane = ir::VecLane::B16; break;
                case 0xEAu: vop = ir::VecBinOpKind::SMin;  lane = ir::VecLane::H8;  break;
                case 0xEEu: vop = ir::VecBinOpKind::SMax;  lane = ir::VecLane::H8;  break;
                case 0xE5u: vop = ir::VecBinOpKind::SMulHi; lane = ir::VecLane::H8; break;
                case 0xE4u: vop = ir::VecBinOpKind::UMulHi; lane = ir::VecLane::H8; break;
                case 0xF4u: vop = ir::VecBinOpKind::UMul32To64; lane = ir::VecLane::S4; break;
                case 0xF6u: vop = ir::VecBinOpKind::SadBw; lane = ir::VecLane::B16; break;
                default: break;
            }
            Decoded d;
            // For VEX-encoded ops the first source is the third operand
            // (vvvv); otherwise it's the SSE-classic dest aliasing src1.
            const std::uint8_t lhs_xmm = vex.present
                ? static_cast<std::uint8_t>(vex.vvvv)
                : static_cast<std::uint8_t>(m.reg);
            std::optional<ir::Ref> r_addr_lo;
            if (m.mod != 0b11) {
                r_addr_lo = emit_address(d.stmts, m, next_ref,
                                         instruction_guest_pc + cursor);
            }
            const ir::Ref r_dst_old = next_ref++;
            const ir::Ref r_src     = next_ref++;
            const ir::Ref r_res     = next_ref++;
            d.stmts.push_back({r_dst_old, ir::LoadVecReg{lhs_xmm}});
            if (m.mod == 0b11) {
                d.stmts.push_back({r_src,
                    ir::LoadVecReg{static_cast<std::uint8_t>(
                        static_cast<unsigned>(m.base))}});
            } else {
                d.stmts.push_back({r_src, ir::LoadVec{*r_addr_lo}});
            }
            d.stmts.push_back({r_res,
                ir::VecBinOp{vop, r_dst_old, r_src, lane}});
            d.stmts.push_back({std::nullopt,
                ir::StoreVecReg{static_cast<std::uint8_t>(m.reg), r_res}});
            // F2-IR-005 — AVX-256 high-lane pair when VEX.L=1.
            if (vex.present && vex.L) {
                const ir::Ref r_dst_hi_old = next_ref++;
                const ir::Ref r_src_hi     = next_ref++;
                const ir::Ref r_res_hi     = next_ref++;
                d.stmts.push_back({r_dst_hi_old, ir::LoadVecRegHi{lhs_xmm}});
                if (m.mod == 0b11) {
                    d.stmts.push_back({r_src_hi,
                        ir::LoadVecRegHi{static_cast<std::uint8_t>(
                            static_cast<unsigned>(m.base))}});
                } else {
                    const ir::Ref r_off16 = next_ref++;
                    const ir::Ref r_addr_hi = next_ref++;
                    d.stmts.push_back({r_off16,
                        ir::Constant{16ULL, ir::OpSize::I64}});
                    d.stmts.push_back({r_addr_hi,
                        ir::BinOp{ir::BinOpKind::Add, *r_addr_lo, r_off16,
                                  ir::OpSize::I64}});
                    d.stmts.push_back({r_src_hi, ir::LoadVec{r_addr_hi}});
                }
                d.stmts.push_back({r_res_hi,
                    ir::VecBinOp{vop, r_dst_hi_old, r_src_hi, lane}});
                d.stmts.push_back({std::nullopt,
                    ir::StoreVecRegHi{static_cast<std::uint8_t>(m.reg), r_res_hi}});
            }
            d.bytes_consumed = cursor;
            return d;
        }

        // F2-IR-015: FP UNPCKLPS/HPS/LPD/HPD share VecUnpack with the
        // integer lane forms.
        //   0F 14 /r — UNPCKLPS  (S4 low)
        //   0F 15 /r — UNPCKHPS  (S4 high)
        //   66 0F 14 /r — UNPCKLPD (D2 low)
        //   66 0F 15 /r — UNPCKHPD (D2 high)
        if (!has_lock && !has_f2 && !has_f3 &&
            (subop == 0x14u || subop == 0x15u)) {
            auto modrm = parse_modrm(bytes, cursor, rex,
                                     has_address_size_override);
            if (std::holds_alternative<DecodeError>(modrm)) {
                return std::get<DecodeError>(modrm);
            }
            const auto& m = std::get<ModRmOperand>(modrm);
            const ir::VecLane lane = has_operand_size_override
                                         ? ir::VecLane::D2
                                         : ir::VecLane::S4;
            const bool is_high = subop == 0x15u;
            Decoded d;
            const std::uint8_t lhs_xmm = vex.present
                ? static_cast<std::uint8_t>(vex.vvvv)
                : static_cast<std::uint8_t>(m.reg);
            std::optional<ir::Ref> r_addr_lo;
            if (m.mod != 0b11) {
                r_addr_lo = emit_address(d.stmts, m, next_ref,
                                         instruction_guest_pc + cursor);
            }
            const ir::Ref r_lhs = next_ref++;
            const ir::Ref r_rhs = next_ref++;
            const ir::Ref r_res = next_ref++;
            d.stmts.push_back({r_lhs, ir::LoadVecReg{lhs_xmm}});
            if (m.mod == 0b11) {
                d.stmts.push_back({r_rhs,
                    ir::LoadVecReg{static_cast<std::uint8_t>(
                        static_cast<unsigned>(m.base))}});
            } else {
                d.stmts.push_back({r_rhs, ir::LoadVec{*r_addr_lo}});
            }
            d.stmts.push_back({r_res, ir::VecUnpack{is_high, r_lhs, r_rhs, lane}});
            d.stmts.push_back({std::nullopt,
                ir::StoreVecReg{static_cast<std::uint8_t>(m.reg), r_res}});
            if (vex.present && vex.L) {
                const ir::Ref r_lhs_hi = next_ref++;
                const ir::Ref r_rhs_hi = next_ref++;
                const ir::Ref r_res_hi = next_ref++;
                d.stmts.push_back({r_lhs_hi, ir::LoadVecRegHi{lhs_xmm}});
                if (m.mod == 0b11) {
                    d.stmts.push_back({r_rhs_hi,
                        ir::LoadVecRegHi{static_cast<std::uint8_t>(
                            static_cast<unsigned>(m.base))}});
                } else {
                    const ir::Ref r_off16 = next_ref++;
                    const ir::Ref r_addr_hi = next_ref++;
                    d.stmts.push_back({r_off16,
                        ir::Constant{16ULL, ir::OpSize::I64}});
                    d.stmts.push_back({r_addr_hi,
                        ir::BinOp{ir::BinOpKind::Add, *r_addr_lo, r_off16,
                                  ir::OpSize::I64}});
                    d.stmts.push_back({r_rhs_hi, ir::LoadVec{r_addr_hi}});
                }
                d.stmts.push_back({r_res_hi,
                    ir::VecUnpack{is_high, r_lhs_hi, r_rhs_hi, lane}});
                d.stmts.push_back({std::nullopt,
                    ir::StoreVecRegHi{static_cast<std::uint8_t>(m.reg), r_res_hi}});
            }
            d.bytes_consumed = cursor;
            return d;
        }

        // F2-IR-011: UNPCKL/UNPCKH — interleave lanes.
        //   66 0F 60/61/62/6C — PUNPCKL{BW,WD,DQ,QDQ}
        //   66 0F 68/69/6A/6D — PUNPCKH{BW,WD,DQ,QDQ}
        if (has_operand_size_override && !has_lock && !has_f2 && !has_f3 &&
            (subop == 0x60u || subop == 0x61u || subop == 0x62u || subop == 0x6Cu ||
             subop == 0x68u || subop == 0x69u || subop == 0x6Au || subop == 0x6Du)) {
            auto modrm = parse_modrm(bytes, cursor, rex,
                                     has_address_size_override);
            if (std::holds_alternative<DecodeError>(modrm)) {
                return std::get<DecodeError>(modrm);
            }
            const auto& m = std::get<ModRmOperand>(modrm);
            const bool is_high = (subop & 0x08u) != 0;
            ir::VecLane lane = ir::VecLane::B16;
            switch (subop & 0x07u) {
                case 0x0u: lane = ir::VecLane::B16; break;  // BW
                case 0x1u: lane = ir::VecLane::H8;  break;  // WD
                case 0x2u: lane = ir::VecLane::S4;  break;  // DQ
                case 0x4u: case 0x5u: lane = ir::VecLane::D2; break;  // QDQ
                default:   lane = ir::VecLane::B16;
            }
            // 6C/6D: QDQ. Override the switch above for those exact codes.
            if (subop == 0x6Cu || subop == 0x6Du) lane = ir::VecLane::D2;
            Decoded d;
            const std::uint8_t lhs_xmm_unpck = vex.present
                ? static_cast<std::uint8_t>(vex.vvvv)
                : static_cast<std::uint8_t>(m.reg);
            std::optional<ir::Ref> r_addr_lo;
            if (m.mod != 0b11) {
                r_addr_lo = emit_address(d.stmts, m, next_ref,
                                         instruction_guest_pc + cursor);
            }
            const ir::Ref r_lhs = next_ref++;
            const ir::Ref r_rhs = next_ref++;
            const ir::Ref r_res = next_ref++;
            d.stmts.push_back({r_lhs, ir::LoadVecReg{lhs_xmm_unpck}});
            if (m.mod == 0b11) {
                d.stmts.push_back({r_rhs,
                    ir::LoadVecReg{static_cast<std::uint8_t>(
                        static_cast<unsigned>(m.base))}});
            } else {
                d.stmts.push_back({r_rhs, ir::LoadVec{*r_addr_lo}});
            }
            d.stmts.push_back({r_res, ir::VecUnpack{is_high, r_lhs, r_rhs, lane}});
            d.stmts.push_back({std::nullopt,
                ir::StoreVecReg{static_cast<std::uint8_t>(m.reg), r_res}});
            if (vex.present && vex.L) {
                const ir::Ref r_lhs_hi = next_ref++;
                const ir::Ref r_rhs_hi = next_ref++;
                const ir::Ref r_res_hi = next_ref++;
                d.stmts.push_back({r_lhs_hi, ir::LoadVecRegHi{lhs_xmm_unpck}});
                if (m.mod == 0b11) {
                    d.stmts.push_back({r_rhs_hi,
                        ir::LoadVecRegHi{static_cast<std::uint8_t>(
                            static_cast<unsigned>(m.base))}});
                } else {
                    const ir::Ref r_off16 = next_ref++;
                    const ir::Ref r_addr_hi = next_ref++;
                    d.stmts.push_back({r_off16,
                        ir::Constant{16ULL, ir::OpSize::I64}});
                    d.stmts.push_back({r_addr_hi,
                        ir::BinOp{ir::BinOpKind::Add, *r_addr_lo, r_off16,
                                  ir::OpSize::I64}});
                    d.stmts.push_back({r_rhs_hi, ir::LoadVec{r_addr_hi}});
                }
                d.stmts.push_back({r_res_hi,
                    ir::VecUnpack{is_high, r_lhs_hi, r_rhs_hi, lane}});
                d.stmts.push_back({std::nullopt,
                    ir::StoreVecRegHi{static_cast<std::uint8_t>(m.reg), r_res_hi}});
            }
            d.bytes_consumed = cursor;
            return d;
        }

        // F2-IR-012: per-lane shift by immediate.
        //   66 0F 71 /6 ib — PSLLW (H8)
        //   66 0F 71 /2 ib — PSRLW
        //   66 0F 71 /4 ib — PSRAW
        //   66 0F 72 /6 ib — PSLLD (S4)
        //   66 0F 72 /2 ib — PSRLD
        //   66 0F 72 /4 ib — PSRAD
        //   66 0F 73 /6 ib — PSLLQ (D2)
        //   66 0F 73 /2 ib — PSRLQ
        // (PSLLDQ /7, PSRLDQ /3 — byte shifts of whole reg, deferred.)
        if (has_operand_size_override && !has_lock && !has_f2 && !has_f3 &&
            (subop == 0x71u || subop == 0x72u || subop == 0x73u)) {
            auto mr = consume_le<1>(bytes, cursor);
            if (std::holds_alternative<DecodeError>(mr)) {
                return std::get<DecodeError>(mr);
            }
            const Byte modrm_b = static_cast<Byte>(std::get<std::uint64_t>(mr));
            const unsigned mod    = (modrm_b >> 6) & 0x3u;
            const unsigned reg_op = (modrm_b >> 3) & 0x7u;  // /digit
            const unsigned rm     = (modrm_b & 0x7u) | (rex.b ? 0x8u : 0x0u);
            if (mod != 0b11) return DecodeError::UnsupportedEncoding;
            // PSLLDQ /7 ib + PSRLDQ /3 ib only valid for subop == 0x73 —
            // they shift the entire register by `count` BYTES.
            if (subop == 0x73u && (reg_op == 7 || reg_op == 3)) {
                auto imm_b = consume_le<1>(bytes, cursor);
                if (std::holds_alternative<DecodeError>(imm_b)) {
                    return std::get<DecodeError>(imm_b);
                }
                const std::uint8_t cb =
                    static_cast<std::uint8_t>(std::get<std::uint64_t>(imm_b));
                Decoded d2;
                const ir::Ref r_src = next_ref++;
                const ir::Ref r_res = next_ref++;
                d2.stmts.push_back({r_src,
                    ir::LoadVecReg{static_cast<std::uint8_t>(rm)}});
                d2.stmts.push_back({r_res,
                    ir::VecShiftBytes{reg_op == 7, r_src, cb}});
                d2.stmts.push_back({std::nullopt,
                    ir::StoreVecReg{static_cast<std::uint8_t>(rm), r_res}});
                d2.bytes_consumed = cursor;
                return d2;
            }
            ir::VecShiftKind kind;
            switch (reg_op) {
                case 6: kind = ir::VecShiftKind::ShiftL;     break;
                case 2: kind = ir::VecShiftKind::LogicalShr; break;
                case 4: kind = ir::VecShiftKind::ArithShr;   break;
                default: return DecodeError::UnsupportedEncoding;
            }
            // PSRAQ doesn't exist in SSE2.
            if (subop == 0x73u && kind == ir::VecShiftKind::ArithShr) {
                return DecodeError::UnsupportedEncoding;
            }
            const ir::VecLane lane =
                subop == 0x71u ? ir::VecLane::H8 :
                subop == 0x72u ? ir::VecLane::S4 :
                                 ir::VecLane::D2;
            auto imm = consume_le<1>(bytes, cursor);
            if (std::holds_alternative<DecodeError>(imm)) {
                return std::get<DecodeError>(imm);
            }
            const std::uint8_t count =
                static_cast<std::uint8_t>(std::get<std::uint64_t>(imm));
            Decoded d;
            const ir::Ref r_src = next_ref++;
            const ir::Ref r_res = next_ref++;
            d.stmts.push_back({r_src,
                ir::LoadVecReg{static_cast<std::uint8_t>(rm)}});
            d.stmts.push_back({r_res,
                ir::VecShiftImm{kind, r_src, count, lane}});
            d.stmts.push_back({std::nullopt,
                ir::StoreVecReg{static_cast<std::uint8_t>(rm), r_res}});
            d.bytes_consumed = cursor;
            return d;
        }

        // F2-IR-034: CMPxxPS / CMPxxPD / CMPxxSS / CMPxxSD (0F C2 /r ib).
        //   no prefix → CMPxxPS  (packed S4)
        //   66 prefix → CMPxxPD  (packed D2)
        //   F3 prefix → CMPxxSS  (scalar F32, upper preserved)
        //   F2 prefix → CMPxxSD  (scalar F64, upper preserved)
        // imm8 selects predicate (0..7).
        if (!has_lock && subop == 0xC2u &&
            !(has_f2 && has_f3) &&
            !(has_operand_size_override && (has_f2 || has_f3))) {
            auto modrm = parse_modrm(bytes, cursor, rex,
                                     has_address_size_override);
            if (std::holds_alternative<DecodeError>(modrm)) {
                return std::get<DecodeError>(modrm);
            }
            const auto& m = std::get<ModRmOperand>(modrm);
            auto imm = consume_le<1>(bytes, cursor);
            if (std::holds_alternative<DecodeError>(imm)) {
                return std::get<DecodeError>(imm);
            }
            const std::uint8_t pred =
                static_cast<std::uint8_t>(std::get<std::uint64_t>(imm)) & 0x7u;
            const bool is_packed = !has_f2 && !has_f3;
            const ir::FpSize size =
                (has_operand_size_override || has_f2) ? ir::FpSize::F64
                                                      : ir::FpSize::F32;
            Decoded d;
            const std::uint8_t lhs_xmm_cmp = vex.present
                ? static_cast<std::uint8_t>(vex.vvvv)
                : static_cast<std::uint8_t>(m.reg);
            std::optional<ir::Ref> r_addr_lo;
            if (m.mod != 0b11) {
                r_addr_lo = emit_address(d.stmts, m, next_ref,
                                         instruction_guest_pc + cursor);
            }
            const ir::Ref r_lhs = next_ref++;
            const ir::Ref r_rhs = next_ref++;
            const ir::Ref r_res = next_ref++;
            d.stmts.push_back({r_lhs, ir::LoadVecReg{lhs_xmm_cmp}});
            if (m.mod == 0b11) {
                d.stmts.push_back({r_rhs,
                    ir::LoadVecReg{static_cast<std::uint8_t>(
                        static_cast<unsigned>(m.base))}});
            } else {
                d.stmts.push_back({r_rhs, ir::LoadVec{*r_addr_lo}});
            }
            d.stmts.push_back({r_res,
                ir::VecFpCompare{r_lhs, r_rhs, size,
                                 static_cast<ir::VecFpCmpPred>(pred), is_packed}});
            d.stmts.push_back({std::nullopt,
                ir::StoreVecReg{static_cast<std::uint8_t>(m.reg), r_res}});
            // F2-IR-005 — AVX-256 high-lane emission. CMPxxSS/SD with
            // VEX.L=1 is #UD (scalar+L=1 illegal); the allowlist already
            // excludes those, so this branch only fires for packed.
            if (vex.present && vex.L && is_packed) {
                const ir::Ref r_lhs_hi = next_ref++;
                const ir::Ref r_rhs_hi = next_ref++;
                const ir::Ref r_res_hi = next_ref++;
                d.stmts.push_back({r_lhs_hi, ir::LoadVecRegHi{lhs_xmm_cmp}});
                if (m.mod == 0b11) {
                    d.stmts.push_back({r_rhs_hi,
                        ir::LoadVecRegHi{static_cast<std::uint8_t>(
                            static_cast<unsigned>(m.base))}});
                } else {
                    const ir::Ref r_off16 = next_ref++;
                    const ir::Ref r_addr_hi = next_ref++;
                    d.stmts.push_back({r_off16,
                        ir::Constant{16ULL, ir::OpSize::I64}});
                    d.stmts.push_back({r_addr_hi,
                        ir::BinOp{ir::BinOpKind::Add, *r_addr_lo, r_off16,
                                  ir::OpSize::I64}});
                    d.stmts.push_back({r_rhs_hi, ir::LoadVec{r_addr_hi}});
                }
                d.stmts.push_back({r_res_hi,
                    ir::VecFpCompare{r_lhs_hi, r_rhs_hi, size,
                                     static_cast<ir::VecFpCmpPred>(pred), is_packed}});
                d.stmts.push_back({std::nullopt,
                    ir::StoreVecRegHi{static_cast<std::uint8_t>(m.reg), r_res_hi}});
            }
            d.bytes_consumed = cursor;
            return d;
        }

        // F2-IR-026: UCOMISS / UCOMISD / COMISS / COMISD — FP compare → flags.
        //   0F 2E /r — UCOMISS xmm1, xmm2/m32
        //   0F 2F /r — COMISS  (signaling on QNaN; we treat the same)
        //   66 0F 2E /r — UCOMISD xmm1, xmm2/m64
        //   66 0F 2F /r — COMISD
        // Models: WriteFlagsFp setting NZCV with fcmp. Result Ref is
        // unused in this lowering (the consumer would be a ReadFlag /
        // CondJumpFlags op decoded separately for the SETcc / Jcc that
        // follows). We still produce a Ref so SETcc/Jcc can refer to
        // it as `flags`.
        if (!has_lock && !has_f2 && !has_f3 &&
            (subop == 0x2Eu || subop == 0x2Fu)) {
            auto modrm = parse_modrm(bytes, cursor, rex,
                                     has_address_size_override);
            if (std::holds_alternative<DecodeError>(modrm)) {
                return std::get<DecodeError>(modrm);
            }
            const auto& m = std::get<ModRmOperand>(modrm);
            const ir::FpSize fp_sz =
                has_operand_size_override ? ir::FpSize::F64 : ir::FpSize::F32;
            Decoded d;
            const ir::Ref r_lhs = next_ref++;
            const ir::Ref r_rhs = next_ref++;
            const ir::Ref r_flags = next_ref++;
            d.stmts.push_back({r_lhs,
                ir::LoadVecReg{static_cast<std::uint8_t>(m.reg)}});
            if (m.mod == 0b11) {
                d.stmts.push_back({r_rhs,
                    ir::LoadVecReg{static_cast<std::uint8_t>(
                        static_cast<unsigned>(m.base))}});
            } else {
                const ir::Ref r_addr = emit_address(d.stmts, m, next_ref,
                                                    instruction_guest_pc + cursor);
                d.stmts.push_back({r_rhs, ir::LoadVec{r_addr}});
            }
            d.stmts.push_back({r_flags,
                ir::WriteFlagsFp{r_lhs, r_rhs, fp_sz}});
            d.bytes_consumed = cursor;
            return d;
        }

        // F2-IR-029: MOVMSKPS / MOVMSKPD (0F 50 / 66 0F 50). Reg-only.
        if (!has_lock && !has_f2 && !has_f3 && subop == 0x50u) {
            auto modrm = parse_modrm(bytes, cursor, rex,
                                     has_address_size_override);
            if (std::holds_alternative<DecodeError>(modrm)) {
                return std::get<DecodeError>(modrm);
            }
            const auto& m = std::get<ModRmOperand>(modrm);
            if (m.mod != 0b11) return DecodeError::UnsupportedEncoding;
            const ir::Gpr dst_gpr = static_cast<ir::Gpr>(m.reg);
            const unsigned src_xmm = static_cast<unsigned>(m.base);
            Decoded d;
            const ir::Ref r_xmm = next_ref++;
            const ir::Ref r_mask = next_ref++;
            d.stmts.push_back({r_xmm,
                ir::LoadVecReg{static_cast<std::uint8_t>(src_xmm)}});
            d.stmts.push_back({r_mask,
                ir::VecMaskFp{r_xmm, has_operand_size_override}});
            d.stmts.push_back({std::nullopt,
                ir::StoreReg{dst_gpr, r_mask, ir::OpSize::I32}});
            d.bytes_consumed = cursor;
            return d;
        }

        // F2-IR-027: PMOVMSKB r32, xmm (66 0F D7 /r). Reg-direct only.
        if (has_operand_size_override && !has_lock && !has_f2 && !has_f3 &&
            subop == 0xD7u) {
            auto modrm = parse_modrm(bytes, cursor, rex,
                                     has_address_size_override);
            if (std::holds_alternative<DecodeError>(modrm)) {
                return std::get<DecodeError>(modrm);
            }
            const auto& m = std::get<ModRmOperand>(modrm);
            if (m.mod != 0b11) return DecodeError::UnsupportedEncoding;
            // /r reg = GPR dest, rm = XMM source.
            const ir::Gpr dst_gpr = static_cast<ir::Gpr>(m.reg);
            const unsigned src_xmm = static_cast<unsigned>(m.base);
            Decoded d;
            const ir::Ref r_xmm = next_ref++;
            const ir::Ref r_mask = next_ref++;
            d.stmts.push_back({r_xmm,
                ir::LoadVecReg{static_cast<std::uint8_t>(src_xmm)}});
            d.stmts.push_back({r_mask, ir::VecMaskMsb{r_xmm}});
            d.stmts.push_back({std::nullopt,
                ir::StoreReg{dst_gpr, r_mask, ir::OpSize::I32}});
            d.bytes_consumed = cursor;
            return d;
        }

        // F2-IR-022: PINSRW / PEXTRW.
        //   66 0F C4 /r ib — PINSRW xmm1, r/m32, imm8 (insert low 16 of r/m at lane imm8 & 7)
        //   66 0F C5 /r ib — PEXTRW r32, xmm, imm8 (extract H8 lane, zero-extend; reg-only)
        if (has_operand_size_override && !has_lock && !has_f2 && !has_f3 &&
            (subop == 0xC4u || subop == 0xC5u)) {
            auto modrm = parse_modrm(bytes, cursor, rex,
                                     has_address_size_override);
            if (std::holds_alternative<DecodeError>(modrm)) {
                return std::get<DecodeError>(modrm);
            }
            const auto& m = std::get<ModRmOperand>(modrm);
            // PEXTRW only supports register-direct (the GPR dst form).
            if (subop == 0xC5u && m.mod != 0b11) {
                return DecodeError::UnsupportedEncoding;
            }
            auto imm = consume_le<1>(bytes, cursor);
            if (std::holds_alternative<DecodeError>(imm)) {
                return std::get<DecodeError>(imm);
            }
            const std::uint8_t lane_idx = static_cast<std::uint8_t>(
                std::get<std::uint64_t>(imm)) & 0x7u;
            Decoded d;
            if (subop == 0xC4u) {
                // PINSRW: xmm dest = m.reg, source = m.base (GPR) at reg-direct
                // or memory r/m. We'll only handle register form for now.
                if (m.mod != 0b11) return DecodeError::UnsupportedEncoding;
                const ir::Ref r_lhs = next_ref++;
                const ir::Ref r_val = next_ref++;
                const ir::Ref r_res = next_ref++;
                d.stmts.push_back({r_lhs,
                    ir::LoadVecReg{static_cast<std::uint8_t>(m.reg)}});
                d.stmts.push_back({r_val,
                    ir::LoadReg{m.base, ir::OpSize::I32}});
                d.stmts.push_back({r_res,
                    ir::VecInsertLane{r_lhs, r_val, lane_idx, ir::VecLane::H8}});
                d.stmts.push_back({std::nullopt,
                    ir::StoreVecReg{static_cast<std::uint8_t>(m.reg), r_res}});
            } else {
                // PEXTRW r32, xmm, imm8: GPR dest = m.reg (raw idx),
                // xmm source = m.base.
                const ir::Gpr dst_gpr = static_cast<ir::Gpr>(m.reg);
                const unsigned src_xmm = static_cast<unsigned>(m.base);
                const ir::Ref r_xmm = next_ref++;
                const ir::Ref r_val = next_ref++;
                d.stmts.push_back({r_xmm,
                    ir::LoadVecReg{static_cast<std::uint8_t>(src_xmm)}});
                d.stmts.push_back({r_val,
                    ir::VecExtractLaneU{r_xmm, lane_idx, ir::VecLane::H8}});
                d.stmts.push_back({std::nullopt,
                    ir::StoreReg{dst_gpr, r_val, ir::OpSize::I32}});
            }
            d.bytes_consumed = cursor;
            return d;
        }

        // F2-IR-020: SHUFPS / SHUFPD (two-source FP shuffle).
        //   0F C6 /r ib — SHUFPS xmm1, xmm2/m128, imm8 (S4 lanes)
        //   66 0F C6 /r ib — SHUFPD                    (D2 lanes)
        if (!has_lock && !has_f2 && !has_f3 && subop == 0xC6u) {
            auto modrm = parse_modrm(bytes, cursor, rex,
                                     has_address_size_override);
            if (std::holds_alternative<DecodeError>(modrm)) {
                return std::get<DecodeError>(modrm);
            }
            const auto& m = std::get<ModRmOperand>(modrm);
            auto imm = consume_le<1>(bytes, cursor);
            if (std::holds_alternative<DecodeError>(imm)) {
                return std::get<DecodeError>(imm);
            }
            const std::uint8_t control =
                static_cast<std::uint8_t>(std::get<std::uint64_t>(imm));
            const bool is_pd = has_operand_size_override;
            Decoded d;
            const std::uint8_t lhs_xmm_shuf = vex.present
                ? static_cast<std::uint8_t>(vex.vvvv)
                : static_cast<std::uint8_t>(m.reg);
            std::optional<ir::Ref> r_addr_lo;
            if (m.mod != 0b11) {
                r_addr_lo = emit_address(d.stmts, m, next_ref,
                                         instruction_guest_pc + cursor);
            }
            const ir::Ref r_lhs = next_ref++;
            const ir::Ref r_rhs = next_ref++;
            const ir::Ref r_res = next_ref++;
            d.stmts.push_back({r_lhs, ir::LoadVecReg{lhs_xmm_shuf}});
            if (m.mod == 0b11) {
                d.stmts.push_back({r_rhs,
                    ir::LoadVecReg{static_cast<std::uint8_t>(
                        static_cast<unsigned>(m.base))}});
            } else {
                d.stmts.push_back({r_rhs, ir::LoadVec{*r_addr_lo}});
            }
            d.stmts.push_back({r_res,
                ir::VecShuffle2Src{is_pd, r_lhs, r_rhs, control}});
            d.stmts.push_back({std::nullopt,
                ir::StoreVecReg{static_cast<std::uint8_t>(m.reg), r_res}});
            // F2-IR-005 — VSHUFPS/PD ymm: per-128-bit-lane semantics, so
            // the same control byte applies to both halves.
            if (vex.present && vex.L) {
                const ir::Ref r_lhs_hi = next_ref++;
                const ir::Ref r_rhs_hi = next_ref++;
                const ir::Ref r_res_hi = next_ref++;
                d.stmts.push_back({r_lhs_hi, ir::LoadVecRegHi{lhs_xmm_shuf}});
                if (m.mod == 0b11) {
                    d.stmts.push_back({r_rhs_hi,
                        ir::LoadVecRegHi{static_cast<std::uint8_t>(
                            static_cast<unsigned>(m.base))}});
                } else {
                    const ir::Ref r_off16 = next_ref++;
                    const ir::Ref r_addr_hi = next_ref++;
                    d.stmts.push_back({r_off16,
                        ir::Constant{16ULL, ir::OpSize::I64}});
                    d.stmts.push_back({r_addr_hi,
                        ir::BinOp{ir::BinOpKind::Add, *r_addr_lo, r_off16,
                                  ir::OpSize::I64}});
                    d.stmts.push_back({r_rhs_hi, ir::LoadVec{r_addr_hi}});
                }
                d.stmts.push_back({r_res_hi,
                    ir::VecShuffle2Src{is_pd, r_lhs_hi, r_rhs_hi, control}});
                d.stmts.push_back({std::nullopt,
                    ir::StoreVecRegHi{static_cast<std::uint8_t>(m.reg), r_res_hi}});
            }
            d.bytes_consumed = cursor;
            return d;
        }

        // F2-IR-028: PSHUFLW (F2 0F 70 /r ib) / PSHUFHW (F3 0F 70 /r ib)
        if ((has_f2 || has_f3) && !has_lock && !has_operand_size_override &&
            !(has_f2 && has_f3) && subop == 0x70u) {
            auto modrm = parse_modrm(bytes, cursor, rex,
                                     has_address_size_override);
            if (std::holds_alternative<DecodeError>(modrm)) {
                return std::get<DecodeError>(modrm);
            }
            const auto& m = std::get<ModRmOperand>(modrm);
            auto imm = consume_le<1>(bytes, cursor);
            if (std::holds_alternative<DecodeError>(imm)) {
                return std::get<DecodeError>(imm);
            }
            const std::uint8_t control =
                static_cast<std::uint8_t>(std::get<std::uint64_t>(imm));
            const bool is_high = has_f3;
            Decoded d;
            const ir::Ref r_src = next_ref++;
            const ir::Ref r_res = next_ref++;
            if (m.mod == 0b11) {
                d.stmts.push_back({r_src,
                    ir::LoadVecReg{static_cast<std::uint8_t>(
                        static_cast<unsigned>(m.base))}});
            } else {
                const ir::Ref r_addr = emit_address(d.stmts, m, next_ref,
                                                    instruction_guest_pc + cursor);
                d.stmts.push_back({r_src, ir::LoadVec{r_addr}});
            }
            d.stmts.push_back({r_res, ir::VecShuffleH4{is_high, r_src, control}});
            d.stmts.push_back({std::nullopt,
                ir::StoreVecReg{static_cast<std::uint8_t>(m.reg), r_res}});
            d.bytes_consumed = cursor;
            return d;
        }

        // F2-IR-010: PSHUFD xmm1, xmm2/m128, imm8 (66 0F 70 /r ib).
        // Permutes 4 32-bit lanes of source per immediate control byte.
        if (has_operand_size_override && !has_lock && !has_f2 && !has_f3 &&
            subop == 0x70u) {
            auto modrm = parse_modrm(bytes, cursor, rex,
                                     has_address_size_override);
            if (std::holds_alternative<DecodeError>(modrm)) {
                return std::get<DecodeError>(modrm);
            }
            const auto& m = std::get<ModRmOperand>(modrm);
            auto imm = consume_le<1>(bytes, cursor);
            if (std::holds_alternative<DecodeError>(imm)) {
                return std::get<DecodeError>(imm);
            }
            const std::uint8_t control =
                static_cast<std::uint8_t>(std::get<std::uint64_t>(imm));
            Decoded d;
            const ir::Ref r_src = next_ref++;
            const ir::Ref r_res = next_ref++;
            if (m.mod == 0b11) {
                d.stmts.push_back({r_src,
                    ir::LoadVecReg{static_cast<std::uint8_t>(
                        static_cast<unsigned>(m.base))}});
            } else {
                const ir::Ref r_addr = emit_address(d.stmts, m, next_ref,
                                                    instruction_guest_pc + cursor);
                d.stmts.push_back({r_src, ir::LoadVec{r_addr}});
            }
            d.stmts.push_back({r_res, ir::VecShuffle32x4{r_src, control}});
            d.stmts.push_back({std::nullopt,
                ir::StoreVecReg{static_cast<std::uint8_t>(m.reg), r_res}});
            d.bytes_consumed = cursor;
            return d;
        }

        // F2-IR-009: PCMPEQB/W/D and PCMPGTB/W/D — lane-wise integer compare.
        //   66 0F 64 /r — PCMPGTB (B16)
        //   66 0F 65 /r — PCMPGTW (H8)
        //   66 0F 66 /r — PCMPGTD (S4)
        //   66 0F 74 /r — PCMPEQB (B16)
        //   66 0F 75 /r — PCMPEQW (H8)
        //   66 0F 76 /r — PCMPEQD (S4)
        if (has_operand_size_override && !has_lock && !has_f2 && !has_f3 &&
            (subop == 0x64u || subop == 0x65u || subop == 0x66u ||
             subop == 0x74u || subop == 0x75u || subop == 0x76u)) {
            auto modrm = parse_modrm(bytes, cursor, rex,
                                     has_address_size_override);
            if (std::holds_alternative<DecodeError>(modrm)) {
                return std::get<DecodeError>(modrm);
            }
            const auto& m = std::get<ModRmOperand>(modrm);
            ir::VecCmpKind kind = (subop & 0xF0u) == 0x70u
                ? ir::VecCmpKind::Eq : ir::VecCmpKind::Gt;
            ir::VecLane lane;
            switch (subop & 0x0Fu) {
                case 0x4u: lane = ir::VecLane::B16; break;
                case 0x5u: lane = ir::VecLane::H8;  break;
                case 0x6u: lane = ir::VecLane::S4;  break;
                default:   lane = ir::VecLane::B16;
            }
            Decoded d;
            const std::uint8_t lhs_xmm = vex.present
                ? static_cast<std::uint8_t>(vex.vvvv)
                : static_cast<std::uint8_t>(m.reg);
            std::optional<ir::Ref> r_addr_lo;
            if (m.mod != 0b11) {
                r_addr_lo = emit_address(d.stmts, m, next_ref,
                                         instruction_guest_pc + cursor);
            }
            const ir::Ref r_lhs = next_ref++;
            const ir::Ref r_rhs = next_ref++;
            const ir::Ref r_res = next_ref++;
            d.stmts.push_back({r_lhs, ir::LoadVecReg{lhs_xmm}});
            if (m.mod == 0b11) {
                d.stmts.push_back({r_rhs,
                    ir::LoadVecReg{static_cast<std::uint8_t>(
                        static_cast<unsigned>(m.base))}});
            } else {
                d.stmts.push_back({r_rhs, ir::LoadVec{*r_addr_lo}});
            }
            d.stmts.push_back({r_res, ir::VecCmp{kind, r_lhs, r_rhs, lane}});
            d.stmts.push_back({std::nullopt,
                ir::StoreVecReg{static_cast<std::uint8_t>(m.reg), r_res}});
            if (vex.present && vex.L) {
                const ir::Ref r_lhs_hi = next_ref++;
                const ir::Ref r_rhs_hi = next_ref++;
                const ir::Ref r_res_hi = next_ref++;
                d.stmts.push_back({r_lhs_hi, ir::LoadVecRegHi{lhs_xmm}});
                if (m.mod == 0b11) {
                    d.stmts.push_back({r_rhs_hi,
                        ir::LoadVecRegHi{static_cast<std::uint8_t>(
                            static_cast<unsigned>(m.base))}});
                } else {
                    const ir::Ref r_off16 = next_ref++;
                    const ir::Ref r_addr_hi = next_ref++;
                    d.stmts.push_back({r_off16,
                        ir::Constant{16ULL, ir::OpSize::I64}});
                    d.stmts.push_back({r_addr_hi,
                        ir::BinOp{ir::BinOpKind::Add, *r_addr_lo, r_off16,
                                  ir::OpSize::I64}});
                    d.stmts.push_back({r_rhs_hi, ir::LoadVec{r_addr_hi}});
                }
                d.stmts.push_back({r_res_hi,
                    ir::VecCmp{kind, r_lhs_hi, r_rhs_hi, lane}});
                d.stmts.push_back({std::nullopt,
                    ir::StoreVecRegHi{static_cast<std::uint8_t>(m.reg), r_res_hi}});
            }
            d.bytes_consumed = cursor;
            return d;
        }

        // F2-IR-017: CVTSS2SD / CVTSD2SS — scalar precision conversion.
        //   F3 0F 5A /r — CVTSS2SD xmm1, xmm2/m32 (single → double)
        //   F2 0F 5A /r — CVTSD2SS xmm1, xmm2/m64 (double → single)
        if ((has_f2 || has_f3) && !has_lock && !has_operand_size_override &&
            !(has_f2 && has_f3) && subop == 0x5Au) {
            auto modrm = parse_modrm(bytes, cursor, rex,
                                     has_address_size_override);
            if (std::holds_alternative<DecodeError>(modrm)) {
                return std::get<DecodeError>(modrm);
            }
            const auto& m = std::get<ModRmOperand>(modrm);
            if (m.mod != 0b11) return DecodeError::UnsupportedEncoding;
            const ir::FpSize src_sz =
                has_f3 ? ir::FpSize::F32 : ir::FpSize::F64;
            const ir::FpSize dst_sz =
                has_f3 ? ir::FpSize::F64 : ir::FpSize::F32;
            Decoded d;
            const ir::Ref r_lhs = next_ref++;
            const ir::Ref r_src = next_ref++;
            const ir::Ref r_res = next_ref++;
            d.stmts.push_back({r_lhs,
                ir::LoadVecReg{static_cast<std::uint8_t>(m.reg)}});
            d.stmts.push_back({r_src,
                ir::LoadVecReg{static_cast<std::uint8_t>(
                    static_cast<unsigned>(m.base))}});
            d.stmts.push_back({r_res,
                ir::FpCvtScalar{r_lhs, r_src, src_sz, dst_sz}});
            d.stmts.push_back({std::nullopt,
                ir::StoreVecReg{static_cast<std::uint8_t>(m.reg), r_res}});
            d.bytes_consumed = cursor;
            return d;
        }

        // F2-IR-033: MOVDDUP / MOVSLDUP / MOVSHDUP (SSE3 broadcasts).
        //   F2 0F 12 /r — MOVDDUP  xmm, xmm/m64  (dup low qword across D2)
        //   F3 0F 12 /r — MOVSLDUP xmm, xmm/m128 (replicate even S4 lanes)
        //   F3 0F 16 /r — MOVSHDUP xmm, xmm/m128 (replicate odd S4 lanes)
        // All map to a VecShuffle32x4 with a fixed control byte.
        if (!has_lock && !has_operand_size_override &&
            ((has_f2 && subop == 0x12u) ||
             (has_f3 && (subop == 0x12u || subop == 0x16u)))) {
            auto modrm = parse_modrm(bytes, cursor, rex,
                                     has_address_size_override);
            if (std::holds_alternative<DecodeError>(modrm)) {
                return std::get<DecodeError>(modrm);
            }
            const auto& m = std::get<ModRmOperand>(modrm);
            std::uint8_t control;
            if (has_f2)                   control = 0x44u;  // MOVDDUP : 0,1,0,1
            else if (subop == 0x12u)      control = 0xA0u;  // MOVSLDUP: 0,0,2,2
            else                          control = 0xF5u;  // MOVSHDUP: 1,1,3,3
            Decoded d;
            const ir::Ref r_src = next_ref++;
            const ir::Ref r_res = next_ref++;
            if (m.mod == 0b11) {
                d.stmts.push_back({r_src,
                    ir::LoadVecReg{static_cast<std::uint8_t>(
                        static_cast<unsigned>(m.base))}});
            } else {
                const ir::Ref r_addr = emit_address(d.stmts, m, next_ref,
                                                    instruction_guest_pc + cursor);
                d.stmts.push_back({r_src, ir::LoadVec{r_addr}});
            }
            d.stmts.push_back({r_res, ir::VecShuffle32x4{r_src, control}});
            d.stmts.push_back({std::nullopt,
                ir::StoreVecReg{static_cast<std::uint8_t>(m.reg), r_res}});
            d.bytes_consumed = cursor;
            return d;
        }

        // F2-IR-021: MOVSS / MOVSD — scalar move (FP).
        //   F3 0F 10 /r — MOVSS load:  reg-reg keeps upper of dst,
        //                              mem→reg zeroes upper of dst.
        //   F3 0F 11 /r — MOVSS store: reg-reg same as load,
        //                              reg→mem writes low 32 bits.
        //   F2 0F 10/11 — MOVSD with qword granularity.
        if ((has_f2 || has_f3) && !has_lock && !has_operand_size_override &&
            !(has_f2 && has_f3) &&
            (subop == 0x10u || subop == 0x11u)) {
            auto modrm = parse_modrm(bytes, cursor, rex,
                                     has_address_size_override);
            if (std::holds_alternative<DecodeError>(modrm)) {
                return std::get<DecodeError>(modrm);
            }
            const auto& m = std::get<ModRmOperand>(modrm);
            const ir::FpSize fp_sz =
                has_f3 ? ir::FpSize::F32 : ir::FpSize::F64;
            const ir::OpSize int_sz =
                has_f3 ? ir::OpSize::I32 : ir::OpSize::I64;
            const bool is_load = subop == 0x10u;
            Decoded d;
            if (m.mod == 0b11) {
                // reg-reg: keep upper bits of dst. Same encoding either
                // way (load and store collapse at reg-reg).
                const ir::Ref r_lhs = next_ref++;
                const ir::Ref r_src = next_ref++;
                const ir::Ref r_res = next_ref++;
                const unsigned dst_xmm = is_load ? m.reg
                                                  : static_cast<unsigned>(m.base);
                const unsigned src_xmm = is_load ? static_cast<unsigned>(m.base)
                                                  : m.reg;
                d.stmts.push_back({r_lhs,
                    ir::LoadVecReg{static_cast<std::uint8_t>(dst_xmm)}});
                d.stmts.push_back({r_src,
                    ir::LoadVecReg{static_cast<std::uint8_t>(src_xmm)}});
                d.stmts.push_back({r_res,
                    ir::FpCvtScalar{r_lhs, r_src, fp_sz, fp_sz}});
                d.stmts.push_back({std::nullopt,
                    ir::StoreVecReg{static_cast<std::uint8_t>(dst_xmm), r_res}});
            } else if (is_load) {
                // mem→reg: low N bits of dst = mem, upper bits = 0.
                const ir::Ref r_addr = emit_address(d.stmts, m, next_ref,
                                                    instruction_guest_pc + cursor);
                const ir::Ref r_val = next_ref++;
                const ir::Ref r_xmm = next_ref++;
                d.stmts.push_back({r_val, ir::LoadMemTSO{r_addr, int_sz}});
                d.stmts.push_back({r_xmm, ir::XmmFromGpr{r_val, int_sz}});
                d.stmts.push_back({std::nullopt,
                    ir::StoreVecReg{static_cast<std::uint8_t>(m.reg), r_xmm}});
            } else {
                // reg→mem: write low N bits of src xmm to memory.
                const ir::Ref r_addr = emit_address(d.stmts, m, next_ref,
                                                    instruction_guest_pc + cursor);
                const ir::Ref r_xmm = next_ref++;
                const ir::Ref r_val = next_ref++;
                d.stmts.push_back({r_xmm,
                    ir::LoadVecReg{static_cast<std::uint8_t>(m.reg)}});
                d.stmts.push_back({r_val,
                    ir::GprFromXmm{r_xmm, int_sz}});
                d.stmts.push_back({std::nullopt,
                    ir::StoreMemTSO{r_addr, r_val, int_sz}});
            }
            d.bytes_consumed = cursor;
            return d;
        }

        // F2-IR-032: HADDPS / HADDPD (SSE3 horizontal add).
        //   F2 0F 7C /r — HADDPS (S4)
        //   66 0F 7C /r — HADDPD (D2)
        if (!has_lock && !has_f3 && subop == 0x7Cu &&
            ((has_f2 && !has_operand_size_override) ||
             (!has_f2 && has_operand_size_override))) {
            auto modrm = parse_modrm(bytes, cursor, rex,
                                     has_address_size_override);
            if (std::holds_alternative<DecodeError>(modrm)) {
                return std::get<DecodeError>(modrm);
            }
            const auto& m = std::get<ModRmOperand>(modrm);
            const ir::VecFpSize size = has_operand_size_override
                                           ? ir::VecFpSize::D2
                                           : ir::VecFpSize::S4;
            Decoded d;
            const std::uint8_t lhs_xmm_hadd = vex.present
                ? static_cast<std::uint8_t>(vex.vvvv)
                : static_cast<std::uint8_t>(m.reg);
            std::optional<ir::Ref> r_addr_lo;
            if (m.mod != 0b11) {
                r_addr_lo = emit_address(d.stmts, m, next_ref,
                                         instruction_guest_pc + cursor);
            }
            const ir::Ref r_lhs = next_ref++;
            const ir::Ref r_rhs = next_ref++;
            const ir::Ref r_res = next_ref++;
            d.stmts.push_back({r_lhs, ir::LoadVecReg{lhs_xmm_hadd}});
            if (m.mod == 0b11) {
                d.stmts.push_back({r_rhs,
                    ir::LoadVecReg{static_cast<std::uint8_t>(
                        static_cast<unsigned>(m.base))}});
            } else {
                d.stmts.push_back({r_rhs, ir::LoadVec{*r_addr_lo}});
            }
            d.stmts.push_back({r_res,
                ir::VecFpBinOp{ir::VecFpBinOpKind::HAdd, r_lhs, r_rhs, size}});
            d.stmts.push_back({std::nullopt,
                ir::StoreVecReg{static_cast<std::uint8_t>(m.reg), r_res}});
            if (vex.present && vex.L) {
                const ir::Ref r_lhs_hi = next_ref++;
                const ir::Ref r_rhs_hi = next_ref++;
                const ir::Ref r_res_hi = next_ref++;
                d.stmts.push_back({r_lhs_hi, ir::LoadVecRegHi{lhs_xmm_hadd}});
                if (m.mod == 0b11) {
                    d.stmts.push_back({r_rhs_hi,
                        ir::LoadVecRegHi{static_cast<std::uint8_t>(
                            static_cast<unsigned>(m.base))}});
                } else {
                    const ir::Ref r_off16 = next_ref++;
                    const ir::Ref r_addr_hi = next_ref++;
                    d.stmts.push_back({r_off16,
                        ir::Constant{16ULL, ir::OpSize::I64}});
                    d.stmts.push_back({r_addr_hi,
                        ir::BinOp{ir::BinOpKind::Add, *r_addr_lo, r_off16,
                                  ir::OpSize::I64}});
                    d.stmts.push_back({r_rhs_hi, ir::LoadVec{r_addr_hi}});
                }
                d.stmts.push_back({r_res_hi,
                    ir::VecFpBinOp{ir::VecFpBinOpKind::HAdd, r_lhs_hi, r_rhs_hi, size}});
                d.stmts.push_back({std::nullopt,
                    ir::StoreVecRegHi{static_cast<std::uint8_t>(m.reg), r_res_hi}});
            }
            d.bytes_consumed = cursor;
            return d;
        }

        // F2-IR-044/045: POPCNT / LZCNT / TZCNT (F3 0F B8/BC/BD).
        //   F3 0F B8 /r — POPCNT
        //   F3 0F BC /r — TZCNT (BMI1)
        //   F3 0F BD /r — LZCNT (BMI1)
        if (has_f3 && !has_lock && !has_f2 && !has_operand_size_override &&
            (subop == 0xB8u || subop == 0xBCu || subop == 0xBDu)) {
            auto modrm = parse_modrm(bytes, cursor, rex,
                                     has_address_size_override);
            if (std::holds_alternative<DecodeError>(modrm)) {
                return std::get<DecodeError>(modrm);
            }
            const auto& m = std::get<ModRmOperand>(modrm);
            if (m.mod != 0b11) return DecodeError::UnsupportedEncoding;
            const ir::OpSize size = rex.w ? ir::OpSize::I64 : ir::OpSize::I32;
            const ir::Gpr dst_gpr = static_cast<ir::Gpr>(m.reg);
            Decoded d;
            const ir::Ref r_src = next_ref++;
            const ir::Ref r_res = next_ref++;
            d.stmts.push_back({r_src, ir::LoadReg{m.base, size}});
            if (subop == 0xB8u) {
                d.stmts.push_back({r_res, ir::Popcnt{r_src, size}});
            } else if (subop == 0xBDu) {
                d.stmts.push_back({r_res, ir::Lzcnt{r_src, size}});
            } else {
                d.stmts.push_back({r_res, ir::Tzcnt{r_src, size}});
            }
            d.stmts.push_back({std::nullopt,
                ir::StoreReg{dst_gpr, r_res, size}});
            d.bytes_consumed = cursor;
            return d;
        }

        // F2-IR-016: scalar int ↔ FP conversions, register-direct only.
        //   F3 0F 2A /r — CVTSI2SS xmm, r/m32   (REX.W → r/m64)
        //   F2 0F 2A /r — CVTSI2SD
        //   F3 0F 2C /r — CVTTSS2SI r32, xmm    (truncating; REX.W → r64)
        //   F2 0F 2C /r — CVTTSD2SI
        if ((has_f2 || has_f3) && !has_lock && !has_operand_size_override &&
            !(has_f2 && has_f3) &&
            (subop == 0x2Au || subop == 0x2Cu)) {
            auto modrm = parse_modrm(bytes, cursor, rex,
                                     has_address_size_override);
            if (std::holds_alternative<DecodeError>(modrm)) {
                return std::get<DecodeError>(modrm);
            }
            const auto& m = std::get<ModRmOperand>(modrm);
            if (m.mod != 0b11) return DecodeError::UnsupportedEncoding;
            const ir::OpSize int_sz =
                rex.w ? ir::OpSize::I64 : ir::OpSize::I32;
            const ir::FpSize fp_sz =
                has_f3 ? ir::FpSize::F32 : ir::FpSize::F64;
            Decoded d;
            if (subop == 0x2Au) {
                // int → FP: read GPR (m.base), produce 128-bit, store xmm[m.reg].
                const ir::Ref r_int = next_ref++;
                const ir::Ref r_xmm = next_ref++;
                d.stmts.push_back({r_int, ir::LoadReg{m.base, int_sz}});
                d.stmts.push_back({r_xmm,
                    ir::IntToFpScalar{r_int, int_sz, fp_sz}});
                d.stmts.push_back({std::nullopt,
                    ir::StoreVecReg{static_cast<std::uint8_t>(m.reg), r_xmm}});
            } else {
                // FP → int (truncate). Encoding: /r reg is GPR DEST,
                // rm is XMM SOURCE. m.reg gives the reg field index;
                // we need to convert it to a GPR enum. m.base names
                // the rm-as-GPR but here rm is actually an XMM index —
                // ir::Gpr happens to be 0..15 in the same order as
                // raw rm so we extract the underlying index.
                const ir::Gpr dst_gpr = static_cast<ir::Gpr>(m.reg);
                const unsigned src_xmm = static_cast<unsigned>(m.base);
                const ir::Ref r_xmm = next_ref++;
                const ir::Ref r_int = next_ref++;
                d.stmts.push_back({r_xmm,
                    ir::LoadVecReg{static_cast<std::uint8_t>(src_xmm)}});
                d.stmts.push_back({r_int,
                    ir::FpToIntScalar{r_xmm, fp_sz, int_sz}});
                d.stmts.push_back({std::nullopt,
                    ir::StoreReg{dst_gpr, r_int, int_sz}});
            }
            d.bytes_consumed = cursor;
            return d;
        }

        // F2-IR-008: MOVD / MOVQ between xmm and r/m32-or-r/m64.
        //   66 0F 6E /r — MOVD xmm, r/m32   (REX.W → MOVQ xmm, r/m64)
        //   66 0F 7E /r — MOVD r/m32, xmm   (REX.W → MOVQ r/m64, xmm)
        // Register-direct only (mod==11) for now; memory form deferred.
        if (has_operand_size_override && !has_lock && !has_f2 && !has_f3 &&
            (subop == 0x6Eu || subop == 0x7Eu)) {
            auto modrm = parse_modrm(bytes, cursor, rex,
                                     has_address_size_override);
            if (std::holds_alternative<DecodeError>(modrm)) {
                return std::get<DecodeError>(modrm);
            }
            const auto& m = std::get<ModRmOperand>(modrm);
            if (m.mod != 0b11) {
                return DecodeError::UnsupportedEncoding;
            }
            const ir::OpSize size = rex.w ? ir::OpSize::I64 : ir::OpSize::I32;
            Decoded d;
            if (subop == 0x6Eu) {
                // GPR -> XMM (load form).
                const ir::Ref r_gpr = next_ref++;
                const ir::Ref r_xmm = next_ref++;
                d.stmts.push_back({r_gpr, ir::LoadReg{m.base, size}});
                d.stmts.push_back({r_xmm, ir::XmmFromGpr{r_gpr, size}});
                d.stmts.push_back({std::nullopt,
                    ir::StoreVecReg{static_cast<std::uint8_t>(m.reg), r_xmm}});
            } else {
                // XMM -> GPR (store form): 7E moves xmm[reg] low lane
                // into r/m operand. With mod==11 the destination is a
                // register; m.base names the GPR.
                const ir::Ref r_xmm = next_ref++;
                const ir::Ref r_gpr = next_ref++;
                d.stmts.push_back({r_xmm,
                    ir::LoadVecReg{static_cast<std::uint8_t>(m.reg)}});
                d.stmts.push_back({r_gpr, ir::GprFromXmm{r_xmm, size}});
                d.stmts.push_back({std::nullopt,
                    ir::StoreReg{m.base, r_gpr, size}});
            }
            d.bytes_consumed = cursor;
            return d;
        }

        // F2-IR-035: LDDQU (F2 0F F0 mem→reg, SSE3) + non-temporal stores
        // MOVNTDQ (66 0F E7), MOVNTPS (0F 2B), MOVNTPD (66 0F 2B). On ARM64
        // we have no non-temporal hint — treat them as plain 128-bit
        // mov to memory or load.
        const bool is_lddqu      = has_f2 && !has_lock && !has_f3 &&
                                   !has_operand_size_override && subop == 0xF0u;
        const bool is_movntdq    = has_operand_size_override && !has_lock &&
                                   !has_f2 && !has_f3 && subop == 0xE7u;
        const bool is_movntps    = !has_operand_size_override && !has_lock &&
                                   !has_f2 && !has_f3 && subop == 0x2Bu;
        const bool is_movntpd    = has_operand_size_override && !has_lock &&
                                   !has_f2 && !has_f3 && subop == 0x2Bu;
        if (is_lddqu || is_movntdq || is_movntps || is_movntpd) {
            auto modrm = parse_modrm(bytes, cursor, rex,
                                     has_address_size_override);
            if (std::holds_alternative<DecodeError>(modrm)) {
                return std::get<DecodeError>(modrm);
            }
            const auto& m = std::get<ModRmOperand>(modrm);
            // LDDQU: only mem→reg form. MOVNT*: only reg→mem form.
            if (m.mod == 0b11) return DecodeError::UnsupportedEncoding;
            const bool is_load = is_lddqu;
            Decoded d;
            const ir::Ref r_addr = emit_address(d.stmts, m, next_ref,
                                                instruction_guest_pc + cursor);
            if (is_load) {
                const ir::Ref r_src = next_ref++;
                d.stmts.push_back({r_src, ir::LoadVec{r_addr}});
                d.stmts.push_back({std::nullopt,
                    ir::StoreVecReg{static_cast<std::uint8_t>(m.reg), r_src}});
            } else {
                const ir::Ref r_val = next_ref++;
                d.stmts.push_back({r_val,
                    ir::LoadVecReg{static_cast<std::uint8_t>(m.reg)}});
                d.stmts.push_back({std::nullopt, ir::StoreVec{r_addr, r_val}});
            }
            d.bytes_consumed = cursor;
            return d;
        }

        // F2-IR-004 / F2-IR-013: 128-bit register-or-memory MOV family.
        //   MOVDQA  66 0F 6F/7F   (integer, aligned)
        //   MOVDQU  F3 0F 6F/7F   (integer, unaligned)
        //   MOVAPS     0F 28/29   (FP packed single, aligned)
        //   MOVUPS     0F 10/11   (FP packed single, unaligned)
        //   MOVAPD  66 0F 28/29   (FP packed double, aligned)
        //   MOVUPD  66 0F 10/11   (FP packed double, unaligned)
        // All decode to the same LoadVecReg / LoadVec / StoreVecReg /
        // StoreVec primitives; the alignment distinction is irrelevant
        // on ARM64 hosts we target.
        const bool is_mov_28_29 =
            (subop == 0x28u || subop == 0x29u) &&
            !has_lock && !has_f2 && !has_f3;
        const bool is_mov_10_11 =
            (subop == 0x10u || subop == 0x11u) &&
            !has_lock && !has_f2 && !has_f3;
        if (((has_operand_size_override || has_f3) && !has_lock && !has_f2 &&
            !(has_operand_size_override && has_f3) &&
            (subop == 0x6Fu || subop == 0x7Fu))
            || is_mov_28_29 || is_mov_10_11) {
            auto modrm = parse_modrm(bytes, cursor, rex,
                                     has_address_size_override);
            if (std::holds_alternative<DecodeError>(modrm)) {
                return std::get<DecodeError>(modrm);
            }
            const auto& m = std::get<ModRmOperand>(modrm);
            Decoded d;
            const bool is_load = (subop == 0x6Fu) || (subop == 0x28u) || (subop == 0x10u);
            if (m.mod == 0b11) {
                const unsigned src_xmm =
                    is_load ? static_cast<unsigned>(m.base) : m.reg;
                const unsigned dst_xmm =
                    is_load ? m.reg : static_cast<unsigned>(m.base);
                const ir::Ref r_src = next_ref++;
                d.stmts.push_back({r_src,
                    ir::LoadVecReg{static_cast<std::uint8_t>(src_xmm)}});
                d.stmts.push_back({std::nullopt,
                    ir::StoreVecReg{static_cast<std::uint8_t>(dst_xmm), r_src}});
            } else if (is_load) {
                // 6F: load 128 bits from memory into xmm[reg].
                const ir::Ref r_addr = emit_address(d.stmts, m, next_ref,
                                                    instruction_guest_pc + cursor);
                const ir::Ref r_src = next_ref++;
                d.stmts.push_back({r_src, ir::LoadVec{r_addr}});
                d.stmts.push_back({std::nullopt,
                    ir::StoreVecReg{static_cast<std::uint8_t>(m.reg), r_src}});
            } else {
                // 7F: store xmm[reg] to memory.
                const ir::Ref r_addr = emit_address(d.stmts, m, next_ref,
                                                    instruction_guest_pc + cursor);
                const ir::Ref r_val = next_ref++;
                d.stmts.push_back({r_val,
                    ir::LoadVecReg{static_cast<std::uint8_t>(m.reg)}});
                d.stmts.push_back({std::nullopt, ir::StoreVec{r_addr, r_val}});
            }
            d.bytes_consumed = cursor;
            return d;
        }

        // F2-IR-005: SSE/SSE2 packed-FP arithmetic, register-direct.
        //   0F 58 /r — ADDPS xmm1, xmm2   (S4 = 4×f32)
        //   0F 59 /r — MULPS
        //   0F 5C /r — SUBPS
        //   0F 5E /r — DIVPS
        //   66 0F 58 /r — ADDPD xmm1, xmm2  (D2 = 2×f64)
        //   66 0F 59 /r — MULPD
        //   66 0F 5C /r — SUBPD
        //   66 0F 5E /r — DIVPD
        // Memory forms (mod != 11) deferred. F2/F3 prefixes select scalar
        // (ADDSS/ADDSD) and are left for a follow-up.
        if (!has_lock && !has_f2 && !has_f3 &&
            (subop == 0x58u || subop == 0x59u ||
             subop == 0x5Cu || subop == 0x5Eu ||
             subop == 0x51u ||                    // SQRTPS/PD
             subop == 0x5Du || subop == 0x5Fu     // MINPS/PD, MAXPS/PD
            )) {
            auto modrm = parse_modrm(bytes, cursor, rex,
                                     has_address_size_override);
            if (std::holds_alternative<DecodeError>(modrm)) {
                return std::get<DecodeError>(modrm);
            }
            const auto& m = std::get<ModRmOperand>(modrm);
            ir::VecFpBinOpKind vop = ir::VecFpBinOpKind::Add;
            switch (subop) {
                case 0x58u: vop = ir::VecFpBinOpKind::Add;  break;
                case 0x59u: vop = ir::VecFpBinOpKind::Mul;  break;
                case 0x5Cu: vop = ir::VecFpBinOpKind::Sub;  break;
                case 0x5Eu: vop = ir::VecFpBinOpKind::Div;  break;
                case 0x5Du: vop = ir::VecFpBinOpKind::Min;  break;
                case 0x5Fu: vop = ir::VecFpBinOpKind::Max;  break;
                case 0x51u: vop = ir::VecFpBinOpKind::Sqrt; break;
                default: break;
            }
            const ir::VecFpSize size = has_operand_size_override
                                           ? ir::VecFpSize::D2
                                           : ir::VecFpSize::S4;
            Decoded d;
            const std::uint8_t lhs_xmm = vex.present
                ? static_cast<std::uint8_t>(vex.vvvv)
                : static_cast<std::uint8_t>(m.reg);
            // Compute the (optional) memory address once, ahead of the
            // load, so both lo and hi halves can reference it when
            // VEX.L=1.
            std::optional<ir::Ref> r_addr_lo;
            if (m.mod != 0b11) {
                r_addr_lo = emit_address(d.stmts, m, next_ref,
                                         instruction_guest_pc + cursor);
            }
            const ir::Ref r_dst_old = next_ref++;
            const ir::Ref r_src     = next_ref++;
            const ir::Ref r_res     = next_ref++;
            d.stmts.push_back({r_dst_old, ir::LoadVecReg{lhs_xmm}});
            if (m.mod == 0b11) {
                d.stmts.push_back({r_src,
                    ir::LoadVecReg{static_cast<std::uint8_t>(
                        static_cast<unsigned>(m.base))}});
            } else {
                d.stmts.push_back({r_src, ir::LoadVec{*r_addr_lo}});
            }
            d.stmts.push_back({r_res,
                ir::VecFpBinOp{vop, r_dst_old, r_src, size}});
            d.stmts.push_back({std::nullopt,
                ir::StoreVecReg{static_cast<std::uint8_t>(m.reg), r_res}});
            // F2-IR-005 — AVX-256: emit the high-lane pair for VEX.L=1.
            // Memory high-lane is at addr+16; reg high-lane comes from
            // ymm_hi[m.base]. The vvvv register is the same xmm/ymm
            // index for both halves.
            if (vex.present && vex.L) {
                const ir::Ref r_dst_hi_old = next_ref++;
                const ir::Ref r_src_hi     = next_ref++;
                const ir::Ref r_res_hi     = next_ref++;
                d.stmts.push_back({r_dst_hi_old, ir::LoadVecRegHi{lhs_xmm}});
                if (m.mod == 0b11) {
                    d.stmts.push_back({r_src_hi,
                        ir::LoadVecRegHi{static_cast<std::uint8_t>(
                            static_cast<unsigned>(m.base))}});
                } else {
                    const ir::Ref r_off16 = next_ref++;
                    const ir::Ref r_addr_hi = next_ref++;
                    d.stmts.push_back({r_off16,
                        ir::Constant{16ULL, ir::OpSize::I64}});
                    d.stmts.push_back({r_addr_hi,
                        ir::BinOp{ir::BinOpKind::Add, *r_addr_lo, r_off16,
                                  ir::OpSize::I64}});
                    d.stmts.push_back({r_src_hi, ir::LoadVec{r_addr_hi}});
                }
                d.stmts.push_back({r_res_hi,
                    ir::VecFpBinOp{vop, r_dst_hi_old, r_src_hi, size}});
                d.stmts.push_back({std::nullopt,
                    ir::StoreVecRegHi{static_cast<std::uint8_t>(m.reg), r_res_hi}});
            }
            d.bytes_consumed = cursor;
            return d;
        }

        // F2-IR-006: SSE/SSE2 scalar-FP arithmetic, register-direct.
        //   F3 0F 58 /r — ADDSS xmm1, xmm2   (scalar f32; upper preserved)
        //   F3 0F 59 /r — MULSS
        //   F3 0F 5C /r — SUBSS
        //   F3 0F 5E /r — DIVSS
        //   F2 0F 58 /r — ADDSD xmm1, xmm2   (scalar f64)
        //   F2 0F 59 /r — MULSD
        //   F2 0F 5C /r — SUBSD
        //   F2 0F 5E /r — DIVSD
        if ((has_f2 || has_f3) && !has_lock && !has_operand_size_override &&
            !(has_f2 && has_f3) &&
            (subop == 0x58u || subop == 0x59u ||
             subop == 0x5Cu || subop == 0x5Eu ||
             subop == 0x5Du || subop == 0x5Fu ||  // MINSS/MINSD, MAXSS/MAXSD
             subop == 0x51u                       // SQRTSS / SQRTSD
            )) {
            auto modrm = parse_modrm(bytes, cursor, rex,
                                     has_address_size_override);
            if (std::holds_alternative<DecodeError>(modrm)) {
                return std::get<DecodeError>(modrm);
            }
            const auto& m = std::get<ModRmOperand>(modrm);
            ir::VecFpBinOpKind vop = ir::VecFpBinOpKind::Add;
            switch (subop) {
                case 0x58u: vop = ir::VecFpBinOpKind::Add; break;
                case 0x59u: vop = ir::VecFpBinOpKind::Mul; break;
                case 0x5Cu: vop = ir::VecFpBinOpKind::Sub; break;
                case 0x5Eu: vop = ir::VecFpBinOpKind::Div; break;
                case 0x5Du: vop = ir::VecFpBinOpKind::Min; break;
                case 0x5Fu: vop = ir::VecFpBinOpKind::Max; break;
                case 0x51u: vop = ir::VecFpBinOpKind::Sqrt; break;
                default: break;
            }
            const ir::FpSize size = has_f3 ? ir::FpSize::F32 : ir::FpSize::F64;
            Decoded d;
            const std::uint8_t lhs_xmm = vex.present
                ? static_cast<std::uint8_t>(vex.vvvv)
                : static_cast<std::uint8_t>(m.reg);
            const ir::Ref r_dst_old = next_ref++;
            const ir::Ref r_src     = next_ref++;
            const ir::Ref r_res     = next_ref++;
            d.stmts.push_back({r_dst_old, ir::LoadVecReg{lhs_xmm}});
            if (m.mod == 0b11) {
                d.stmts.push_back({r_src,
                    ir::LoadVecReg{static_cast<std::uint8_t>(
                        static_cast<unsigned>(m.base))}});
            } else {
                const ir::Ref r_addr = emit_address(d.stmts, m, next_ref,
                                                    instruction_guest_pc + cursor);
                d.stmts.push_back({r_src, ir::LoadVec{r_addr}});
            }
            d.stmts.push_back({r_res,
                ir::VecFpScalarBinOp{vop, r_dst_old, r_src, size}});
            d.stmts.push_back({std::nullopt,
                ir::StoreVecReg{static_cast<std::uint8_t>(m.reg), r_res}});
            d.bytes_consumed = cursor;
            return d;
        }

        // We currently consume LOCK/HLE only for the exchange-family atomics
        // added in this slice. Everywhere else they remain unsupported.
        if ((has_lock || has_f2) &&
            subop != 0xB1u &&
            subop != 0xC1u &&
            subop != 0xC7u) {
            return DecodeError::UnsupportedEncoding;
        }
        if (auto decoded = try_decode_two_byte_dispatch(
                subop,
                bytes,
                cursor,
                rex,
                has_operand_size_override,
                has_address_size_override,
                has_f2,
                has_f3,
                has_lock,
                instruction_guest_pc,
                next_ref)) {
            return *decoded;
        }
        return DecodeError::UnknownOpcode;
    }

    // --- MOVSXD r64, r/m32 (63 /r) --------------------------------------
    if (opcode == 0x63u) {
        // MOVSXD r64, r/m32 (sign-extend 32-bit source to 64).
        if (!rex.w) return DecodeError::UnsupportedEncoding;
        if (has_operand_size_override) return DecodeError::UnsupportedEncoding;
        if (has_f3) return DecodeError::UnsupportedEncoding;
        return decode_movx_r64_rm(
            bytes, cursor, rex, instruction_guest_pc, has_address_size_override, next_ref, ir::OpSize::I32, true);
    }

    // --- Table-driven 1-byte opcode families ----------------------------
    if (auto decoded = try_decode_one_byte_dispatch(
            opcode,
            bytes,
            cursor,
            rex,
            has_operand_size_override,
            has_address_size_override,
            has_f3,
            instruction_guest_pc,
            next_ref)) {
        return *decoded;
    }

    return DecodeError::UnknownOpcode;
}

}  // namespace prisma::decoder
