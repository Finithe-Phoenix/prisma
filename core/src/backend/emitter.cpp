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

// Build a vixl WRegister from our Reg enum (for 32-bit form loads/stores).
vixl_aa::WRegister to_vixl_w(arm64::Reg r) noexcept {
    return vixl_aa::WRegister(static_cast<int>(r));
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

void Emitter::mov_reg_reg(arm64::Reg rd, arm64::Reg rs, ir::OpSize size) {
    zero_extend(rd, rs, size);
}

void Emitter::store_reg_reg(arm64::Reg rd, arm64::Reg rs, ir::OpSize size) {
    switch (size) {
        case ir::OpSize::I8:
            impl_->masm.Bfxil(to_vixl_x(rd), to_vixl_x(rs), 0, 8);
            return;
        case ir::OpSize::I16:
            impl_->masm.Bfxil(to_vixl_x(rd), to_vixl_x(rs), 0, 16);
            return;
        case ir::OpSize::I32:
            impl_->masm.Mov(to_vixl_w(rd), to_vixl_w(rs));
            return;
        case ir::OpSize::I64:
            impl_->masm.Mov(to_vixl_x(rd), to_vixl_x(rs));
            return;
    }
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
void Emitter::lsr_imm(arm64::Reg rd, arm64::Reg rn, unsigned shift) {
    impl_->masm.Lsr(to_vixl_x(rd), to_vixl_x(rn), shift);
}
void Emitter::add_lsl(arm64::Reg rd, arm64::Reg rn, arm64::Reg rm,
                      unsigned shift) {
    impl_->masm.Add(to_vixl_x(rd), to_vixl_x(rn),
                    vixl_aa::Operand(to_vixl_x(rm), vixl_aa::LSL, shift));
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

void Emitter::zero_extend(arm64::Reg rd, arm64::Reg rn, ir::OpSize from_size) {
    switch (from_size) {
        case ir::OpSize::I8:  uxtb(rd, rn); return;
        case ir::OpSize::I16: uxth(rd, rn); return;
        case ir::OpSize::I32: uxtw(rd, rn); return;
        case ir::OpSize::I64: mov_reg_reg(rd, rn); return;
    }
}

void Emitter::sign_extend(arm64::Reg rd, arm64::Reg rn, ir::OpSize from_size) {
    switch (from_size) {
        case ir::OpSize::I8:  sxtb(rd, rn); return;
        case ir::OpSize::I16: sxth(rd, rn); return;
        case ir::OpSize::I32: sxtw(rd, rn); return;
        case ir::OpSize::I64: mov_reg_reg(rd, rn); return;
    }
}

void Emitter::truncate(arm64::Reg rd, arm64::Reg rn, ir::OpSize to_size) {
    zero_extend(rd, rn, to_size);
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
void Emitter::vsqadd_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane) {
    impl_->masm.Sqadd(to_vixl_q(rd, lane), to_vixl_q(rn, lane),
                      to_vixl_q(rm, lane));
}
void Emitter::vuqadd_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane) {
    impl_->masm.Uqadd(to_vixl_q(rd, lane), to_vixl_q(rn, lane),
                      to_vixl_q(rm, lane));
}
void Emitter::vsqsub_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane) {
    impl_->masm.Sqsub(to_vixl_q(rd, lane), to_vixl_q(rn, lane),
                      to_vixl_q(rm, lane));
}
void Emitter::vuqsub_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane) {
    impl_->masm.Uqsub(to_vixl_q(rd, lane), to_vixl_q(rn, lane),
                      to_vixl_q(rm, lane));
}
void Emitter::vumin_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane) {
    impl_->masm.Umin(to_vixl_q(rd, lane), to_vixl_q(rn, lane), to_vixl_q(rm, lane));
}
void Emitter::vumax_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane) {
    impl_->masm.Umax(to_vixl_q(rd, lane), to_vixl_q(rn, lane), to_vixl_q(rm, lane));
}
void Emitter::vsmin_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane) {
    impl_->masm.Smin(to_vixl_q(rd, lane), to_vixl_q(rn, lane), to_vixl_q(rm, lane));
}
void Emitter::vsmax_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane) {
    impl_->masm.Smax(to_vixl_q(rd, lane), to_vixl_q(rn, lane), to_vixl_q(rm, lane));
}

void Emitter::vaddp_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane) {
    impl_->masm.Addp(to_vixl_q(rd, lane), to_vixl_q(rn, lane), to_vixl_q(rm, lane));
}
void Emitter::vsubp_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane) {
    // ARM has no integer pairwise sub. Emulate: pairwise add of (a, -b).
    // For each input vector form pairs (lane2i, lane2i+1) → result =
    // lane2i - lane2i+1. We compute v_a = vn, v_b = -vn-or-vm pattern.
    //
    // Concretely: pair-sub of (a,b) = ((a0-a1), (a2-a3),...,(b0-b1)...)
    // Build by negating odd lanes of vn and vm, then addp.
    constexpr int kAux2 = 30;
    const auto fmt = (lane == VecLane::B16) ? vixl_aa::kFormat16B
                  : (lane == VecLane::H8 ) ? vixl_aa::kFormat8H
                  : (lane == VecLane::S4 ) ? vixl_aa::kFormat4S
                                           : vixl_aa::kFormat2D;
    const vixl_aa::VRegister v_t (kInternalFpScratchV, fmt);
    const vixl_aa::VRegister v_t2(kAux2,               fmt);
    const vixl_aa::VRegister v_n (static_cast<int>(rn), fmt);
    const vixl_aa::VRegister v_m (static_cast<int>(rm), fmt);
    const vixl_aa::VRegister v_d (static_cast<int>(rd), fmt);
    // v_t = -vn (whole), then we'll select alternate lanes via trn1/trn2.
    // Simpler: addp(vn, vm) gives sums; but we want differences. Use neg
    // then addp:
    //   For pair (a, b), we want a - b = a + (-b). So negate every odd
    //   lane of vn and vm. The trn2/uzp2 family extracts odd lanes.
    //
    // Cheaper trick: split using zip1/zip2-style or use neg+ext:
    //   neg v_t,  v_n  (full negate)
    //   trn2 v_t2, v_n, v_t  -> picks odd lanes of v_n (a1) and v_t (-a1) alternately. Hmm.
    //
    // Cleanest: build {a0, -a1, a2, -a3, ...} by neg + zip.
    //   neg v_t, v_n              ; v_t = -v_n
    //   trn1 v_t,  v_n, v_t        ; alternates v_n.even, v_t.even = even lanes of {a0, -a1, a2, -a3,...}
    // But that's still not right. The simplest is to use uzp1/uzp2 to
    // separate even and odd lanes, then sub:
    //   uzp1 v_t,  v_n, v_m  → v_t  = {n.even, m.even}
    //   uzp2 v_t2, v_n, v_m  → v_t2 = {n.odd,  m.odd}
    //   sub  v_d,  v_t, v_t2 → result
    impl_->masm.Uzp1(v_t,  v_n, v_m);
    impl_->masm.Uzp2(v_t2, v_n, v_m);
    impl_->masm.Sub (v_d, v_t, v_t2);
}

void Emitter::vsad_bw(FpReg rd, FpReg rn, FpReg rm) {
    // PSADBW: low qword = sum |a[i]-b[i]| for i in 0..7; high qword
    // same for i in 8..15. Sequence:
    //   uabd  v_t.16b, vn.16b, vm.16b
    //   uaddlp v_t.8h, v_t.16b      ; pairs of bytes → halfwords
    //   uaddlp v_t.4s, v_t.8h
    //   uaddlp v_t.2d, v_t.4s        ; final → 2 dwords (each = sum of 8 bytes)
    //   mov    v_d, v_t
    const vixl_aa::VRegister v_t_16b(kInternalFpScratchV, vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_t_8h (kInternalFpScratchV, vixl_aa::kFormat8H);
    const vixl_aa::VRegister v_t_4s (kInternalFpScratchV, vixl_aa::kFormat4S);
    const vixl_aa::VRegister v_t_2d (kInternalFpScratchV, vixl_aa::kFormat2D);
    const vixl_aa::VRegister v_n_16b(static_cast<int>(rn), vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_m_16b(static_cast<int>(rm), vixl_aa::kFormat16B);
    impl_->masm.Uabd(v_t_16b, v_n_16b, v_m_16b);
    impl_->masm.Uaddlp(v_t_8h, v_t_16b);
    impl_->masm.Uaddlp(v_t_4s, v_t_8h);
    impl_->masm.Uaddlp(v_t_2d, v_t_4s);
    const vixl_aa::VRegister v_d_q(static_cast<int>(rd), vixl_aa::kFormat16B);
    impl_->masm.Mov(v_d_q, v_t_16b);
}

void Emitter::vmul_u32_to_64(FpReg rd, FpReg rn, FpReg rm) {
    // PMULUDQ: result.d2[0] = (u64)vn.s4[0] * (u64)vm.s4[0]
    //          result.d2[1] = (u64)vn.s4[2] * (u64)vm.s4[2]
    // NEON: uzp1 picks even lanes (0, 2) into low half. Then umull
    // multiplies low 2S → 2D.
    constexpr int kAuxV = 30;  // alongside V31 (kInternalFpScratchV).
    const vixl_aa::VRegister v_a_4s(kInternalFpScratchV, vixl_aa::kFormat4S);
    const vixl_aa::VRegister v_b_4s(kAuxV,               vixl_aa::kFormat4S);
    const vixl_aa::VRegister v_n_4s(static_cast<int>(rn), vixl_aa::kFormat4S);
    const vixl_aa::VRegister v_m_4s(static_cast<int>(rm), vixl_aa::kFormat4S);
    impl_->masm.Uzp1(v_a_4s, v_n_4s, v_n_4s);
    impl_->masm.Uzp1(v_b_4s, v_m_4s, v_m_4s);
    const vixl_aa::VRegister v_d_2d(static_cast<int>(rd), vixl_aa::kFormat2D);
    const vixl_aa::VRegister v_a_2s(kInternalFpScratchV, vixl_aa::kFormat2S);
    const vixl_aa::VRegister v_b_2s(kAuxV,               vixl_aa::kFormat2S);
    impl_->masm.Umull(v_d_2d, v_a_2s, v_b_2s);
}

void Emitter::vmulhi_h8(FpReg rd, FpReg rn, FpReg rm, bool is_signed) {
    // Sequence:
    //   smull/umull v31.4s, vn.4h, vm.4h        ; lower 4 i32 results
    //   smull2/umull2 v_t.4s, vn.8h, vm.8h      ; upper 4 i32 results (use rd as second scratch)
    //   shrn  v31.4h, v31.4s, #16               ; high 16 of each i32 → low half of v31
    //   shrn2 v31.8h, v_t.4s, #16               ; high 16 → upper half of v31
    //   mov   vd, v31
    // We need a second scratch besides V31. Use v_rd directly as the
    // intermediate for the upper-half products, since we overwrite it.
    const vixl_aa::VRegister v_n4h(static_cast<int>(rn), vixl_aa::kFormat4H);
    const vixl_aa::VRegister v_m4h(static_cast<int>(rm), vixl_aa::kFormat4H);
    const vixl_aa::VRegister v_n8h(static_cast<int>(rn), vixl_aa::kFormat8H);
    const vixl_aa::VRegister v_m8h(static_cast<int>(rm), vixl_aa::kFormat8H);
    const vixl_aa::VRegister v_t4s(kInternalFpScratchV, vixl_aa::kFormat4S);
    const vixl_aa::VRegister v_d4s(static_cast<int>(rd), vixl_aa::kFormat4S);
    if (is_signed) {
        impl_->masm.Smull (v_t4s, v_n4h, v_m4h);
        impl_->masm.Smull2(v_d4s, v_n8h, v_m8h);
    } else {
        impl_->masm.Umull (v_t4s, v_n4h, v_m4h);
        impl_->masm.Umull2(v_d4s, v_n8h, v_m8h);
    }
    const vixl_aa::VRegister v_t4h(kInternalFpScratchV, vixl_aa::kFormat4H);
    const vixl_aa::VRegister v_t8h(kInternalFpScratchV, vixl_aa::kFormat8H);
    impl_->masm.Shrn (v_t4h, v_t4s, 16);
    impl_->masm.Shrn2(v_t8h, v_d4s, 16);
    const vixl_aa::VRegister v_t_q(kInternalFpScratchV, vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_d_q(static_cast<int>(rd), vixl_aa::kFormat16B);
    impl_->masm.Mov(v_d_q, v_t_q);
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
void Emitter::vfmin_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane) {
    impl_->masm.Fmin(to_vixl_q_fp(rd, lane), to_vixl_q_fp(rn, lane),
                     to_vixl_q_fp(rm, lane));
}
void Emitter::vfmax_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane) {
    impl_->masm.Fmax(to_vixl_q_fp(rd, lane), to_vixl_q_fp(rn, lane),
                     to_vixl_q_fp(rm, lane));
}
void Emitter::vfsqrt_q(FpReg rd, FpReg rn, VecLane lane) {
    impl_->masm.Fsqrt(to_vixl_q_fp(rd, lane), to_vixl_q_fp(rn, lane));
}
void Emitter::vfaddp_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane) {
    impl_->masm.Faddp(to_vixl_q_fp(rd, lane), to_vixl_q_fp(rn, lane),
                      to_vixl_q_fp(rm, lane));
}

// F2-IR-006 — fused multiply-add primitives. ARM64 FMLA is destructive
// (Vd += Vn*Vm); the lowerer materialises the addend into Vd via vmov_q
// before calling vfmla_q.
void Emitter::vfmla_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane) {
    impl_->masm.Fmla(to_vixl_q_fp(rd, lane), to_vixl_q_fp(rn, lane),
                     to_vixl_q_fp(rm, lane));
}
void Emitter::vfmls_q(FpReg rd, FpReg rn, FpReg rm, VecLane lane) {
    impl_->masm.Fmls(to_vixl_q_fp(rd, lane), to_vixl_q_fp(rn, lane),
                     to_vixl_q_fp(rm, lane));
}
void Emitter::vfneg_q(FpReg rd, FpReg rn, VecLane lane) {
    impl_->masm.Fneg(to_vixl_q_fp(rd, lane), to_vixl_q_fp(rn, lane));
}
void Emitter::vmov_q(FpReg rd, FpReg rn) {
    if (static_cast<int>(rd) == static_cast<int>(rn)) return;
    const vixl_aa::VRegister vd(static_cast<int>(rd), vixl_aa::kFormat16B);
    const vixl_aa::VRegister vn(static_cast<int>(rn), vixl_aa::kFormat16B);
    impl_->masm.Mov(vd, vn);
}

void Emitter::vec_const_128(FpReg rd, std::uint64_t lo, std::uint64_t hi) {
    // Load the low 64 bits via the literal pool (Fmov d_d, imm).
    double d_lo;
    std::memcpy(&d_lo, &lo, sizeof d_lo);
    impl_->masm.Fmov(vixl_aa::DRegister(static_cast<int>(rd)), d_lo);
    // Load the high 64 bits into the internal scratch.
    double d_hi;
    std::memcpy(&d_hi, &hi, sizeof d_hi);
    impl_->masm.Fmov(vixl_aa::DRegister(kInternalFpScratchV), d_hi);
    // INS rd.d[1] ← v31.d[0]; rd's lane 0 (lo) stays put.
    const vixl_aa::VRegister v_d_d2 (static_cast<int>(rd),     vixl_aa::kFormat2D);
    const vixl_aa::VRegister v_t_d2 (kInternalFpScratchV,      vixl_aa::kFormat2D);
    impl_->masm.Mov(v_d_d2, 1, v_t_d2, 0);
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
// `which` codes:
//   '+' Add  '-' Sub  '*' Mul  '/' Div  'm' Min  'M' Max  's' Sqrt (unary, uses rm only)
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
            case 'm': masm.Fmin(s_t, s_n, s_m); break;
            case 'M': masm.Fmax(s_t, s_n, s_m); break;
            case 's': masm.Fsqrt(s_t, s_m);     break;
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
            case 'm': masm.Fmin(d_t, d_n, d_m); break;
            case 'M': masm.Fmax(d_t, d_n, d_m); break;
            case 's': masm.Fsqrt(d_t, d_m);     break;
        }
        masm.Mov(v_d_q, v_n_q);
        const vixl_aa::VRegister v_d_d2(rd_c,                vixl_aa::kFormat2D);
        const vixl_aa::VRegister v_t_d2(kInternalFpScratchV, vixl_aa::kFormat2D);
        masm.Mov(v_d_d2, 0, v_t_d2, 0);
    }
}
}  // namespace

// F2-IR-006 — scalar FMA. Computes one of the four ARM64 4-operand
// fused-multiply-add primitives into V31's low lane, then preserves
// the upper bits of `rupper` in `rd`. ARM mnemonics map to canonical
// (neg_addend, neg_mul) flags as:
//   FMADD  (Sd = Sa + Sn*Sm)  ↔ (F, F)
//   FNMSUB (Sd = -Sa + Sn*Sm) ↔ (T, F)
//   FMSUB  (Sd = Sa - Sn*Sm)  ↔ (F, T)
//   FNMADD (Sd = -Sa - Sn*Sm) ↔ (T, T)
// The argument order in vixl is masm.Fmadd(Sd, Sn, Sm, Sa) — note
// the addend is the LAST operand.
namespace {
void emit_scalar_fma_op(vixl_aa::MacroAssembler& masm,
                        int rd_c, int rupper_c,
                        int ra_c, int rb_c, int rc_c,
                        ir::FpSize sz,
                        bool neg_addend, bool neg_mul) {
    const vixl_aa::VRegister v_upper_q(rupper_c, vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_d_q    (rd_c,     vixl_aa::kFormat16B);
    if (sz == ir::FpSize::F32) {
        const vixl_aa::SRegister s_t(kInternalFpScratchV);
        const vixl_aa::SRegister s_a(ra_c);
        const vixl_aa::SRegister s_b(rb_c);
        const vixl_aa::SRegister s_c(rc_c);
        if (!neg_addend && !neg_mul) {
            masm.Fmadd (s_t, s_b, s_c, s_a);
        } else if (neg_addend && !neg_mul) {
            masm.Fnmsub(s_t, s_b, s_c, s_a);
        } else if (!neg_addend && neg_mul) {
            masm.Fmsub (s_t, s_b, s_c, s_a);
        } else {
            masm.Fnmadd(s_t, s_b, s_c, s_a);
        }
        masm.Mov(v_d_q, v_upper_q);
        const vixl_aa::VRegister v_d_s4(rd_c,                vixl_aa::kFormat4S);
        const vixl_aa::VRegister v_t_s4(kInternalFpScratchV, vixl_aa::kFormat4S);
        masm.Mov(v_d_s4, 0, v_t_s4, 0);
    } else {
        const vixl_aa::DRegister d_t(kInternalFpScratchV);
        const vixl_aa::DRegister d_a(ra_c);
        const vixl_aa::DRegister d_b(rb_c);
        const vixl_aa::DRegister d_c(rc_c);
        if (!neg_addend && !neg_mul) {
            masm.Fmadd (d_t, d_b, d_c, d_a);
        } else if (neg_addend && !neg_mul) {
            masm.Fnmsub(d_t, d_b, d_c, d_a);
        } else if (!neg_addend && neg_mul) {
            masm.Fmsub (d_t, d_b, d_c, d_a);
        } else {
            masm.Fnmadd(d_t, d_b, d_c, d_a);
        }
        masm.Mov(v_d_q, v_upper_q);
        const vixl_aa::VRegister v_d_d2(rd_c,                vixl_aa::kFormat2D);
        const vixl_aa::VRegister v_t_d2(kInternalFpScratchV, vixl_aa::kFormat2D);
        masm.Mov(v_d_d2, 0, v_t_d2, 0);
    }
}
}  // namespace

void Emitter::vfma_scalar(FpReg rd, FpReg rupper,
                          FpReg ra, FpReg rb, FpReg rc,
                          ir::FpSize sz, bool neg_addend, bool neg_mul) {
    emit_scalar_fma_op(impl_->masm,
        static_cast<int>(rd), static_cast<int>(rupper),
        static_cast<int>(ra), static_cast<int>(rb), static_cast<int>(rc),
        sz, neg_addend, neg_mul);
}

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
void Emitter::vfmin_scalar(FpReg rd, FpReg rn, FpReg rm, ir::FpSize sz) {
    emit_scalar_sse_op(impl_->masm,
        static_cast<int>(rd), static_cast<int>(rn), static_cast<int>(rm), sz, 'm');
}
void Emitter::vfmax_scalar(FpReg rd, FpReg rn, FpReg rm, ir::FpSize sz) {
    emit_scalar_sse_op(impl_->masm,
        static_cast<int>(rd), static_cast<int>(rn), static_cast<int>(rm), sz, 'M');
}
void Emitter::vfsqrt_scalar(FpReg rd, FpReg rn, FpReg rm, ir::FpSize sz) {
    emit_scalar_sse_op(impl_->masm,
        static_cast<int>(rd), static_cast<int>(rn), static_cast<int>(rm), sz, 's');
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

namespace {
vixl_aa::VectorFormat vec_format_for_lane(Emitter::VecLane lane) noexcept {
    using L = Emitter::VecLane;
    switch (lane) {
        case L::B16: return vixl_aa::kFormat16B;
        case L::H8:  return vixl_aa::kFormat8H;
        case L::S4:  return vixl_aa::kFormat4S;
        case L::D2:  return vixl_aa::kFormat2D;
    }
    return vixl_aa::kFormat16B;
}
}  // namespace

void Emitter::vins_lane_from_w(FpReg rd, FpReg rn, std::uint8_t lane_idx,
                               arm64::Reg w_value, VecLane lane) {
    // Copy lhs into V31 first (preserve other lanes), insert from GPR, mov to dst.
    const auto fmt = vec_format_for_lane(lane);
    const vixl_aa::VRegister v_t_q(kInternalFpScratchV, vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_n_q(static_cast<int>(rn), vixl_aa::kFormat16B);
    impl_->masm.Mov(v_t_q, v_n_q);
    const vixl_aa::VRegister v_t_lane(kInternalFpScratchV, fmt);
    if (lane == VecLane::D2) {
        impl_->masm.Mov(v_t_lane, lane_idx,
                        vixl_aa::Register(to_vixl_x(w_value)));
    } else {
        impl_->masm.Mov(v_t_lane, lane_idx,
                        vixl_aa::Register(to_vixl_w(w_value)));
    }
    const vixl_aa::VRegister v_d_q(static_cast<int>(rd), vixl_aa::kFormat16B);
    impl_->masm.Mov(v_d_q, v_t_q);
}

namespace {

// Compute the mask for a "ordered AND <pred>" relation across two FP
// vectors using NEON Fcmeq/Fcmgt/Fcmge. The base predicates 0..2 set
// `result` to lane = all-1s if (a, b are ordered) AND <pred>(a,b).
// 3 (unord) is computed separately.
//
// Caller guarantees v_t (V31) is free as scratch.
void emit_fp_pred_packed(vixl_aa::MacroAssembler& masm,
                         const vixl_aa::VRegister& v_d,
                         const vixl_aa::VRegister& v_n,
                         const vixl_aa::VRegister& v_m,
                         vixl_aa::VectorFormat fmt,
                         std::uint8_t pred) {
    const std::uint8_t base = pred & 0x3u;
    const bool negate = (pred & 0x4u) != 0;
    const vixl_aa::VRegister v_t(kInternalFpScratchV, fmt);
    if (base == 0) {            // eq
        masm.Fcmeq(v_d, v_n, v_m);
    } else if (base == 1) {     // lt: a < b == b > a
        masm.Fcmgt(v_d, v_m, v_n);
    } else if (base == 2) {     // le: a <= b == b >= a
        masm.Fcmge(v_d, v_m, v_n);
    } else {                    // unord: NaN(a) || NaN(b)
        // Fcmeq(a,a) sets all-1s when a is ordered; mvn gives unord-a mask.
        masm.Fcmeq(v_d, v_n, v_n);
        masm.Fcmeq(v_t, v_m, v_m);
        masm.And  (v_d.V16B(), v_d.V16B(), v_t.V16B());
        masm.Mvn  (v_d.V16B(), v_d.V16B());
    }
    if (negate) {
        masm.Mvn(v_d.V16B(), v_d.V16B());
    }
}

}  // namespace

void Emitter::vpshufb(FpReg rd, FpReg rn, FpReg rm) {
    // PSHUFB: rd[i] = rn[mask[i] & 0xF] if mask[i].MSB == 0, else 0.
    // NEON tbl returns 0 when index >= 16. So we OR the mask with 0xFF
    // wherever its MSB is set, forcing those slots to 0 in the lookup.
    constexpr int kAux2 = 30;
    const vixl_aa::VRegister v_t  (kInternalFpScratchV, vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_idx(kAux2,               vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_n  (static_cast<int>(rn), vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_m  (static_cast<int>(rm), vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_d  (static_cast<int>(rd), vixl_aa::kFormat16B);
    impl_->masm.Cmlt(v_t, v_m, 0);          // t[i] = 0xFF if mask MSB set
    impl_->masm.Orr (v_idx, v_m, v_t);      // copy mask, force MSB-set bytes to 0xFF
    impl_->masm.Tbl (v_d, v_n, v_idx);
}

void Emitter::vabs_q(FpReg rd, FpReg rn, VecLane lane) {
    impl_->masm.Abs(to_vixl_q(rd, lane), to_vixl_q(rn, lane));
}

namespace {

void emit_vfrint(vixl_aa::MacroAssembler& masm,
                 const vixl_aa::VRegister& vd,
                 const vixl_aa::VRegister& vn,
                 std::uint8_t mode) {
    switch (mode & 0x3u) {
        case 0: masm.Frintn(vd, vn); break;  // round to nearest (ties to even)
        case 1: masm.Frintm(vd, vn); break;  // round down (toward -inf)
        case 2: masm.Frintp(vd, vn); break;  // round up   (toward +inf)
        case 3: masm.Frintz(vd, vn); break;  // truncate (toward zero)
    }
}

}  // namespace

void Emitter::vptest(FpReg lhs, FpReg rhs, arm64::Reg w_tmp) {
    // 1. v_a = lhs AND rhs
    // 2. v_b = lhs AND NOT rhs   (NEON: bic v_b, vlhs, vrhs)
    // 3. orr each across two halves to detect zero per 128-bit
    // 4. compute is_zero_a, is_zero_b in GPRs, build NZCV, msr.
    constexpr int kAux2 = 30;
    const vixl_aa::VRegister v_a (kInternalFpScratchV, vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_b (kAux2,               vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_l (static_cast<int>(lhs), vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_r (static_cast<int>(rhs), vixl_aa::kFormat16B);
    impl_->masm.And(v_a, v_l, v_r);
    impl_->masm.Bic(v_b, v_l, v_r);
    // umaxv reduces to a single byte = max byte; > 0 iff any byte set.
    const vixl_aa::BRegister b_a(kInternalFpScratchV);
    const vixl_aa::BRegister b_b(kAux2);
    impl_->masm.Umaxv(b_a, v_a);
    impl_->masm.Umaxv(b_b, v_b);
    vixl_aa::UseScratchRegisterScope temps(&impl_->masm);
    const vixl_aa::Register x_a = temps.AcquireX();
    const vixl_aa::Register x_b = temps.AcquireX();
    impl_->masm.Umov(vixl_aa::WRegister(static_cast<int>(x_a.GetCode())), v_a.V16B(), 0);
    impl_->masm.Umov(vixl_aa::WRegister(static_cast<int>(x_b.GetCode())), v_b.V16B(), 0);
    // is_zero_a = (x_a == 0); is_zero_b = (x_b == 0).
    // Build NZCV: bit 30 = is_zero_a, bit 29 = NOT is_zero_b.
    // Use cmp + cset to materialise the bools, then shift+or.
    impl_->masm.Cmp(vixl_aa::WRegister(static_cast<int>(x_a.GetCode())), 0);
    impl_->masm.Cset(vixl_aa::WRegister(static_cast<int>(x_a.GetCode())), vixl_aa::eq);
    impl_->masm.Cmp(vixl_aa::WRegister(static_cast<int>(x_b.GetCode())), 0);
    impl_->masm.Cset(vixl_aa::WRegister(static_cast<int>(x_b.GetCode())), vixl_aa::ne);  // NOT is_zero_b
    // w_tmp = (x_a << 30) | (x_b << 29).
    impl_->masm.Lsl(to_vixl_w(w_tmp),
                    vixl_aa::WRegister(static_cast<int>(x_a.GetCode())), 30);
    impl_->masm.Orr(to_vixl_w(w_tmp), to_vixl_w(w_tmp),
                    vixl_aa::Operand(vixl_aa::WRegister(static_cast<int>(x_b.GetCode())),
                                     vixl_aa::LSL, 29));
    // msr nzcv, x_tmp
    impl_->masm.Msr(vixl_aa::NZCV, vixl_aa::Register(to_vixl_x(w_tmp)));
}

void Emitter::vptest_ymm(FpReg lo_lhs, FpReg lo_rhs,
                         FpReg hi_lhs, FpReg hi_rhs,
                         arm64::Reg w_tmp) {
    // Same shape as `vptest`, but the two inputs to the NZCV-build are
    // OR-reductions across the two 128-bit halves:
    //   v_a = (lo_lhs & lo_rhs) | (hi_lhs & hi_rhs)        ; ZF source
    //   v_b = (lo_rhs & ~lo_lhs) | (hi_rhs & ~hi_lhs)      ; CF source
    // Then umaxv → cmp 0 → cset → shift+orr into w_tmp → msr NZCV,
    // identical to the xmm path.
    //
    // Uses three caller-saved VRegs (V29..V31) as scratch.
    // V0..V23 are the FP regalloc pool (F2-BK-006); V24..V31 are
    // reserved for emitter internals.
    constexpr int kAux2 = 30;
    constexpr int kAux3 = 29;
    const vixl_aa::VRegister v_a (kInternalFpScratchV, vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_b (kAux2,               vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_t (kAux3,               vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_ll(static_cast<int>(lo_lhs), vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_lr(static_cast<int>(lo_rhs), vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_hl(static_cast<int>(hi_lhs), vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_hr(static_cast<int>(hi_rhs), vixl_aa::kFormat16B);
    // ZF input:  v_a = (lo_lhs & lo_rhs) | (hi_lhs & hi_rhs).
    impl_->masm.And(v_a, v_ll, v_lr);
    impl_->masm.And(v_t, v_hl, v_hr);
    impl_->masm.Orr(v_a, v_a, v_t);
    // CF input:  v_b = (lo_rhs &~ lo_lhs) | (hi_rhs &~ hi_lhs).
    impl_->masm.Bic(v_b, v_lr, v_ll);
    impl_->masm.Bic(v_t, v_hr, v_hl);
    impl_->masm.Orr(v_b, v_b, v_t);
    // NZCV build — identical to `vptest`.
    const vixl_aa::BRegister b_a(kInternalFpScratchV);
    const vixl_aa::BRegister b_b(kAux2);
    impl_->masm.Umaxv(b_a, v_a);
    impl_->masm.Umaxv(b_b, v_b);
    vixl_aa::UseScratchRegisterScope temps(&impl_->masm);
    const vixl_aa::Register x_a = temps.AcquireX();
    const vixl_aa::Register x_b = temps.AcquireX();
    impl_->masm.Umov(vixl_aa::WRegister(static_cast<int>(x_a.GetCode())), v_a.V16B(), 0);
    impl_->masm.Umov(vixl_aa::WRegister(static_cast<int>(x_b.GetCode())), v_b.V16B(), 0);
    impl_->masm.Cmp(vixl_aa::WRegister(static_cast<int>(x_a.GetCode())), 0);
    impl_->masm.Cset(vixl_aa::WRegister(static_cast<int>(x_a.GetCode())), vixl_aa::eq);
    impl_->masm.Cmp(vixl_aa::WRegister(static_cast<int>(x_b.GetCode())), 0);
    impl_->masm.Cset(vixl_aa::WRegister(static_cast<int>(x_b.GetCode())), vixl_aa::ne);
    impl_->masm.Lsl(to_vixl_w(w_tmp),
                    vixl_aa::WRegister(static_cast<int>(x_a.GetCode())), 30);
    impl_->masm.Orr(to_vixl_w(w_tmp), to_vixl_w(w_tmp),
                    vixl_aa::Operand(vixl_aa::WRegister(static_cast<int>(x_b.GetCode())),
                                     vixl_aa::LSL, 29));
    impl_->masm.Msr(vixl_aa::NZCV, vixl_aa::Register(to_vixl_x(w_tmp)));
}

void Emitter::vtbl2_q(FpReg dst, FpReg src_lo, FpReg src_hi, FpReg idx) {
    // ARM64 TBL with two sources requires consecutive V registers.
    // Copy through V30/V31 (caller-saved, outside the V0..V23 regalloc
    // pool — same convention as vptest_ymm's auxiliary scratches).
    constexpr int kAuxLo = 30;
    constexpr int kAuxHi = 31;   // == kInternalFpScratchV
    static_assert(kInternalFpScratchV == kAuxHi,
                  "vtbl2_q assumes kInternalFpScratchV == V31");
    const vixl_aa::VRegister v_lo_dst(kAuxLo, vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_hi_dst(kAuxHi, vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_src_lo(static_cast<int>(src_lo), vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_src_hi(static_cast<int>(src_hi), vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_idx  (static_cast<int>(idx),    vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_dst  (static_cast<int>(dst),    vixl_aa::kFormat16B);
    impl_->masm.Mov(v_lo_dst, v_src_lo);
    impl_->masm.Mov(v_hi_dst, v_src_hi);
    impl_->masm.Tbl(v_dst, v_lo_dst, v_hi_dst, v_idx);
}

void Emitter::crc32c(arm64::Reg rd, arm64::Reg rcrc, arm64::Reg rdata,
                     ir::OpSize data_size) {
    // ARM64 CRC32C is a feature-flagged extension. The destination
    // and source CRC inputs are always 32-bit (Wd / Wn); the data
    // operand width selects the variant. For I64 the data input is
    // Xn; for the smaller widths, Wn is used and the upper bits are
    // don't-care per the ARM ARM.
    switch (data_size) {
        case ir::OpSize::I8:
            impl_->masm.Crc32cb(to_vixl_w(rd), to_vixl_w(rcrc),
                                to_vixl_w(rdata));
            break;
        case ir::OpSize::I16:
            impl_->masm.Crc32ch(to_vixl_w(rd), to_vixl_w(rcrc),
                                to_vixl_w(rdata));
            break;
        case ir::OpSize::I32:
            impl_->masm.Crc32cw(to_vixl_w(rd), to_vixl_w(rcrc),
                                to_vixl_w(rdata));
            break;
        case ir::OpSize::I64:
            impl_->masm.Crc32cx(to_vixl_w(rd), to_vixl_w(rcrc),
                                to_vixl_x(rdata));
            break;
    }
}

void Emitter::bswap(arm64::Reg rd, arm64::Reg rn, ir::OpSize size) {
    // ARM64 has REV for 32 / 64-bit and REV16 for 16-bit byte-pair
    // swap. I8 has no byte order, so fall back to a mov.
    switch (size) {
        case ir::OpSize::I8:
            impl_->masm.Mov(to_vixl_x(rd), to_vixl_x(rn));
            break;
        case ir::OpSize::I16: {
            // REV16 wd, wn reverses bytes within each 16-bit half of
            // the 32-bit register. We only need the low half — the
            // high half is don't-care for guest I16 ops.
            impl_->masm.Rev16(to_vixl_w(rd), to_vixl_w(rn));
            break;
        }
        case ir::OpSize::I32:
            impl_->masm.Rev(to_vixl_w(rd), to_vixl_w(rn));
            break;
        case ir::OpSize::I64:
            impl_->masm.Rev(to_vixl_x(rd), to_vixl_x(rn));
            break;
    }
}

void Emitter::x87_load(arm64::Reg state_ptr, arm64::Reg dst,
                       arm64::Reg scratch_tos, arm64::Reg scratch_slot,
                       std::int32_t array_offset, std::int32_t tos_byte_offset,
                       std::uint8_t st_index) {
    // Load logical ST(i): physical slot = (TOS + i) mod 8.
    impl_->masm.Ldrb(to_vixl_w(scratch_tos),
                     vixl_aa::MemOperand(to_vixl_x(state_ptr), tos_byte_offset));
    if (st_index != 0u) {
        impl_->masm.Add(to_vixl_x(scratch_tos), to_vixl_x(scratch_tos),
                        static_cast<unsigned>(st_index));
        impl_->masm.And(to_vixl_x(scratch_tos), to_vixl_x(scratch_tos), 0x7);
    }
    impl_->masm.Lsl(to_vixl_x(scratch_slot), to_vixl_x(scratch_tos), 4);
    impl_->masm.Add(to_vixl_x(scratch_slot), to_vixl_x(state_ptr),
                    to_vixl_x(scratch_slot));
    impl_->masm.Ldr(to_vixl_x(dst),
                    vixl_aa::MemOperand(to_vixl_x(scratch_slot), array_offset));
}

void Emitter::x87_store(arm64::Reg state_ptr, arm64::Reg value,
                        arm64::Reg scratch_tos, arm64::Reg scratch_slot,
                        std::int32_t array_offset, std::int32_t tos_byte_offset,
                        std::uint8_t st_index) {
    // Store logical ST(i): physical slot = (TOS + i) mod 8.
    impl_->masm.Ldrb(to_vixl_w(scratch_tos),
                     vixl_aa::MemOperand(to_vixl_x(state_ptr), tos_byte_offset));
    if (st_index != 0u) {
        impl_->masm.Add(to_vixl_x(scratch_tos), to_vixl_x(scratch_tos),
                        static_cast<unsigned>(st_index));
        impl_->masm.And(to_vixl_x(scratch_tos), to_vixl_x(scratch_tos), 0x7);
    }
    impl_->masm.Lsl(to_vixl_x(scratch_slot), to_vixl_x(scratch_tos), 4);
    impl_->masm.Add(to_vixl_x(scratch_slot), to_vixl_x(state_ptr),
                    to_vixl_x(scratch_slot));
    impl_->masm.Str(to_vixl_x(value),
                    vixl_aa::MemOperand(to_vixl_x(scratch_slot), array_offset));
}

void Emitter::x87_push(arm64::Reg state_ptr, arm64::Reg value,
                       arm64::Reg scratch_tos, arm64::Reg scratch_slot,
                       std::int32_t array_offset, std::int32_t tos_byte_offset) {
    // F2-IR-007. Push `value` (I64 FP bits) onto the x87 stack. TOS is
    // a 3-bit counter modeled as a u8 at `[state_ptr, tos_byte_offset]`.
    //
    //   ldrb w_tos, [state_ptr, tos_byte_offset]
    //   sub  x_tos, x_tos, #1                ; decrement (wraps under mod 8)
    //   and  x_tos, x_tos, #7                ; modulo 8
    //   strb w_tos, [state_ptr, tos_byte_offset]
    //   lsl  x_slot, x_tos, #4              ; * sizeof(X87Slot) = 16
    //   add  x_slot, state_ptr, x_slot
    //   str  x_value, [x_slot, array_offset]
    impl_->masm.Ldrb(to_vixl_w(scratch_tos),
                     vixl_aa::MemOperand(to_vixl_x(state_ptr), tos_byte_offset));
    impl_->masm.Sub(to_vixl_x(scratch_tos), to_vixl_x(scratch_tos), 1);
    impl_->masm.And(to_vixl_x(scratch_tos), to_vixl_x(scratch_tos), 0x7);
    impl_->masm.Strb(to_vixl_w(scratch_tos),
                     vixl_aa::MemOperand(to_vixl_x(state_ptr), tos_byte_offset));
    impl_->masm.Lsl(to_vixl_x(scratch_slot), to_vixl_x(scratch_tos), 4);
    impl_->masm.Add(to_vixl_x(scratch_slot), to_vixl_x(state_ptr),
                    to_vixl_x(scratch_slot));
    impl_->masm.Str(to_vixl_x(value),
                    vixl_aa::MemOperand(to_vixl_x(scratch_slot), array_offset));
}

void Emitter::x87_pop(arm64::Reg state_ptr, arm64::Reg dst,
                      arm64::Reg scratch_tos, arm64::Reg scratch_slot,
                      std::int32_t array_offset, std::int32_t tos_byte_offset) {
    // F2-IR-008. Pop the top of the x87 stack into `dst` (I64 FP bits).
    //
    //   ldrb w_tos, [state_ptr, tos_byte_offset]
    //   lsl  x_slot, x_tos, #4
    //   add  x_slot, state_ptr, x_slot
    //   ldr  x_dst, [x_slot, array_offset]
    //   add  x_tos, x_tos, #1
    //   and  x_tos, x_tos, #7
    //   strb w_tos, [state_ptr, tos_byte_offset]
    impl_->masm.Ldrb(to_vixl_w(scratch_tos),
                     vixl_aa::MemOperand(to_vixl_x(state_ptr), tos_byte_offset));
    impl_->masm.Lsl(to_vixl_x(scratch_slot), to_vixl_x(scratch_tos), 4);
    impl_->masm.Add(to_vixl_x(scratch_slot), to_vixl_x(state_ptr),
                    to_vixl_x(scratch_slot));
    impl_->masm.Ldr(to_vixl_x(dst),
                    vixl_aa::MemOperand(to_vixl_x(scratch_slot), array_offset));
    impl_->masm.Add(to_vixl_x(scratch_tos), to_vixl_x(scratch_tos), 1);
    impl_->masm.And(to_vixl_x(scratch_tos), to_vixl_x(scratch_tos), 0x7);
    impl_->masm.Strb(to_vixl_w(scratch_tos),
                     vixl_aa::MemOperand(to_vixl_x(state_ptr), tos_byte_offset));
}

void Emitter::vaes(FpReg dst, FpReg src, FpReg key, ir::VecAesKind kind) {
    // V31 (= kInternalFpScratchV) is used as the working scratch.
    const vixl_aa::VRegister v_tmp(kInternalFpScratchV, vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_src(static_cast<int>(src), vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_key(static_cast<int>(key), vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_dst(static_cast<int>(dst), vixl_aa::kFormat16B);

    if (kind == ir::VecAesKind::Imc) {
        // Single AESIMC, no key involvement.
        impl_->masm.Aesimc(v_dst, v_src);
        return;
    }

    // For Enc/EncLast/Dec/DecLast we first need to compute the
    // ShiftRows + SubBytes (or their inverses) on `src` without
    // XOR-ing the key in. ARM AESE/AESD always XOR with their Vn,
    // so we feed them a zero vector via `eor v_tmp.16b, v_src.16b, v_src.16b`
    // and then AESE/AESD against it. Equivalent: `mov v_tmp, v_src`
    // + `eor v_tmp, v_tmp, v_tmp` would lose `src`, so instead we
    // initialise v_tmp to v_src first (the AES family is destructive
    // on its first operand).
    impl_->masm.Mov(v_tmp, v_src);
    // Zero a vector to XOR-with-zero (== identity AddRoundKey).
    // vixl provides Movi for immediate-zero load.
    constexpr int kAuxZero = 30;
    const vixl_aa::VRegister v_zero(kAuxZero, vixl_aa::kFormat16B);
    impl_->masm.Movi(v_zero, 0);

    switch (kind) {
        case ir::VecAesKind::Enc:
        case ir::VecAesKind::EncLast:
            impl_->masm.Aese(v_tmp, v_zero);
            if (kind == ir::VecAesKind::Enc) {
                impl_->masm.Aesmc(v_dst, v_tmp);
                impl_->masm.Eor(v_dst, v_dst, v_key);
            } else {
                impl_->masm.Eor(v_dst, v_tmp, v_key);
            }
            break;
        case ir::VecAesKind::Dec:
        case ir::VecAesKind::DecLast:
            impl_->masm.Aesd(v_tmp, v_zero);
            if (kind == ir::VecAesKind::Dec) {
                impl_->masm.Aesimc(v_dst, v_tmp);
                impl_->masm.Eor(v_dst, v_dst, v_key);
            } else {
                impl_->masm.Eor(v_dst, v_tmp, v_key);
            }
            break;
        case ir::VecAesKind::Imc:
            // Unreachable — handled above.
            break;
    }
}

void Emitter::vaes_keygenassist(FpReg dst, FpReg src, std::uint8_t rcon) {
    // V29..V31 are outside the lowerer's V0..V23 FP allocation pool and
    // are reserved for emitter internals. This helper needs three V regs:
    // the S-box table result, the output index/RCON vector, and a high
    // 64-bit lane scratch for materialising 128-bit constants.
    constexpr int kSboxV = kInternalFpScratchV;  // V31
    constexpr int kAuxV  = 30;
    constexpr int kHiV   = 29;
    const vixl_aa::VRegister v_sbox(kSboxV, vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_aux (kAuxV,  vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_src (static_cast<int>(src), vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_dst (static_cast<int>(dst), vixl_aa::kFormat16B);

    auto load_q = [&](int vreg, std::uint64_t lo, std::uint64_t hi) {
        double d_lo;
        double d_hi;
        std::memcpy(&d_lo, &lo, sizeof d_lo);
        std::memcpy(&d_hi, &hi, sizeof d_hi);
        impl_->masm.Fmov(vixl_aa::DRegister(vreg), d_lo);
        impl_->masm.Fmov(vixl_aa::DRegister(kHiV), d_hi);
        const vixl_aa::VRegister v_d(vreg, vixl_aa::kFormat2D);
        const vixl_aa::VRegister v_h(kHiV, vixl_aa::kFormat2D);
        impl_->masm.Mov(v_d, 1, v_h, 0);
    };

    // AESE with a zero key applies SubBytes+ShiftRows. The TBL index
    // then selects S(src dword1), RotWord(S(src dword1)), S(src dword3),
    // and RotWord(S(src dword3)) in x86 byte order.
    impl_->masm.Mov(v_sbox, v_src);
    impl_->masm.Movi(v_aux, 0);
    impl_->masm.Aese(v_sbox, v_aux);

    constexpr std::uint64_t kIdxLo = 0x040b0e010b0e0104ULL;
    constexpr std::uint64_t kIdxHi = 0x0c0306090306090cULL;
    load_q(kAuxV, kIdxLo, kIdxHi);
    impl_->masm.Tbl(v_dst, v_sbox, v_aux);

    if (rcon != 0u) {
        const std::uint64_t rcon_lane =
            static_cast<std::uint64_t>(rcon) << 32u;
        load_q(kAuxV, rcon_lane, rcon_lane);
        impl_->masm.Eor(v_dst, v_dst, v_aux);
    }
}

void Emitter::vsha1_rnds4(FpReg dst, FpReg a, FpReg b, std::uint8_t sel) {
    // x86 keeps {A,B,C,D} / {W0+E,W1,W2,W3} in lanes 3..0; ARM's
    // SHA1C/P/M are ascending, so the dword order reverses on the
    // way in and out (EXT #8 + REV64). ARM does not add K and takes
    // E in Sn: the K constant is pre-added into the message vector
    // and Sn is zero because x86 already folded E into the W0 lane
    // (that is SHA1NEXTE's job).
    constexpr int kStateV = kInternalFpScratchV;  // V31
    constexpr int kMsgV   = 30;
    constexpr int kConstV = 29;
    const vixl_aa::VRegister v_state16(kStateV, vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_state4s(kStateV, vixl_aa::kFormat4S);
    const vixl_aa::VRegister v_msg16(kMsgV, vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_msg4s(kMsgV, vixl_aa::kFormat4S);
    const vixl_aa::VRegister v_k4s(kConstV, vixl_aa::kFormat4S);
    const vixl_aa::VRegister v_k2d(kConstV, vixl_aa::kFormat2D);
    const vixl_aa::VRegister v_zero16(kConstV, vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_a16(static_cast<int>(a), vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_b16(static_cast<int>(b), vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_dst16(static_cast<int>(dst),
                                     vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_dst4s(static_cast<int>(dst),
                                     vixl_aa::kFormat4S);

    impl_->masm.Ext(v_state16, v_a16, v_a16, 8);
    impl_->masm.Rev64(v_state4s, v_state4s);
    impl_->masm.Ext(v_msg16, v_b16, v_b16, 8);
    impl_->masm.Rev64(v_msg4s, v_msg4s);

    static constexpr std::uint32_t kSha1K[4] = {
        0x5A827999u, 0x6ED9EBA1u, 0x8F1BBCDCu, 0xCA62C1D6u};
    const std::uint64_t k  = kSha1K[sel & 3u];
    const std::uint64_t kk = k | (k << 32u);
    double d_kk;
    std::memcpy(&d_kk, &kk, sizeof d_kk);
    impl_->masm.Fmov(vixl_aa::DRegister(kConstV), d_kk);
    impl_->masm.Mov(v_k2d, 1, v_k2d, 0);
    impl_->masm.Add(v_msg4s, v_msg4s, v_k4s);

    impl_->masm.Movi(v_zero16, 0);
    const vixl_aa::QRegister q_state(kStateV);
    const vixl_aa::SRegister s_e(kConstV);
    switch (sel & 3u) {
        case 0:  impl_->masm.Sha1c(q_state, s_e, v_msg4s); break;
        case 2:  impl_->masm.Sha1m(q_state, s_e, v_msg4s); break;
        default: impl_->masm.Sha1p(q_state, s_e, v_msg4s); break;
    }

    impl_->masm.Ext(v_dst16, v_state16, v_state16, 8);
    impl_->masm.Rev64(v_dst4s, v_dst4s);
}

void Emitter::vsha1_nexte(FpReg dst, FpReg a, FpReg b) {
    // dst = b, with rol30(a.lane3) added into lane 3. rol30 computes
    // on all four lanes (shl+ushr+orr), then only lane 3 survives
    // into the zeroed addend vector.
    constexpr int kRolV = kInternalFpScratchV;  // V31
    constexpr int kAddV = 30;
    const vixl_aa::VRegister v_rol4s(kRolV, vixl_aa::kFormat4S);
    const vixl_aa::VRegister v_rol16(kRolV, vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_add4s(kAddV, vixl_aa::kFormat4S);
    const vixl_aa::VRegister v_add16(kAddV, vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_a4s(static_cast<int>(a), vixl_aa::kFormat4S);
    const vixl_aa::VRegister v_b4s(static_cast<int>(b), vixl_aa::kFormat4S);
    const vixl_aa::VRegister v_dst4s(static_cast<int>(dst),
                                     vixl_aa::kFormat4S);
    impl_->masm.Shl(v_rol4s, v_a4s, 30);
    impl_->masm.Ushr(v_add4s, v_a4s, 2);
    impl_->masm.Orr(v_rol16, v_rol16, v_add16);
    impl_->masm.Movi(v_add16, 0);
    impl_->masm.Mov(v_add4s, 3, v_rol4s, 3);
    impl_->masm.Add(v_dst4s, v_b4s, v_add4s);
}

void Emitter::vsha1_msg1(FpReg dst, FpReg a, FpReg b) {
    // dst = a ^ {W2,W3,W4,W5}: the shifted window is EXT(b:a, #8)
    // (b supplies W4/W5 from its high half, a supplies W2/W3 from
    // its low half). SHA1SU0 is NOT used — it would fold the W[t-8]
    // term that x86 leaves to the caller's explicit PXOR.
    constexpr int kWinV = kInternalFpScratchV;  // V31
    const vixl_aa::VRegister v_win16(kWinV, vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_a16(static_cast<int>(a), vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_b16(static_cast<int>(b), vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_dst16(static_cast<int>(dst),
                                     vixl_aa::kFormat16B);
    impl_->masm.Ext(v_win16, v_b16, v_a16, 8);
    impl_->masm.Eor(v_dst16, v_win16, v_a16);
}

void Emitter::vsha1_msg2(FpReg dst, FpReg a, FpReg b) {
    // SHA1SU1 matches SHA1MSG2 exactly in ascending lane order
    // (including the lane-chained ROL: rotate distributes over XOR),
    // so reverse both inputs, run it, reverse the result back.
    constexpr int kAccV = kInternalFpScratchV;  // V31
    constexpr int kWinV = 30;
    const vixl_aa::VRegister v_acc16(kAccV, vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_acc4s(kAccV, vixl_aa::kFormat4S);
    const vixl_aa::VRegister v_win16(kWinV, vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_win4s(kWinV, vixl_aa::kFormat4S);
    const vixl_aa::VRegister v_a16(static_cast<int>(a), vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_b16(static_cast<int>(b), vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_dst16(static_cast<int>(dst),
                                     vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_dst4s(static_cast<int>(dst),
                                     vixl_aa::kFormat4S);
    impl_->masm.Ext(v_acc16, v_a16, v_a16, 8);
    impl_->masm.Rev64(v_acc4s, v_acc4s);
    impl_->masm.Ext(v_win16, v_b16, v_b16, 8);
    impl_->masm.Rev64(v_win4s, v_win4s);
    impl_->masm.Sha1su1(v_acc4s, v_win4s);
    impl_->masm.Ext(v_dst16, v_acc16, v_acc16, 8);
    impl_->masm.Rev64(v_dst4s, v_dst4s);
}

void Emitter::vsha256_rnds2(FpReg dst, FpReg a, FpReg b, FpReg wk) {
    // a = {C,D,G,H}, b = {A,B,E,F} (x86 lanes 3..0); wk = XMM0 with
    // {WK1,WK0} in lanes 1..0. ARM SHA256H/H2 run FOUR rounds, but
    // the round recurrence is a shift register: with WK2=WK3=0 the
    // garbage of rounds 3..4 only reaches elements 0..1 of the ARM
    // outputs {A4,A3,A2,A1} / {E4,E3,E2,E1}; elements 2..3 are the
    // exact x86 two-round results.
    constexpr int kAbcdV = kInternalFpScratchV;  // V31
    constexpr int kEfghV = 30;
    constexpr int kWV    = 29;
    const vixl_aa::VRegister v_abcd16(kAbcdV, vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_abcd4s(kAbcdV, vixl_aa::kFormat4S);
    const vixl_aa::VRegister v_abcd2d(kAbcdV, vixl_aa::kFormat2D);
    const vixl_aa::VRegister v_efgh4s(kEfghV, vixl_aa::kFormat4S);
    const vixl_aa::VRegister v_efgh2d(kEfghV, vixl_aa::kFormat2D);
    const vixl_aa::VRegister v_w16(kWV, vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_w4s(kWV, vixl_aa::kFormat4S);
    const vixl_aa::VRegister v_w2d(kWV, vixl_aa::kFormat2D);
    const vixl_aa::VRegister v_a2d(static_cast<int>(a), vixl_aa::kFormat2D);
    const vixl_aa::VRegister v_b2d(static_cast<int>(b), vixl_aa::kFormat2D);
    const vixl_aa::VRegister v_wk2d(static_cast<int>(wk),
                                    vixl_aa::kFormat2D);
    const vixl_aa::VRegister v_dst16(static_cast<int>(dst),
                                     vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_dst2d(static_cast<int>(dst),
                                     vixl_aa::kFormat2D);

    impl_->masm.Zip2(v_abcd2d, v_b2d, v_a2d);   // {B,A,D,C}
    impl_->masm.Rev64(v_abcd4s, v_abcd4s);      // {A,B,C,D}
    impl_->masm.Zip1(v_efgh2d, v_b2d, v_a2d);   // {F,E,H,G}
    impl_->masm.Rev64(v_efgh4s, v_efgh4s);      // {E,F,G,H}
    impl_->masm.Movi(v_w16, 0);
    impl_->masm.Mov(v_w2d, 0, v_wk2d, 0);       // {WK0,WK1,0,0}

    impl_->masm.Mov(v_dst16, v_abcd16);
    const vixl_aa::QRegister q_dst(static_cast<int>(dst));
    const vixl_aa::QRegister q_abcd(kAbcdV);
    const vixl_aa::QRegister q_efgh(kEfghV);
    impl_->masm.Sha256h(q_dst, q_efgh, v_w4s);   // {A4,A3,A2,A1}
    impl_->masm.Sha256h2(q_efgh, q_abcd, v_w4s); // {E4,E3,E2,E1}

    impl_->masm.Zip2(v_abcd2d, v_efgh2d, v_dst2d);  // {E2,E1,A2,A1}
    impl_->masm.Rev64(v_abcd4s, v_abcd4s);          // x86 {F2,E2,B2,A2}
    impl_->masm.Mov(v_dst16, v_abcd16);
}

void Emitter::vsha256_msg1(FpReg dst, FpReg a, FpReg b) {
    // SHA256SU0 is an exact match: both ISAs are ascending here and
    // both take W4 from the second operand's low dword. Stage the
    // accumulator through V31 so a dst == b caller cannot clobber
    // the W4 source before SHA256SU0 reads it.
    constexpr int kAccV = kInternalFpScratchV;  // V31
    const vixl_aa::VRegister v_acc16(kAccV, vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_acc4s(kAccV, vixl_aa::kFormat4S);
    const vixl_aa::VRegister v_a16(static_cast<int>(a), vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_b4s(static_cast<int>(b), vixl_aa::kFormat4S);
    const vixl_aa::VRegister v_dst16(static_cast<int>(dst),
                                     vixl_aa::kFormat16B);
    impl_->masm.Mov(v_acc16, v_a16);
    impl_->masm.Sha256su0(v_acc4s, v_b4s);
    impl_->masm.Mov(v_dst16, v_acc16);
}

void Emitter::vsha256_msg2(FpReg dst, FpReg a, FpReg b) {
    // SHA256SU1's internal W[t-7] addend reads Vn lanes 1..3 and Vm
    // lane 0 — disjoint from its sigma1 source (Vm lanes 2..3). With
    // Vn = 0 and b.lane0 cleared the addend is zero and what remains
    // is exactly SHA256MSG2, including the lane 0/1 -> 2/3 chaining.
    constexpr int kZeroV = kInternalFpScratchV;  // V31
    constexpr int kWinV  = 30;
    const vixl_aa::VRegister v_zero16(kZeroV, vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_zero4s(kZeroV, vixl_aa::kFormat4S);
    const vixl_aa::VRegister v_win16(kWinV, vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_win4s(kWinV, vixl_aa::kFormat4S);
    const vixl_aa::VRegister v_a16(static_cast<int>(a), vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_b16(static_cast<int>(b), vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_dst16(static_cast<int>(dst),
                                     vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_dst4s(static_cast<int>(dst),
                                     vixl_aa::kFormat4S);
    impl_->masm.Movi(v_zero16, 0);
    impl_->masm.Mov(v_win16, v_b16);
    impl_->masm.Mov(v_win4s, 0, vixl_aa::wzr);
    impl_->masm.Mov(v_dst16, v_a16);
    impl_->masm.Sha256su1(v_dst4s, v_zero4s, v_win4s);
}

void Emitter::vblend(FpReg rd, FpReg rdst, FpReg rsrc, FpReg rmask, VecLane lane) {
    // Sequence:
    //   cmlt v_t.<lane>, vmask.<lane>, #0    ; t[i] = all-1s if mask[i] MSB set
    //   bsl  v_t.16b,    vsrc.16b, vdst.16b   ; result = (t & src) | (~t & dst)
    //   mov  vd.16b,     v_t.16b
    const auto fmt = (lane == VecLane::B16) ? vixl_aa::kFormat16B
                  : (lane == VecLane::H8 ) ? vixl_aa::kFormat8H
                  : (lane == VecLane::S4 ) ? vixl_aa::kFormat4S
                                           : vixl_aa::kFormat2D;
    const vixl_aa::VRegister v_t   (kInternalFpScratchV, fmt);
    const vixl_aa::VRegister v_mask(static_cast<int>(rmask), fmt);
    impl_->masm.Cmlt(v_t, v_mask, 0);
    const vixl_aa::VRegister v_t_q   (kInternalFpScratchV,    vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_src_q (static_cast<int>(rsrc), vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_dst_q (static_cast<int>(rdst), vixl_aa::kFormat16B);
    impl_->masm.Bsl(v_t_q, v_src_q, v_dst_q);
    const vixl_aa::VRegister v_d_q(static_cast<int>(rd), vixl_aa::kFormat16B);
    impl_->masm.Mov(v_d_q, v_t_q);
}

void Emitter::clz_gpr(arm64::Reg rd, arm64::Reg rn, ir::OpSize sz) {
    if (sz == ir::OpSize::I32) {
        impl_->masm.Clz(to_vixl_w(rd), to_vixl_w(rn));
    } else {
        impl_->masm.Clz(to_vixl_x(rd), to_vixl_x(rn));
    }
}
void Emitter::rbit_clz_gpr(arm64::Reg rd, arm64::Reg rn, ir::OpSize sz) {
    // tzcnt = clz(rbit(x)).
    if (sz == ir::OpSize::I32) {
        vixl_aa::UseScratchRegisterScope temps(&impl_->masm);
        const vixl_aa::Register x_t = temps.AcquireX();
        const vixl_aa::WRegister w_t(static_cast<int>(x_t.GetCode()));
        impl_->masm.Rbit(w_t, to_vixl_w(rn));
        impl_->masm.Clz (to_vixl_w(rd), w_t);
    } else {
        vixl_aa::UseScratchRegisterScope temps(&impl_->masm);
        const vixl_aa::Register x_t = temps.AcquireX();
        impl_->masm.Rbit(x_t, to_vixl_x(rn));
        impl_->masm.Clz (to_vixl_x(rd), x_t);
    }
}

void Emitter::popcnt_gpr(arm64::Reg rd, arm64::Reg rn, ir::OpSize sz) {
    // Sequence:
    //   fmov  d31, x_n       ; copy 64-bit input into V31 low half
    //   cnt   v31.8b, v31.8b ; per-byte popcount (counts 8 bytes)
    //   addv  b31, v31.8b    ; horizontal sum of bytes (max 64 fits in 7 bits)
    //   fmov  w_d, s31       ; move byte sum back to GPR
    // For I32 we mask the upper 32 bits to zero first; the simplest is
    // `mov w_t, w_n` which zero-extends, then fmov d31, x_t.
    const vixl_aa::DRegister d_t(kInternalFpScratchV);
    const vixl_aa::VRegister v_t_8b(kInternalFpScratchV, vixl_aa::kFormat8B);
    const vixl_aa::BRegister b_t(kInternalFpScratchV);
    if (sz == ir::OpSize::I32) {
        // Move w_n (zero-extends to 64) into V31 low half via fmov d, x.
        // We can move via an X scratch first to ensure zero-extension.
        vixl_aa::UseScratchRegisterScope temps(&impl_->masm);
        const vixl_aa::Register x_t = temps.AcquireX();
        const vixl_aa::WRegister w_t(static_cast<int>(x_t.GetCode()));
        impl_->masm.Mov(w_t, to_vixl_w(rn));  // zero-extends to x_t
        impl_->masm.Fmov(d_t, vixl_aa::Register(x_t));
    } else {
        impl_->masm.Fmov(d_t, vixl_aa::Register(to_vixl_x(rn)));
    }
    impl_->masm.Cnt(v_t_8b, v_t_8b);
    impl_->masm.Addv(b_t, v_t_8b);
    impl_->masm.Fmov(to_vixl_w(rd), vixl_aa::SRegister(kInternalFpScratchV));
}

void Emitter::vfrint_q(FpReg rd, FpReg rn, ir::FpSize sz, std::uint8_t mode) {
    const auto fmt = (sz == ir::FpSize::F32) ? vixl_aa::kFormat4S
                                              : vixl_aa::kFormat2D;
    const vixl_aa::VRegister vd(static_cast<int>(rd), fmt);
    const vixl_aa::VRegister vn(static_cast<int>(rn), fmt);
    emit_vfrint(impl_->masm, vd, vn, mode);
}

void Emitter::vfrint_scalar_with_upper(FpReg rd, FpReg lhs, FpReg rhs,
                                       ir::FpSize sz, std::uint8_t mode) {
    // 1. frint scalar into V31's low lane.
    if (sz == ir::FpSize::F32) {
        const vixl_aa::SRegister s_t(kInternalFpScratchV);
        const vixl_aa::SRegister s_n(static_cast<int>(rhs));
        switch (mode & 0x3u) {
            case 0: impl_->masm.Frintn(s_t, s_n); break;
            case 1: impl_->masm.Frintm(s_t, s_n); break;
            case 2: impl_->masm.Frintp(s_t, s_n); break;
            case 3: impl_->masm.Frintz(s_t, s_n); break;
        }
    } else {
        const vixl_aa::DRegister d_t(kInternalFpScratchV);
        const vixl_aa::DRegister d_n(static_cast<int>(rhs));
        switch (mode & 0x3u) {
            case 0: impl_->masm.Frintn(d_t, d_n); break;
            case 1: impl_->masm.Frintm(d_t, d_n); break;
            case 2: impl_->masm.Frintp(d_t, d_n); break;
            case 3: impl_->masm.Frintz(d_t, d_n); break;
        }
    }
    // 2. mov rd <- lhs  3. ins lane 0 from V31.
    const vixl_aa::VRegister v_d_q  (static_cast<int>(rd),  vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_lhs_q(static_cast<int>(lhs), vixl_aa::kFormat16B);
    impl_->masm.Mov(v_d_q, v_lhs_q);
    if (sz == ir::FpSize::F32) {
        const vixl_aa::VRegister v_d_s4(static_cast<int>(rd), vixl_aa::kFormat4S);
        const vixl_aa::VRegister v_t_s4(kInternalFpScratchV,  vixl_aa::kFormat4S);
        impl_->masm.Mov(v_d_s4, 0, v_t_s4, 0);
    } else {
        const vixl_aa::VRegister v_d_d2(static_cast<int>(rd), vixl_aa::kFormat2D);
        const vixl_aa::VRegister v_t_d2(kInternalFpScratchV,  vixl_aa::kFormat2D);
        impl_->masm.Mov(v_d_d2, 0, v_t_d2, 0);
    }
}

void Emitter::vextend(FpReg rd, FpReg rn,
                      VecLane narrow_lane, VecLane wide_lane, bool is_signed) {
    // Chain sxtl/uxtl as needed. Each step doubles lane width and uses
    // only the low half of the source. Use V31 internal scratch for
    // intermediates so we can land in rd at the end.
    auto step = [&](int dst_v_code, int src_v_code,
                    VecLane narrow) {
        // narrow = lane width currently in low half of src.
        if (narrow == VecLane::B16) {
            // narrow → H8 (8 lanes). Use .8b → .8h.
            const vixl_aa::VRegister vd(dst_v_code, vixl_aa::kFormat8H);
            const vixl_aa::VRegister vs(src_v_code, vixl_aa::kFormat8B);
            if (is_signed) impl_->masm.Sxtl(vd, vs);
            else           impl_->masm.Uxtl(vd, vs);
        } else if (narrow == VecLane::H8) {
            const vixl_aa::VRegister vd(dst_v_code, vixl_aa::kFormat4S);
            const vixl_aa::VRegister vs(src_v_code, vixl_aa::kFormat4H);
            if (is_signed) impl_->masm.Sxtl(vd, vs);
            else           impl_->masm.Uxtl(vd, vs);
        } else {  // S4 → D2
            const vixl_aa::VRegister vd(dst_v_code, vixl_aa::kFormat2D);
            const vixl_aa::VRegister vs(src_v_code, vixl_aa::kFormat2S);
            if (is_signed) impl_->masm.Sxtl(vd, vs);
            else           impl_->masm.Uxtl(vd, vs);
        }
    };
    int cur = static_cast<int>(rn);
    VecLane cur_narrow = narrow_lane;
    while (cur_narrow != wide_lane) {
        const VecLane next = (cur_narrow == VecLane::B16) ? VecLane::H8 :
                             (cur_narrow == VecLane::H8 ) ? VecLane::S4 :
                                                             VecLane::D2;
        const int dst = (next == wide_lane) ? static_cast<int>(rd)
                                            : kInternalFpScratchV;
        step(dst, cur, cur_narrow);
        cur = dst;
        cur_narrow = next;
    }
}

void Emitter::valignr(FpReg rd, FpReg lhs, FpReg rhs, std::uint8_t count) {
    if (count >= 32) {
        impl_->masm.Movi(to_vixl_q_bitwise(rd), 0);
        return;
    }
    if (count == 0) {
        impl_->masm.Mov(to_vixl_q_bitwise(rd), to_vixl_q_bitwise(rhs));
        return;
    }
    if (count < 16) {
        // ext picks bytes from rhs[count..15] then lhs[0..count-1] —
        // exactly the low half of (lhs || rhs) shifted right by `count`.
        impl_->masm.Ext(to_vixl_q_bitwise(rd),
                        to_vixl_q_bitwise(rhs),
                        to_vixl_q_bitwise(lhs),
                        static_cast<int>(count));
    } else {
        // count == 16 → result = lhs (whole). count > 16 takes lhs
        // shifted right by (count - 16) zero-filled.
        if (count == 16) {
            impl_->masm.Mov(to_vixl_q_bitwise(rd), to_vixl_q_bitwise(lhs));
        } else {
            const vixl_aa::VRegister v_z(kInternalFpScratchV, vixl_aa::kFormat16B);
            impl_->masm.Movi(v_z, 0);
            impl_->masm.Ext(to_vixl_q_bitwise(rd),
                            to_vixl_q_bitwise(lhs), v_z,
                            static_cast<int>(count - 16));
        }
    }
}

void Emitter::vfcmp_packed(FpReg rd, FpReg rn, FpReg rm, ir::FpSize sz, std::uint8_t pred) {
    const auto fmt = (sz == ir::FpSize::F32) ? vixl_aa::kFormat4S
                                              : vixl_aa::kFormat2D;
    const vixl_aa::VRegister v_d(static_cast<int>(rd), fmt);
    const vixl_aa::VRegister v_n(static_cast<int>(rn), fmt);
    const vixl_aa::VRegister v_m(static_cast<int>(rm), fmt);
    emit_fp_pred_packed(impl_->masm, v_d, v_n, v_m, fmt, pred);
}

void Emitter::vfcmp_scalar_with_upper(FpReg rd, FpReg lhs, FpReg rhs,
                                      ir::FpSize sz, std::uint8_t pred) {
    // Compute the packed result into V31 (full 128-bit), then mov rd
    // = lhs (preserves upper) and INS lane 0 from V31.
    const auto fmt = (sz == ir::FpSize::F32) ? vixl_aa::kFormat4S
                                              : vixl_aa::kFormat2D;
    const vixl_aa::VRegister v_t(kInternalFpScratchV, fmt);
    const vixl_aa::VRegister v_n(static_cast<int>(lhs), fmt);
    const vixl_aa::VRegister v_m(static_cast<int>(rhs), fmt);
    // Use a scratch other than V31 because V31 is the result temp here.
    constexpr int kAux2 = 30;
    const vixl_aa::VRegister v_t2(kAux2, fmt);
    // Re-implement the predicate sequence using V31 as v_d and V30 as
    // the secondary scratch. We can't reuse emit_fp_pred_packed because
    // it pulls v_t (V31) from the same alias. Inline it here.
    const std::uint8_t base = pred & 0x3u;
    const bool negate = (pred & 0x4u) != 0;
    if (base == 0) {
        impl_->masm.Fcmeq(v_t, v_n, v_m);
    } else if (base == 1) {
        impl_->masm.Fcmgt(v_t, v_m, v_n);
    } else if (base == 2) {
        impl_->masm.Fcmge(v_t, v_m, v_n);
    } else {
        impl_->masm.Fcmeq(v_t, v_n, v_n);
        impl_->masm.Fcmeq(v_t2, v_m, v_m);
        impl_->masm.And  (v_t.V16B(), v_t.V16B(), v_t2.V16B());
        impl_->masm.Mvn  (v_t.V16B(), v_t.V16B());
    }
    if (negate) {
        impl_->masm.Mvn(v_t.V16B(), v_t.V16B());
    }
    // Now mov rd <- lhs, then INS lane 0 from V31.
    const vixl_aa::VRegister v_d_q  (static_cast<int>(rd),  vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_lhs_q(static_cast<int>(lhs), vixl_aa::kFormat16B);
    impl_->masm.Mov(v_d_q, v_lhs_q);
    if (sz == ir::FpSize::F32) {
        const vixl_aa::VRegister v_d_s4(static_cast<int>(rd), vixl_aa::kFormat4S);
        const vixl_aa::VRegister v_t_s4(kInternalFpScratchV,  vixl_aa::kFormat4S);
        impl_->masm.Mov(v_d_s4, 0, v_t_s4, 0);
    } else {
        const vixl_aa::VRegister v_d_d2(static_cast<int>(rd), vixl_aa::kFormat2D);
        const vixl_aa::VRegister v_t_d2(kInternalFpScratchV,  vixl_aa::kFormat2D);
        impl_->masm.Mov(v_d_d2, 0, v_t_d2, 0);
    }
}

void Emitter::vmask_fp(arm64::Reg w_dst, FpReg rn, bool is_pd, arm64::Reg w_tmp) {
    // Sequence: ushr v_t.4s/2d, vn.4s/2d, #31/63 → each lane = 0 or 1.
    // Then umov + orr-shifted to assemble bits 0..3 (or 0..1).
    // w_tmp is a caller-supplied scratch (the SSA register allocator
    // hands us one via allocate_temporary).
    if (is_pd) {
        const vixl_aa::VRegister v_t_2d(kInternalFpScratchV, vixl_aa::kFormat2D);
        const vixl_aa::VRegister v_n_2d(static_cast<int>(rn), vixl_aa::kFormat2D);
        impl_->masm.Ushr(v_t_2d, v_n_2d, 63);
        // Lane 0 → w_dst (low bit). Lane 1 → w_tmp, then OR << 1.
        impl_->masm.Umov(to_vixl_w(w_dst), v_t_2d.S(), 0);  // 2D lane viewed as 32-bit low half
        impl_->masm.Umov(to_vixl_w(w_tmp), v_t_2d.S(), 2);  // S-lane index 2 = D-lane 1, low half
        impl_->masm.Orr(to_vixl_w(w_dst), to_vixl_w(w_dst),
                        vixl_aa::Operand(to_vixl_w(w_tmp), vixl_aa::LSL, 1));
    } else {
        const vixl_aa::VRegister v_t_4s(kInternalFpScratchV, vixl_aa::kFormat4S);
        const vixl_aa::VRegister v_n_4s(static_cast<int>(rn), vixl_aa::kFormat4S);
        impl_->masm.Ushr(v_t_4s, v_n_4s, 31);
        impl_->masm.Umov(to_vixl_w(w_dst), v_t_4s, 0);
        for (unsigned i = 1; i < 4; ++i) {
            impl_->masm.Umov(to_vixl_w(w_tmp), v_t_4s, static_cast<int>(i));
            impl_->masm.Orr(to_vixl_w(w_dst), to_vixl_w(w_dst),
                            vixl_aa::Operand(to_vixl_w(w_tmp), vixl_aa::LSL, i));
        }
    }
}

void Emitter::vshuffle_h4(FpReg rd, FpReg rn, std::uint8_t control, bool is_high) {
    // Build the result in V31 starting from a full copy of src so the
    // passthrough half is preserved when rd == rn.
    const vixl_aa::VRegister v_t_q (kInternalFpScratchV, vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_n_q (static_cast<int>(rn), vixl_aa::kFormat16B);
    impl_->masm.Mov(v_t_q, v_n_q);
    const vixl_aa::VRegister v_t_h (kInternalFpScratchV, vixl_aa::kFormat8H);
    const vixl_aa::VRegister v_n_h (static_cast<int>(rn), vixl_aa::kFormat8H);
    const int base = is_high ? 4 : 0;
    for (int i = 0; i < 4; ++i) {
        const int src_lane = base + ((control >> (2 * i)) & 0x3);
        impl_->masm.Mov(v_t_h, base + i, v_n_h, src_lane);
    }
    const vixl_aa::VRegister v_d_q(static_cast<int>(rd), vixl_aa::kFormat16B);
    impl_->masm.Mov(v_d_q, v_t_q);
}

void Emitter::fcmp_scalar(FpReg rn, FpReg rm, ir::FpSize sz) {
    if (sz == ir::FpSize::F32) {
        impl_->masm.Fcmp(to_vixl_s(rn), to_vixl_s(rm));
    } else {
        impl_->masm.Fcmp(to_vixl_d(rn), to_vixl_d(rm));
    }
}

void Emitter::vmask_msb_b16(arm64::Reg w_dst, FpReg rn) {
    // PMOVMSKB sequence (well-known):
    //   1. cmlt v_t.16b, vn.16b, #0      ; t[i] = 0xFF if vn[i] MSB set
    //   2. build mask {1,2,...,128, 1,...,128} in V_mask via two .d[] inserts
    //   3. and v_t.16b, v_t.16b, v_mask.16b
    //   4. uaddlv h_lo, v_t.8b           ; sum bytes 0..7  → h_lo (low byte of mask)
    //   5. ext v_hi.16b, v_t.16b, v_t.16b, #8
    //   6. uaddlv h_hi, v_hi.8b
    //   7. umov w_lo, v_lo.h[0]; umov w_hi, v_hi.h[0]
    //   8. orr w_dst, w_lo, w_hi << 8
    //
    // Uses V31 (existing internal scratch) and V30 for the mask, plus
    // a vixl X scratch via UseScratchRegisterScope. The V regs we use
    // are outside the SSA pool (V0..V7).
    constexpr int kMaskV   = 30;
    constexpr int kHiTmpV  = 29;
    constexpr int kInternalV = kInternalFpScratchV;  // V31

    const vixl_aa::VRegister v_n_16b(static_cast<int>(rn), vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_t_16b(kInternalV, vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_t_8b (kInternalV, vixl_aa::kFormat8B);
    const vixl_aa::VRegister v_t_8h (kInternalV, vixl_aa::kFormat8H);
    const vixl_aa::VRegister v_mask_16b(kMaskV,  vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_mask_2d (kMaskV,  vixl_aa::kFormat2D);
    const vixl_aa::VRegister v_hi_16b(kHiTmpV,   vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_hi_8b (kHiTmpV,   vixl_aa::kFormat8B);
    const vixl_aa::VRegister v_hi_8h (kHiTmpV,   vixl_aa::kFormat8H);

    // 1. signed-less-than-zero per byte → mask of 0xFF/0x00.
    impl_->masm.Cmlt(v_t_16b, v_n_16b, 0);

    // 2. construct {1,2,4,8,16,32,64,128} repeated.
    {
        vixl_aa::UseScratchRegisterScope temps(&impl_->masm);
        const vixl_aa::Register x_t = temps.AcquireX();
        impl_->masm.Mov(x_t, 0x8040201008040201ULL);
        impl_->masm.Mov(v_mask_2d, 0, x_t);
        impl_->masm.Mov(v_mask_2d, 1, x_t);
    }

    // 3. and.
    impl_->masm.And(v_t_16b, v_t_16b, v_mask_16b);

    // 4-6. Compute upper-half sum FIRST (since it reads from v_t),
    // THEN compute lower-half sum (which clobbers v_t lane 0).
    impl_->masm.Ext(v_hi_16b, v_t_16b, v_t_16b, 8);
    impl_->masm.Uaddlv(vixl_aa::HRegister(kHiTmpV), v_hi_8b);
    impl_->masm.Uaddlv(vixl_aa::HRegister(kInternalV), v_t_8b);

    // 7. extract lo and hi to GPRs.
    {
        vixl_aa::UseScratchRegisterScope temps(&impl_->masm);
        const vixl_aa::Register w_hi = temps.AcquireW();
        impl_->masm.Umov(to_vixl_w(w_dst), v_t_8h, 0);
        impl_->masm.Umov(w_hi,             v_hi_8h, 0);
        // 8. w_dst = w_dst | (w_hi << 8)
        impl_->masm.Orr(to_vixl_w(w_dst), to_vixl_w(w_dst),
                        vixl_aa::Operand(w_hi, vixl_aa::LSL, 8));
    }
}

void Emitter::vumov_w_from_lane(arm64::Reg w_dst, FpReg rn,
                                std::uint8_t lane_idx, VecLane lane) {
    const auto fmt = vec_format_for_lane(lane);
    const vixl_aa::VRegister v_n_lane(static_cast<int>(rn), fmt);
    if (lane == VecLane::D2) {
        // 64-bit lane requires X-form GPR.
        impl_->masm.Umov(to_vixl_x(w_dst), v_n_lane, lane_idx);
    } else {
        impl_->masm.Umov(to_vixl_w(w_dst), v_n_lane, lane_idx);
    }
}

void Emitter::vshuffle_2src_s4(FpReg rd, FpReg rn, FpReg rm, std::uint8_t control) {
    const vixl_aa::VRegister v_t_s4(kInternalFpScratchV, vixl_aa::kFormat4S);
    const vixl_aa::VRegister v_n_s4(static_cast<int>(rn), vixl_aa::kFormat4S);
    const vixl_aa::VRegister v_m_s4(static_cast<int>(rm), vixl_aa::kFormat4S);
    // Lanes 0,1 from rn; 2,3 from rm.
    impl_->masm.Mov(v_t_s4, 0, v_n_s4, (control >> 0) & 0x3);
    impl_->masm.Mov(v_t_s4, 1, v_n_s4, (control >> 2) & 0x3);
    impl_->masm.Mov(v_t_s4, 2, v_m_s4, (control >> 4) & 0x3);
    impl_->masm.Mov(v_t_s4, 3, v_m_s4, (control >> 6) & 0x3);
    const vixl_aa::VRegister v_t_q(kInternalFpScratchV, vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_d_q(static_cast<int>(rd), vixl_aa::kFormat16B);
    impl_->masm.Mov(v_d_q, v_t_q);
}

void Emitter::vshuffle_2src_d2(FpReg rd, FpReg rn, FpReg rm, std::uint8_t control) {
    const vixl_aa::VRegister v_t_d2(kInternalFpScratchV, vixl_aa::kFormat2D);
    const vixl_aa::VRegister v_n_d2(static_cast<int>(rn), vixl_aa::kFormat2D);
    const vixl_aa::VRegister v_m_d2(static_cast<int>(rm), vixl_aa::kFormat2D);
    impl_->masm.Mov(v_t_d2, 0, v_n_d2, (control >> 0) & 0x1);
    impl_->masm.Mov(v_t_d2, 1, v_m_d2, (control >> 1) & 0x1);
    const vixl_aa::VRegister v_t_q(kInternalFpScratchV, vixl_aa::kFormat16B);
    const vixl_aa::VRegister v_d_q(static_cast<int>(rd), vixl_aa::kFormat16B);
    impl_->masm.Mov(v_d_q, v_t_q);
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
    BarrierType bt;
    switch (k) {
        case BarrierKind::Ish:   bt = vixl_aa::BarrierAll;    break;
        case BarrierKind::IshLd: bt = vixl_aa::BarrierReads;  break;
        case BarrierKind::IshSt: bt = vixl_aa::BarrierWrites; break;
    }
    impl_->masm.Dmb(vixl_aa::InnerShareable, bt);
}

void Emitter::fence(ir::FenceKind kind) {
    switch (kind) {
        case ir::FenceKind::Mfence:
            dmb(BarrierKind::Ish);
            return;
        case ir::FenceKind::Lfence:
            dmb(BarrierKind::IshLd);
            return;
        case ir::FenceKind::Sfence:
            dmb(BarrierKind::IshSt);
            return;
    }
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
