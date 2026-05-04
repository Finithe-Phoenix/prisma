// prisma/lowering.hpp — IR → ARM64 lowering.
//
// First real lowering. Replaces the ad-hoc "trivial lowering" that lived
// inside test_e2e.cpp with a proper Lowerer class that consumes IR
// statements and drives the Emitter.
//
// Scope for this version (Fase 0):
//
//   Supported IR ops:
//     Constant, LoadReg, StoreReg, BinOp (Add/Sub/And/Or/Xor/Shl/Shr/Sar),
//     Return.
//
//   Rejected IR ops (returns LowerError::Unsupported):
//     Compare, Jump, CondJump, LoadMem, StoreMem, LoadMemTSO, StoreMemTSO.
//     These land in subsequent sessions (flags, CFG, memory).
//
// Register allocation strategy:
//
//   Guest GPRs rax..r15 are mapped to host x10..x25 per the Fase 1 fixed
//   mapping (`arm64::host_reg_for`). Those are "pinned" — StoreReg writes
//   the host register directly, and LoadReg reads from it.
//
//   SSA values produced by Constant / LoadReg / BinOp live in a pool of
//   scratch registers x0..x9 (10 registers). Allocation is linear-scan
//   (F1-BK-007):
//     1. A pre-pass computes each Ref's last-use statement index.
//     2. Lowering proceeds forward; after each statement any Ref whose
//        last_use has passed returns its scratch to a free-list pool.
//     3. New allocations pop from the free-list (prefer lowest-numbered
//        reg for reproducible encodings).
//   If a block's simultaneously-live set exceeds 10 registers, lowering
//   fails with LowerError::OutOfScratchRegs. Spilling to the stack frame
//   lands next in F1-BK-008.

#pragma once

#include <cstddef>
#include <cstdint>
#include <span>
#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

#include "prisma/arm64_encoding.hpp"  // for arm64::Reg
#include "prisma/emitter.hpp"
#include "prisma/ir.hpp"

namespace prisma::backend {

enum class LowerError {
    UnsupportedOp,       // IR op we do not lower yet (e.g. Compare, Jump).
    OutOfScratchRegs,    // More than 10 simultaneously-live SSA values.
    DanglingRef,         // A statement references a Ref that was never defined.
};

struct LowerResult {
    bool success{true};
    LowerError error{LowerError::UnsupportedOp};
    std::string message{};  // human-readable context when !success
};

// Lowering options.
//
// `emit_ret_on_terminator`: controls whether IR::Return, IR::JumpRel,
// and IR::CondJumpRel end with an ARM64 `ret` instruction.
//   true  (default) — suitable for tests that exercise the Lowerer in
//                     isolation and want self-contained code bytes.
//   false           — the Translator uses this so it can insert a
//                     register-save epilogue between the terminator's
//                     "put next PC in x0" and the final `ret`.
//
// `spill_slots` + `spill_slot_base_offset`: enable register spilling
// (F1-BK-008). When the scratch pool is exhausted the allocator picks
// a victim by Belady's heuristic (farthest next-use), stores it to
// `[sp, #spill_slot_base_offset + slot*8]`, and reloads on next use.
//   spill_slots == 0            — spilling disabled (MVP default).
//   spill_slots > 0             — up to that many concurrent spills.
//   spill_slot_base_offset      — byte offset of slot 0 from sp.
// The caller (Translator) owns reserving the stack space; the Lowerer
// only emits str/ldr referencing the pre-agreed offsets.
struct LowerOptions {
    bool          emit_ret_on_terminator{true};
    unsigned      spill_slots{0};
    std::int32_t  spill_slot_base_offset{0};
};

class Lowerer {
public:
    explicit Lowerer(Emitter& emitter, LowerOptions options = {})
        : emitter_(emitter), options_(options) {}

    // Lower the given statement list onto the underlying Emitter. Returns
    // a result indicating success or the first error encountered. After
    // an error the Emitter state is undefined — throw it away.
    [[nodiscard]] LowerResult lower(std::span<const ir::Stmt> stmts);

    // Lower a multi-block ir::Function with explicit CFG (F1-BK-003,
    // F1-BK-004, F1-BK-006). Pre-creates one Emitter::Label per
    // BasicBlock, walks the blocks in their stored order, binds each
    // block's label, and lowers its statements. Within a block,
    // Jump{target_block} emits an unconditional `b <label>` and
    // CondJump{cond, if_true, if_false} emits `cbnz xcond, label_true; b
    // label_false`. vixl's MacroAssembler resolves forward references at
    // finalize time — that is the "label fix-up" of F1-BK-006.
    //
    // The flat lower(stmts) overload above is unchanged: it still rejects
    // Jump / CondJump with UnsupportedOp because no block context exists.
    //
    // This call mutates internal state (ref allocator, block label map)
    // exactly like the flat lower(); they share private members.
    [[nodiscard]] LowerResult lower(const ir::Function& fn);

    // For tests: the peak number of scratch registers that were
    // simultaneously live during the last call to lower(). A value of 10
    // means the pool was saturated.
    [[nodiscard]] unsigned scratch_used() const noexcept { return peak_live_; }

private:
    Emitter& emitter_;
    LowerOptions options_;

    // Map IR Ref → host scratch register (active assignments only — entries
    // are erased on expiry).
    std::unordered_map<ir::Ref, arm64::Reg> ref_to_scratch_;

    // Liveness info: last_use_[r] == max statement index that reads r, or
    // the def index if r is never used. Populated once by compute_liveness
    // at the start of lower().
    std::unordered_map<ir::Ref, std::size_t> last_use_;

    // Free-list of scratch registers. Used as a stack: back is popped
    // first. Initialised with x9..x0 so x0 comes out first.
    std::vector<arm64::Reg> free_regs_;

    // Scratch regs allocated as single-statement temporaries (e.g. the
    // Rol/Rcl `neg` helper). Returned to the free list after lower_stmt.
    std::vector<arm64::Reg> stmt_temporaries_;

    // Monotonically increasing during lower(): the index of the stmt we
    // are about to emit. Used both for post-stmt expiry and for debug.
    std::size_t stmt_index_{0};

    // Peak occupancy of ref_to_scratch_ + stmt_temporaries_ observed
    // during the last lower() call. Exposed by scratch_used().
    unsigned peak_live_{0};

    [[nodiscard]] LowerResult lower_stmt(const ir::Stmt& s);

    // Walk `stmts` once to populate last_use_ for every def'd Ref.
    void compute_liveness(std::span<const ir::Stmt> stmts);

    // Called after each lower_stmt: free any Ref whose last_use_ has
    // passed, plus return this stmt's temporaries to the free list.
    void expire_intervals();

    // Allocate a scratch register bound to `ref`. Returns false on
    // exhaustion (and spilling, if enabled, also failed to free one).
    [[nodiscard]] bool allocate_scratch(ir::Ref ref, arm64::Reg& out);

    // Allocate one extra scratch register for temporary use within a
    // single stmt (e.g. emulated rotate-left). Auto-freed at stmt end.
    [[nodiscard]] bool allocate_temporary(arm64::Reg& out);

    // Look up the host register that currently holds an SSA Ref's
    // value, reloading from a spill slot if necessary. Non-const because
    // a reload may emit a `ldr` + allocate a fresh scratch (which may
    // itself evict another ref).
    [[nodiscard]] bool reg_of(ir::Ref ref, arm64::Reg& out);

    // Spill one currently-held ref to a stack slot, returning its reg
    // to the free list. Picks the victim with the farthest next-use
    // (Belady). Returns false if spill_slots are exhausted or no ref
    // is evictable (e.g. all refs expire at this stmt).
    [[nodiscard]] bool spill_one_ref();

    // Spill tracking (F1-BK-008). `spilled_to_slot_[r] == i` means r
    // lives in `[sp, #spill_slot_base_offset + i*8]`. `free_slots_`
    // is a stack of available slot indices.
    std::unordered_map<ir::Ref, std::uint32_t> spilled_to_slot_;
    std::vector<std::uint32_t>                 free_slots_;
    unsigned                                   peak_spills_{0};

    // Per-Function lowering: maps BasicBlock id → its Emitter::Label.
    // Empty for the flat lower(span<Stmt>) path; non-empty (and
    // populated up-front) for the lower(Function) path. Looked up by
    // Jump / CondJump lowering inside lower_stmt.
    std::unordered_map<std::uint32_t, Emitter::Label> block_labels_;

    // FP scratch pool (F1-BK-013). 8-register pool V0..V7, separate
    // from the integer pool because ARM64's V regs encode in their
    // own namespace. Non-spillable in the MVP — too many simultaneous
    // FP refs returns OutOfScratchRegs.
    std::unordered_map<ir::Ref, Emitter::FpReg> ref_to_fp_;
    std::vector<Emitter::FpReg>                 fp_free_;
    [[nodiscard]] bool allocate_fp_scratch(ir::Ref ref, Emitter::FpReg& out);
    [[nodiscard]] bool fp_reg_of(ir::Ref ref, Emitter::FpReg& out);

    // Flags refs (F1-IR-003 .. F1-IR-007) don't have a host register
    // — they live in NZCV. We just track which Refs are valid Flags
    // values so consumers (ReadFlag / CondJumpFlags) can verify
    // their operand was produced by a prior WriteFlags.
    std::unordered_set<ir::Ref> flag_refs_;

public:
    // For tests: peak number of slots in concurrent use during the
    // last lower() call.
    [[nodiscard]] unsigned peak_spills() const noexcept { return peak_spills_; }
};

}  // namespace prisma::backend
