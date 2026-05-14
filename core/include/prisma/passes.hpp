// prisma/passes.hpp — IR optimisation passes.
//
// Each pass is a pure function `Function → Function` (or a statement list
// transformer when useful). Passes NEVER mutate their input; they return
// the transformed IR. This lets us reason about soundness per-pass in Lean
// (Pillar 2) without worrying about ownership.
//
// Status: Fase 0 MVP — one pass (constant propagation). The pass manager
// arrives in Fase 1 once we have more than a couple of passes.

#pragma once

#include <cstddef>
#include <cstdint>
#include <functional>
#include <memory>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

#include "prisma/ir.hpp"

namespace prisma::passes {

// Constant propagation + folding for the pure arithmetic fragment.
//
// Transformations:
//   * `BinOp(op, ca, cb)` where both operands are Constants collapses to
//     a new Constant whose value is `mask_to_size(evalBinOp(op, ca, cb))`.
//   * Any StoreReg / Return / side-effecting op passes through unchanged,
//     possibly with updated refs where its operand was folded.
//
// The pass is local: it reasons within a single statement list, does not
// cross basic-block boundaries, and does not delete defs (a separate DCE
// pass handles that). A folded BinOp leaves its original LoadReg / other
// dependencies in place — they become dead but correctness is unchanged.
//
// See docs/rfc/0001-ir-ssa-over-template-based.md for the soundness
// obligation this pass will eventually satisfy formally in Lean.
[[nodiscard]] std::vector<ir::Stmt>
constant_propagate(const std::vector<ir::Stmt>& stmts);

// Dead Code Elimination.
//
// Removes pure statements whose bound Ref is never read by any subsequent
// statement. "Pure" here means: Constant, LoadReg, LoadSegBase, BinOp,
// Compare, LoadMem (non-TSO), LoadMemTSO (see note below). Side-effecting
// statements (StoreReg, StoreMem*, Jump, CondJump, Return) are never removed.
//
// Note on LoadMemTSO: strictly speaking a TSO load is observable under a
// weak memory model (it synchronises), so removing it when its value is
// dead changes the observable behaviour. For now we do keep LoadMemTSO
// alive even if the result ref is dead — it is not in the "pure" set for
// DCE. When the TSO-adaptive pass (Pillar 3) proves a region is
// single-threaded, those loads downgrade to plain LoadMem, and then
// become eligible for DCE.
//
// Algorithm: one backward pass that seeds `live_refs` from every side-
// effecting op's operands, plus one forward pass that filters. Correct-
// ness follows from the invariant that every Ref has a unique def.
[[nodiscard]] std::vector<ir::Stmt>
dead_code_eliminate(const std::vector<ir::Stmt>& stmts);

// Algebraic simplification — identities that hold for any value of the
// non-constant operand. Unlike constant_propagate, this fires when ONE
// side is a known constant with a special value, not both. Patterns:
//
//   x + 0    → x       x * 0 → 0
//   x - 0    → x       x * 1 → x
//   x - x    → 0       x << 0, x >> 0, x >>> 0 → x
//   x & 0    → 0       x | 0 → x
//   x & -1   → x       x | -1 → -1
//   x ^ x    → 0       x ^ 0 → x
//
// The transformation rewrites the Stmt's op in place (same result ref)
// so downstream uses see the simplified form. A later dead_code_eliminate
// removes any newly-unreferenced defs.
//
// All identities are sound without additional flag modelling — the
// semantics match x86 pure-value semantics. When flag-setting variants
// arrive (F1-IR-003), the pass will learn to skip rewrites that would
// change NZCV observability.
[[nodiscard]] std::vector<ir::Stmt>
algebraic_simplify(const std::vector<ir::Stmt>& stmts);

// Common Subexpression Elimination — within a single statement list,
// if two pure BinOps compute the same `(op, lhs, rhs, size)` with refs
// whose values have not been invalidated between the two statements,
// replace the second with a copy of the first's result (modeled as a
// trivial `or` with zero-aliasing — we use BinOp Or with the first
// result and itself, which the emitter lowers to a single `mov`).
//
// MVP scope: only considers BinOp. Does not track register aliasing
// through LoadReg → StoreReg chains — that's a harder pass (copy
// propagation, F1-PS-006).
[[nodiscard]] std::vector<ir::Stmt>
common_subexpression_eliminate(const std::vector<ir::Stmt>& stmts);

// Copy propagation — chase "move" chains produced by CSE.
//
// When CSE dedupes a duplicate BinOp, it emits `%b = Or %a, %a` (our
// IR idiom for "copy ref %a into ref %b"). Later uses of %b read an
// identical value through an extra scratch. Copy propagation rewrites
// every subsequent use of %b to reference %a directly, making the
// copy's result dead so DCE can remove it.
//
// MVP scope:
//   * Detects the `Or x, x` shape only. More general (Add x, 0) moves
//     are handled by algebraic_simplify.
//   * Intra-block only. No CFG reasoning.
[[nodiscard]] std::vector<ir::Stmt>
copy_propagate(const std::vector<ir::Stmt>& stmts);

// Strength reduction — cheaper primitive for the same value.
//
// For integer constants:
//   x * (1 << k)  →  x << k       (for k in 1..63)
//   x / (1 << k)  →  x >> k       NOT fired: rounding semantics differ
//                                  for signed division; wait for a
//                                  proper signed/unsigned split.
//
// The pass replaces `BinOp Mul, x, const_pow2` with
// `BinOp Shl, x, const_k`. It adds a new Constant for `k` and leaves
// the old one in place; DCE handles the cleanup.
[[nodiscard]] std::vector<ir::Stmt>
strength_reduce(const std::vector<ir::Stmt>& stmts);

// Branch folding — collapse a CondJumpRel whose condition is always
// taken or always not-taken to an unconditional JumpRel / fallthrough.
//
// Currently fires when the immediately-preceding CmpFlags compares
// two Constants whose concrete values make the condition statically
// knowable. The CmpFlags is retained (conservative) in case a future
// ReadFlag uses it; DCE prunes it if not.
//
// MVP scope:
//   * Only the (CmpFlags constA, constB) pattern.
//   * Not yet following bool-producing Compare → Select chains.
[[nodiscard]] std::vector<ir::Stmt>
branch_fold(const std::vector<ir::Stmt>& stmts);

// Flag-write elimination — drop `CmpFlags` whose implicit flags are
// never read later.
//
// In this MVP, `CondJumpRel` is the consumer of implicit flags and
// `Compare` / `CmpFlags` are the known writers.
//
// We keep the nearest `CmpFlags` before each surviving flag reader.
// This pass intentionally runs after branch folding, so when
// `branch_fold` rewrites a conditional branch to `JumpRel`, the
// orphaned `CmpFlags` becomes removable.
[[nodiscard]] std::vector<ir::Stmt>
flag_write_elimination(const std::vector<ir::Stmt>& stmts);

// Redundant-Load Elimination — drop a LoadMem whose `(addr_ref, size)`
// pair was just read without any intervening memory write.
//
// Typical pattern after aggressive decoding:
//   %a = LoadReg rax
//   %v1 = LoadMem %a, I64
//   ...pure ops...
//   %v2 = LoadMem %a, I64        ← redundant; rewrite to a copy of %v1
//
// Rewrite: `%v2 = Or %v1, %v1` (our "move" idiom, dedup-friendly for
// copy_propagate and DCE).
//
// MVP scope:
//   * Plain LoadMem only. LoadMemTSO is intentionally NOT deduplicated
//     — it has acquire semantics that a second load observes
//     independently; removing one changes observable behaviour.
//   * Any store (StoreMem/StoreMemTSO) invalidates the whole table
//     conservatively (no alias analysis yet).
//   * CmpFlags, CondJumpRel, JumpRel etc. don't touch memory — left
//     alone.
[[nodiscard]] std::vector<ir::Stmt>
redundant_load_eliminate(const std::vector<ir::Stmt>& stmts);

// Tail-call optimisation (F1-PS-015).
//
// Folds the canonical `CallRel{T, R}; RetAdjusted{0};` pair into a
// single `JumpRel{T}`. Sound under x86 calling conventions:
//
//   CallRel pushes R onto the stack and jumps to T. T eventually
//   executes its own RetAdjusted, popping R and returning to R —
//   which, in this pattern, is the byte after our CallRel and
//   therefore points at our RetAdjusted{0}. RetAdjusted{0} pops the
//   caller's return address (the one we inherited on entry) and
//   returns to it.
//
//   Net: T runs, then control returns to OUR caller after popping
//   one return address. JumpRel{T} achieves the same: T runs (with
//   our caller's return address still on top of the stack), then
//   T's RetAdjusted pops that and returns to our caller.
//
// MVP scope:
//   * Only `RetAdjusted{0}` qualifies. Non-zero pop_bytes would
//     require a stack adjustment before the jump; not in scope.
//   * Only the immediately-adjacent CallRel + RetAdjusted pair.
//   * CallReg (indirect) is not folded — same idea applies but its
//     interaction with the dispatcher's indirect-call path needs
//     more thought.
[[nodiscard]] std::vector<ir::Stmt>
tail_call_optimise(const std::vector<ir::Stmt>& stmts);

// Dead-Store Elimination — drop a StoreMem whose result is provably
// overwritten by a later StoreMem before any read. Conservative MVP
// treats only the "same addr ref, same size, no intervening memory op"
// case as an overwrite.
//
// Example:
//   StoreMem %a, %v1, I64       ← dead
//   StoreMem %a, %v2, I64       ← kept
// Anything between the two (LoadMem / StoreMem to any addr / TSO ops
// / fences / calls) disables the rewrite — we can't prove the first
// write went unobserved.
//
// MVP scope:
//   * Plain StoreMem only. StoreMemTSO is release-ordered and
//     observable by other cores; removing one changes behaviour.
//   * LoadMem*, any other StoreMem, JumpReg, Return, Call flush the
//     pending-store table.
[[nodiscard]] std::vector<ir::Stmt>
dead_store_eliminate(const std::vector<ir::Stmt>& stmts);

// ---------------------------------------------------------------------------
// PassManager — ordered pipeline of named passes, with run statistics.
// ---------------------------------------------------------------------------
//
// Why a manager at all: once we have more than a couple of passes, we
// want control over ordering, the ability to skip passes for debugging,
// and a place to hang telemetry for the Pillar 1 (NPU) classifier. The
// manager is intentionally simple today — no fixed-point iteration, no
// dependency declarations — so we can evolve it alongside actual needs.

struct PassRunStats {
    // Statement count after each pass (in order). The first entry is the
    // count AFTER the first pass ran; `initial_stmt_count` carries the
    // count BEFORE any pass. This makes it trivial to compute per-pass
    // deltas.
    //
    // `duration_ns` is the wall-clock cost of running that pass on its
    // input (steady_clock, one sample per invocation). It is useful for
    // the Pillar 1 NPU feature extractor and for "which pass is hot"
    // debugging; treat it as noisy — single-sample timings at microsecond
    // scale jitter a lot.
    struct PassEntry {
        std::string   name;
        std::size_t   stmts_after;
        std::uint64_t duration_ns{0};
    };
    std::size_t initial_stmt_count{0};
    std::vector<PassEntry> passes;
};

class PassManager {
public:
    using PassFn = std::function<std::vector<ir::Stmt>(const std::vector<ir::Stmt>&)>;

    // Dump hook — invoked once per pass inside `run()` with the pass's
    // name and the statement list AFTER the pass has run. Multiple hooks
    // can be registered; they fire in registration order.
    //
    // Intended uses:
    //   * Tests inspecting intermediate IR between passes.
    //   * Debug-build dumps gated on a `--debug-pass=<name>` filter
    //     (the filtering is the caller's responsibility — the hook
    //     fires unconditionally so the caller sees every pass).
    //   * Future NPU-classifier feature extraction (Pillar 1).
    using DumpHook = std::function<void(
        const std::string& pass_name,
        const std::vector<ir::Stmt>& after)>;

    // Register a pass to be run in order. Insertion order == run order.
    // `name` should be unique within a manager — duplicates are legal but
    // make stats harder to read.
    PassManager& add(std::string name, PassFn fn);

    // Register a dump hook. Returns *this for fluent chaining.
    PassManager& on_pass_run(DumpHook hook);

    // Number of registered passes.
    [[nodiscard]] std::size_t size() const noexcept { return passes_.size(); }

    // Run all passes in order and return the final statements plus
    // per-pass statistics. Never mutates its argument.
    [[nodiscard]] std::pair<std::vector<ir::Stmt>, PassRunStats>
    run(const std::vector<ir::Stmt>& input) const;

private:
    struct Entry {
        std::string name;
        PassFn fn;
    };
    std::vector<Entry>    passes_;
    std::vector<DumpHook> hooks_;
};

// ---------------------------------------------------------------------------
// F1-PS-009 peephole pattern matcher.
// ---------------------------------------------------------------------------
//
// A `PeepholeRule` looks at a statement list at a given index and either
// declines (returns `nullopt`) or returns the rewrite — a sequence of 0
// or more replacement statements that should occupy `match_len` slots
// starting at `idx`.
//
// The orchestrator `peephole_optimise(stmts, rules)` walks the list,
// tries each rule at each position, applies the first match, and starts
// over. Termination is bounded by `kPeepholeMaxIterations` to guard
// against rules that loop. Rules that *grow* the program count toward
// the iteration cap too.
//
// Pre-baked rule sets:
//   * `peephole_default_rules()` — the small canonical set used by the
//     default pipeline (BinOp xor-self → 0; redundant Truncate I64→I64;
//     Extend identity).

struct PeepholeMatch {
    std::size_t            consumed;     // how many input stmts to remove
    std::vector<ir::Stmt>  replacement;  // 0..N replacements
};

class PeepholeRule {
public:
    virtual ~PeepholeRule() = default;
    [[nodiscard]] virtual std::optional<PeepholeMatch>
    match(const std::vector<ir::Stmt>& stmts, std::size_t idx) const = 0;

    // Human-readable rule name, surfaced in stats and dumps.
    [[nodiscard]] virtual std::string_view name() const noexcept = 0;
};

inline constexpr std::size_t kPeepholeMaxIterations = 8u;

[[nodiscard]] std::vector<ir::Stmt>
peephole_optimise(const std::vector<ir::Stmt>& stmts,
                  std::span<const PeepholeRule* const> rules);

[[nodiscard]] std::vector<std::unique_ptr<PeepholeRule>>
peephole_default_rules();

// Convenience wrapper for the most common case: load the default rule
// set, run a pass over `stmts`. Allocates per call; for hot paths
// build the rule list once.
[[nodiscard]] std::vector<ir::Stmt>
peephole_optimise_default(const std::vector<ir::Stmt>& stmts);

// Returns the default pipeline used by the rest of the codebase:
//   constant_propagate → dead_code_eliminate
// Fase 1+ will grow this; for now these two passes are the whole story.
[[nodiscard]] PassManager default_pipeline();

// ---------------------------------------------------------------------------
// F2-PS-004 — Function-level pipeline + Global CSE via dominators.
// ---------------------------------------------------------------------------
//
// The stmt-level `PassManager` runs over a flat `std::vector<Stmt>` and is
// the workhorse for intra-block transformations (the existing 12 passes).
// Once the translator starts emitting multi-block `ir::Function`s, we
// also want passes that see the whole CFG — at minimum to forward
// available-expression information along dominator-tree edges.
//
// Pipeline shape: each pass is `Function → Function`. The manager is
// otherwise structurally identical to `PassManager`.
//
// **Today's translator emits single-block functions** (one x86 instr →
// one Decoded → one BB), so global CSE on what the translator produces
// is equivalent to the existing intra-block CSE. The plumbing here
// is the deliverable; the algorithm unlocks real wins when the
// translator gains multi-instruction fusion.

struct FunctionPassRunStats {
    struct PassEntry {
        std::string   name;
        std::size_t   blocks_after;
        std::size_t   stmts_after;
        std::uint64_t duration_ns{0};
    };
    std::size_t initial_block_count{0};
    std::size_t initial_stmt_count{0};
    std::vector<PassEntry> passes;
};

class FunctionPassManager {
public:
    using PassFn = std::function<ir::Function(const ir::Function&)>;
    using DumpHook = std::function<void(
        const std::string& pass_name,
        const ir::Function& after)>;

    FunctionPassManager& add(std::string name, PassFn fn);
    FunctionPassManager& on_pass_run(DumpHook hook);
    [[nodiscard]] std::size_t size() const noexcept { return passes_.size(); }

    [[nodiscard]] std::pair<ir::Function, FunctionPassRunStats>
    run(const ir::Function& input) const;

private:
    struct Entry {
        std::string name;
        PassFn fn;
    };
    std::vector<Entry>    passes_;
    std::vector<DumpHook> hooks_;
};

// Global Common Subexpression Elimination — same canonical-form
// hashing as intra-block CSE, but forwarded across blocks along
// dominator-tree edges where the child's only predecessor in the
// CFG is its immediate dominator. Diamond / join blocks (more than
// one predecessor) start with an empty available-expression table,
// which is correct but conservative: a value computed on all
// incoming paths is not commoned. Tightening this requires a
// classical available-expressions dataflow analysis, deferred.
//
// Within each block the algorithm matches `common_subexpression_eliminate`
// — flushing on StoreReg / StoreMem / StoreMemTSO / CmpFlags, rewriting
// a duplicate BinOp to `BinOp Or prev,prev` (the IR's "copy" idiom).
[[nodiscard]] ir::Function global_cse(const ir::Function& fn);

// Default function-level pipeline. Currently just `global_cse`.
// **Not yet wired into the translator** — see the caveat above.
[[nodiscard]] FunctionPassManager default_function_pipeline();

}  // namespace prisma::passes
