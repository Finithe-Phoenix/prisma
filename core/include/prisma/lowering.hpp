// prisma/lowering.hpp — IR → ARM64 lowering.
//
// First real lowering. Replaces the ad-hoc "trivial lowering" that lived
// inside test_e2e.cpp with a proper Lowerer class that consumes IR
// statements and drives the Emitter.
//
// Status: the lowerer now covers the growing Fase 1/Fase 2 IR surface,
// including bounded integer-register spilling for Translator-owned blocks.

#pragma once

#include "prisma/abi.hpp"
#include "prisma/arm64_encoding.hpp"  // for arm64::Reg
#include "prisma/emitter.hpp"
#include "prisma/ir.hpp"

#include <cstddef>
#include <cstdint>
#include <optional>
#include <span>
#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

namespace prisma::runtime {
struct CpuStateFrame;
}

namespace prisma::backend {

using SyscallHandlerFn = void (*)(runtime::CpuStateFrame*);

enum class LowerError {
  UnsupportedOp,
  OutOfScratchRegs,
  DanglingRef,
  InvalidBlock,
};

struct LowerResult {
  bool success{true};
  LowerError error{LowerError::UnsupportedOp};
  std::string message{};
};

// Lowering options.
//
// `emit_ret_on_terminator` is true for standalone Lowerer tests and false
// for Translator-owned blocks, where the ABI layer appends the state-saving
// epilogue. The Translator ABI exposes two bounded spill lanes in its existing
// aligned frame. Aggregate initialization with the first member set to false
// therefore enables those slots automatically; standalone lowering keeps
// spilling disabled and never writes into an unreserved caller stack frame.
//
// Explicit non-zero spill settings still override these defaults.
struct LowerOptions {
  bool emit_ret_on_terminator{true};
  unsigned spill_slots{emit_ret_on_terminator ? 0u : abi::kTranslatorSpillSlotCount};
  std::int32_t spill_slot_base_offset{
      emit_ret_on_terminator ? 0 : abi::kTranslatorSpillSlotBaseOffset};
  std::uint32_t cpuid_max_leaf{0};
  std::uint32_t cpuid_vendor_ebx{0};
  std::uint32_t cpuid_vendor_ecx{0};
  std::uint32_t cpuid_vendor_edx{0};
  std::uint32_t cpuid_leaf1_eax{0};
  std::uint32_t cpuid_leaf1_ebx{0};
  std::uint32_t cpuid_leaf1_ecx{0};
  std::uint32_t cpuid_leaf1_edx{0};
  std::uint32_t cpuid_leaf7_ebx{0};
  std::uint64_t xgetbv_xcr0{0};

  // Syscall dispatch: when non-null, `Syscall` IR ops emit a `blr` to
  // this function instead of halting. The handler receives the guest
  // CpuStateFrame, reads guest registers, performs the host operation,
  // and writes results back to the frame.
  SyscallHandlerFn syscall_handler{nullptr};
};

class Lowerer {
 public:
  explicit Lowerer(Emitter& emitter, LowerOptions options = {})
      : emitter_(emitter), options_(options) {}

  [[nodiscard]] LowerResult lower(std::span<const ir::Stmt> stmts);

  // Lower a multi-block ir::Function with explicit CFG. Pre-creates one
  // Emitter::Label per BasicBlock, binds each block, and resolves forward
  // Jump / CondJump references at finalize time.
  [[nodiscard]] LowerResult lower(const ir::Function& fn);

  [[nodiscard]] unsigned scratch_used() const noexcept { return peak_live_; }

 private:
  Emitter& emitter_;
  LowerOptions options_;

  // Active integer SSA allocations.
  std::unordered_map<ir::Ref, arm64::Reg> ref_to_scratch_;

  // Last-use position for each SSA ref.
  std::unordered_map<ir::Ref, std::size_t> last_use_;

  // x0..x9 free-list, seeded in deterministic order.
  std::vector<arm64::Reg> free_regs_;

  // Single-statement temporary registers.
  std::vector<arm64::Reg> stmt_temporaries_;

  std::size_t stmt_index_{0};
  unsigned peak_live_{0};

  [[nodiscard]] LowerResult lower_stmt(const ir::Stmt& s);
  void compute_liveness(std::span<const ir::Stmt> stmts);
  void expire_intervals();

  [[nodiscard]] bool allocate_scratch(ir::Ref ref, arm64::Reg& out);
  [[nodiscard]] bool allocate_temporary(arm64::Reg& out);
  [[nodiscard]] bool reg_of(ir::Ref ref, arm64::Reg& out);

  [[nodiscard]] LowerResult align_flag_operands(arm64::Reg lhs, arm64::Reg rhs,
                                                ir::OpSize size,
                                                arm64::Reg& out_lhs,
                                                arm64::Reg& out_rhs);

  // Belady-style bounded spill support.
  [[nodiscard]] bool spill_one_ref();
  std::unordered_map<ir::Ref, std::uint32_t> spilled_to_slot_;
  std::vector<std::uint32_t> free_slots_;
  unsigned peak_spills_{0};

  // Per-function block labels.
  std::unordered_map<std::uint32_t, Emitter::Label> block_labels_;

  // FP scratch pool (separate, currently non-spillable).
  std::unordered_map<ir::Ref, Emitter::FpReg> ref_to_fp_;
  std::vector<Emitter::FpReg> fp_free_;
  [[nodiscard]] bool allocate_fp_scratch(ir::Ref ref, Emitter::FpReg& out);
  [[nodiscard]] bool fp_reg_of(ir::Ref ref, Emitter::FpReg& out);

  [[nodiscard]] LowerResult lower_pcmpstr_index(const ir::PcmpStrIndex& op,
                                                ir::Ref result);
  [[nodiscard]] LowerResult lower_pcmpstr_mask(const ir::PcmpStrMask& op,
                                               ir::Ref result);
  [[nodiscard]] LowerResult lower_pcmpstr_flags(const ir::PcmpStrFlags& op,
                                                ir::Ref result);
  [[nodiscard]] LowerResult emit_pcmpstr_scalar_helper(
      ir::Ref lhs, ir::Ref rhs, std::optional<ir::Ref> lhs_len,
      std::optional<ir::Ref> rhs_len, std::uint8_t imm8,
      std::uint64_t helper_addr, const char* name, ir::Ref result);
  [[nodiscard]] LowerResult emit_pcmpstr_mask_helper(
      ir::Ref lhs, ir::Ref rhs, std::optional<ir::Ref> lhs_len,
      std::optional<ir::Ref> rhs_len, std::uint8_t imm8, ir::Ref result);
  [[nodiscard]] LowerResult lower_vec_f16cvt(const ir::VecF16Cvt& op,
                                             ir::Ref result);

  // Flags refs live in NZCV rather than a host GPR.
  std::unordered_set<ir::Ref> flag_refs_;
  std::unordered_set<ir::Ref> fp_flag_refs_;

 public:
  [[nodiscard]] unsigned peak_spills() const noexcept { return peak_spills_; }
};

}  // namespace prisma::backend
