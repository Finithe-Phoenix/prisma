# Rust Migration Review Checklist (Para Gemini)

> Template para revisar un crate Rust migrado. Copiar + rellenar para
> cada review.

## Crate: _______________
## Reviewer: _________ (Gemini)
## Date: _______________
## Commit SHA(s): _______________

---

### 1. IR Type Alignment

- [ ] All `Op` variants in Rust match C++ `ir.hpp` (check each one)
- [ ] All `Op` variants match Lean `Syntax.lean` spec
- [ ] Missing variants listed: __________
- [ ] Extra variants listed: __________
- [ ] `#[repr(u8)]` on enums that cross FFI
- [ ] `#[repr(C)]` on structs that cross FFI
- [ ] `#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]` on all IR types

### 2. Safety Audit

- [ ] No `unsafe` blocks without `// SAFETY:` comment
- [ ] `#![deny(unsafe_op_in_unsafe_fn)]` is active
- [ ] `#![deny(unused_must_use)]` is active
- [ ] Unsafe blocks are minimal (prefer safe abstractions)
- [ ] FFI functions use `extern "C"` with correct ABI
- [ ] FFI functions handle null pointers
- [ ] FFI functions catch panics with `catch_unwind`
- [ ] C API returns `prisma_status` enum (not raw `c_int`)

### 3. Differential Tests

- [ ] At least one differential test comparing C++ vs Rust output
- [ ] Test covers: same input → same output
- [ ] Test covers: edge cases (empty, error, large)
- [ ] `cargo test` passes on the crate
- [ ] C++ test suite still passes with FFI bridge

### 4. Code Quality

- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo fmt` applied
- [ ] No commented-out code
- [ ] No `unwrap()` or `expect()` in library code (only in tests)
- [ ] Error types use `thiserror` or custom `Error` + `Display`
- [ ] Public API is documented with doc comments
- [ ] Internal functions are `pub(crate)` or private
- [ ] Assertions use `debug_assert!` (not `assert!`) in hot paths

### 5. Performance

- [ ] Hot paths avoid allocation (reuse buffers, use `Vec::with_capacity`)
- [ ] No unnecessary cloning of large IR statements
- [ ] Decoder: table-driven dispatch vs match-on-match chains
- [ ] Cache: LRU eviction is O(1)

### 6. FFI Bridge

- [ ] C API functions use `#[no_mangle]`
- [ ] C API header is kept in sync with Rust definitions
- [ ] Ownership model is documented:
  - Who allocates? Who frees?
  - Is it caller-owned or callee-owned?
- [ ] `extern "C"` structs have stable layout
- [ ] No `#[repr(packed)]` on structs that cross FFI

### 7. Testing Completeness

- [ ] Unit tests for each public function
- [ ] Integration tests for common use cases
- [ ] Error path tests (truncated input, invalid opcode, etc.)
- [ ] Edge case tests (boundary values, empty input)
- [ ] Property-based tests (proptest) for decoding any byte sequence
- [ ] Benchmark tests (optional, but nice to have)

### 8. Lean Cross-Check

- [ ] Any new IR variants are added to `ir-spec/PrismaIR/Syntax.lean`
- [ ] Lean spec builds without new `sorry`s
- [ ] `lake build` passes
- [ ] `.sorry-budget` is not exceeded

### 9. Commit Discipline

- [ ] Commit message matches format: `<scope>: <what>`
- [ ] Commit is atomic (one logical change per commit)
- [ ] No "WIP" or "fixup" commits in the review set
- [ ] Tests are in the same commit as the code they test

### 10. Security

- [ ] No allocation from signal handler context
- [ ] No use-after-free in cache / JIT memory
- [ ] Cache deserialization validates magic + version + SHA-256
- [ ] JIT memory is W^X (no RWX mappings)
- [ ] No hardcoded secrets or test-only secrets in code

---

## Summary

| Category | ✅ Pass | ⚠️ Minor | ❌ Major |
|----------|---------|----------|---------|
| IR Alignment | | | |
| Safety | | | |
| Differential | | | |
| Code Quality | | | |
| Performance | | | |
| FFI Bridge | | | |
| Testing | | | |
| Lean Cross-Check | | | |
| Commit Discipline | | | |
| Security | | | |

### Final verdict:

- [ ] **APPROVED** — safe to merge
- [ ] **APPROVED WITH COMMENTS** — minor issues, fix before next phase
- [️] **CHANGES REQUESTED** — must fix before merge
- [ ] **BLOCKED** — fundamental issue, needs discussion

### Detailed findings:

```
(Write findings here: what is correct, what needs fixing, what is missing)
```

---

*Template v1.0 — docs/REVIEW_TEMPLATES/rust-review-checklist.md*
