// core/src/backend/emitter.cpp — vixl-backed implementation of Emitter.
//
// Keeps vixl headers out of `emitter.hpp` via pimpl: clients of prisma_emitter
// do not need vixl on their include path.

#include "prisma/emitter.hpp"

#include <cstddef>
#include <cstdint>
#include <memory>
#include <sstream>
#include <string>
#include <vector>

// vixl headers. They reside under `aarch64/...` relative to the include root.
#include "aarch64/macro-assembler-aarch64.h"
#include "aarch64/disasm-aarch64.h"

namespace prisma::backend {

namespace vixl_aa = vixl::aarch64;

namespace {

// Translate Prisma's Reg enum to vixl's XRegister. Both use the same 5-bit
// encoding, so this is pure sugar, but doing the translation explicitly
// keeps us honest. vixl's XRegister ctor takes `int`, hence the cast.
vixl_aa::XRegister to_vixl_x(arm64::Reg r) noexcept {
    return vixl_aa::XRegister(static_cast<int>(r));
}

}  // namespace

struct Emitter::Impl {
    vixl_aa::MacroAssembler masm;
    bool finalized{false};

    // Label store. Entry 0 is the sentinel "not a label", so valid
    // label IDs start at 1 and index into `labels[id - 1]`.
    std::vector<std::unique_ptr<vixl_aa::Label>> labels;

    Impl() : masm() {}
};

Emitter::Emitter() : impl_(new Impl{}) {}

Emitter::~Emitter() { delete impl_; }

void Emitter::movz(arm64::Reg rd, std::uint16_t imm16, unsigned hw) {
    // vixl's movz signature: void movz(const Register&, uint64_t imm, int shift).
    // `shift = -1` lets vixl pick; we pass the explicit bit shift.
    const int shift = static_cast<int>(hw) * 16;
    impl_->masm.movz(to_vixl_x(rd), static_cast<std::uint64_t>(imm16), shift);
}

void Emitter::mov_imm64(arm64::Reg rd, std::uint64_t imm) {
    impl_->masm.Mov(to_vixl_x(rd), imm);
}

void Emitter::mov_reg_reg(arm64::Reg rd, arm64::Reg rs) {
    impl_->masm.Mov(to_vixl_x(rd), to_vixl_x(rs));
}

void Emitter::add(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm) {
    impl_->masm.Add(to_vixl_x(rd), to_vixl_x(rn), to_vixl_x(rm));
}
void Emitter::sub(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm) {
    impl_->masm.Sub(to_vixl_x(rd), to_vixl_x(rn), to_vixl_x(rm));
}
void Emitter::mul(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm) {
    impl_->masm.Mul(to_vixl_x(rd), to_vixl_x(rn), to_vixl_x(rm));
}
void Emitter::and_(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm) {
    impl_->masm.And(to_vixl_x(rd), to_vixl_x(rn), to_vixl_x(rm));
}
void Emitter::orr(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm) {
    impl_->masm.Orr(to_vixl_x(rd), to_vixl_x(rn), to_vixl_x(rm));
}
void Emitter::eor(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm) {
    impl_->masm.Eor(to_vixl_x(rd), to_vixl_x(rn), to_vixl_x(rm));
}
void Emitter::lsl(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm) {
    impl_->masm.Lsl(to_vixl_x(rd), to_vixl_x(rn), to_vixl_x(rm));
}
void Emitter::lsr(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm) {
    impl_->masm.Lsr(to_vixl_x(rd), to_vixl_x(rn), to_vixl_x(rm));
}
void Emitter::asr(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm) {
    impl_->masm.Asr(to_vixl_x(rd), to_vixl_x(rn), to_vixl_x(rm));
}
void Emitter::ror(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm) {
    impl_->masm.Ror(to_vixl_x(rd), to_vixl_x(rn), to_vixl_x(rm));
}
void Emitter::rol(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm, arm64::Reg tmp) {
    // `rol rn, count` = `ror rn, (-count)` on ARM64. neg lowers to
    // `sub tmp, xzr, rm`, a single fast instruction.
    impl_->masm.Neg(to_vixl_x(tmp), to_vixl_x(rm));
    impl_->masm.Ror(to_vixl_x(rd),  to_vixl_x(rn), to_vixl_x(tmp));
}
void Emitter::neg(arm64::Reg rd, arm64::Reg rn) {
    impl_->masm.Neg(to_vixl_x(rd), to_vixl_x(rn));
}
void Emitter::clz(arm64::Reg rd, arm64::Reg rn) {
    impl_->masm.Clz(to_vixl_x(rd), to_vixl_x(rn));
}
void Emitter::cls(arm64::Reg rd, arm64::Reg rn) {
    impl_->masm.Cls(to_vixl_x(rd), to_vixl_x(rn));
}
void Emitter::rbit(arm64::Reg rd, arm64::Reg rn) {
    impl_->masm.Rbit(to_vixl_x(rd), to_vixl_x(rn));
}

void Emitter::zero_extend(arm64::Reg rd, arm64::Reg rn, ir::OpSize from_size) {
    switch (from_size) {
        case ir::OpSize::I8:
            impl_->masm.Uxtb(to_vixl_x(rd), to_vixl_x(rn));
            return;
        case ir::OpSize::I16:
            impl_->masm.Uxth(to_vixl_x(rd), to_vixl_x(rn));
            return;
        case ir::OpSize::I32:
            impl_->masm.Uxtw(to_vixl_x(rd), to_vixl_x(rn));
            return;
        case ir::OpSize::I64:
            impl_->masm.Mov(to_vixl_x(rd), to_vixl_x(rn));
            return;
    }
}

void Emitter::sign_extend(arm64::Reg rd, arm64::Reg rn, ir::OpSize from_size) {
    switch (from_size) {
        case ir::OpSize::I8:
            impl_->masm.Sxtb(to_vixl_x(rd), to_vixl_x(rn));
            return;
        case ir::OpSize::I16:
            impl_->masm.Sxth(to_vixl_x(rd), to_vixl_x(rn));
            return;
        case ir::OpSize::I32:
            impl_->masm.Sxtw(to_vixl_x(rd), to_vixl_x(rn));
            return;
        case ir::OpSize::I64:
            impl_->masm.Mov(to_vixl_x(rd), to_vixl_x(rn));
            return;
    }
}

void Emitter::truncate(arm64::Reg rd, arm64::Reg rn, ir::OpSize to_size) {
    zero_extend(rd, rn, to_size);
}

void Emitter::umulh(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm) {
    impl_->masm.Umulh(to_vixl_x(rd), to_vixl_x(rn), to_vixl_x(rm));
}
void Emitter::smulh(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm) {
    impl_->masm.Smulh(to_vixl_x(rd), to_vixl_x(rn), to_vixl_x(rm));
}
void Emitter::udiv(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm) {
    impl_->masm.Udiv(to_vixl_x(rd), to_vixl_x(rn), to_vixl_x(rm));
}
void Emitter::sdiv(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm) {
    impl_->masm.Sdiv(to_vixl_x(rd), to_vixl_x(rn), to_vixl_x(rm));
}
void Emitter::msub(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm, arm64::Reg ra) {
    impl_->masm.Msub(to_vixl_x(rd), to_vixl_x(rn),
                     to_vixl_x(rm), to_vixl_x(ra));
}

void Emitter::cmp(arm64::Reg rn, arm64::Reg rm) {
    impl_->masm.Cmp(to_vixl_x(rn), to_vixl_x(rm));
}

namespace {

// Build a vixl WRegister from our Reg enum (for 32-bit form loads/stores).
vixl_aa::WRegister to_vixl_w(arm64::Reg r) noexcept {
    return vixl_aa::WRegister(static_cast<int>(r));
}

}  // namespace

void Emitter::load(arm64::Reg rd, arm64::Reg raddr, ir::OpSize size) {
    const vixl_aa::MemOperand mo(to_vixl_x(raddr));
    switch (size) {
        case ir::OpSize::I8:  impl_->masm.Ldrb(to_vixl_x(rd), mo); return;
        case ir::OpSize::I16: impl_->masm.Ldrh(to_vixl_x(rd), mo); return;
        case ir::OpSize::I32: impl_->masm.Ldr (to_vixl_w(rd), mo); return;
        case ir::OpSize::I64: impl_->masm.Ldr (to_vixl_x(rd), mo); return;
    }
}

void Emitter::store(arm64::Reg rv, arm64::Reg raddr, ir::OpSize size) {
    const vixl_aa::MemOperand mo(to_vixl_x(raddr));
    switch (size) {
        case ir::OpSize::I8:  impl_->masm.Strb(to_vixl_x(rv), mo); return;
        case ir::OpSize::I16: impl_->masm.Strh(to_vixl_x(rv), mo); return;
        case ir::OpSize::I32: impl_->masm.Str (to_vixl_w(rv), mo); return;
        case ir::OpSize::I64: impl_->masm.Str (to_vixl_x(rv), mo); return;
    }
}

void Emitter::load_acquire(arm64::Reg rd, arm64::Reg raddr, ir::OpSize size) {
    const vixl_aa::MemOperand mo(to_vixl_x(raddr));
    switch (size) {
        case ir::OpSize::I8:  impl_->masm.Ldarb(to_vixl_x(rd), mo); return;
        case ir::OpSize::I16: impl_->masm.Ldarh(to_vixl_x(rd), mo); return;
        case ir::OpSize::I32: impl_->masm.Ldar (to_vixl_w(rd), mo); return;
        case ir::OpSize::I64: impl_->masm.Ldar (to_vixl_x(rd), mo); return;
    }
}

void Emitter::store_release(arm64::Reg rv, arm64::Reg raddr, ir::OpSize size) {
    const vixl_aa::MemOperand mo(to_vixl_x(raddr));
    switch (size) {
        case ir::OpSize::I8:  impl_->masm.Stlrb(to_vixl_x(rv), mo); return;
        case ir::OpSize::I16: impl_->masm.Stlrh(to_vixl_x(rv), mo); return;
        case ir::OpSize::I32: impl_->masm.Stlr (to_vixl_w(rv), mo); return;
        case ir::OpSize::I64: impl_->masm.Stlr (to_vixl_x(rv), mo); return;
    }
}

void Emitter::ldxr(arm64::Reg rd, arm64::Reg raddr, ir::OpSize size) {
    const vixl_aa::MemOperand mo(to_vixl_x(raddr));
    switch (size) {
        case ir::OpSize::I8:  impl_->masm.Ldxrb(to_vixl_w(rd), mo); return;
        case ir::OpSize::I16: impl_->masm.Ldxrh(to_vixl_w(rd), mo); return;
        case ir::OpSize::I32: impl_->masm.Ldxr (to_vixl_w(rd), mo); return;
        case ir::OpSize::I64: impl_->masm.Ldxr (to_vixl_x(rd), mo); return;
    }
}

void Emitter::stxr(arm64::Reg rs, arm64::Reg rv,
                   arm64::Reg raddr, ir::OpSize size) {
    const vixl_aa::MemOperand mo(to_vixl_x(raddr));
    switch (size) {
        case ir::OpSize::I8:
            impl_->masm.Stxrb(to_vixl_w(rs), to_vixl_w(rv), mo); return;
        case ir::OpSize::I16:
            impl_->masm.Stxrh(to_vixl_w(rs), to_vixl_w(rv), mo); return;
        case ir::OpSize::I32:
            impl_->masm.Stxr (to_vixl_w(rs), to_vixl_w(rv), mo); return;
        case ir::OpSize::I64:
            impl_->masm.Stxr (to_vixl_w(rs), to_vixl_x(rv), mo); return;
    }
}

void Emitter::ldaxr(arm64::Reg rd, arm64::Reg raddr, ir::OpSize size) {
    const vixl_aa::MemOperand mo(to_vixl_x(raddr));
    switch (size) {
        case ir::OpSize::I8:  impl_->masm.Ldaxrb(to_vixl_w(rd), mo); return;
        case ir::OpSize::I16: impl_->masm.Ldaxrh(to_vixl_w(rd), mo); return;
        case ir::OpSize::I32: impl_->masm.Ldaxr (to_vixl_w(rd), mo); return;
        case ir::OpSize::I64: impl_->masm.Ldaxr (to_vixl_x(rd), mo); return;
    }
}

void Emitter::stlxr(arm64::Reg rs, arm64::Reg rv,
                    arm64::Reg raddr, ir::OpSize size) {
    const vixl_aa::MemOperand mo(to_vixl_x(raddr));
    switch (size) {
        case ir::OpSize::I8:
            impl_->masm.Stlxrb(to_vixl_w(rs), to_vixl_w(rv), mo); return;
        case ir::OpSize::I16:
            impl_->masm.Stlxrh(to_vixl_w(rs), to_vixl_w(rv), mo); return;
        case ir::OpSize::I32:
            impl_->masm.Stlxr (to_vixl_w(rs), to_vixl_w(rv), mo); return;
        case ir::OpSize::I64:
            impl_->masm.Stlxr (to_vixl_w(rs), to_vixl_x(rv), mo); return;
    }
}

void Emitter::casal(arm64::Reg rs, arm64::Reg rt,
                    arm64::Reg raddr, ir::OpSize size) {
    const vixl_aa::MemOperand mo(to_vixl_x(raddr));
    switch (size) {
        case ir::OpSize::I8:
            impl_->masm.Casalb(to_vixl_w(rs), to_vixl_w(rt), mo); return;
        case ir::OpSize::I16:
            impl_->masm.Casalh(to_vixl_w(rs), to_vixl_w(rt), mo); return;
        case ir::OpSize::I32:
            impl_->masm.Casal (to_vixl_w(rs), to_vixl_w(rt), mo); return;
        case ir::OpSize::I64:
            impl_->masm.Casal (to_vixl_x(rs), to_vixl_x(rt), mo); return;
    }
}

void Emitter::ldaddal(arm64::Reg rs, arm64::Reg rt,
                      arm64::Reg raddr, ir::OpSize size) {
    const vixl_aa::MemOperand mo(to_vixl_x(raddr));
    switch (size) {
        case ir::OpSize::I8:
            impl_->masm.Ldaddalb(to_vixl_w(rs), to_vixl_w(rt), mo); return;
        case ir::OpSize::I16:
            impl_->masm.Ldaddalh(to_vixl_w(rs), to_vixl_w(rt), mo); return;
        case ir::OpSize::I32:
            impl_->masm.Ldaddal (to_vixl_w(rs), to_vixl_w(rt), mo); return;
        case ir::OpSize::I64:
            impl_->masm.Ldaddal (to_vixl_x(rs), to_vixl_x(rt), mo); return;
    }
}

void Emitter::load_offset(arm64::Reg rd, arm64::Reg rbase, std::int32_t imm) {
    impl_->masm.Ldr(to_vixl_x(rd),
                    vixl_aa::MemOperand(to_vixl_x(rbase), imm));
}

void Emitter::store_offset(arm64::Reg rv, arm64::Reg rbase, std::int32_t imm) {
    impl_->masm.Str(to_vixl_x(rv),
                    vixl_aa::MemOperand(to_vixl_x(rbase), imm));
}

void Emitter::sp_load(arm64::Reg rd, std::int32_t imm) {
    impl_->masm.Ldr(to_vixl_x(rd),
                    vixl_aa::MemOperand(vixl_aa::sp, imm));
}

void Emitter::sp_store(arm64::Reg rv, std::int32_t imm) {
    impl_->masm.Str(to_vixl_x(rv),
                    vixl_aa::MemOperand(vixl_aa::sp, imm));
}

void Emitter::push_pair(arm64::Reg r1, arm64::Reg r2) {
    // stp r1, r2, [sp, #-16]!  (pre-index)
    impl_->masm.Stp(to_vixl_x(r1), to_vixl_x(r2),
                    vixl_aa::MemOperand(vixl_aa::sp, -16, vixl_aa::PreIndex));
}

void Emitter::pop_pair(arm64::Reg r1, arm64::Reg r2) {
    // ldp r1, r2, [sp], #16  (post-index)
    impl_->masm.Ldp(to_vixl_x(r1), to_vixl_x(r2),
                    vixl_aa::MemOperand(vixl_aa::sp, 16, vixl_aa::PostIndex));
}

namespace {

// Shared helper: Prisma CondCode → vixl Condition. ARM64 unsigned / signed
// mnemonics align with x86 once you know the mapping.
vixl_aa::Condition to_vixl_cond(ir::CondCode cc) noexcept {
    switch (cc) {
        case ir::CondCode::Eq:  return vixl_aa::eq;
        case ir::CondCode::Ne:  return vixl_aa::ne;
        case ir::CondCode::Ult: return vixl_aa::lo;  // unsigned <
        case ir::CondCode::Ule: return vixl_aa::ls;  // unsigned <=
        case ir::CondCode::Ugt: return vixl_aa::hi;  // unsigned >
        case ir::CondCode::Uge: return vixl_aa::hs;  // unsigned >=
        case ir::CondCode::Slt: return vixl_aa::lt;
        case ir::CondCode::Sle: return vixl_aa::le;
        case ir::CondCode::Sgt: return vixl_aa::gt;
        case ir::CondCode::Sge: return vixl_aa::ge;
        case ir::CondCode::Cc:  return vixl_aa::cc;  // carry clear (unsigned <)
        case ir::CondCode::Nc:  return vixl_aa::cs;  // carry set   (unsigned >=)
        case ir::CondCode::Ov:  return vixl_aa::vs;  // overflow set
        case ir::CondCode::NoOv: return vixl_aa::vc; // overflow clear
        case ir::CondCode::Mi:  return vixl_aa::mi;  // sign set (negative)
        case ir::CondCode::Pl:  return vixl_aa::pl;  // sign clear (non-negative)
    }
    return vixl_aa::nv;  // unreachable
}

}  // namespace

void Emitter::cset(arm64::Reg rd, ir::CondCode cc) {
    impl_->masm.Cset(to_vixl_x(rd), to_vixl_cond(cc));
}

void Emitter::csel(arm64::Reg rd, arm64::Reg rn_true, arm64::Reg rn_false,
                   ir::CondCode cc) {
    impl_->masm.Csel(to_vixl_x(rd),
                     to_vixl_x(rn_true),
                     to_vixl_x(rn_false),
                     to_vixl_cond(cc));
}

void Emitter::ret(arm64::Reg rn) {
    impl_->masm.Ret(to_vixl_x(rn));
}

// --- Label management ------------------------------------------------------

Emitter::Label Emitter::create_label() {
    impl_->labels.push_back(std::make_unique<vixl_aa::Label>());
    return Label{impl_->labels.size()};  // 1-based: id 0 is the sentinel
}

void Emitter::bind(Label label) {
    // Precondition: valid id. Out-of-range or 0 is a programming bug;
    // we silently drop rather than throw to keep the API exception-free.
    if (label.id == 0 || label.id > impl_->labels.size()) return;
    impl_->masm.Bind(impl_->labels[label.id - 1].get());
}

void Emitter::branch(Label label) {
    if (label.id == 0 || label.id > impl_->labels.size()) return;
    impl_->masm.B(impl_->labels[label.id - 1].get());
}

void Emitter::branch_cc(Label label, ir::CondCode cc) {
    if (label.id == 0 || label.id > impl_->labels.size()) return;
    impl_->masm.B(impl_->labels[label.id - 1].get(), to_vixl_cond(cc));
}

std::size_t Emitter::literal_pool_size() const noexcept {
    return impl_->masm.GetLiteralPoolSize();
}

void Emitter::flush_literal_pool() {
    impl_->masm.EmitLiteralPool(vixl_aa::LiteralPool::kBranchRequired);
}

void Emitter::finalize() {
    if (impl_->finalized) return;
    impl_->masm.FinalizeCode();
    impl_->finalized = true;
}

std::span<const std::uint8_t> Emitter::code_bytes() const noexcept {
    // After FinalizeCode the buffer is stable; unsafe to read before.
    if (!impl_->finalized) return {};
    const auto* buf = impl_->masm.GetBuffer();
    const auto* data = reinterpret_cast<const std::uint8_t*>(buf->GetStartAddress<const void*>());
    const std::size_t size = buf->GetSizeInBytes();
    return std::span<const std::uint8_t>{data, size};
}

std::string Emitter::disassemble() const {
    if (!impl_->finalized) return "<not finalized>";

    std::ostringstream os;
    vixl_aa::Decoder decoder;
    vixl_aa::Disassembler disasm;
    decoder.AppendVisitor(&disasm);

    const auto bytes = code_bytes();
    const auto* start = reinterpret_cast<const vixl_aa::Instruction*>(bytes.data());
    const auto* end   = reinterpret_cast<const vixl_aa::Instruction*>(bytes.data() + bytes.size());
    for (const vixl_aa::Instruction* inst = start; inst < end;
         inst = inst->GetNextInstruction()) {
        decoder.Decode(inst);
        os << disasm.GetOutput() << "\n";
    }
    return os.str();
}

}  // namespace prisma::backend
