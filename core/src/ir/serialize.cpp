// core/src/ir/serialize.cpp — IR ↔ compact binary stream.
//
// Implementation of `prisma/ir_serialize.hpp`. The on-disk layout is
// specified by RFC 0009. Two invariants are load-bearing:
//
//   1. OpKind tags are STABLE — once assigned, never reused. The
//      switch in `decode_op` and the `op_kind_for` helper below are
//      the canonical table; bumping `kSerializeVersion` is mandatory
//      before retiring or repurposing a tag.
//   2. Every integer is little-endian, every payload is fixed-shape
//      per OpKind. There is no implicit padding — every byte written
//      is read back.
//
// Style: keep the writer / reader symmetric and boring. Each variant
// gets one `write_*` and one `read_*` block. Avoid clever
// metaprogramming; future maintainers and the Lean spec are easier
// to keep in sync with explicit code.

#include "prisma/ir_serialize.hpp"

#include <cstdint>
#include <cstring>
#include <type_traits>
#include <variant>

#include "serialize_crc.hpp"

namespace prisma::ir {

namespace {

// ---- OpKind tag table -------------------------------------------------
// 1-based, dense, NEVER reused. See RFC 0009.

enum class OpKind : std::uint8_t {
    kReserved    = 0,
    kConstant    = 1,
    kLoadReg     = 2,
    kStoreReg    = 3,
    kLoadSegBase = 4,
    kBinOp       = 5,
    kCompare     = 6,
    kSelect      = 7,
    kLoadMem     = 8,
    kStoreMem    = 9,
    kLoadMemTSO  = 10,
    kStoreMemTSO = 11,
    kJump        = 12,
    kCondJump    = 13,
    kReturn      = 14,
    kJumpReg     = 15,
    kCmpFlags    = 16,
    kJumpRel     = 17,
    kCondJumpRel = 18,
    kCallRel     = 19,
    kCallReg     = 20,
    kRetAdjusted = 21,
    kCpuid       = 22,
    kSyscall     = 23,
    kTrap        = 24,
    kExtend      = 25,
    kTruncate    = 26,
    kFence       = 27,
    kGuestPc     = 28,
    kInlineAsm   = 29,
    kFpConstant  = 30,
    kFpBinOp     = 31,
    kWriteFlags  = 32,
    kReadFlag    = 33,
    kCondJumpFlags = 34,
    kRspAdjust   = 35,
    kVecConstant = 36,
    kVecBinOp    = 37,
    kLoadVecReg  = 38,
    kStoreVecReg = 39,
    kVecFpBinOp  = 40,
    kVecFpScalarBinOp = 41,
    kLoadVec     = 42,
    kStoreVec    = 43,
    kXmmFromGpr  = 44,
    kGprFromXmm  = 45,
    kVecCmp      = 46,
    kVecShuffle32x4 = 47,
    kVecUnpack     = 48,
    kVecShiftImm   = 49,
    kVecShiftBytes = 50,
    kIntToFpScalar = 51,
    kFpToIntScalar = 52,
    kFpCvtScalar = 53,
    kVecShuffle2Src = 54,
    kVecInsertLane = 55,
    kVecExtractLaneU = 56,
};

// Highest tag the current version knows about. Anything higher in a
// stream → `UnknownOpKind`.
constexpr std::uint8_t kMaxOpKind = static_cast<std::uint8_t>(OpKind::kVecExtractLaneU);

// ---- Little-endian writers --------------------------------------------

void put_u8(std::vector<std::uint8_t>& out, std::uint8_t v) {
    out.push_back(v);
}

void put_u16(std::vector<std::uint8_t>& out, std::uint16_t v) {
    out.push_back(static_cast<std::uint8_t>(v));
    out.push_back(static_cast<std::uint8_t>(v >> 8));
}

void put_u32(std::vector<std::uint8_t>& out, std::uint32_t v) {
    out.push_back(static_cast<std::uint8_t>(v));
    out.push_back(static_cast<std::uint8_t>(v >> 8));
    out.push_back(static_cast<std::uint8_t>(v >> 16));
    out.push_back(static_cast<std::uint8_t>(v >> 24));
}

void put_u64(std::vector<std::uint8_t>& out, std::uint64_t v) {
    for (int i = 0; i < 8; ++i) {
        out.push_back(static_cast<std::uint8_t>(v >> (8 * i)));
    }
}

// ---- Little-endian readers (cursor-based) -----------------------------

struct Cursor {
    std::span<const std::uint8_t> data;
    std::size_t                   pos{0};

    [[nodiscard]] bool remaining(std::size_t n) const noexcept {
        return pos + n <= data.size();
    }

    [[nodiscard]] std::uint8_t take_u8() noexcept {
        return data[pos++];
    }

    [[nodiscard]] std::uint16_t take_u16() noexcept {
        std::uint16_t v = static_cast<std::uint16_t>(data[pos])
                        | static_cast<std::uint16_t>(
                              static_cast<std::uint16_t>(data[pos + 1]) << 8);
        pos += 2;
        return v;
    }

    [[nodiscard]] std::uint32_t take_u32() noexcept {
        std::uint32_t v = static_cast<std::uint32_t>(data[pos])
                        | (static_cast<std::uint32_t>(data[pos + 1]) << 8)
                        | (static_cast<std::uint32_t>(data[pos + 2]) << 16)
                        | (static_cast<std::uint32_t>(data[pos + 3]) << 24);
        pos += 4;
        return v;
    }

    [[nodiscard]] std::uint64_t take_u64() noexcept {
        std::uint64_t v = 0;
        for (int i = 0; i < 8; ++i) {
            v |= static_cast<std::uint64_t>(data[pos + static_cast<std::size_t>(i)])
              << (8 * i);
        }
        pos += 8;
        return v;
    }
};

// ---- Enum range guards -----------------------------------------------

[[nodiscard]] bool is_valid_size(std::uint8_t v) noexcept {
    return v <= static_cast<std::uint8_t>(OpSize::I64);
}
[[nodiscard]] bool is_valid_gpr(std::uint8_t v) noexcept {
    return v < kGprCount;
}
[[nodiscard]] bool is_valid_seg(std::uint8_t v) noexcept {
    return v <= static_cast<std::uint8_t>(SegmentReg::Gs);
}
[[nodiscard]] bool is_valid_binop(std::uint8_t v) noexcept {
    return v <= static_cast<std::uint8_t>(BinOpKind::Rcr);
}
[[nodiscard]] bool is_valid_cc(std::uint8_t v) noexcept {
    return v <= static_cast<std::uint8_t>(CondCode::Pl);
}
[[nodiscard]] bool is_valid_trap(std::uint8_t v) noexcept {
    return v <= static_cast<std::uint8_t>(TrapKind::Sigfpe);
}
[[nodiscard]] bool is_valid_fence(std::uint8_t v) noexcept {
    return v <= static_cast<std::uint8_t>(FenceKind::Sfence);
}
[[nodiscard]] bool is_valid_veclane(std::uint8_t v) noexcept {
    return v <= static_cast<std::uint8_t>(VecLane::D2);
}
[[nodiscard]] bool is_valid_vecbinop(std::uint8_t v) noexcept {
    return v <= static_cast<std::uint8_t>(VecBinOpKind::UMulHi);
}
[[nodiscard]] bool is_valid_vecfpbinop(std::uint8_t v) noexcept {
    return v <= static_cast<std::uint8_t>(VecFpBinOpKind::Sqrt);
}
[[nodiscard]] bool is_valid_vecfpsize(std::uint8_t v) noexcept {
    return v <= static_cast<std::uint8_t>(VecFpSize::D2);
}
[[nodiscard]] bool is_valid_fpsize(std::uint8_t v) noexcept {
    return v <= static_cast<std::uint8_t>(FpSize::F64);
}
[[nodiscard]] bool is_valid_fpbinop(std::uint8_t v) noexcept {
    return v <= static_cast<std::uint8_t>(FpBinOpKind::Div);
}
[[nodiscard]] bool is_valid_flagbit(std::uint8_t v) noexcept {
    return v <= static_cast<std::uint8_t>(FlagBit::Aux);
}

// ---- OpKind dispatch on serialize side --------------------------------

[[nodiscard]] OpKind op_kind_for(const Op& op) noexcept {
    return std::visit([](const auto& x) -> OpKind {
        using T = std::decay_t<decltype(x)>;
        if      constexpr (std::is_same_v<T, Constant>)    return OpKind::kConstant;
        else if constexpr (std::is_same_v<T, LoadReg>)     return OpKind::kLoadReg;
        else if constexpr (std::is_same_v<T, StoreReg>)    return OpKind::kStoreReg;
        else if constexpr (std::is_same_v<T, LoadSegBase>) return OpKind::kLoadSegBase;
        else if constexpr (std::is_same_v<T, BinOp>)       return OpKind::kBinOp;
        else if constexpr (std::is_same_v<T, Compare>)     return OpKind::kCompare;
        else if constexpr (std::is_same_v<T, Select>)      return OpKind::kSelect;
        else if constexpr (std::is_same_v<T, LoadMem>)     return OpKind::kLoadMem;
        else if constexpr (std::is_same_v<T, StoreMem>)    return OpKind::kStoreMem;
        else if constexpr (std::is_same_v<T, LoadMemTSO>)  return OpKind::kLoadMemTSO;
        else if constexpr (std::is_same_v<T, StoreMemTSO>) return OpKind::kStoreMemTSO;
        else if constexpr (std::is_same_v<T, Jump>)        return OpKind::kJump;
        else if constexpr (std::is_same_v<T, CondJump>)    return OpKind::kCondJump;
        else if constexpr (std::is_same_v<T, Return>)      return OpKind::kReturn;
        else if constexpr (std::is_same_v<T, JumpReg>)     return OpKind::kJumpReg;
        else if constexpr (std::is_same_v<T, CmpFlags>)    return OpKind::kCmpFlags;
        else if constexpr (std::is_same_v<T, JumpRel>)     return OpKind::kJumpRel;
        else if constexpr (std::is_same_v<T, CondJumpRel>) return OpKind::kCondJumpRel;
        else if constexpr (std::is_same_v<T, CallRel>)     return OpKind::kCallRel;
        else if constexpr (std::is_same_v<T, CallReg>)     return OpKind::kCallReg;
        else if constexpr (std::is_same_v<T, RetAdjusted>) return OpKind::kRetAdjusted;
        else if constexpr (std::is_same_v<T, Cpuid>)       return OpKind::kCpuid;
        else if constexpr (std::is_same_v<T, Syscall>)     return OpKind::kSyscall;
        else if constexpr (std::is_same_v<T, Trap>)        return OpKind::kTrap;
        else if constexpr (std::is_same_v<T, Extend>)      return OpKind::kExtend;
        else if constexpr (std::is_same_v<T, Truncate>)    return OpKind::kTruncate;
        else if constexpr (std::is_same_v<T, Fence>)       return OpKind::kFence;
        else if constexpr (std::is_same_v<T, GuestPc>)     return OpKind::kGuestPc;
        else if constexpr (std::is_same_v<T, InlineAsm>)   return OpKind::kInlineAsm;
        else if constexpr (std::is_same_v<T, FpConstant>)  return OpKind::kFpConstant;
        else if constexpr (std::is_same_v<T, FpBinOp>)     return OpKind::kFpBinOp;
        else if constexpr (std::is_same_v<T, WriteFlags>)    return OpKind::kWriteFlags;
        else if constexpr (std::is_same_v<T, ReadFlag>)      return OpKind::kReadFlag;
        else if constexpr (std::is_same_v<T, CondJumpFlags>) return OpKind::kCondJumpFlags;
        else if constexpr (std::is_same_v<T, RspAdjust>)     return OpKind::kRspAdjust;
        else if constexpr (std::is_same_v<T, VecConstant>)   return OpKind::kVecConstant;
        else if constexpr (std::is_same_v<T, VecBinOp>)      return OpKind::kVecBinOp;
        else if constexpr (std::is_same_v<T, LoadVecReg>)    return OpKind::kLoadVecReg;
        else if constexpr (std::is_same_v<T, StoreVecReg>)   return OpKind::kStoreVecReg;
        else if constexpr (std::is_same_v<T, VecFpBinOp>)    return OpKind::kVecFpBinOp;
        else if constexpr (std::is_same_v<T, VecFpScalarBinOp>) return OpKind::kVecFpScalarBinOp;
        else if constexpr (std::is_same_v<T, LoadVec>)       return OpKind::kLoadVec;
        else if constexpr (std::is_same_v<T, StoreVec>)      return OpKind::kStoreVec;
        else if constexpr (std::is_same_v<T, XmmFromGpr>)    return OpKind::kXmmFromGpr;
        else if constexpr (std::is_same_v<T, GprFromXmm>)    return OpKind::kGprFromXmm;
        else if constexpr (std::is_same_v<T, VecCmp>)        return OpKind::kVecCmp;
        else if constexpr (std::is_same_v<T, VecShuffle32x4>) return OpKind::kVecShuffle32x4;
        else if constexpr (std::is_same_v<T, VecUnpack>)     return OpKind::kVecUnpack;
        else if constexpr (std::is_same_v<T, VecShiftImm>)   return OpKind::kVecShiftImm;
        else if constexpr (std::is_same_v<T, VecShiftBytes>) return OpKind::kVecShiftBytes;
        else if constexpr (std::is_same_v<T, IntToFpScalar>) return OpKind::kIntToFpScalar;
        else if constexpr (std::is_same_v<T, FpToIntScalar>) return OpKind::kFpToIntScalar;
        else if constexpr (std::is_same_v<T, FpCvtScalar>)   return OpKind::kFpCvtScalar;
        else if constexpr (std::is_same_v<T, VecShuffle2Src>) return OpKind::kVecShuffle2Src;
        else if constexpr (std::is_same_v<T, VecInsertLane>) return OpKind::kVecInsertLane;
        else if constexpr (std::is_same_v<T, VecExtractLaneU>) return OpKind::kVecExtractLaneU;
    }, op);
}

// ---- Per-variant payload encoders ------------------------------------

void write_payload(std::vector<std::uint8_t>& out, const Constant& x) {
    put_u64(out, x.value);
    put_u8(out, static_cast<std::uint8_t>(x.size));
}
void write_payload(std::vector<std::uint8_t>& out, const LoadReg& x) {
    put_u8(out, static_cast<std::uint8_t>(x.reg));
    put_u8(out, static_cast<std::uint8_t>(x.size));
}
void write_payload(std::vector<std::uint8_t>& out, const StoreReg& x) {
    put_u8(out, static_cast<std::uint8_t>(x.reg));
    put_u32(out, x.value);
    put_u8(out, static_cast<std::uint8_t>(x.size));
}
void write_payload(std::vector<std::uint8_t>& out, const LoadSegBase& x) {
    put_u8(out, static_cast<std::uint8_t>(x.seg));
}
void write_payload(std::vector<std::uint8_t>& out, const BinOp& x) {
    put_u8(out, static_cast<std::uint8_t>(x.op));
    put_u32(out, x.lhs);
    put_u32(out, x.rhs);
    put_u8(out, static_cast<std::uint8_t>(x.size));
}
void write_payload(std::vector<std::uint8_t>& out, const Compare& x) {
    put_u8(out, static_cast<std::uint8_t>(x.cc));
    put_u32(out, x.lhs);
    put_u32(out, x.rhs);
    put_u8(out, static_cast<std::uint8_t>(x.size));
}
void write_payload(std::vector<std::uint8_t>& out, const Select& x) {
    put_u8(out, static_cast<std::uint8_t>(x.cc));
    put_u32(out, x.true_value);
    put_u32(out, x.false_value);
    put_u8(out, static_cast<std::uint8_t>(x.size));
}
void write_payload(std::vector<std::uint8_t>& out, const LoadMem& x) {
    put_u32(out, x.addr);
    put_u8(out, static_cast<std::uint8_t>(x.size));
}
void write_payload(std::vector<std::uint8_t>& out, const StoreMem& x) {
    put_u32(out, x.addr);
    put_u32(out, x.value);
    put_u8(out, static_cast<std::uint8_t>(x.size));
}
void write_payload(std::vector<std::uint8_t>& out, const LoadMemTSO& x) {
    put_u32(out, x.addr);
    put_u8(out, static_cast<std::uint8_t>(x.size));
}
void write_payload(std::vector<std::uint8_t>& out, const StoreMemTSO& x) {
    put_u32(out, x.addr);
    put_u32(out, x.value);
    put_u8(out, static_cast<std::uint8_t>(x.size));
}
void write_payload(std::vector<std::uint8_t>& out, const Jump& x) {
    put_u32(out, x.target_block);
}
void write_payload(std::vector<std::uint8_t>& out, const CondJump& x) {
    put_u32(out, x.cond);
    put_u32(out, x.if_true);
    put_u32(out, x.if_false);
}
void write_payload(std::vector<std::uint8_t>& /*out*/, const Return&) {
    // empty payload
}
void write_payload(std::vector<std::uint8_t>& out, const JumpReg& x) {
    put_u32(out, x.target);
}
void write_payload(std::vector<std::uint8_t>& out, const CmpFlags& x) {
    put_u32(out, x.lhs);
    put_u32(out, x.rhs);
    put_u8(out, static_cast<std::uint8_t>(x.size));
}
void write_payload(std::vector<std::uint8_t>& out, const JumpRel& x) {
    put_u64(out, x.target_guest_pc);
}
void write_payload(std::vector<std::uint8_t>& out, const CondJumpRel& x) {
    put_u8(out, static_cast<std::uint8_t>(x.cc));
    put_u64(out, x.target_guest_pc);
    put_u64(out, x.fallthrough_guest_pc);
}
void write_payload(std::vector<std::uint8_t>& out, const CallRel& x) {
    put_u64(out, x.target_guest_pc);
    put_u64(out, x.return_guest_pc);
}
void write_payload(std::vector<std::uint8_t>& out, const CallReg& x) {
    put_u32(out, x.target);
    put_u64(out, x.return_guest_pc);
}
void write_payload(std::vector<std::uint8_t>& out, const RetAdjusted& x) {
    put_u64(out, x.pop_bytes);
}
void write_payload(std::vector<std::uint8_t>& /*out*/, const Cpuid&) {
    // empty payload
}
void write_payload(std::vector<std::uint8_t>& /*out*/, const Syscall&) {
    // empty payload
}
void write_payload(std::vector<std::uint8_t>& out, const Trap& x) {
    put_u8(out, static_cast<std::uint8_t>(x.kind));
}
void write_payload(std::vector<std::uint8_t>& out, const Extend& x) {
    put_u32(out, x.value);
    put_u8(out, static_cast<std::uint8_t>(x.from_size));
    put_u8(out, static_cast<std::uint8_t>(x.to_size));
    put_u8(out, x.is_signed ? 1u : 0u);
}
void write_payload(std::vector<std::uint8_t>& out, const Truncate& x) {
    put_u32(out, x.value);
    put_u8(out, static_cast<std::uint8_t>(x.to_size));
}
void write_payload(std::vector<std::uint8_t>& out, const Fence& x) {
    put_u8(out, static_cast<std::uint8_t>(x.kind));
}
void write_payload(std::vector<std::uint8_t>& out, const GuestPc& x) {
    put_u64(out, x.pc);
}
void write_payload(std::vector<std::uint8_t>& out, const InlineAsm& x) {
    put_u32(out, static_cast<std::uint32_t>(x.bytes.size()));
    for (std::uint8_t b : x.bytes) put_u8(out, b);
}
void write_payload(std::vector<std::uint8_t>& out, const FpConstant& x) {
    put_u64(out, x.bits);
    put_u8(out, static_cast<std::uint8_t>(x.size));
}
void write_payload(std::vector<std::uint8_t>& out, const FpBinOp& x) {
    put_u8(out, static_cast<std::uint8_t>(x.op));
    put_u32(out, x.lhs);
    put_u32(out, x.rhs);
    put_u8(out, static_cast<std::uint8_t>(x.size));
}
void write_payload(std::vector<std::uint8_t>& out, const WriteFlags& x) {
    put_u8(out, static_cast<std::uint8_t>(x.op));
    put_u32(out, x.lhs);
    put_u32(out, x.rhs);
    put_u8(out, static_cast<std::uint8_t>(x.size));
}
void write_payload(std::vector<std::uint8_t>& out, const ReadFlag& x) {
    put_u32(out, x.flags);
    put_u8(out, static_cast<std::uint8_t>(x.which));
}
void write_payload(std::vector<std::uint8_t>& out, const CondJumpFlags& x) {
    put_u32(out, x.flags);
    put_u8(out, static_cast<std::uint8_t>(x.cc));
    put_u32(out, x.if_true);
    put_u32(out, x.if_false);
}
void write_payload(std::vector<std::uint8_t>& out, const RspAdjust& x) {
    put_u64(out, static_cast<std::uint64_t>(x.delta_bytes));
}
void write_payload(std::vector<std::uint8_t>& out, const VecConstant& x) {
    put_u64(out, x.lo);
    put_u64(out, x.hi);
}
void write_payload(std::vector<std::uint8_t>& out, const VecBinOp& x) {
    put_u8(out, static_cast<std::uint8_t>(x.op));
    put_u32(out, x.lhs);
    put_u32(out, x.rhs);
    put_u8(out, static_cast<std::uint8_t>(x.lane));
}
void write_payload(std::vector<std::uint8_t>& out, const LoadVecReg& x) {
    put_u8(out, x.xmm_index);
}
void write_payload(std::vector<std::uint8_t>& out, const StoreVecReg& x) {
    put_u8(out, x.xmm_index);
    put_u32(out, x.value);
}
void write_payload(std::vector<std::uint8_t>& out, const VecFpBinOp& x) {
    put_u8(out, static_cast<std::uint8_t>(x.op));
    put_u32(out, x.lhs);
    put_u32(out, x.rhs);
    put_u8(out, static_cast<std::uint8_t>(x.size));
}
void write_payload(std::vector<std::uint8_t>& out, const VecFpScalarBinOp& x) {
    put_u8(out, static_cast<std::uint8_t>(x.op));
    put_u32(out, x.lhs);
    put_u32(out, x.rhs);
    put_u8(out, static_cast<std::uint8_t>(x.size));
}
void write_payload(std::vector<std::uint8_t>& out, const LoadVec& x) {
    put_u32(out, x.addr);
}
void write_payload(std::vector<std::uint8_t>& out, const StoreVec& x) {
    put_u32(out, x.addr);
    put_u32(out, x.value);
}
void write_payload(std::vector<std::uint8_t>& out, const XmmFromGpr& x) {
    put_u32(out, x.value);
    put_u8(out, static_cast<std::uint8_t>(x.size));
}
void write_payload(std::vector<std::uint8_t>& out, const GprFromXmm& x) {
    put_u32(out, x.value);
    put_u8(out, static_cast<std::uint8_t>(x.size));
}
void write_payload(std::vector<std::uint8_t>& out, const VecCmp& x) {
    put_u8(out, static_cast<std::uint8_t>(x.kind));
    put_u32(out, x.lhs);
    put_u32(out, x.rhs);
    put_u8(out, static_cast<std::uint8_t>(x.lane));
}
void write_payload(std::vector<std::uint8_t>& out, const VecShuffle32x4& x) {
    put_u32(out, x.src);
    put_u8(out, x.control);
}
void write_payload(std::vector<std::uint8_t>& out, const VecUnpack& x) {
    put_u8(out, x.is_high ? 1u : 0u);
    put_u32(out, x.lhs);
    put_u32(out, x.rhs);
    put_u8(out, static_cast<std::uint8_t>(x.lane));
}
void write_payload(std::vector<std::uint8_t>& out, const VecShiftImm& x) {
    put_u8(out, static_cast<std::uint8_t>(x.kind));
    put_u32(out, x.src);
    put_u8(out, x.count);
    put_u8(out, static_cast<std::uint8_t>(x.lane));
}
void write_payload(std::vector<std::uint8_t>& out, const VecShiftBytes& x) {
    put_u8(out, x.is_left ? 1u : 0u);
    put_u32(out, x.src);
    put_u8(out, x.count);
}
void write_payload(std::vector<std::uint8_t>& out, const IntToFpScalar& x) {
    put_u32(out, x.value);
    put_u8(out, static_cast<std::uint8_t>(x.int_size));
    put_u8(out, static_cast<std::uint8_t>(x.fp_size));
}
void write_payload(std::vector<std::uint8_t>& out, const FpToIntScalar& x) {
    put_u32(out, x.value);
    put_u8(out, static_cast<std::uint8_t>(x.fp_size));
    put_u8(out, static_cast<std::uint8_t>(x.int_size));
}
void write_payload(std::vector<std::uint8_t>& out, const FpCvtScalar& x) {
    put_u32(out, x.lhs);
    put_u32(out, x.src);
    put_u8(out, static_cast<std::uint8_t>(x.src_size));
    put_u8(out, static_cast<std::uint8_t>(x.dst_size));
}
void write_payload(std::vector<std::uint8_t>& out, const VecShuffle2Src& x) {
    put_u8(out, x.is_pd ? 1u : 0u);
    put_u32(out, x.lhs);
    put_u32(out, x.rhs);
    put_u8(out, x.control);
}
void write_payload(std::vector<std::uint8_t>& out, const VecInsertLane& x) {
    put_u32(out, x.lhs_xmm);
    put_u32(out, x.value);
    put_u8(out, x.lane_idx);
    put_u8(out, static_cast<std::uint8_t>(x.lane));
}
void write_payload(std::vector<std::uint8_t>& out, const VecExtractLaneU& x) {
    put_u32(out, x.src_xmm);
    put_u8(out, x.lane_idx);
    put_u8(out, static_cast<std::uint8_t>(x.lane));
}

void write_stmt(std::vector<std::uint8_t>& out, const Stmt& s) {
    if (s.result) {
        put_u8(out, 1u);
        put_u32(out, *s.result);
    } else {
        put_u8(out, 0u);
    }
    put_u8(out, static_cast<std::uint8_t>(op_kind_for(s.op)));
    std::visit([&](const auto& x) { write_payload(out, x); }, s.op);
}

// ---- Per-variant payload decoders. Each returns a DeserializeError;
// on Ok, fills `*out` and advances the cursor. On any error, the
// statement list is dropped at the top level.

DeserializeError read_payload_constant(Cursor& c, Stmt& s) {
    if (!c.remaining(8 + 1)) return DeserializeError::Truncated;
    Constant v;
    v.value = c.take_u64();
    const std::uint8_t sz = c.take_u8();
    if (!is_valid_size(sz)) return DeserializeError::BadSize;
    v.size = static_cast<OpSize>(sz);
    s.op = v;
    return DeserializeError::Ok;
}

DeserializeError read_payload_load_reg(Cursor& c, Stmt& s) {
    if (!c.remaining(2)) return DeserializeError::Truncated;
    const std::uint8_t reg = c.take_u8();
    const std::uint8_t sz  = c.take_u8();
    if (!is_valid_gpr(reg)) return DeserializeError::BadSize;
    if (!is_valid_size(sz)) return DeserializeError::BadSize;
    s.op = LoadReg{static_cast<Gpr>(reg), static_cast<OpSize>(sz)};
    return DeserializeError::Ok;
}

DeserializeError read_payload_store_reg(Cursor& c, Stmt& s) {
    if (!c.remaining(1 + 4 + 1)) return DeserializeError::Truncated;
    const std::uint8_t reg = c.take_u8();
    const std::uint32_t val = c.take_u32();
    const std::uint8_t sz  = c.take_u8();
    if (!is_valid_gpr(reg)) return DeserializeError::BadSize;
    if (!is_valid_size(sz)) return DeserializeError::BadSize;
    s.op = StoreReg{static_cast<Gpr>(reg), val, static_cast<OpSize>(sz)};
    return DeserializeError::Ok;
}

DeserializeError read_payload_load_seg_base(Cursor& c, Stmt& s) {
    if (!c.remaining(1)) return DeserializeError::Truncated;
    const std::uint8_t seg = c.take_u8();
    if (!is_valid_seg(seg)) return DeserializeError::BadSize;
    s.op = LoadSegBase{static_cast<SegmentReg>(seg)};
    return DeserializeError::Ok;
}

DeserializeError read_payload_binop(Cursor& c, Stmt& s) {
    if (!c.remaining(1 + 4 + 4 + 1)) return DeserializeError::Truncated;
    const std::uint8_t op = c.take_u8();
    const std::uint32_t lhs = c.take_u32();
    const std::uint32_t rhs = c.take_u32();
    const std::uint8_t sz  = c.take_u8();
    if (!is_valid_binop(op)) return DeserializeError::BadSize;
    if (!is_valid_size(sz))  return DeserializeError::BadSize;
    s.op = BinOp{static_cast<BinOpKind>(op), lhs, rhs, static_cast<OpSize>(sz)};
    return DeserializeError::Ok;
}

DeserializeError read_payload_compare(Cursor& c, Stmt& s) {
    if (!c.remaining(1 + 4 + 4 + 1)) return DeserializeError::Truncated;
    const std::uint8_t cc = c.take_u8();
    const std::uint32_t lhs = c.take_u32();
    const std::uint32_t rhs = c.take_u32();
    const std::uint8_t sz  = c.take_u8();
    if (!is_valid_cc(cc))   return DeserializeError::BadSize;
    if (!is_valid_size(sz)) return DeserializeError::BadSize;
    s.op = Compare{static_cast<CondCode>(cc), lhs, rhs, static_cast<OpSize>(sz)};
    return DeserializeError::Ok;
}

DeserializeError read_payload_select(Cursor& c, Stmt& s) {
    if (!c.remaining(1 + 4 + 4 + 1)) return DeserializeError::Truncated;
    const std::uint8_t cc = c.take_u8();
    const std::uint32_t tv = c.take_u32();
    const std::uint32_t fv = c.take_u32();
    const std::uint8_t sz  = c.take_u8();
    if (!is_valid_cc(cc))   return DeserializeError::BadSize;
    if (!is_valid_size(sz)) return DeserializeError::BadSize;
    s.op = Select{static_cast<CondCode>(cc), tv, fv, static_cast<OpSize>(sz)};
    return DeserializeError::Ok;
}

DeserializeError read_payload_load_mem(Cursor& c, Stmt& s) {
    if (!c.remaining(4 + 1)) return DeserializeError::Truncated;
    const std::uint32_t addr = c.take_u32();
    const std::uint8_t sz   = c.take_u8();
    if (!is_valid_size(sz)) return DeserializeError::BadSize;
    s.op = LoadMem{addr, static_cast<OpSize>(sz)};
    return DeserializeError::Ok;
}

DeserializeError read_payload_store_mem(Cursor& c, Stmt& s) {
    if (!c.remaining(4 + 4 + 1)) return DeserializeError::Truncated;
    const std::uint32_t addr = c.take_u32();
    const std::uint32_t val  = c.take_u32();
    const std::uint8_t sz   = c.take_u8();
    if (!is_valid_size(sz)) return DeserializeError::BadSize;
    s.op = StoreMem{addr, val, static_cast<OpSize>(sz)};
    return DeserializeError::Ok;
}

DeserializeError read_payload_load_mem_tso(Cursor& c, Stmt& s) {
    if (!c.remaining(4 + 1)) return DeserializeError::Truncated;
    const std::uint32_t addr = c.take_u32();
    const std::uint8_t sz   = c.take_u8();
    if (!is_valid_size(sz)) return DeserializeError::BadSize;
    s.op = LoadMemTSO{addr, static_cast<OpSize>(sz)};
    return DeserializeError::Ok;
}

DeserializeError read_payload_store_mem_tso(Cursor& c, Stmt& s) {
    if (!c.remaining(4 + 4 + 1)) return DeserializeError::Truncated;
    const std::uint32_t addr = c.take_u32();
    const std::uint32_t val  = c.take_u32();
    const std::uint8_t sz   = c.take_u8();
    if (!is_valid_size(sz)) return DeserializeError::BadSize;
    s.op = StoreMemTSO{addr, val, static_cast<OpSize>(sz)};
    return DeserializeError::Ok;
}

DeserializeError read_payload_jump(Cursor& c, Stmt& s) {
    if (!c.remaining(4)) return DeserializeError::Truncated;
    s.op = Jump{c.take_u32()};
    return DeserializeError::Ok;
}

DeserializeError read_payload_cond_jump(Cursor& c, Stmt& s) {
    if (!c.remaining(4 + 4 + 4)) return DeserializeError::Truncated;
    const std::uint32_t cond = c.take_u32();
    const std::uint32_t t    = c.take_u32();
    const std::uint32_t f    = c.take_u32();
    s.op = CondJump{cond, t, f};
    return DeserializeError::Ok;
}

DeserializeError read_payload_return(Cursor& /*c*/, Stmt& s) {
    s.op = Return{};
    return DeserializeError::Ok;
}

DeserializeError read_payload_jump_reg(Cursor& c, Stmt& s) {
    if (!c.remaining(4)) return DeserializeError::Truncated;
    s.op = JumpReg{c.take_u32()};
    return DeserializeError::Ok;
}

DeserializeError read_payload_cmp_flags(Cursor& c, Stmt& s) {
    if (!c.remaining(4 + 4 + 1)) return DeserializeError::Truncated;
    const std::uint32_t lhs = c.take_u32();
    const std::uint32_t rhs = c.take_u32();
    const std::uint8_t sz  = c.take_u8();
    if (!is_valid_size(sz)) return DeserializeError::BadSize;
    s.op = CmpFlags{lhs, rhs, static_cast<OpSize>(sz)};
    return DeserializeError::Ok;
}

DeserializeError read_payload_jump_rel(Cursor& c, Stmt& s) {
    if (!c.remaining(8)) return DeserializeError::Truncated;
    s.op = JumpRel{c.take_u64()};
    return DeserializeError::Ok;
}

DeserializeError read_payload_cond_jump_rel(Cursor& c, Stmt& s) {
    if (!c.remaining(1 + 8 + 8)) return DeserializeError::Truncated;
    const std::uint8_t cc = c.take_u8();
    const std::uint64_t tgt = c.take_u64();
    const std::uint64_t fall = c.take_u64();
    if (!is_valid_cc(cc)) return DeserializeError::BadSize;
    s.op = CondJumpRel{static_cast<CondCode>(cc), tgt, fall};
    return DeserializeError::Ok;
}

DeserializeError read_payload_call_rel(Cursor& c, Stmt& s) {
    if (!c.remaining(8 + 8)) return DeserializeError::Truncated;
    const std::uint64_t tgt = c.take_u64();
    const std::uint64_t ret = c.take_u64();
    s.op = CallRel{tgt, ret};
    return DeserializeError::Ok;
}

DeserializeError read_payload_call_reg(Cursor& c, Stmt& s) {
    if (!c.remaining(4 + 8)) return DeserializeError::Truncated;
    const std::uint32_t tgt = c.take_u32();
    const std::uint64_t ret = c.take_u64();
    s.op = CallReg{tgt, ret};
    return DeserializeError::Ok;
}

DeserializeError read_payload_ret_adjusted(Cursor& c, Stmt& s) {
    if (!c.remaining(8)) return DeserializeError::Truncated;
    s.op = RetAdjusted{c.take_u64()};
    return DeserializeError::Ok;
}

DeserializeError read_payload_cpuid(Cursor& /*c*/, Stmt& s) {
    s.op = Cpuid{};
    return DeserializeError::Ok;
}

DeserializeError read_payload_syscall(Cursor& /*c*/, Stmt& s) {
    s.op = Syscall{};
    return DeserializeError::Ok;
}

DeserializeError read_payload_trap(Cursor& c, Stmt& s) {
    if (!c.remaining(1)) return DeserializeError::Truncated;
    const std::uint8_t k = c.take_u8();
    if (!is_valid_trap(k)) return DeserializeError::BadSize;
    s.op = Trap{static_cast<TrapKind>(k)};
    return DeserializeError::Ok;
}

DeserializeError read_payload_extend(Cursor& c, Stmt& s) {
    if (!c.remaining(4 + 1 + 1 + 1)) return DeserializeError::Truncated;
    const std::uint32_t v  = c.take_u32();
    const std::uint8_t  fs = c.take_u8();
    const std::uint8_t  ts = c.take_u8();
    const std::uint8_t  sg = c.take_u8();
    if (!is_valid_size(fs) || !is_valid_size(ts)) return DeserializeError::BadSize;
    if (sg > 1u) return DeserializeError::BadSize;
    s.op = Extend{v,
                  static_cast<OpSize>(fs),
                  static_cast<OpSize>(ts),
                  sg == 1u};
    return DeserializeError::Ok;
}

DeserializeError read_payload_truncate(Cursor& c, Stmt& s) {
    if (!c.remaining(4 + 1)) return DeserializeError::Truncated;
    const std::uint32_t v  = c.take_u32();
    const std::uint8_t  ts = c.take_u8();
    if (!is_valid_size(ts)) return DeserializeError::BadSize;
    s.op = Truncate{v, static_cast<OpSize>(ts)};
    return DeserializeError::Ok;
}

DeserializeError read_payload_fence(Cursor& c, Stmt& s) {
    if (!c.remaining(1)) return DeserializeError::Truncated;
    const std::uint8_t k = c.take_u8();
    if (!is_valid_fence(k)) return DeserializeError::BadSize;
    s.op = Fence{static_cast<FenceKind>(k)};
    return DeserializeError::Ok;
}

DeserializeError read_payload_guest_pc(Cursor& c, Stmt& s) {
    if (!c.remaining(8)) return DeserializeError::Truncated;
    s.op = GuestPc{c.take_u64()};
    return DeserializeError::Ok;
}

DeserializeError read_payload_inline_asm(Cursor& c, Stmt& s) {
    if (!c.remaining(4)) return DeserializeError::Truncated;
    const std::uint32_t n = c.take_u32();
    if (!c.remaining(n)) return DeserializeError::Truncated;
    std::vector<std::uint8_t> bytes;
    bytes.reserve(n);
    for (std::uint32_t i = 0; i < n; ++i) bytes.push_back(c.take_u8());
    s.op = InlineAsm{std::move(bytes)};
    return DeserializeError::Ok;
}

DeserializeError read_payload_fp_constant(Cursor& c, Stmt& s) {
    if (!c.remaining(8 + 1)) return DeserializeError::Truncated;
    const std::uint64_t bits = c.take_u64();
    const std::uint8_t  sz   = c.take_u8();
    if (!is_valid_fpsize(sz)) return DeserializeError::BadSize;
    s.op = FpConstant{bits, static_cast<FpSize>(sz)};
    return DeserializeError::Ok;
}

DeserializeError read_payload_fp_binop(Cursor& c, Stmt& s) {
    if (!c.remaining(1 + 4 + 4 + 1)) return DeserializeError::Truncated;
    const std::uint8_t  op  = c.take_u8();
    const std::uint32_t lhs = c.take_u32();
    const std::uint32_t rhs = c.take_u32();
    const std::uint8_t  sz  = c.take_u8();
    if (!is_valid_fpbinop(op)) return DeserializeError::BadSize;
    if (!is_valid_fpsize(sz))  return DeserializeError::BadSize;
    s.op = FpBinOp{static_cast<FpBinOpKind>(op), lhs, rhs,
                   static_cast<FpSize>(sz)};
    return DeserializeError::Ok;
}

DeserializeError read_payload_write_flags(Cursor& c, Stmt& s) {
    if (!c.remaining(1 + 4 + 4 + 1)) return DeserializeError::Truncated;
    const std::uint8_t  op  = c.take_u8();
    const std::uint32_t lhs = c.take_u32();
    const std::uint32_t rhs = c.take_u32();
    const std::uint8_t  sz  = c.take_u8();
    if (!is_valid_binop(op)) return DeserializeError::BadSize;
    if (!is_valid_size(sz))  return DeserializeError::BadSize;
    s.op = WriteFlags{static_cast<BinOpKind>(op), lhs, rhs,
                      static_cast<OpSize>(sz)};
    return DeserializeError::Ok;
}

DeserializeError read_payload_read_flag(Cursor& c, Stmt& s) {
    if (!c.remaining(4 + 1)) return DeserializeError::Truncated;
    const std::uint32_t flags = c.take_u32();
    const std::uint8_t  which = c.take_u8();
    if (!is_valid_flagbit(which)) return DeserializeError::BadSize;
    s.op = ReadFlag{flags, static_cast<FlagBit>(which)};
    return DeserializeError::Ok;
}

DeserializeError read_payload_cond_jump_flags(Cursor& c, Stmt& s) {
    if (!c.remaining(4 + 1 + 4 + 4)) return DeserializeError::Truncated;
    const std::uint32_t flags = c.take_u32();
    const std::uint8_t  cc    = c.take_u8();
    const std::uint32_t t     = c.take_u32();
    const std::uint32_t f     = c.take_u32();
    if (!is_valid_cc(cc)) return DeserializeError::BadSize;
    s.op = CondJumpFlags{flags, static_cast<CondCode>(cc), t, f};
    return DeserializeError::Ok;
}

DeserializeError read_payload_rsp_adjust(Cursor& c, Stmt& s) {
    if (!c.remaining(8)) return DeserializeError::Truncated;
    s.op = RspAdjust{static_cast<std::int64_t>(c.take_u64())};
    return DeserializeError::Ok;
}

DeserializeError read_payload_vec_constant(Cursor& c, Stmt& s) {
    if (!c.remaining(16)) return DeserializeError::Truncated;
    const std::uint64_t lo = c.take_u64();
    const std::uint64_t hi = c.take_u64();
    s.op = VecConstant{lo, hi};
    return DeserializeError::Ok;
}

DeserializeError read_payload_vec_binop(Cursor& c, Stmt& s) {
    if (!c.remaining(1 + 4 + 4 + 1)) return DeserializeError::Truncated;
    const std::uint8_t  op   = c.take_u8();
    const std::uint32_t lhs  = c.take_u32();
    const std::uint32_t rhs  = c.take_u32();
    const std::uint8_t  lane = c.take_u8();
    if (!is_valid_vecbinop(op)) return DeserializeError::BadSize;
    if (!is_valid_veclane(lane)) return DeserializeError::BadSize;
    s.op = VecBinOp{static_cast<VecBinOpKind>(op), lhs, rhs,
                    static_cast<VecLane>(lane)};
    return DeserializeError::Ok;
}

DeserializeError read_payload_load_vec_reg(Cursor& c, Stmt& s) {
    if (!c.remaining(1)) return DeserializeError::Truncated;
    const std::uint8_t idx = c.take_u8();
    if (idx >= kXmmCount) return DeserializeError::BadSize;
    s.op = LoadVecReg{idx};
    return DeserializeError::Ok;
}

DeserializeError read_payload_store_vec_reg(Cursor& c, Stmt& s) {
    if (!c.remaining(1 + 4)) return DeserializeError::Truncated;
    const std::uint8_t  idx = c.take_u8();
    const std::uint32_t val = c.take_u32();
    if (idx >= kXmmCount) return DeserializeError::BadSize;
    s.op = StoreVecReg{idx, val};
    return DeserializeError::Ok;
}

DeserializeError read_payload_vec_fp_binop(Cursor& c, Stmt& s) {
    if (!c.remaining(1 + 4 + 4 + 1)) return DeserializeError::Truncated;
    const std::uint8_t  op   = c.take_u8();
    const std::uint32_t lhs  = c.take_u32();
    const std::uint32_t rhs  = c.take_u32();
    const std::uint8_t  size = c.take_u8();
    if (!is_valid_vecfpbinop(op)) return DeserializeError::BadSize;
    if (!is_valid_vecfpsize(size)) return DeserializeError::BadSize;
    s.op = VecFpBinOp{static_cast<VecFpBinOpKind>(op), lhs, rhs,
                      static_cast<VecFpSize>(size)};
    return DeserializeError::Ok;
}

DeserializeError read_payload_vec_insert_lane(Cursor& c, Stmt& s) {
    if (!c.remaining(4 + 4 + 1 + 1)) return DeserializeError::Truncated;
    const std::uint32_t lhs   = c.take_u32();
    const std::uint32_t value = c.take_u32();
    const std::uint8_t  idx   = c.take_u8();
    const std::uint8_t  lane  = c.take_u8();
    if (!is_valid_veclane(lane)) return DeserializeError::BadSize;
    s.op = VecInsertLane{lhs, value, idx, static_cast<VecLane>(lane)};
    return DeserializeError::Ok;
}
DeserializeError read_payload_vec_extract_lane_u(Cursor& c, Stmt& s) {
    if (!c.remaining(4 + 1 + 1)) return DeserializeError::Truncated;
    const std::uint32_t src  = c.take_u32();
    const std::uint8_t  idx  = c.take_u8();
    const std::uint8_t  lane = c.take_u8();
    if (!is_valid_veclane(lane)) return DeserializeError::BadSize;
    s.op = VecExtractLaneU{src, idx, static_cast<VecLane>(lane)};
    return DeserializeError::Ok;
}

DeserializeError read_payload_vec_shuffle_2src(Cursor& c, Stmt& s) {
    if (!c.remaining(1 + 4 + 4 + 1)) return DeserializeError::Truncated;
    const std::uint8_t  is_pd   = c.take_u8();
    const std::uint32_t lhs     = c.take_u32();
    const std::uint32_t rhs     = c.take_u32();
    const std::uint8_t  control = c.take_u8();
    if (is_pd > 1) return DeserializeError::BadSize;
    s.op = VecShuffle2Src{is_pd == 1, lhs, rhs, control};
    return DeserializeError::Ok;
}

DeserializeError read_payload_fp_cvt_scalar(Cursor& c, Stmt& s) {
    if (!c.remaining(4 + 4 + 1 + 1)) return DeserializeError::Truncated;
    const std::uint32_t lhs = c.take_u32();
    const std::uint32_t src = c.take_u32();
    const std::uint8_t  ss  = c.take_u8();
    const std::uint8_t  ds  = c.take_u8();
    if (!is_valid_fpsize(ss) || !is_valid_fpsize(ds)) return DeserializeError::BadSize;
    s.op = FpCvtScalar{lhs, src, static_cast<FpSize>(ss), static_cast<FpSize>(ds)};
    return DeserializeError::Ok;
}

DeserializeError read_payload_int_to_fp_scalar(Cursor& c, Stmt& s) {
    if (!c.remaining(4 + 1 + 1)) return DeserializeError::Truncated;
    const std::uint32_t v = c.take_u32();
    const std::uint8_t isz = c.take_u8();
    const std::uint8_t fsz = c.take_u8();
    if (!is_valid_size(isz)) return DeserializeError::BadSize;
    if (!is_valid_fpsize(fsz)) return DeserializeError::BadSize;
    s.op = IntToFpScalar{v, static_cast<OpSize>(isz), static_cast<FpSize>(fsz)};
    return DeserializeError::Ok;
}
DeserializeError read_payload_fp_to_int_scalar(Cursor& c, Stmt& s) {
    if (!c.remaining(4 + 1 + 1)) return DeserializeError::Truncated;
    const std::uint32_t v = c.take_u32();
    const std::uint8_t fsz = c.take_u8();
    const std::uint8_t isz = c.take_u8();
    if (!is_valid_fpsize(fsz)) return DeserializeError::BadSize;
    if (!is_valid_size(isz)) return DeserializeError::BadSize;
    s.op = FpToIntScalar{v, static_cast<FpSize>(fsz), static_cast<OpSize>(isz)};
    return DeserializeError::Ok;
}

DeserializeError read_payload_vec_shift_bytes(Cursor& c, Stmt& s) {
    if (!c.remaining(1 + 4 + 1)) return DeserializeError::Truncated;
    const std::uint8_t  is_left = c.take_u8();
    const std::uint32_t src     = c.take_u32();
    const std::uint8_t  count   = c.take_u8();
    if (is_left > 1) return DeserializeError::BadSize;
    s.op = VecShiftBytes{is_left == 1, src, count};
    return DeserializeError::Ok;
}

DeserializeError read_payload_vec_unpack(Cursor& c, Stmt& s) {
    if (!c.remaining(1 + 4 + 4 + 1)) return DeserializeError::Truncated;
    const std::uint8_t  is_high = c.take_u8();
    const std::uint32_t lhs     = c.take_u32();
    const std::uint32_t rhs     = c.take_u32();
    const std::uint8_t  lane    = c.take_u8();
    if (is_high > 1) return DeserializeError::BadSize;
    if (!is_valid_veclane(lane)) return DeserializeError::BadSize;
    s.op = VecUnpack{is_high == 1, lhs, rhs, static_cast<VecLane>(lane)};
    return DeserializeError::Ok;
}
DeserializeError read_payload_vec_shift_imm(Cursor& c, Stmt& s) {
    if (!c.remaining(1 + 4 + 1 + 1)) return DeserializeError::Truncated;
    const std::uint8_t  kind  = c.take_u8();
    const std::uint32_t src   = c.take_u32();
    const std::uint8_t  count = c.take_u8();
    const std::uint8_t  lane  = c.take_u8();
    if (kind > static_cast<std::uint8_t>(VecShiftKind::ArithShr)) return DeserializeError::BadSize;
    if (!is_valid_veclane(lane)) return DeserializeError::BadSize;
    s.op = VecShiftImm{static_cast<VecShiftKind>(kind), src, count,
                       static_cast<VecLane>(lane)};
    return DeserializeError::Ok;
}

DeserializeError read_payload_vec_shuffle32x4(Cursor& c, Stmt& s) {
    if (!c.remaining(4 + 1)) return DeserializeError::Truncated;
    const std::uint32_t src     = c.take_u32();
    const std::uint8_t  control = c.take_u8();
    s.op = VecShuffle32x4{src, control};
    return DeserializeError::Ok;
}

DeserializeError read_payload_vec_cmp(Cursor& c, Stmt& s) {
    if (!c.remaining(1 + 4 + 4 + 1)) return DeserializeError::Truncated;
    const std::uint8_t  kind = c.take_u8();
    const std::uint32_t lhs  = c.take_u32();
    const std::uint32_t rhs  = c.take_u32();
    const std::uint8_t  lane = c.take_u8();
    if (kind > static_cast<std::uint8_t>(VecCmpKind::Gt)) return DeserializeError::BadSize;
    if (!is_valid_veclane(lane)) return DeserializeError::BadSize;
    s.op = VecCmp{static_cast<VecCmpKind>(kind), lhs, rhs,
                  static_cast<VecLane>(lane)};
    return DeserializeError::Ok;
}

DeserializeError read_payload_xmm_from_gpr(Cursor& c, Stmt& s) {
    if (!c.remaining(4 + 1)) return DeserializeError::Truncated;
    const std::uint32_t v  = c.take_u32();
    const std::uint8_t  sz = c.take_u8();
    if (!is_valid_size(sz)) return DeserializeError::BadSize;
    s.op = XmmFromGpr{v, static_cast<OpSize>(sz)};
    return DeserializeError::Ok;
}
DeserializeError read_payload_gpr_from_xmm(Cursor& c, Stmt& s) {
    if (!c.remaining(4 + 1)) return DeserializeError::Truncated;
    const std::uint32_t v  = c.take_u32();
    const std::uint8_t  sz = c.take_u8();
    if (!is_valid_size(sz)) return DeserializeError::BadSize;
    s.op = GprFromXmm{v, static_cast<OpSize>(sz)};
    return DeserializeError::Ok;
}

DeserializeError read_payload_load_vec(Cursor& c, Stmt& s) {
    if (!c.remaining(4)) return DeserializeError::Truncated;
    s.op = LoadVec{c.take_u32()};
    return DeserializeError::Ok;
}
DeserializeError read_payload_store_vec(Cursor& c, Stmt& s) {
    if (!c.remaining(4 + 4)) return DeserializeError::Truncated;
    const std::uint32_t addr = c.take_u32();
    const std::uint32_t val  = c.take_u32();
    s.op = StoreVec{addr, val};
    return DeserializeError::Ok;
}

DeserializeError read_payload_vec_fp_scalar_binop(Cursor& c, Stmt& s) {
    if (!c.remaining(1 + 4 + 4 + 1)) return DeserializeError::Truncated;
    const std::uint8_t  op   = c.take_u8();
    const std::uint32_t lhs  = c.take_u32();
    const std::uint32_t rhs  = c.take_u32();
    const std::uint8_t  size = c.take_u8();
    if (!is_valid_vecfpbinop(op)) return DeserializeError::BadSize;
    if (!is_valid_fpsize(size)) return DeserializeError::BadSize;
    s.op = VecFpScalarBinOp{static_cast<VecFpBinOpKind>(op), lhs, rhs,
                            static_cast<FpSize>(size)};
    return DeserializeError::Ok;
}

DeserializeError read_stmt(Cursor& c, Stmt& s) {
    if (!c.remaining(1)) return DeserializeError::Truncated;
    const std::uint8_t has_result = c.take_u8();
    if (has_result == 1) {
        if (!c.remaining(4)) return DeserializeError::Truncated;
        s.result = c.take_u32();
    } else if (has_result == 0) {
        s.result = std::nullopt;
    } else {
        return DeserializeError::BadSize;
    }
    if (!c.remaining(1)) return DeserializeError::Truncated;
    const std::uint8_t kind = c.take_u8();
    if (kind == 0 || kind > kMaxOpKind) {
        return DeserializeError::UnknownOpKind;
    }
    switch (static_cast<OpKind>(kind)) {
        case OpKind::kConstant:    return read_payload_constant(c, s);
        case OpKind::kLoadReg:     return read_payload_load_reg(c, s);
        case OpKind::kStoreReg:    return read_payload_store_reg(c, s);
        case OpKind::kLoadSegBase: return read_payload_load_seg_base(c, s);
        case OpKind::kBinOp:       return read_payload_binop(c, s);
        case OpKind::kCompare:     return read_payload_compare(c, s);
        case OpKind::kSelect:      return read_payload_select(c, s);
        case OpKind::kLoadMem:     return read_payload_load_mem(c, s);
        case OpKind::kStoreMem:    return read_payload_store_mem(c, s);
        case OpKind::kLoadMemTSO:  return read_payload_load_mem_tso(c, s);
        case OpKind::kStoreMemTSO: return read_payload_store_mem_tso(c, s);
        case OpKind::kJump:        return read_payload_jump(c, s);
        case OpKind::kCondJump:    return read_payload_cond_jump(c, s);
        case OpKind::kReturn:      return read_payload_return(c, s);
        case OpKind::kJumpReg:     return read_payload_jump_reg(c, s);
        case OpKind::kCmpFlags:    return read_payload_cmp_flags(c, s);
        case OpKind::kJumpRel:     return read_payload_jump_rel(c, s);
        case OpKind::kCondJumpRel: return read_payload_cond_jump_rel(c, s);
        case OpKind::kCallRel:     return read_payload_call_rel(c, s);
        case OpKind::kCallReg:     return read_payload_call_reg(c, s);
        case OpKind::kRetAdjusted: return read_payload_ret_adjusted(c, s);
        case OpKind::kCpuid:       return read_payload_cpuid(c, s);
        case OpKind::kSyscall:     return read_payload_syscall(c, s);
        case OpKind::kTrap:        return read_payload_trap(c, s);
        case OpKind::kExtend:      return read_payload_extend(c, s);
        case OpKind::kTruncate:    return read_payload_truncate(c, s);
        case OpKind::kFence:       return read_payload_fence(c, s);
        case OpKind::kGuestPc:     return read_payload_guest_pc(c, s);
        case OpKind::kInlineAsm:   return read_payload_inline_asm(c, s);
        case OpKind::kFpConstant:  return read_payload_fp_constant(c, s);
        case OpKind::kFpBinOp:     return read_payload_fp_binop(c, s);
        case OpKind::kWriteFlags:  return read_payload_write_flags(c, s);
        case OpKind::kReadFlag:    return read_payload_read_flag(c, s);
        case OpKind::kCondJumpFlags: return read_payload_cond_jump_flags(c, s);
        case OpKind::kRspAdjust:   return read_payload_rsp_adjust(c, s);
        case OpKind::kVecConstant: return read_payload_vec_constant(c, s);
        case OpKind::kVecBinOp:    return read_payload_vec_binop(c, s);
        case OpKind::kLoadVecReg:  return read_payload_load_vec_reg(c, s);
        case OpKind::kStoreVecReg: return read_payload_store_vec_reg(c, s);
        case OpKind::kVecFpBinOp:  return read_payload_vec_fp_binop(c, s);
        case OpKind::kVecFpScalarBinOp: return read_payload_vec_fp_scalar_binop(c, s);
        case OpKind::kLoadVec:     return read_payload_load_vec(c, s);
        case OpKind::kStoreVec:    return read_payload_store_vec(c, s);
        case OpKind::kXmmFromGpr:  return read_payload_xmm_from_gpr(c, s);
        case OpKind::kGprFromXmm:  return read_payload_gpr_from_xmm(c, s);
        case OpKind::kVecCmp:      return read_payload_vec_cmp(c, s);
        case OpKind::kVecShuffle32x4: return read_payload_vec_shuffle32x4(c, s);
        case OpKind::kVecUnpack:   return read_payload_vec_unpack(c, s);
        case OpKind::kVecShiftImm: return read_payload_vec_shift_imm(c, s);
        case OpKind::kVecShiftBytes: return read_payload_vec_shift_bytes(c, s);
        case OpKind::kIntToFpScalar: return read_payload_int_to_fp_scalar(c, s);
        case OpKind::kFpToIntScalar: return read_payload_fp_to_int_scalar(c, s);
        case OpKind::kFpCvtScalar: return read_payload_fp_cvt_scalar(c, s);
        case OpKind::kVecShuffle2Src: return read_payload_vec_shuffle_2src(c, s);
        case OpKind::kVecInsertLane: return read_payload_vec_insert_lane(c, s);
        case OpKind::kVecExtractLaneU: return read_payload_vec_extract_lane_u(c, s);
        case OpKind::kReserved:    break;
    }
    return DeserializeError::UnknownOpKind;
}

// ---- Header / footer helpers -----------------------------------------

void write_header(std::vector<std::uint8_t>& out) {
    put_u32(out, kSerializeMagic);
    put_u16(out, kSerializeVersion);
    put_u16(out, 0u);  // reserved
}

void append_crc(std::vector<std::uint8_t>& out) {
    const std::uint32_t crc = detail::crc32c(std::span<const std::uint8_t>(out));
    put_u32(out, crc);
}

}  // namespace

// =====================================================================
// Public API
// =====================================================================

std::vector<std::uint8_t> serialize(const std::vector<Stmt>& stmts) {
    std::vector<std::uint8_t> out;
    out.reserve(16 + stmts.size() * 16);
    write_header(out);
    put_u32(out, static_cast<std::uint32_t>(stmts.size()));
    for (const auto& s : stmts) {
        write_stmt(out, s);
    }
    append_crc(out);
    return out;
}

std::vector<std::uint8_t> serialize(const Function& fn) {
    std::vector<std::uint8_t> out;
    out.reserve(32 + fn.blocks.size() * 32);
    write_header(out);
    // Function-shaped payload: marker u32 = 0xFFFFFFFF (distinguishes
    // it from a plain stmt-stream which begins with a stmt count) is
    // overkill; we keep streams disjoint at the API level instead and
    // just write `entry`, `block_count`, then per-block
    // `(id, stmt_count, stmts)`.
    put_u32(out, fn.entry);
    put_u32(out, static_cast<std::uint32_t>(fn.blocks.size()));
    for (const auto& b : fn.blocks) {
        put_u32(out, b.id);
        put_u32(out, static_cast<std::uint32_t>(b.stmts.size()));
        for (const auto& s : b.stmts) {
            write_stmt(out, s);
        }
    }
    append_crc(out);
    return out;
}

DeserializeResult deserialize_stmts(std::span<const std::uint8_t> bytes) {
    DeserializeResult r{DeserializeError::Ok, {}};
    // Header (8) + count (4) + crc (4) = 16 bytes minimum.
    if (bytes.size() < 16) {
        r.error = DeserializeError::Truncated;
        return r;
    }
    Cursor c{bytes, 0};
    if (c.take_u32() != kSerializeMagic) {
        r.error = DeserializeError::BadMagic;
        return r;
    }
    if (c.take_u16() != kSerializeVersion) {
        r.error = DeserializeError::BadVersion;
        return r;
    }
    (void)c.take_u16();  // reserved

    // CRC check: covers everything from byte 0 up to but not
    // including the trailing 4 bytes.
    const std::span<const std::uint8_t> body{bytes.data(), bytes.size() - 4};
    const std::uint32_t expected = detail::crc32c(body);
    const std::size_t crc_pos = bytes.size() - 4;
    const std::uint32_t got =
          static_cast<std::uint32_t>(bytes[crc_pos])
        | (static_cast<std::uint32_t>(bytes[crc_pos + 1]) << 8)
        | (static_cast<std::uint32_t>(bytes[crc_pos + 2]) << 16)
        | (static_cast<std::uint32_t>(bytes[crc_pos + 3]) << 24);
    if (expected != got) {
        r.error = DeserializeError::BadCrc;
        return r;
    }

    if (!c.remaining(4)) {
        r.error = DeserializeError::Truncated;
        return r;
    }
    const std::uint32_t n = c.take_u32();
    r.stmts.reserve(n);
    for (std::uint32_t i = 0; i < n; ++i) {
        Stmt s{};
        const auto e = read_stmt(c, s);
        if (e != DeserializeError::Ok) {
            r.error = e;
            r.stmts.clear();
            return r;
        }
        r.stmts.push_back(std::move(s));
    }
    // Cursor must now point exactly at the trailing CRC.
    if (c.pos != bytes.size() - 4) {
        r.error = DeserializeError::Truncated;
        r.stmts.clear();
        return r;
    }
    return r;
}

std::pair<DeserializeError, std::optional<Function>>
deserialize_function(std::span<const std::uint8_t> bytes) {
    if (bytes.size() < 16) {
        return {DeserializeError::Truncated, std::nullopt};
    }
    Cursor c{bytes, 0};
    if (c.take_u32() != kSerializeMagic) {
        return {DeserializeError::BadMagic, std::nullopt};
    }
    if (c.take_u16() != kSerializeVersion) {
        return {DeserializeError::BadVersion, std::nullopt};
    }
    (void)c.take_u16();  // reserved

    const std::span<const std::uint8_t> body{bytes.data(), bytes.size() - 4};
    const std::uint32_t expected = detail::crc32c(body);
    const std::size_t crc_pos = bytes.size() - 4;
    const std::uint32_t got =
          static_cast<std::uint32_t>(bytes[crc_pos])
        | (static_cast<std::uint32_t>(bytes[crc_pos + 1]) << 8)
        | (static_cast<std::uint32_t>(bytes[crc_pos + 2]) << 16)
        | (static_cast<std::uint32_t>(bytes[crc_pos + 3]) << 24);
    if (expected != got) {
        return {DeserializeError::BadCrc, std::nullopt};
    }

    if (!c.remaining(4 + 4)) {
        return {DeserializeError::Truncated, std::nullopt};
    }
    Function fn;
    fn.entry = c.take_u32();
    const std::uint32_t bc = c.take_u32();
    fn.blocks.reserve(bc);
    for (std::uint32_t bi = 0; bi < bc; ++bi) {
        if (!c.remaining(4 + 4)) {
            return {DeserializeError::Truncated, std::nullopt};
        }
        BasicBlock b;
        b.id = c.take_u32();
        const std::uint32_t sc = c.take_u32();
        b.stmts.reserve(sc);
        for (std::uint32_t si = 0; si < sc; ++si) {
            Stmt s{};
            const auto e = read_stmt(c, s);
            if (e != DeserializeError::Ok) {
                return {e, std::nullopt};
            }
            b.stmts.push_back(std::move(s));
        }
        fn.blocks.push_back(std::move(b));
    }
    if (c.pos != bytes.size() - 4) {
        return {DeserializeError::Truncated, std::nullopt};
    }
    return {DeserializeError::Ok, std::move(fn)};
}

}  // namespace prisma::ir
