// core/src/backend/emitter.cpp — vixl-backed implementation of Emitter.
//
// Keeps vixl headers out of `emitter.hpp` via pimpl: clients of prisma_emitter
// do not need vixl on their include path.

#include "prisma/emitter.hpp"

#include <cstddef>
#include <cstdint>
#include <cstring>
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

void Emitter::adds(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm) {
    impl_->masm.Adds(to_vixl_x(rd), to_vixl_x(rn), to_vixl_x(rm));
}
void Emitter::ands(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm) {
    impl_->masm.Ands(to_vixl_x(rd), to_vixl_x(rn), to_vixl_x(rm));
}

// 32-bit W-register ALU forms — defined after the anonymous namespace
// that hosts `to_vixl_w` (further down in this file).


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

// --- 32-bit W-register ALU forms (F1-BK-010) ------------------------------

void Emitter::add_w(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm) {
    impl_->masm.Add(to_vixl_w(rd), to_vixl_w(rn), to_vixl_w(rm));
}
void Emitter::sub_w(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm) {
    impl_->masm.Sub(to_vixl_w(rd), to_vixl_w(rn), to_vixl_w(rm));
}
void Emitter::and_w(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm) {
    impl_->masm.And(to_vixl_w(rd), to_vixl_w(rn), to_vixl_w(rm));
}
void Emitter::orr_w(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm) {
    impl_->masm.Orr(to_vixl_w(rd), to_vixl_w(rn), to_vixl_w(rm));
}
void Emitter::eor_w(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm) {
    impl_->masm.Eor(to_vixl_w(rd), to_vixl_w(rn), to_vixl_w(rm));
}
void Emitter::mov_w_reg_reg(arm64::Reg rd, arm64::Reg rs) {
    impl_->masm.Mov(to_vixl_w(rd), to_vixl_w(rs));
}

// --- Width adjustment (F1-BK-022) -----------------------------------------
//
// `to_vixl_w` is defined earlier in this file; reuse it.

void Emitter::sxtb(arm64::Reg rd, arm64::Reg rn) {
    impl_->masm.Sxtb(to_vixl_x(rd), to_vixl_w(rn));
}
void Emitter::sxth(arm64::Reg rd, arm64::Reg rn) {
    impl_->masm.Sxth(to_vixl_x(rd), to_vixl_w(rn));
}
void Emitter::sxtw(arm64::Reg rd, arm64::Reg rn) {
    impl_->masm.Sxtw(to_vixl_x(rd), to_vixl_w(rn));
}
void Emitter::uxtb(arm64::Reg rd, arm64::Reg rn) {
    impl_->masm.Uxtb(to_vixl_x(rd), to_vixl_w(rn));
}
void Emitter::uxth(arm64::Reg rd, arm64::Reg rn) {
    impl_->masm.Uxth(to_vixl_x(rd), to_vixl_w(rn));
}
void Emitter::uxtw(arm64::Reg rd, arm64::Reg rn) {
    // AArch64 zero-extends 32→64 implicitly when the destination is
    // written through its W-view.  `mov wd, wn` is the canonical idiom.
    impl_->masm.Mov(to_vixl_w(rd), to_vixl_w(rn));
}

// --- Floating-point ALU (F1-BK-013) ---------------------------------------

namespace {

vixl_aa::SRegister to_vixl_s(Emitter::FpReg r) noexcept {
    return vixl_aa::SRegister(static_cast<int>(r));
}
vixl_aa::DRegister to_vixl_d(Emitter::FpReg r) noexcept {
    return vixl_aa::DRegister(static_cast<int>(r));
}

}  // namespace

void Emitter::fadd(FpReg rd, FpReg rn, FpReg rm, ir::FpSize sz) {
    if (sz == ir::FpSize::F32) {
        impl_->masm.Fadd(to_vixl_s(rd), to_vixl_s(rn), to_vixl_s(rm));
    } else {
        impl_->masm.Fadd(to_vixl_d(rd), to_vixl_d(rn), to_vixl_d(rm));
    }
}
void Emitter::fsub(FpReg rd, FpReg rn, FpReg rm, ir::FpSize sz) {
    if (sz == ir::FpSize::F32) {
        impl_->masm.Fsub(to_vixl_s(rd), to_vixl_s(rn), to_vixl_s(rm));
    } else {
        impl_->masm.Fsub(to_vixl_d(rd), to_vixl_d(rn), to_vixl_d(rm));
    }
}
void Emitter::fmul(FpReg rd, FpReg rn, FpReg rm, ir::FpSize sz) {
    if (sz == ir::FpSize::F32) {
        impl_->masm.Fmul(to_vixl_s(rd), to_vixl_s(rn), to_vixl_s(rm));
    } else {
        impl_->masm.Fmul(to_vixl_d(rd), to_vixl_d(rn), to_vixl_d(rm));
    }
}
void Emitter::fdiv(FpReg rd, FpReg rn, FpReg rm, ir::FpSize sz) {
    if (sz == ir::FpSize::F32) {
        impl_->masm.Fdiv(to_vixl_s(rd), to_vixl_s(rn), to_vixl_s(rm));
    } else {
        impl_->masm.Fdiv(to_vixl_d(rd), to_vixl_d(rn), to_vixl_d(rm));
    }
}

void Emitter::fmov_v_from_x(FpReg rd, arm64::Reg rn, ir::FpSize sz) {
    const vixl_aa::Register r =
        sz == ir::FpSize::F32 ? vixl_aa::Register(to_vixl_w(rn))
                              : vixl_aa::Register(to_vixl_x(rn));
    if (sz == ir::FpSize::F32) {
        impl_->masm.Fmov(vixl_aa::VRegister(to_vixl_s(rd)), r);
    } else {
        impl_->masm.Fmov(vixl_aa::VRegister(to_vixl_d(rd)), r);
    }
}
void Emitter::fmov_x_from_v(arm64::Reg rd, FpReg rn, ir::FpSize sz) {
    const vixl_aa::Register r =
        sz == ir::FpSize::F32 ? vixl_aa::Register(to_vixl_w(rd))
                              : vixl_aa::Register(to_vixl_x(rd));
    if (sz == ir::FpSize::F32) {
        impl_->masm.Fmov(r, vixl_aa::VRegister(to_vixl_s(rn)));
    } else {
        impl_->masm.Fmov(r, vixl_aa::VRegister(to_vixl_d(rn)));
    }
}

namespace { constexpr int kInternalFpScratchV = 31; }

void Emitter::scvtf(FpReg rd, arm64::Reg rn, ir::OpSize int_sz, ir::FpSize fp_sz) {
    const vixl_aa::Register r =
        int_sz == ir::OpSize::I32 ? vixl_aa::Register(to_vixl_w(rn))
                                  : vixl_aa::Register(to_vixl_x(rn));
    const vixl_aa::VRegister v =
        fp_sz == ir::FpSize::F32 ? vixl_aa::VRegister(to_vixl_s(rd))
                                 : vixl_aa::VRegister(to_vixl_d(rd));
    impl_->masm.Scvtf(v, r);
}
void Emitter::fcvt_scalar_with_upper(FpReg rd, FpReg lhs, FpReg src,
                                     ir::FpSize src_sz, ir::FpSize dst_sz) {
    // 1. fcvt v31.dst[0] from src.src. Wrap in VRegister to disambig
    // vixl's Fmov overloads.
    if (src_sz == ir::FpSize::F32 && dst_sz == ir::FpSize::F64) {
        impl_->masm.Fcvt(vixl_aa::VRegister(vixl_aa::DRegister(kInternalFpScratchV)),
                         vixl_aa::VRegister(to_vixl_s(src)));
    } else if (src_sz == ir::FpSize::F64 && dst_sz == ir::FpSize::F32) {
        impl_->masm.Fcvt(vixl_aa::VRegister(vixl_aa::SRegister(kInternalFpScratchV)),
                         vixl_aa::VRegister(to_vixl_d(src)));
    } else {
        if (dst_sz == ir::FpSize::F32) {
            impl_->masm.Fmov(vixl_aa::VRegister(vixl_aa::SRegister(kInternalFpScratchV)),
                             vixl_aa::VRegister(to_vixl_s(src)));
        } else {
            impl_->masm.Fmov(vixl_aa::VRegister(vixl_aa::DRegister(kInternalFpScratchV)),
                             vixl_aa::VRegister(to_vixl_d(src)));
        }
    }
    // 2. mov rd.16b, lhs.16b — preserve upper from lhs.
    const vixl_aa::VRegister v_d_q  (static_cast<int>(rd),  vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_lhs_q(static_cast<int>(lhs), vixl_aa::kFormat16B);
    impl_->masm.Mov(v_d_q, v_lhs_q);
    // 3. ins rd.dst[0] from v31.dst[0].
    if (dst_sz == ir::FpSize::F32) {
        const vixl_aa::VRegister v_d_s4(static_cast<int>(rd),    vixl_aa::kFormat4S);
        const vixl_aa::VRegister v_t_s4(kInternalFpScratchV,     vixl_aa::kFormat4S);
        impl_->masm.Mov(v_d_s4, 0, v_t_s4, 0);
    } else {
        const vixl_aa::VRegister v_d_d2(static_cast<int>(rd),    vixl_aa::kFormat2D);
        const vixl_aa::VRegister v_t_d2(kInternalFpScratchV,     vixl_aa::kFormat2D);
        impl_->masm.Mov(v_d_d2, 0, v_t_d2, 0);
    }
}

void Emitter::fcvtzs(arm64::Reg rd, FpReg rn, ir::FpSize fp_sz, ir::OpSize int_sz) {
    const vixl_aa::Register r =
        int_sz == ir::OpSize::I32 ? vixl_aa::Register(to_vixl_w(rd))
                                  : vixl_aa::Register(to_vixl_x(rd));
    const vixl_aa::VRegister v =
        fp_sz == ir::FpSize::F32 ? vixl_aa::VRegister(to_vixl_s(rn))
                                 : vixl_aa::VRegister(to_vixl_d(rn));
    impl_->masm.Fcvtzs(r, v);
}

void Emitter::fmov_imm(FpReg rd, std::uint64_t bits, ir::FpSize sz) {
    if (sz == ir::FpSize::F32) {
        float f;
        const std::uint32_t low = static_cast<std::uint32_t>(bits);
        std::memcpy(&f, &low, sizeof f);
        impl_->masm.Fmov(to_vixl_s(rd), f);
    } else {
        double d;
        std::memcpy(&d, &bits, sizeof d);
        impl_->masm.Fmov(to_vixl_d(rd), d);
    }
}

// --- 128-bit NEON SIMD (F1-BK-012) ----------------------------------------

namespace {

// Pick the correct vixl VRegister "arrangement" for the lane size.
vixl_aa::VRegister to_vixl_q(Emitter::FpReg r, Emitter::VecLane lane) noexcept {
    using L = Emitter::VecLane;
    const int code = static_cast<int>(r);
    switch (lane) {
        case L::B16: return vixl_aa::VRegister(code, vixl_aa::kFormat16B);
        case L::H8:  return vixl_aa::VRegister(code, vixl_aa::kFormat8H);
        case L::S4:  return vixl_aa::VRegister(code, vixl_aa::kFormat4S);
        case L::D2:  return vixl_aa::VRegister(code, vixl_aa::kFormat2D);
    }
    return vixl_aa::VRegister(code, vixl_aa::kFormat16B);
}

// Bitwise ops are lane-agnostic; use the byte view.
vixl_aa::VRegister to_vixl_q_bitwise(Emitter::FpReg r) noexcept {
    return vixl_aa::VRegister(static_cast<int>(r), vixl_aa::kFormat16B);
}

// 128-bit Q-register (single full vector, no arrangement needed for
// some load/store variants).
vixl_aa::QRegister to_vixl_qreg(Emitter::FpReg r) noexcept {
    return vixl_aa::QRegister(static_cast<int>(r));
}

}  // namespace

void Emitter::vadd_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane) {
    impl_->masm.Add(to_vixl_q(rd, lane), to_vixl_q(rn, lane),
                    to_vixl_q(rm, lane));
}
void Emitter::vsub_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane) {
    impl_->masm.Sub(to_vixl_q(rd, lane), to_vixl_q(rn, lane),
                    to_vixl_q(rm, lane));
}
void Emitter::vand_q(FpReg rd, FpReg rn, FpReg rm) {
    impl_->masm.And(to_vixl_q_bitwise(rd), to_vixl_q_bitwise(rn),
                    to_vixl_q_bitwise(rm));
}
void Emitter::vorr_q(FpReg rd, FpReg rn, FpReg rm) {
    impl_->masm.Orr(to_vixl_q_bitwise(rd), to_vixl_q_bitwise(rn),
                    to_vixl_q_bitwise(rm));
}
void Emitter::veor_q(FpReg rd, FpReg rn, FpReg rm) {
    impl_->masm.Eor(to_vixl_q_bitwise(rd), to_vixl_q_bitwise(rn),
                    to_vixl_q_bitwise(rm));
}
void Emitter::vmul_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane) {
    impl_->masm.Mul(to_vixl_q(rd, lane), to_vixl_q(rn, lane),
                    to_vixl_q(rm, lane));
}

namespace {
vixl_aa::VRegister to_vixl_q_fp(Emitter::FpReg r, Emitter::VecLane lane) noexcept {
    using L = Emitter::VecLane;
    int code = static_cast<int>(r);
    switch (lane) {
        case L::S4: return vixl_aa::VRegister(code, vixl_aa::kFormat4S);
        case L::D2: return vixl_aa::VRegister(code, vixl_aa::kFormat2D);
        default:    break;
    }
    // Lane invariant: only S4/D2 reach here; gated by IR lowerer.
    return vixl_aa::VRegister(code, vixl_aa::kFormat4S);
}
}  // namespace

void Emitter::vfadd_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane) {
    impl_->masm.Fadd(to_vixl_q_fp(rd, lane), to_vixl_q_fp(rn, lane),
                     to_vixl_q_fp(rm, lane));
}
void Emitter::vfsub_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane) {
    impl_->masm.Fsub(to_vixl_q_fp(rd, lane), to_vixl_q_fp(rn, lane),
                     to_vixl_q_fp(rm, lane));
}
void Emitter::vfmul_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane) {
    impl_->masm.Fmul(to_vixl_q_fp(rd, lane), to_vixl_q_fp(rn, lane),
                     to_vixl_q_fp(rm, lane));
}
void Emitter::vfdiv_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane) {
    impl_->masm.Fdiv(to_vixl_q_fp(rd, lane), to_vixl_q_fp(rn, lane),
                     to_vixl_q_fp(rm, lane));
}

// F2-IR-006 — internal scratch V31, never used by the SSA scratch pool
// (which only allocates V0..V7). Common impl for the four scalar SSE ops:
//   1. mov  v31.16b, vn.16b               ; copy lhs into scratch (preserves upper)
//   2. f<op> s31/d31, sn/dn, sm/dm        ; scalar op writes low lane only
//                                          (zero-extends via INS in step 3)
//   3. ins  v31.s/d[0], v_tmp.s/d[0]      ; not needed — step 2 wrote v31[0] directly
//   4. mov  vd.16b, v31.16b               ; final result into rd
//
// vixl will fold (1)+(4) when rd == rn, but we keep the sequence simple
// and let the assembler optimise. Using V31 as a stable scratch means
// the SSA register allocator (which only uses V0..V7) never aliases it.
namespace {
void emit_scalar_sse_op(vixl_aa::MacroAssembler& masm,
                        int rd_c, int rn_c, int rm_c,
                        ir::FpSize sz,
                        char which) {
    // Sequence (preserves upper bits of lhs):
    //   1. f<op> s31/d31, sn/dn, sm/dm   — scalar result into V31's
    //      low lane; ARMv8 zero-extends the upper 96/64 bits, but we
    //      only care about V31's low lane.
    //   2. mov   v_rd.16b, v_rn.16b      — full 128-bit copy of lhs.
    //      vixl folds this away when rd == rn.
    //   3. mov   v_rd.s[0], v31.s[0]     — INS lane 0 from V31; upper
    //      lanes of v_rd are unchanged.
    const vixl_aa::VRegister v_n_q (rn_c, vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_d_q (rd_c, vixl_aa::kFormat16B);
    if (sz == ir::FpSize::F32) {
        const vixl_aa::SRegister s_t(kInternalFpScratchV);
        const vixl_aa::SRegister s_n(rn_c);
        const vixl_aa::SRegister s_m(rm_c);
        switch (which) {
            case '+': masm.Fadd(s_t, s_n, s_m); break;
            case '-': masm.Fsub(s_t, s_n, s_m); break;
            case '*': masm.Fmul(s_t, s_n, s_m); break;
            case '/': masm.Fdiv(s_t, s_n, s_m); break;
        }
        masm.Mov(v_d_q, v_n_q);
        const vixl_aa::VRegister v_d_s4(rd_c,                vixl_aa::kFormat4S);
        const vixl_aa::VRegister v_t_s4(kInternalFpScratchV, vixl_aa::kFormat4S);
        masm.Mov(v_d_s4, 0, v_t_s4, 0);
    } else {
        const vixl_aa::DRegister d_t(kInternalFpScratchV);
        const vixl_aa::DRegister d_n(rn_c);
        const vixl_aa::DRegister d_m(rm_c);
        switch (which) {
            case '+': masm.Fadd(d_t, d_n, d_m); break;
            case '-': masm.Fsub(d_t, d_n, d_m); break;
            case '*': masm.Fmul(d_t, d_n, d_m); break;
            case '/': masm.Fdiv(d_t, d_n, d_m); break;
        }
        masm.Mov(v_d_q, v_n_q);
        const vixl_aa::VRegister v_d_d2(rd_c,                vixl_aa::kFormat2D);
        const vixl_aa::VRegister v_t_d2(kInternalFpScratchV, vixl_aa::kFormat2D);
        masm.Mov(v_d_d2, 0, v_t_d2, 0);
    }
}
}  // namespace

void Emitter::vfadd_scalar(FpReg rd, FpReg rn, FpReg rm, ir::FpSize sz) {
    emit_scalar_sse_op(impl_->masm,
        static_cast<int>(rd), static_cast<int>(rn), static_cast<int>(rm), sz, '+');
}
void Emitter::vfsub_scalar(FpReg rd, FpReg rn, FpReg rm, ir::FpSize sz) {
    emit_scalar_sse_op(impl_->masm,
        static_cast<int>(rd), static_cast<int>(rn), static_cast<int>(rm), sz, '-');
}
void Emitter::vfmul_scalar(FpReg rd, FpReg rn, FpReg rm, ir::FpSize sz) {
    emit_scalar_sse_op(impl_->masm,
        static_cast<int>(rd), static_cast<int>(rn), static_cast<int>(rm), sz, '*');
}
void Emitter::vfdiv_scalar(FpReg rd, FpReg rn, FpReg rm, ir::FpSize sz) {
    emit_scalar_sse_op(impl_->masm,
        static_cast<int>(rd), static_cast<int>(rn), static_cast<int>(rm), sz, '/');
}

void Emitter::vcmeq_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane) {
    impl_->masm.Cmeq(to_vixl_q(rd, lane), to_vixl_q(rn, lane),
                     to_vixl_q(rm, lane));
}
void Emitter::vcmgt_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane) {
    impl_->masm.Cmgt(to_vixl_q(rd, lane), to_vixl_q(rn, lane),
                     to_vixl_q(rm, lane));
}

void Emitter::vzip1_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane) {
    impl_->masm.Zip1(to_vixl_q(rd, lane), to_vixl_q(rn, lane),
                     to_vixl_q(rm, lane));
}
void Emitter::vzip2_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane) {
    impl_->masm.Zip2(to_vixl_q(rd, lane), to_vixl_q(rn, lane),
                     to_vixl_q(rm, lane));
}

namespace {
unsigned lane_bits(Emitter::VecLane lane) noexcept {
    switch (lane) {
        case Emitter::VecLane::B16: return 8;
        case Emitter::VecLane::H8:  return 16;
        case Emitter::VecLane::S4:  return 32;
        case Emitter::VecLane::D2:  return 64;
    }
    return 8;
}
}  // namespace

void Emitter::vshl_imm_q(FpReg rd, FpReg rn, std::uint8_t count, VecLane lane) {
    const unsigned bits = lane_bits(lane);
    if (count >= bits) {
        // SSE: shift count >= lane width → result is zero.
        impl_->masm.Movi(to_vixl_q_bitwise(rd), 0);
        return;
    }
    impl_->masm.Shl(to_vixl_q(rd, lane), to_vixl_q(rn, lane),
                    static_cast<int>(count));
}
void Emitter::vushr_imm_q(FpReg rd, FpReg rn, std::uint8_t count, VecLane lane) {
    const unsigned bits = lane_bits(lane);
    if (count >= bits) {
        impl_->masm.Movi(to_vixl_q_bitwise(rd), 0);
        return;
    }
    if (count == 0) {
        impl_->masm.Mov(to_vixl_q_bitwise(rd), to_vixl_q_bitwise(rn));
        return;
    }
    impl_->masm.Ushr(to_vixl_q(rd, lane), to_vixl_q(rn, lane),
                     static_cast<int>(count));
}
void Emitter::vshlb_imm_q(FpReg rd, FpReg rn, std::uint8_t count) {
    if (count >= 16) {
        impl_->masm.Movi(to_vixl_q_bitwise(rd), 0);
        return;
    }
    if (count == 0) {
        impl_->masm.Mov(to_vixl_q_bitwise(rd), to_vixl_q_bitwise(rn));
        return;
    }
    // Build a zero scratch in V31 (movi clears it). Then ext with
    // (16 - count) selects bytes from { zero, src }.
    const vixl_aa::VRegister v_t(kInternalFpScratchV, vixl_aa::kFormat16B);
    impl_->masm.Movi(v_t, 0);
    impl_->masm.Ext(to_vixl_q_bitwise(rd), v_t, to_vixl_q_bitwise(rn),
                    static_cast<int>(16 - count));
}
void Emitter::vshrb_imm_q(FpReg rd, FpReg rn, std::uint8_t count) {
    if (count >= 16) {
        impl_->masm.Movi(to_vixl_q_bitwise(rd), 0);
        return;
    }
    if (count == 0) {
        impl_->masm.Mov(to_vixl_q_bitwise(rd), to_vixl_q_bitwise(rn));
        return;
    }
    const vixl_aa::VRegister v_t(kInternalFpScratchV, vixl_aa::kFormat16B);
    impl_->masm.Movi(v_t, 0);
    impl_->masm.Ext(to_vixl_q_bitwise(rd), to_vixl_q_bitwise(rn), v_t,
                    static_cast<int>(count));
}

void Emitter::vsshr_imm_q(FpReg rd, FpReg rn, std::uint8_t count, VecLane lane) {
    const unsigned bits = lane_bits(lane);
    // PSRA semantics: count >= lane bits → result lane = sign-fill, i.e.
    // shift by (bits - 1) to broadcast the sign bit.
    const unsigned eff = count >= bits ? bits - 1 : count;
    if (eff == 0) {
        impl_->masm.Mov(to_vixl_q_bitwise(rd), to_vixl_q_bitwise(rn));
        return;
    }
    impl_->masm.Sshr(to_vixl_q(rd, lane), to_vixl_q(rn, lane),
                     static_cast<int>(eff));
}

void Emitter::vshuffle_s4(FpReg rd, FpReg rn, std::uint8_t control) {
    // Build the result in V31 first to handle the rd == rn case.
    const vixl_aa::VRegister v_t_s4(kInternalFpScratchV, vixl_aa::kFormat4S);
    const vixl_aa::VRegister v_n_s4(static_cast<int>(rn), vixl_aa::kFormat4S);
    for (int i = 0; i < 4; ++i) {
        const int src_lane = (control >> (2 * i)) & 0x3;
        impl_->masm.Mov(v_t_s4, i, v_n_s4, src_lane);
    }
    const vixl_aa::VRegister v_t_q(kInternalFpScratchV, vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_d_q(static_cast<int>(rd), vixl_aa::kFormat16B);
    impl_->masm.Mov(v_d_q, v_t_q);
}

void Emitter::vld1_q(FpReg rd, arm64::Reg base) {
    impl_->masm.Ldr(to_vixl_qreg(rd), vixl_aa::MemOperand(to_vixl_x(base)));
}
void Emitter::vst1_q(FpReg rs, arm64::Reg base) {
    impl_->masm.Str(to_vixl_qreg(rs), vixl_aa::MemOperand(to_vixl_x(base)));
}
void Emitter::vld1_q_offset(FpReg rd, arm64::Reg base, std::int32_t imm) {
    impl_->masm.Ldr(to_vixl_qreg(rd),
                    vixl_aa::MemOperand(to_vixl_x(base), imm));
}
void Emitter::vst1_q_offset(FpReg rs, arm64::Reg base, std::int32_t imm) {
    impl_->masm.Str(to_vixl_qreg(rs),
                    vixl_aa::MemOperand(to_vixl_x(base), imm));
}

// --- Memory fences (F1-BK-023) --------------------------------------------

void Emitter::dmb(BarrierKind k) {
    using vixl_aa::BarrierType;
    using vixl_aa::BarrierDomain;
    BarrierType bt;
    switch (k) {
        case BarrierKind::Ish:   bt = vixl_aa::BarrierAll;    break;
        case BarrierKind::IshLd: bt = vixl_aa::BarrierReads;  break;
        case BarrierKind::IshSt: bt = vixl_aa::BarrierWrites; break;
    }
    impl_->masm.Dmb(vixl_aa::InnerShareable, bt);
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

void Emitter::cbnz(arm64::Reg r, Label label) {
    if (label.id == 0 || label.id > impl_->labels.size()) return;
    impl_->masm.Cbnz(to_vixl_x(r), impl_->labels[label.id - 1].get());
}

void Emitter::cbz(arm64::Reg r, Label label) {
    if (label.id == 0 || label.id > impl_->labels.size()) return;
    impl_->masm.Cbz(to_vixl_x(r), impl_->labels[label.id - 1].get());
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
