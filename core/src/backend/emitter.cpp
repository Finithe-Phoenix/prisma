// core/src/backend/emitter.cpp — vixl-backed implementation of Emitter.
//
// Keeps vixl headers out of `emitter.hpp` via pimpl: clients of prisma_emitter
// do not need vixl on their include path.

#include "prisma/emitter.hpp"

#include <cstddef>
#include <cstdint>
#include <sstream>
#include <string>

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

void Emitter::ret(arm64::Reg rn) {
    impl_->masm.Ret(to_vixl_x(rn));
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
