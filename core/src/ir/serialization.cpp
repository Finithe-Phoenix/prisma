// core/src/ir/serialization.cpp - deterministic binary Prisma IR writer.

#include "prisma/ir_serialization.hpp"

#include <cstddef>
#include <optional>
#include <type_traits>
#include <utility>
#include <variant>

namespace prisma::ir {

namespace {

enum class OpTag : std::uint8_t {
    Constant = 1,
    LoadReg = 2,
    LoadSegBase = 3,
    StoreReg = 4,
    BinOp = 5,
    Extend = 6,
    Truncate = 7,
    Compare = 8,
    Select = 9,
    LoadMem = 10,
    StoreMem = 11,
    LoadMemTSO = 12,
    StoreMemTSO = 13,
    GuestPc = 14,
    Jump = 15,
    CondJump = 16,
    Return = 17,
    JumpReg = 18,
    CmpFlags = 19,
    JumpRel = 20,
    CallRel = 21,
    CallReg = 22,
    RetAdjusted = 23,
    Cpuid = 24,
    Syscall = 25,
    Trap = 26,
    Fence = 27,
    CondJumpRel = 28,
};

constexpr std::uint32_t kMaxDeserializedItems = 1'000'000u;

class Writer {
public:
    void u8(std::uint8_t value) {
        bytes_.push_back(value);
    }

    void u16(std::uint16_t value) {
        u8(static_cast<std::uint8_t>(value & 0xFFu));
        u8(static_cast<std::uint8_t>((value >> 8u) & 0xFFu));
    }

    void u32(std::uint32_t value) {
        for (unsigned shift = 0; shift < 32; shift += 8) {
            u8(static_cast<std::uint8_t>((value >> shift) & 0xFFu));
        }
    }

    void u64(std::uint64_t value) {
        for (unsigned shift = 0; shift < 64; shift += 8) {
            u8(static_cast<std::uint8_t>((value >> shift) & 0xFFu));
        }
    }

    template <typename E>
    void enum8(E value) {
        static_assert(std::is_enum_v<E>);
        u8(static_cast<std::uint8_t>(value));
    }

    void header(IrBinaryKind kind) {
        bytes_.insert(bytes_.end(), kIrBinaryMagic.begin(), kIrBinaryMagic.end());
        u16(kIrBinaryVersion);
        enum8(kind);
        u8(0);  // reserved flags byte for future deserializers.
    }

    void stmt(const Stmt& stmt) {
        if (stmt.result) {
            u8(1);
            u32(*stmt.result);
        } else {
            u8(0);
        }
        op(stmt.op);
    }

    void root_op(const Op& op) {
        this->op(op);
    }

    void stmt_list(std::span<const Stmt> stmts) {
        u32(static_cast<std::uint32_t>(stmts.size()));
        for (const auto& stmt : stmts) {
            this->stmt(stmt);
        }
    }

    [[nodiscard]] std::vector<std::uint8_t> finish() && {
        return std::move(bytes_);
    }

private:
    void op(const Op& op) {
        std::visit([&](const auto& x) {
            using T = std::decay_t<decltype(x)>;
            if constexpr (std::is_same_v<T, Constant>) {
                enum8(OpTag::Constant);
                u64(x.value);
                enum8(x.size);
            } else if constexpr (std::is_same_v<T, LoadReg>) {
                enum8(OpTag::LoadReg);
                enum8(x.reg);
                enum8(x.size);
            } else if constexpr (std::is_same_v<T, LoadSegBase>) {
                enum8(OpTag::LoadSegBase);
                enum8(x.segment);
            } else if constexpr (std::is_same_v<T, StoreReg>) {
                enum8(OpTag::StoreReg);
                enum8(x.reg);
                u32(x.value);
                enum8(x.size);
            } else if constexpr (std::is_same_v<T, BinOp>) {
                enum8(OpTag::BinOp);
                enum8(x.op);
                u32(x.lhs);
                u32(x.rhs);
                enum8(x.size);
            } else if constexpr (std::is_same_v<T, Extend>) {
                enum8(OpTag::Extend);
                u32(x.value);
                enum8(x.from_size);
                enum8(x.to_size);
                u8(x.is_signed ? 1 : 0);
            } else if constexpr (std::is_same_v<T, Truncate>) {
                enum8(OpTag::Truncate);
                u32(x.value);
                enum8(x.to_size);
            } else if constexpr (std::is_same_v<T, Compare>) {
                enum8(OpTag::Compare);
                enum8(x.cc);
                u32(x.lhs);
                u32(x.rhs);
                enum8(x.size);
            } else if constexpr (std::is_same_v<T, Select>) {
                enum8(OpTag::Select);
                enum8(x.cc);
                u32(x.true_value);
                u32(x.false_value);
                enum8(x.size);
            } else if constexpr (std::is_same_v<T, LoadMem>) {
                enum8(OpTag::LoadMem);
                u32(x.addr);
                enum8(x.size);
            } else if constexpr (std::is_same_v<T, StoreMem>) {
                enum8(OpTag::StoreMem);
                u32(x.addr);
                u32(x.value);
                enum8(x.size);
            } else if constexpr (std::is_same_v<T, LoadMemTSO>) {
                enum8(OpTag::LoadMemTSO);
                u32(x.addr);
                enum8(x.size);
            } else if constexpr (std::is_same_v<T, StoreMemTSO>) {
                enum8(OpTag::StoreMemTSO);
                u32(x.addr);
                u32(x.value);
                enum8(x.size);
            } else if constexpr (std::is_same_v<T, GuestPc>) {
                enum8(OpTag::GuestPc);
                u64(x.pc);
            } else if constexpr (std::is_same_v<T, Jump>) {
                enum8(OpTag::Jump);
                u32(x.target_block);
            } else if constexpr (std::is_same_v<T, CondJump>) {
                enum8(OpTag::CondJump);
                u32(x.cond);
                u32(x.if_true);
                u32(x.if_false);
            } else if constexpr (std::is_same_v<T, Return>) {
                enum8(OpTag::Return);
            } else if constexpr (std::is_same_v<T, JumpReg>) {
                enum8(OpTag::JumpReg);
                u32(x.target);
            } else if constexpr (std::is_same_v<T, CmpFlags>) {
                enum8(OpTag::CmpFlags);
                u32(x.lhs);
                u32(x.rhs);
                enum8(x.size);
            } else if constexpr (std::is_same_v<T, JumpRel>) {
                enum8(OpTag::JumpRel);
                u64(x.target_guest_pc);
            } else if constexpr (std::is_same_v<T, CallRel>) {
                enum8(OpTag::CallRel);
                u64(x.target_guest_pc);
                u64(x.return_guest_pc);
            } else if constexpr (std::is_same_v<T, CallReg>) {
                enum8(OpTag::CallReg);
                u32(x.target);
                u64(x.return_guest_pc);
            } else if constexpr (std::is_same_v<T, RetAdjusted>) {
                enum8(OpTag::RetAdjusted);
                u32(x.pop_bytes);
            } else if constexpr (std::is_same_v<T, Cpuid>) {
                enum8(OpTag::Cpuid);
            } else if constexpr (std::is_same_v<T, Syscall>) {
                enum8(OpTag::Syscall);
            } else if constexpr (std::is_same_v<T, Trap>) {
                enum8(OpTag::Trap);
                enum8(x.kind);
            } else if constexpr (std::is_same_v<T, Fence>) {
                enum8(OpTag::Fence);
                enum8(x.kind);
            } else if constexpr (std::is_same_v<T, CondJumpRel>) {
                enum8(OpTag::CondJumpRel);
                enum8(x.cc);
                u64(x.target_guest_pc);
                u64(x.fallthrough_guest_pc);
            }
        }, op);
    }

    std::vector<std::uint8_t> bytes_;
};

class Reader {
public:
    explicit Reader(std::span<const std::uint8_t> bytes) : bytes_(bytes) {}

    [[nodiscard]] std::size_t remaining() const {
        return bytes_.size() - offset_;
    }

    [[nodiscard]] bool empty() const {
        return remaining() == 0;
    }

    [[nodiscard]] IrDeserializeError error() const {
        return error_.value_or(IrDeserializeError::Truncated);
    }

    bool u8(std::uint8_t& out) {
        if (remaining() < 1u) {
            error_ = IrDeserializeError::Truncated;
            return false;
        }
        out = bytes_[offset_++];
        return true;
    }

    bool u16(std::uint16_t& out) {
        std::uint8_t lo = 0;
        std::uint8_t hi = 0;
        if (!u8(lo) || !u8(hi)) return false;
        out = static_cast<std::uint16_t>(lo)
            | static_cast<std::uint16_t>(static_cast<std::uint16_t>(hi) << 8u);
        return true;
    }

    bool u32(std::uint32_t& out) {
        out = 0;
        for (unsigned shift = 0; shift < 32; shift += 8) {
            std::uint8_t byte = 0;
            if (!u8(byte)) return false;
            out |= static_cast<std::uint32_t>(byte) << shift;
        }
        return true;
    }

    bool u64(std::uint64_t& out) {
        out = 0;
        for (unsigned shift = 0; shift < 64; shift += 8) {
            std::uint8_t byte = 0;
            if (!u8(byte)) return false;
            out |= static_cast<std::uint64_t>(byte) << shift;
        }
        return true;
    }

private:
    std::span<const std::uint8_t> bytes_;
    std::size_t offset_{0};
    std::optional<IrDeserializeError> error_;
};

[[nodiscard]] bool decode_gpr(std::uint8_t raw, Gpr& out) {
    if (raw > static_cast<std::uint8_t>(Gpr::R15)) return false;
    out = static_cast<Gpr>(raw);
    return true;
}

[[nodiscard]] bool decode_size(std::uint8_t raw, OpSize& out) {
    if (raw > static_cast<std::uint8_t>(OpSize::I64)) return false;
    out = static_cast<OpSize>(raw);
    return true;
}

[[nodiscard]] bool decode_binop(std::uint8_t raw, BinOpKind& out) {
    if (raw > static_cast<std::uint8_t>(BinOpKind::Rcr)) return false;
    out = static_cast<BinOpKind>(raw);
    return true;
}

[[nodiscard]] bool decode_cond(std::uint8_t raw, CondCode& out) {
    if (raw > static_cast<std::uint8_t>(CondCode::Pl)) return false;
    out = static_cast<CondCode>(raw);
    return true;
}

[[nodiscard]] bool decode_segment(std::uint8_t raw, SegmentReg& out) {
    if (raw > static_cast<std::uint8_t>(SegmentReg::Gs)) return false;
    out = static_cast<SegmentReg>(raw);
    return true;
}

[[nodiscard]] bool decode_trap(std::uint8_t raw, TrapKind& out) {
    if (raw != static_cast<std::uint8_t>(TrapKind::Sigtrap)) return false;
    out = static_cast<TrapKind>(raw);
    return true;
}

[[nodiscard]] bool decode_fence(std::uint8_t raw, FenceKind& out) {
    if (raw > static_cast<std::uint8_t>(FenceKind::Sfence)) return false;
    out = static_cast<FenceKind>(raw);
    return true;
}

[[nodiscard]] bool decode_tag(std::uint8_t raw, OpTag& out) {
    if (raw < static_cast<std::uint8_t>(OpTag::Constant)
        || raw > static_cast<std::uint8_t>(OpTag::CondJumpRel)) {
        return false;
    }
    out = static_cast<OpTag>(raw);
    return true;
}

template <typename E, typename Decode>
[[nodiscard]] std::variant<E, IrDeserializeError> read_enum(Reader& reader, Decode decode) {
    std::uint8_t raw = 0;
    if (!reader.u8(raw)) return reader.error();
    E value{};
    if (!decode(raw, value)) return IrDeserializeError::InvalidEnum;
    return value;
}

[[nodiscard]] std::variant<bool, IrDeserializeError> read_bool(Reader& reader) {
    std::uint8_t raw = 0;
    if (!reader.u8(raw)) return reader.error();
    if (raw > 1u) return IrDeserializeError::InvalidBool;
    return raw == 1u;
}

[[nodiscard]] std::variant<Op, IrDeserializeError> read_op(Reader& reader) {
    std::uint8_t raw_tag = 0;
    if (!reader.u8(raw_tag)) return reader.error();
    OpTag tag{};
    if (!decode_tag(raw_tag, tag)) return IrDeserializeError::InvalidTag;

    switch (tag) {
        case OpTag::Constant: {
            std::uint64_t value = 0;
            if (!reader.u64(value)) return reader.error();
            auto size = read_enum<OpSize>(reader, decode_size);
            if (std::holds_alternative<IrDeserializeError>(size)) {
                return std::get<IrDeserializeError>(size);
            }
            return Op{Constant{value, std::get<OpSize>(size)}};
        }
        case OpTag::LoadReg: {
            auto reg = read_enum<Gpr>(reader, decode_gpr);
            if (std::holds_alternative<IrDeserializeError>(reg)) {
                return std::get<IrDeserializeError>(reg);
            }
            auto size = read_enum<OpSize>(reader, decode_size);
            if (std::holds_alternative<IrDeserializeError>(size)) {
                return std::get<IrDeserializeError>(size);
            }
            return Op{LoadReg{std::get<Gpr>(reg), std::get<OpSize>(size)}};
        }
        case OpTag::LoadSegBase: {
            auto segment = read_enum<SegmentReg>(reader, decode_segment);
            if (std::holds_alternative<IrDeserializeError>(segment)) {
                return std::get<IrDeserializeError>(segment);
            }
            return Op{LoadSegBase{std::get<SegmentReg>(segment)}};
        }
        case OpTag::StoreReg: {
            auto reg = read_enum<Gpr>(reader, decode_gpr);
            if (std::holds_alternative<IrDeserializeError>(reg)) {
                return std::get<IrDeserializeError>(reg);
            }
            std::uint32_t value = 0;
            if (!reader.u32(value)) return reader.error();
            auto size = read_enum<OpSize>(reader, decode_size);
            if (std::holds_alternative<IrDeserializeError>(size)) {
                return std::get<IrDeserializeError>(size);
            }
            return Op{StoreReg{std::get<Gpr>(reg), value, std::get<OpSize>(size)}};
        }
        case OpTag::BinOp: {
            auto op = read_enum<BinOpKind>(reader, decode_binop);
            if (std::holds_alternative<IrDeserializeError>(op)) {
                return std::get<IrDeserializeError>(op);
            }
            std::uint32_t lhs = 0;
            std::uint32_t rhs = 0;
            if (!reader.u32(lhs) || !reader.u32(rhs)) return reader.error();
            auto size = read_enum<OpSize>(reader, decode_size);
            if (std::holds_alternative<IrDeserializeError>(size)) {
                return std::get<IrDeserializeError>(size);
            }
            return Op{BinOp{std::get<BinOpKind>(op), lhs, rhs, std::get<OpSize>(size)}};
        }
        case OpTag::Extend: {
            std::uint32_t value = 0;
            if (!reader.u32(value)) return reader.error();
            auto from_size = read_enum<OpSize>(reader, decode_size);
            if (std::holds_alternative<IrDeserializeError>(from_size)) {
                return std::get<IrDeserializeError>(from_size);
            }
            auto to_size = read_enum<OpSize>(reader, decode_size);
            if (std::holds_alternative<IrDeserializeError>(to_size)) {
                return std::get<IrDeserializeError>(to_size);
            }
            auto is_signed = read_bool(reader);
            if (std::holds_alternative<IrDeserializeError>(is_signed)) {
                return std::get<IrDeserializeError>(is_signed);
            }
            return Op{Extend{value, std::get<OpSize>(from_size),
                             std::get<OpSize>(to_size), std::get<bool>(is_signed)}};
        }
        case OpTag::Truncate: {
            std::uint32_t value = 0;
            if (!reader.u32(value)) return reader.error();
            auto to_size = read_enum<OpSize>(reader, decode_size);
            if (std::holds_alternative<IrDeserializeError>(to_size)) {
                return std::get<IrDeserializeError>(to_size);
            }
            return Op{Truncate{value, std::get<OpSize>(to_size)}};
        }
        case OpTag::Compare: {
            auto cc = read_enum<CondCode>(reader, decode_cond);
            if (std::holds_alternative<IrDeserializeError>(cc)) {
                return std::get<IrDeserializeError>(cc);
            }
            std::uint32_t lhs = 0;
            std::uint32_t rhs = 0;
            if (!reader.u32(lhs) || !reader.u32(rhs)) return reader.error();
            auto size = read_enum<OpSize>(reader, decode_size);
            if (std::holds_alternative<IrDeserializeError>(size)) {
                return std::get<IrDeserializeError>(size);
            }
            return Op{Compare{std::get<CondCode>(cc), lhs, rhs, std::get<OpSize>(size)}};
        }
        case OpTag::Select: {
            auto cc = read_enum<CondCode>(reader, decode_cond);
            if (std::holds_alternative<IrDeserializeError>(cc)) {
                return std::get<IrDeserializeError>(cc);
            }
            std::uint32_t true_value = 0;
            std::uint32_t false_value = 0;
            if (!reader.u32(true_value) || !reader.u32(false_value)) return reader.error();
            auto size = read_enum<OpSize>(reader, decode_size);
            if (std::holds_alternative<IrDeserializeError>(size)) {
                return std::get<IrDeserializeError>(size);
            }
            return Op{Select{std::get<CondCode>(cc), true_value, false_value,
                             std::get<OpSize>(size)}};
        }
        case OpTag::LoadMem: {
            std::uint32_t addr = 0;
            if (!reader.u32(addr)) return reader.error();
            auto size = read_enum<OpSize>(reader, decode_size);
            if (std::holds_alternative<IrDeserializeError>(size)) {
                return std::get<IrDeserializeError>(size);
            }
            return Op{LoadMem{addr, std::get<OpSize>(size)}};
        }
        case OpTag::StoreMem: {
            std::uint32_t addr = 0;
            std::uint32_t value = 0;
            if (!reader.u32(addr) || !reader.u32(value)) return reader.error();
            auto size = read_enum<OpSize>(reader, decode_size);
            if (std::holds_alternative<IrDeserializeError>(size)) {
                return std::get<IrDeserializeError>(size);
            }
            return Op{StoreMem{addr, value, std::get<OpSize>(size)}};
        }
        case OpTag::LoadMemTSO: {
            std::uint32_t addr = 0;
            if (!reader.u32(addr)) return reader.error();
            auto size = read_enum<OpSize>(reader, decode_size);
            if (std::holds_alternative<IrDeserializeError>(size)) {
                return std::get<IrDeserializeError>(size);
            }
            return Op{LoadMemTSO{addr, std::get<OpSize>(size)}};
        }
        case OpTag::StoreMemTSO: {
            std::uint32_t addr = 0;
            std::uint32_t value = 0;
            if (!reader.u32(addr) || !reader.u32(value)) return reader.error();
            auto size = read_enum<OpSize>(reader, decode_size);
            if (std::holds_alternative<IrDeserializeError>(size)) {
                return std::get<IrDeserializeError>(size);
            }
            return Op{StoreMemTSO{addr, value, std::get<OpSize>(size)}};
        }
        case OpTag::GuestPc: {
            std::uint64_t pc = 0;
            if (!reader.u64(pc)) return reader.error();
            return Op{GuestPc{pc}};
        }
        case OpTag::Jump: {
            std::uint32_t target_block = 0;
            if (!reader.u32(target_block)) return reader.error();
            return Op{Jump{target_block}};
        }
        case OpTag::CondJump: {
            std::uint32_t cond = 0;
            std::uint32_t if_true = 0;
            std::uint32_t if_false = 0;
            if (!reader.u32(cond) || !reader.u32(if_true) || !reader.u32(if_false)) {
                return reader.error();
            }
            return Op{CondJump{cond, if_true, if_false}};
        }
        case OpTag::Return:
            return Op{Return{}};
        case OpTag::JumpReg: {
            std::uint32_t target = 0;
            if (!reader.u32(target)) return reader.error();
            return Op{JumpReg{target}};
        }
        case OpTag::CmpFlags: {
            std::uint32_t lhs = 0;
            std::uint32_t rhs = 0;
            if (!reader.u32(lhs) || !reader.u32(rhs)) return reader.error();
            auto size = read_enum<OpSize>(reader, decode_size);
            if (std::holds_alternative<IrDeserializeError>(size)) {
                return std::get<IrDeserializeError>(size);
            }
            return Op{CmpFlags{lhs, rhs, std::get<OpSize>(size)}};
        }
        case OpTag::JumpRel: {
            std::uint64_t target_guest_pc = 0;
            if (!reader.u64(target_guest_pc)) return reader.error();
            return Op{JumpRel{target_guest_pc}};
        }
        case OpTag::CallRel: {
            std::uint64_t target_guest_pc = 0;
            std::uint64_t return_guest_pc = 0;
            if (!reader.u64(target_guest_pc) || !reader.u64(return_guest_pc)) {
                return reader.error();
            }
            return Op{CallRel{target_guest_pc, return_guest_pc}};
        }
        case OpTag::CallReg: {
            std::uint32_t target = 0;
            std::uint64_t return_guest_pc = 0;
            if (!reader.u32(target) || !reader.u64(return_guest_pc)) return reader.error();
            return Op{CallReg{target, return_guest_pc}};
        }
        case OpTag::RetAdjusted: {
            std::uint32_t pop_bytes = 0;
            if (!reader.u32(pop_bytes)) return reader.error();
            return Op{RetAdjusted{pop_bytes}};
        }
        case OpTag::Cpuid:
            return Op{Cpuid{}};
        case OpTag::Syscall:
            return Op{Syscall{}};
        case OpTag::Trap: {
            auto kind = read_enum<TrapKind>(reader, decode_trap);
            if (std::holds_alternative<IrDeserializeError>(kind)) {
                return std::get<IrDeserializeError>(kind);
            }
            return Op{Trap{std::get<TrapKind>(kind)}};
        }
        case OpTag::Fence: {
            auto kind = read_enum<FenceKind>(reader, decode_fence);
            if (std::holds_alternative<IrDeserializeError>(kind)) {
                return std::get<IrDeserializeError>(kind);
            }
            return Op{Fence{std::get<FenceKind>(kind)}};
        }
        case OpTag::CondJumpRel: {
            auto cc = read_enum<CondCode>(reader, decode_cond);
            if (std::holds_alternative<IrDeserializeError>(cc)) {
                return std::get<IrDeserializeError>(cc);
            }
            std::uint64_t target_guest_pc = 0;
            std::uint64_t fallthrough_guest_pc = 0;
            if (!reader.u64(target_guest_pc) || !reader.u64(fallthrough_guest_pc)) {
                return reader.error();
            }
            return Op{CondJumpRel{std::get<CondCode>(cc), target_guest_pc,
                                  fallthrough_guest_pc}};
        }
    }
    return IrDeserializeError::InvalidTag;
}

[[nodiscard]] std::variant<Stmt, IrDeserializeError> read_stmt(Reader& reader) {
    auto has_result = read_bool(reader);
    if (std::holds_alternative<IrDeserializeError>(has_result)) {
        return std::get<IrDeserializeError>(has_result);
    }

    std::optional<Ref> result;
    if (std::get<bool>(has_result)) {
        std::uint32_t ref = 0;
        if (!reader.u32(ref)) return reader.error();
        result = ref;
    }

    auto op = read_op(reader);
    if (std::holds_alternative<IrDeserializeError>(op)) {
        return std::get<IrDeserializeError>(op);
    }
    return Stmt{result, std::get<Op>(std::move(op))};
}

[[nodiscard]] std::variant<std::vector<Stmt>, IrDeserializeError>
read_stmt_list_payload(Reader& reader) {
    std::uint32_t count = 0;
    if (!reader.u32(count)) return reader.error();
    if (count > kMaxDeserializedItems) return IrDeserializeError::TooLarge;
    if (count > reader.remaining() / 2u) return IrDeserializeError::Truncated;

    std::vector<Stmt> stmts;
    stmts.reserve(count);
    for (std::uint32_t i = 0; i < count; ++i) {
        auto stmt = read_stmt(reader);
        if (std::holds_alternative<IrDeserializeError>(stmt)) {
            return std::get<IrDeserializeError>(stmt);
        }
        stmts.push_back(std::get<Stmt>(std::move(stmt)));
    }
    return stmts;
}

[[nodiscard]] std::optional<IrDeserializeError> read_header(Reader& reader,
                                                            IrBinaryKind expected_kind) {
    for (const auto expected : kIrBinaryMagic) {
        std::uint8_t actual = 0;
        if (!reader.u8(actual)) return reader.error();
        if (actual != expected) return IrDeserializeError::BadMagic;
    }

    std::uint16_t version = 0;
    if (!reader.u16(version)) return reader.error();
    if (version != kIrBinaryVersion) return IrDeserializeError::UnsupportedVersion;

    std::uint8_t raw_kind = 0;
    if (!reader.u8(raw_kind)) return reader.error();
    if (raw_kind != static_cast<std::uint8_t>(expected_kind)) {
        return IrDeserializeError::WrongKind;
    }

    std::uint8_t flags = 0;
    if (!reader.u8(flags)) return reader.error();
    if (flags != 0u) return IrDeserializeError::UnsupportedVersion;

    return std::nullopt;
}

[[nodiscard]] std::optional<IrDeserializeError> check_trailing(const Reader& reader) {
    if (!reader.empty()) return IrDeserializeError::TrailingBytes;
    return std::nullopt;
}

}  // namespace

std::vector<std::uint8_t> serialize_stmts(std::span<const Stmt> stmts) {
    Writer writer;
    writer.header(IrBinaryKind::StmtList);
    writer.stmt_list(stmts);
    return std::move(writer).finish();
}

std::vector<std::uint8_t> serialize_op(const Op& op) {
    Writer writer;
    writer.root_op(op);
    return std::move(writer).finish();
}

std::vector<std::uint8_t> serialize_function(const Function& function) {
    Writer writer;
    writer.header(IrBinaryKind::Function);
    writer.u32(function.entry);
    writer.u32(static_cast<std::uint32_t>(function.blocks.size()));
    for (const auto& block : function.blocks) {
        writer.u32(block.id);
        writer.stmt_list(block.stmts);
    }
    return std::move(writer).finish();
}

std::variant<std::vector<Stmt>, IrDeserializeError>
deserialize_stmts(std::span<const std::uint8_t> bytes) {
    Reader reader(bytes);
    if (auto error = read_header(reader, IrBinaryKind::StmtList)) return *error;
    auto stmts = read_stmt_list_payload(reader);
    if (std::holds_alternative<IrDeserializeError>(stmts)) {
        return std::get<IrDeserializeError>(stmts);
    }
    if (auto error = check_trailing(reader)) return *error;
    return std::get<std::vector<Stmt>>(std::move(stmts));
}

std::variant<Op, IrDeserializeError>
deserialize_op(std::span<const std::uint8_t> bytes) {
    Reader reader(bytes);
    auto op = read_op(reader);
    if (std::holds_alternative<IrDeserializeError>(op)) {
        return std::get<IrDeserializeError>(op);
    }
    if (auto error = check_trailing(reader)) return *error;
    return std::get<Op>(std::move(op));
}

std::variant<Function, IrDeserializeError>
deserialize_function(std::span<const std::uint8_t> bytes) {
    Reader reader(bytes);
    if (auto error = read_header(reader, IrBinaryKind::Function)) return *error;

    Function function;
    if (!reader.u32(function.entry)) return reader.error();

    std::uint32_t block_count = 0;
    if (!reader.u32(block_count)) return reader.error();
    if (block_count > kMaxDeserializedItems) return IrDeserializeError::TooLarge;
    if (block_count > reader.remaining() / 8u) return IrDeserializeError::Truncated;

    function.blocks.reserve(block_count);
    for (std::uint32_t i = 0; i < block_count; ++i) {
        BasicBlock block;
        if (!reader.u32(block.id)) return reader.error();
        auto stmts = read_stmt_list_payload(reader);
        if (std::holds_alternative<IrDeserializeError>(stmts)) {
            return std::get<IrDeserializeError>(stmts);
        }
        block.stmts = std::get<std::vector<Stmt>>(std::move(stmts));
        function.blocks.push_back(std::move(block));
    }

    if (auto error = check_trailing(reader)) return *error;
    return function;
}

}  // namespace prisma::ir
