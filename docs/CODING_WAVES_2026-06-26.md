# Prisma - Coding Waves 2026-06-26

> Purpose: executable task queue for extreme coding waves. This file does not
> replace `docs/BACKLOG.md`, `docs/BACKLOG_EXTREMO.md`, or
> `docs/WORK_QUEUE.md`; it is the current-session attack plan derived from the
> live worktree plus those queues.

## Current Anchor

- Branch: `main`.
- Live WIP: Rust decoder/backend/runtime/passes changes around persistent
  RFLAGS, carry preservation, wider decoder coverage, and smoke differential
  fixtures.
- Dirty files at generation time:
  - `shell/prisma-ir/src/lib.rs`
  - `shell/prisma-decoder/src/decode.rs`
  - `shell/prisma-decoder/src/tables.rs`
  - `shell/prisma-backend/src/assembler.rs`
  - `shell/prisma-backend/src/lowerer.rs`
  - `shell/prisma-passes/src/{branch_fold,copy_prop,cse,dce,dead_store,flag_write_elim,global_cse,redundant_load}.rs`
  - `shell/prisma-runtime/src/{dispatcher,executor}.rs`
  - `shell/prisma-translator/src/interp.rs`
  - `shell/prisma-runtime/tests/smoke_differential.rs`
  - `shell/core/tests/smoke_differential.rs`
- Most important active theme: finish the Rust flags/RFLAGS spine without
  lying to the optimizer or to `PUSHFQ`/`POPFQ`.

## Current Validation Snapshot

- 2026-06-26 targeted Rust gate passed:
  - `cargo fmt --all --manifest-path shell\Cargo.toml -- --check`
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-ir --lib`
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-passes --lib`
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-backend --lib`
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-translator --lib`
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-decoder --lib`
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-runtime --test smoke_differential -- --nocapture`
  - `cargo fmt --all --manifest-path shell\Cargo.toml -- --check`
- 2026-06-26 latest targeted Rust gate passed:
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-decoder --lib`
    (`233 passed`)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-ir --lib`
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-passes --lib`
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-backend --lib`
    (`108 passed`)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-translator --lib`
    (`44 passed`)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-runtime --test smoke_differential -- --nocapture`
    (`3 passed`)
  - `PRISMA_CORE_LIB_DIR=core\build\Debug` plus `PATH=core\build\Debug;%PATH%`
    `cargo test --manifest-path shell\Cargo.toml -p prisma-core --test smoke_differential -- --nocapture`
    (`3 passed`)
  - `core\build\Debug\prisma_core_tests.exe "decode LAHF placeholder via 9F,decode SAHF placeholder via 9E" --reporter compact`
    (`2 test cases`, `14 assertions`)
  - `cargo fmt --all --manifest-path shell\Cargo.toml -- --check`
- 2026-06-26 W2-04 focal Rust gate passed:
  - `cargo fmt --all --manifest-path shell\Cargo.toml`
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-decoder --lib decode_group3_ -- --nocapture`
    (`14 passed`)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-translator --lib decoded_group3 -- --nocapture`
    (`2 passed`)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-runtime --test exec_muldiv -- --nocapture`
    (`11 passed`)
- 2026-06-26 W2-07 focal C++/core gate passed:
  - `cmake --build core\build --config Debug --target prisma_core_tests --parallel 4`
  - `core\build\Debug\prisma_core_tests.exe "decode ANDN*,decode BLSR*,decode BEXTR*,decode BMI1 BLS group rejects invalid subop" --reporter compact`
    (`4 test cases`, `39 assertions`)
  - First `prisma-core` smoke attempt with relative `PRISMA_CORE_LIB_DIR=core\build\Debug`
    failed at link (`LNK1181 prisma_core_c.lib`); rerun with absolute
    `PRISMA_CORE_LIB_DIR` passed:
    `cargo test --manifest-path shell\Cargo.toml -p prisma-core --test smoke_differential live_cpp_translator_accepts_rust_smoke_fixtures -- --nocapture`
    (`1 passed`)
- 2026-06-26 W2-04 focal C++/core gate passed:
  - `cmake --build core\build --config Debug --target prisma_core_tests --parallel 4`
  - `core\build\Debug\prisma_core_tests.exe "decode MUL/IMUL byte Group 3 writes AX only,decode DIV/IDIV byte Group 3 uses AX and stores AH:AL,decode DIV word Group 3 uses DX:AX dividend,decode IDIV dword Group 3 uses EDX:EAX signed dividend,*smulhi*,decode ANDN*,decode BLSR*,decode BEXTR*,decode BMI1 BLS group rejects invalid subop" --reporter compact`
    (`9 test cases`, `94 assertions`)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-core --test smoke_differential live_cpp_translator_accepts_rust_smoke_fixtures -- --nocapture`
    with absolute `PRISMA_CORE_LIB_DIR`
    (`1 passed`)
- 2026-06-26 W2-07 C++ CPUID compile/lowering gate passed:
  - `cmake --build core\build --config Debug --target prisma_core_tests --parallel 4`
  - `core\build\Debug\prisma_core_tests.exe "e2e: CPUID leaf model + SHA advertisement*" --reporter compact`
    (`1 assertion`; ARM64 semantic sections skipped on this host)
  - `core\build\Debug\prisma_core_tests.exe "Lowerer: Cpuid emits*" --reporter compact`
    (`12 assertions`)
- 2026-06-26 W2-03 Rust CMPXCHG16B pair-CAS gate passed:
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-ir --lib map_refs_shifts_atomic_pair_secondary_result -- --nocapture`
    (`1 passed`)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-backend --lib encodes_exclusive_memory_ops -- --nocapture`
    (`1 passed`)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-backend --lib lowers_atomic_cmpxchg_pair_to_exclusive_pair_loop -- --nocapture`
    (`1 passed`)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-decoder --lib cmpxchg -- --nocapture`
    (`8 passed`)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-passes --lib -- --nocapture`
    (`82 passed`)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-runtime --test smoke_differential rust_smoke_translator_matches_pinned_backend_bytes -- --nocapture`
    (`1 passed`)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-backend --lib -- --nocapture`
    (`109 passed`)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-decoder --lib -- --nocapture`
    (`236 passed`)
  - `cargo fmt --all --manifest-path shell\Cargo.toml -- --check`
  - `git diff --check` (clean except CRLF normalization warnings)
- 2026-06-26 W2-03 C++ CMPXCHG16B pair-CAS gate passed:
  - `cmake --build core\build --config Debug --target prisma_core_tests --parallel 4`
  - `core\build\Debug\prisma_core_tests.exe "*CMPXCHG16B*" --reporter compact`
    (`2 test cases`, `23 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "*AtomicCmpxchgPair*" --reporter compact`
    (`2 test cases`, `9 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "*ldaxp*" --reporter compact`
    (`1 test case`, `3 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "OpCounter: Kind covers every Op variant" --reporter compact`
    (`1 test case`, `1 assertion`)
- 2026-06-26 W2-03 C++ scalar CMPXCHG/CMPXCHG8B CAS gate passed:
  - `cmake --build core\build --config Debug --target prisma_core_tests --parallel 4`
  - `core\build\Debug\prisma_core_tests.exe "*CMPXCHG*" --reporter compact`
    (`16 test cases`, `152 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "*AtomicCmpxchg*" --reporter compact`
    (`4 test cases`, `18 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "OpCounter: Kind covers every Op variant" --reporter compact`
    (`1 test case`, `1 assertion`)
- 2026-06-26 W2-02/W2-12 C++ carry-slot infrastructure gate passed:
  - `cmake --build core\build --config Debug --target prisma_core_tests --parallel 4`
  - `core\build\Debug\prisma_core_tests.exe "*Carry*" --reporter compact`
    (`7 test cases`, `28 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "OpCounter: Kind covers every Op variant" --reporter compact`
    (`1 test case`, `1 assertion`)
  - `core\build\Debug\prisma_core_tests.exe "ir_serialize: LoadCarry / StoreCarry round-trip" --reporter compact`
    (`1 test case`, `8 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "CpuStateFrame: cf and rflags offsets/defaults are stable" --reporter compact`
    (`1 test case`, `6 assertions`)
- 2026-06-26 W2-02 C++ ADC/SBB carry-in dataflow gate passed:
  - `cmake --build core\build --config Debug --target prisma_core_tests --parallel 4`
  - `core\build\Debug\prisma_core_tests.exe "*ADC*,*SBB*" --reporter compact`
    (`11 test cases`, `107 assertions`)
- 2026-06-26 W2-12 C++ partial-RFLAGS stack/AH gate passed:
  - `cmake --build core\build --config Debug --target prisma_core_tests --parallel 4`
  - `core\build\Debug\prisma_core_tests.exe "*PUSHFQ*,*POPFQ*,*LAHF*,*SAHF*,*Rflags*" --reporter compact`
    (`8 test cases`, `77 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "*Carry*" --reporter compact`
    (`11 test cases`, `110 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "ir_serialize: LoadRflags / StoreRflags round-trip" --reporter compact`
    (`1 test case`, `8 assertions`)
- 2026-06-26 W2-12 C++ persistent RFLAGS publication gate passed:
  - `cmake --build core\build --config Debug --target prisma_core_tests --parallel 4`
  - `core\build\Debug\prisma_core_tests.exe "*Rflags*,*ADC*,*SBB*,*Group 1 imm8*,*TEST*,*CMP rax, rbx*,*SCAS*,*CMPS*,*XADD*" --reporter compact`
    passed the matched broad decoder/RFLAGS set (`50 test cases`, `469 assertions`);
    Catch split the comma in the CMP filter, so CMP was rerun separately.
  - `core\build\Debug\prisma_core_tests.exe "*CMP rax*" --reporter compact`
    (`3 test cases`, `22 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "IR deserialization: every Op variant round-trips,IR serialization: every Op variant serializes deterministically,ir_serialize: LoadRflags / StoreRflags round-trip,OpCounter: Kind covers every Op variant,Lowerer: StoreRflagsFrom*" --reporter compact`
    passed matched profiler/lowerer/legacy-serialize cases (`4 test cases`,
    `26 assertions`); exact modern serialization names did not match this
    Catch filter.
  - `core\build\Debug\prisma_core_tests.exe "*Op variant*" --reporter compact`
    (`2 test cases`, `6 assertions`)
  - `git diff --check` (clean except CRLF normalization warnings)
- 2026-06-26 W2-12 C++ OR/XOR persistent logical-RFLAGS gate passed:
  - `cmake --build core\build --config Debug --target prisma_core_tests --parallel 4`
  - `core\build\Debug\prisma_core_tests.exe "*OR rax*,*XOR rax*,*AND rax*,*ADD rax*,*SUB rax*,*logical memory*,*RIP-relative memory operands*,*OR rbp*,*AND rsi*" --reporter compact`
    (`19 test cases`, `183 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "Lowerer: WriteFlags logical Or/Xor*,Lowerer: AluFlags logical Or/Xor*,Lowerer: StoreRflagsFrom*" --reporter compact`
    (`4 test cases`, `23 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "decode*" --reporter compact`
    (`369 test cases`, `2605 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "Lowerer:*" --reporter compact`
    (`103 test cases`, `423 assertions`)
  - Full `core\build\Debug\prisma_core_tests.exe --reporter compact` was
    attempted and timed out after 184s without returning failure output; use
    the targeted gates above as the current evidence.
- 2026-06-26 W2-12 C++ INC/DEC carry-preserving RFLAGS gate passed:
  - Initial rebuild hit `LNK1168` because the earlier full-suite timeout left
    `prisma_core_tests.exe` running and holding `prisma_core_c.dll`; that stale
    process was stopped and the rebuild was rerun.
  - `cmake --build core\build --config Debug --target prisma_core_tests --parallel 4`
  - `core\build\Debug\prisma_core_tests.exe "*INC rax*,*DEC rax*" --reporter compact`
    (`2 test cases`, `16 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "decode*" --reporter compact`
    (`369 test cases`, `2607 assertions`)
- 2026-06-27 W2-07 C++ BMI1 persistent-RFLAGS gate passed:
  - `cmake --build core\build --config Debug --target prisma_core_tests --parallel 4`
  - `core\build\Debug\prisma_core_tests.exe "decode ANDN*,decode BLSR*,decode BEXTR*,decode BMI1 BLS group rejects invalid subop" --reporter compact`
    (`4 test cases`, `45 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "decode*" --reporter compact`
    (`369 test cases`, `2613 assertions`)
- 2026-06-27 W2-12 C++ BSF/BSR partial-RFLAGS gate passed:
  - `cmake --build core\build --config Debug --target prisma_core_tests --parallel 4`
  - `core\build\Debug\prisma_core_tests.exe "*BSF*,*BSR*" --reporter compact`
    (`4 test cases`, `27 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "decode*" --reporter compact`
    (`369 test cases`, `2619 assertions`)
- 2026-06-27 W1-08/W2-12 C++ bit-test CF gate passed:
  - `cmake --build core\build --config Debug --target prisma_core_tests --parallel 4`
  - `core\build\Debug\prisma_core_tests.exe "*BT rax*,*BTS rax*,*BTR rax*,*BTC rax*" --reporter compact`
    (`4 test cases`, `46 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "decode*" --reporter compact`
    (`369 test cases`, `2613 assertions`)
- 2026-06-27 W2-01/W2-05 C++ RCL/RCR persistent-carry gate passed:
  - `cmake --build core\build --config Debug --target prisma_core_tests --parallel 4`
  - `core\build\Debug\prisma_core_tests.exe "*RCL*,*RCR*" --reporter compact`
    (`5 test cases`, `36 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "decode*" --reporter compact`
    (`370 test cases`, `2617 assertions`)
- 2026-06-27 W2-05 C++ D1 non-carry shift/rotate persistent-flags gate passed:
  - `cmake --build core\build --config Debug --target prisma_core_tests --parallel 4`
  - `core\build\Debug\prisma_core_tests.exe "*Group 2 rax*" --reporter compact`
    (`1 test case`, `47 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "*RCL/RCR rax*" --reporter compact`
    (`1 test case`, `12 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "decode*" --reporter compact`
    (`371 test cases`, `2664 assertions`)
- 2026-06-27 W2-05 C++ D1 memory shift/rotate gate passed:
  - `cmake --build core\build --config Debug --target prisma_core_tests --parallel 4`
  - `core\build\Debug\prisma_core_tests.exe "*Group 2 memory*" --reporter compact`
    (`1 test case`, `54 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "decode*" --reporter compact`
    (`372 test cases`, `2718 assertions`)
- 2026-06-27 W2-05 C++ C1/D3 shift/rotate persistent-flags gate passed:
  - `cmake --build core\build --config Debug --target prisma_core_tests --parallel 4`
  - `core\build\Debug\prisma_core_tests.exe "*publishes persistent flags via 48 C1*" --reporter compact`
    (`5 test cases`, `40 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "*publishes persistent flags via 48 D3*" --reporter compact`
    (`5 test cases`, `35 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "*Group 2 memory imm8/CL*" --reporter compact`
    (`1 test case`, `118 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "decode*" --reporter compact`
    (`373 test cases`, `2831 assertions`)
- 2026-06-27 W2-12 C++ Group 3 NOT/NEG gate passed:
  - `cmake --build core\build --config Debug --target prisma_core_tests --parallel 4`
  - `core\build\Debug\prisma_core_tests.exe "*NOT*","*NEG*" --reporter compact`
    (`28 test cases`, `109 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "decode*" --reporter compact`
    (`374 test cases`, `2845 assertions`)
- 2026-06-27 W2-04 C++ IMUL memory-source gate passed:
  - `cmake --build core\build --config Debug --target prisma_core_tests --parallel 4`
  - `core\build\Debug\prisma_core_tests.exe "*IMUL*" --reporter compact`
    (`7 test cases`, `66 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "decode*" --reporter compact`
    (`375 test cases`, `2862 assertions`)
- 2026-06-27 W2-04/W2-12 C++ MUL/IMUL CF/OF gate passed:
  - `cmake --build core\build --config Debug --target prisma_core_tests --parallel 4`
  - `core\build\Debug\prisma_core_tests.exe "*MUL*","*IMUL*" --reporter compact`
    (`43 test cases`, `186 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "decode*" --reporter compact`
    (`375 test cases`, `2881 assertions`)
- 2026-06-27 W2-04 C++ DIV/IDIV memory-divisor gate passed:
  - `cmake --build core\build --config Debug --target prisma_core_tests --parallel 4`
  - `core\build\Debug\prisma_core_tests.exe "*DIV*","*IDIV*" --reporter compact`
    (`14 test cases`, `117 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "decode*" --reporter compact`
    (`376 test cases`, `2917 assertions`)
- 2026-06-27 W1-08/W2-12 C++ bit-test memory gate passed:
  - `cmake --build core\build --config Debug --target prisma_core_tests --parallel 4`
  - `core\build\Debug\prisma_core_tests.exe "*BT*","*BTS*","*BTR*","*BTC*" --reporter compact`
    (`7 test cases`, `93 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "decode*" --reporter compact`
    (`378 test cases`, `2963 assertions`)
- 2026-06-27 W1-08 Rust bit-test immediate-offset gate passed:
  - `cargo fmt --all --manifest-path shell\Cargo.toml`
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-decoder --lib decode_bt -- --nocapture`
    (`6 passed`)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-decoder --lib`
    (`237 passed`)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-runtime --test smoke_differential -- --nocapture`
    (`3 passed`; includes BT/BTS memory immediate qword-offset smoke fixtures)
- 2026-06-27 W2-04 Rust WideDiv IR/interpreter gate passed:
  - `cargo fmt --all --manifest-path shell\Cargo.toml`
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-ir --lib`
    (`11 passed`)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-decoder --lib group3_qword_div -- --nocapture`
    (`1 passed`)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-translator --lib wide_ -- --nocapture`
    (`2 passed`)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-passes --lib`
    (`82 passed`)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-backend --lib`
    (`109 passed`)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-decoder --lib`
    (`238 passed`)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-translator --lib`
    (`47 passed`)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-runtime --test exec_muldiv -- --nocapture`
    (`11 passed`; this pre-backend gate proved qword DIV/IDIV were deferred instead of emitting the old RAX-only lowering)
- 2026-06-27 W2-04 C++ WideDiv IR/frontier gate passed:
  - `cmake --build core\build --config Debug --target prisma_core_tests --parallel 4`
  - `core\build\Debug\prisma_core_tests.exe "*DIV*" "*IDIV*" --reporter compact`
    (`4 test cases`, `73 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "*WideDiv*" --reporter compact`
    (`2 test cases`, `10 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "[ir_serialize]" --reporter compact`
    (`44 test cases`, `469 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "*OpCounter*" --reporter compact`
    (`15 test cases`, `37 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "Lowerer: WideDiv*" --reporter compact`
    (`1 test case`, `2 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "decode*" --reporter compact`
    (`378 test cases`, `2967 assertions`)
  - Full `core\build\Debug\prisma_core_tests.exe --reporter compact` was
    attempted and timed out after 184s without returning a result; do not count
    it as a full-suite green.
- 2026-06-27 W2-04 Rust WideDiv backend gate passed:
  - `cargo fmt --all --manifest-path shell\Cargo.toml`
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-backend --lib wide_div -- --nocapture`
    (`3 passed`)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-runtime --test exec_muldiv -- --nocapture`
    (`11 passed`; qword DIV/IDIV now translate through backend instead of deferring)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-backend --lib`
    (`112 passed`)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-translator`
    (`47 lib passed`, `4 fuzz_translate passed`, `1 interp_loop_diag passed`)
- 2026-06-27 W2-04 C++ WideDiv backend gate passed:
  - First rebuild hit `LNK1168` because stale `prisma_core_tests.exe` was
    holding `prisma_core_c.dll`; stale pid `44040` was stopped and rebuild
    rerun.
  - `cmake --build core\build --config Debug --target prisma_core_tests --parallel 4`
  - `core\build\Debug\prisma_core_tests.exe "*WideDiv*" --reporter compact`
    (`3 test cases`, `20 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "*DIV*" "*IDIV*" --reporter compact`
    (`4 test cases`, `73 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "Lowerer: WideDiv*" --reporter compact`
    (`2 test cases`, `12 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "decode*" --reporter compact`
    (`378 test cases`, `2967 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "Lowerer:*" --reporter compact`
    (`105 test cases`, `435 assertions`)
- 2026-06-27 W2-04 Rust/C++ DIV #DE backend guard gate passed:
  - `cargo fmt --all --manifest-path shell\Cargo.toml`
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-backend --lib div -- --nocapture`
    (`5 passed`; scalar div/mod now guard divisor-zero before ARM64 `UDIV`/`SDIV`, and `WideDiv` guards divisor-zero plus unsigned/signed quotient overflow)
  - `cmake --build core\build --config Debug --target prisma_core_tests --parallel 4`
  - `core\build\Debug\prisma_core_tests.exe "Lowerer:*" --reporter compact`
    (`108 test cases`, `446 assertions`)
  - `git diff --check -- shell\prisma-backend\src\lowerer.rs core\src\backend\lowering.cpp`
    (clean except LF-to-CRLF normalization warnings)
- 2026-06-27 W2-04 Rust/C++ `TrapIf` DIV/IDIV guard gate passed:
  - Rust IR gained `TrapIf { condition, kind }`; decoder emits `Sigfpe`
    guards for DIV/IDIV divisor-zero and narrow quotient overflow; backend,
    passes, and interpreter preserve the conditional trap side effect.
  - C++ IR gained the same `TrapIf`; decoder emits divisor-zero guards,
    lowering branches around the placeholder `SIGFPE` return, and profiler,
    validation, serializers, copy-prop, and DCE all include the op.
  - `core/src/ir/serialization.cpp` and `core/tests/test_ir_serialization.cpp`
    are now wired into `prisma_core_tests`; the legacy serializer path is no
    longer a dead source/test file.
  - `cargo fmt --all --manifest-path shell\Cargo.toml`
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-decoder --lib div -- --nocapture`
    (`4 passed`)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-passes --lib -- --nocapture`
    (`82 passed`)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-backend --lib -- --nocapture`
    (`117 passed`)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-translator --lib -- --nocapture`
    (`49 passed`)
  - `cmake --build core\build --config Debug --target prisma_core_tests --parallel 4`
  - `core\build\Debug\prisma_core_tests.exe "decode DIV*" --reporter compact`
    (`6 test cases`, `93 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "decode IDIV*" --reporter compact`
    (`2 test cases`, `25 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "IR serialization:*" --reporter compact`
    (`4 test cases`, `407 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "IR deserialization:*" --reporter compact`
    (`4 test cases`, `156 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "ir_serialize:*" --reporter compact`
    (`45 test cases`, `482 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "*Trap*" --reporter compact`
    (`3 test cases`, `27 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "copy_prop:*" --reporter compact`
    (`7 test cases`, `13 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "dce:*" --reporter compact`
    (`17 test cases`, `47 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "OpCounter*" --reporter compact`
    (`14 test cases`, `36 assertions`)
- 2026-06-27 W2-06 Rust PCMPxSTRx skeleton/flags gate passed:
  - `cargo fmt --all --manifest-path shell\Cargo.toml`
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-ir --lib pcmp -- --nocapture`
    (`1 passed`)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-decoder --lib pcmp -- --nocapture`
    (`4 passed`)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-translator --lib pcmp -- --nocapture`
    (`2 passed`; result plus CF/ZF/SF/OF publication)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-passes --lib pcmp -- --nocapture`
    (`0 matched`; crate compiled)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-ir --lib`
    (`12 passed`)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-decoder --lib`
    (`242 passed`)
- 2026-06-27 W2-06 Rust backend XMM/PCMP semantic gate passed:
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-backend --lib vec -- --nocapture`
    (`2 passed`)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-backend --lib pcmp -- --nocapture`
    (`3 passed`)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-backend --lib cpuid -- --nocapture`
    (`2 passed`; includes SSE4.2 bit 20 for PCMPxSTRx discovery)
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-backend --lib`
    (`117 passed`)
  - `cargo fmt --all --manifest-path shell\Cargo.toml`
  - `git diff --check -- shell/prisma-backend/src/lowerer.rs`
    (clean except CRLF normalization warning)
- 2026-06-27 W2-06 C++ PCMPxSTRx IR/plumbing gate passed:
  - `cmake --build core\build --config Debug --target prisma_core_tests --parallel 4`
  - `core\build\Debug\prisma_core_tests.exe "*PcmpStr*" --reporter compact`
    (`2 test cases`, `18 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "ir_serialize: PcmpStr ops round-trip" --reporter compact`
    (`1 test case`, `12 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "OpCounter: Kind covers every Op variant" --reporter compact`
    (`1 assertion`)
- 2026-06-27 W2-06 C++ PCMPxSTRx decoder gate passed:
  - `cmake --build core\build --config Debug --target prisma_core_tests --parallel 4`
  - `core\build\Debug\prisma_core_tests.exe "*PCMP*" --reporter compact`
    (`13 test cases`, `83 assertions`; covers PCMPISTRI index, PCMPESTRM explicit lengths,
    PCMPISTRM memory RHS mask, extended XMM registers through REX.R/REX.B,
    truncated imm8 error handling, PCMPGTQ, and existing PCMP-adjacent vector fixtures)
  - `core\build\Debug\prisma_core_tests.exe "decode*" --reporter compact`
    (`382 test cases`, `3016 assertions`)
  - `git diff --check -- core/src/decoder/x86_decoder.cpp core/tests/test_decoder.cpp docs/CODING_WAVES_2026-06-26.md`
    (clean except LF-to-CRLF normalization warnings)
- 2026-06-27 W2-06 C++ PCMPxSTRx backend helper gate passed:
  - `cmake --build core\build --config Debug --target prisma_core_tests --parallel 4`
  - `core\build\Debug\prisma_core_tests.exe "Lowerer: PcmpStr*" --reporter compact`
    (`3 test cases`, `11 assertions`; covers index, mask, and packed flags lowering through helpers)
  - `core\build\Debug\prisma_core_tests.exe "*PCMP*" --reporter compact`
    (`15 test cases`, `88 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "Lowerer:*" --reporter compact`
    (`108 test cases`, `446 assertions`)
  - `git diff --check -- core/include/prisma/emitter.hpp core/src/backend/emitter.cpp core/include/prisma/lowering.hpp core/src/backend/lowering.cpp core/tests/test_lowering.cpp docs/CODING_WAVES_2026-06-26.md`
    (clean except LF-to-CRLF normalization warnings)
- 2026-06-27 W2-06 C++ CPUID SSE4.2 advertisement gate passed:
  - `cmake --build core\build --config Debug --target prisma_core_tests --parallel 4`
  - `core\build\Debug\prisma_core_tests.exe "*CPUID*" --reporter compact`
    (`4 test cases`, `37 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "Lowerer: Cpuid*" --reporter compact`
    (`2 test cases`, `16 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "e2e: CPUID*" --reporter compact`
    (`1 assertion`; non-ARM64 host skips runtime body)
- 2026-06-27 W2-08 PCLMULQDQ focal Rust/C++ gate passed:
  - `cargo fmt --all --manifest-path shell\Cargo.toml`
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-ir map_refs_shifts_vec_clmul_operands -- --nocapture`
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-decoder decode_pclmulqdq_register_form -- --nocapture`
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-translator decoded_pclmulqdq_multiplies_selected_qword_lanes -- --nocapture`
  - `cargo test --manifest-path shell\Cargo.toml -p prisma-backend lowers_vec_clmul_with_scalar_carryless_loop -- --nocapture`
  - `cargo test --manifest-path shell\Cargo.toml -p prisma_orchestrator cpuid_leaves::tests::sse_family_lands_in_leaf1_edx_and_ecx -- --nocapture`
  - `cargo test --manifest-path shell\Cargo.toml -p prisma_orchestrator cpu_features::tests::advertise_default_accepts_the_full_translatable_set -- --nocapture`
  - `cmake --build core\build --config Debug --target prisma_core_tests --parallel 4`
  - `core\build\Debug\prisma_core_tests.exe *PCLMUL*`
    (`2 test cases`, `19 assertions`)
  - `core\build\Debug\prisma_core_tests.exe *VecClMul*`
    (`1 test case`, `2 assertions`)
  - `core\build\Debug\prisma_core_tests.exe [ir_serialize]`
    (`45 test cases`, `486 assertions`)
  - `core\build\Debug\prisma_core_tests.exe OpCounter:*`
    (`14 test cases`, `36 assertions`)
  - `core\build\Debug\prisma_core_tests.exe "IR serialization:*"`
    (`4 test cases`, `416 assertions`)
  - `core\build\Debug\prisma_core_tests.exe *CPUID*`
    (`4 test cases`, `37 assertions`)
  - `git diff --check`
    (clean except LF-to-CRLF normalization warnings)
- 2026-07-05 W2-09 F16C Rust/C++ gate passed:
  - C++ `VecF16Cvt` plumbing completed (the WIP had IR/operation/profiler
    only): legacy serializer tag 108, modern serializer tag 46,
    pretty-print, validator, DCE/copy-prop/LICM, ARM64 lowering through
    bit-exact software helpers, VEX.128.66.0F38.W0 13 / VEX.128.66.0F3A.W0
    1D decode, CPUID.1:ECX.F16C bit 29, and `/bigobj` for MSVC (the Op
    variant outgrew the default COFF section limit).
  - Fixed a live Rust decoder bug: F16C guards required decoded
    `vvvv == 0x0f`, i.e. wire vvvv 0000, so canonical encodings
    (`C4 E2 79 13 /r`, `C4 E3 79 1D /r ib`) were rejected and only
    invalid ones accepted; guards now require decoded `vvvv == 0`
    (wire 1111) and decoder/interpreter fixtures use canonical bytes.
  - `cargo test -p prisma-{ir,passes,decoder,backend,translator} --lib`
    (120/82/246/120/51 passed) plus
    `-p prisma-runtime --test smoke_differential` (3 passed, four new
    F16C fixtures) and `cargo fmt --check`.
  - `cmake --build core\build --config Debug --target prisma_core_tests
    --parallel 1` (parallel 4/2 hit `C1060` out-of-heap on a 16 GB
    host; build serially), then `decode VCVT*,decode F16C*`
    (6 cases, 35 assertions), `Lowerer: VecF16Cvt*` (6 assertions),
    `decode*` (389 cases, 3086 assertions), `Lowerer:*` (111 cases),
    `[ir_serialize]` (45 cases, 494 assertions),
    `IR serialization/deserialization:*` (8 cases, 599 assertions),
    `OpCounter*` (14 cases, after adding the VecF16Cvt visit).
  - `prisma-core --test smoke_differential` against the freshly built
    Debug DLL with absolute `PRISMA_CORE_LIB_DIR` (3 passed — live C++
    translator accepts the F16C fixtures).
- Closed in the latest coding pass:
  - Decoder exact fixture refresh for explicit PF/AF scaffolding; decoder lib
    is now green with normalized flag-side-effect assertions where appropriate.
  - Backend and interpreter restore transient NZCV from persistent RFLAGS for
    block-entry condition consumers; standalone Jcc smoke fixtures now require
    the restore prefix while cmp+jcc keeps local flags live.
  - `CpuStateFrame` layout test pins `cf=808`, `rflags=816`,
    `exit_reason=824`, `next_pc=832`, and `mem_base=840`.
  - Runtime smoke differential now accepts intentional persistent-RFLAGS
    expansion while guarding stable non-memory words, branch tails, and the
    RFLAGS store side effect.
  - Explicit `StoreRflagsFromNzcv` IR and pass/backend/interpreter plumbing.
  - Explicit `StoreRflagsFromBits` IR and pass/backend/interpreter plumbing for
    non-NZCV flag publication.
  - `StoreRflagsFromBits` now has optional PF/AF refs; backend and interpreter
    clear/write those bits only when the producer supplies them.
  - `StoreRflagsFromNzcv` now has optional PF/AF refs; backend/interpreter
    publish them after reading NZCV, so ZF/SF/OF/CF still come from transient
    flags while parity/auxiliary bits come from SSA refs.
  - INC/DEC now publish ZF/SF/OF into persistent RFLAGS while preserving CF.
  - ADD/SUB/CMP/TEST/logical flag writers now publish the NZCV-derived
    RFLAGS subset in the Rust decoder; ADD/SUB/CMP now also publish PF/AF,
    and TEST/logical publish PF while preserving undefined AF.
  - ADC/SBB now publish explicit persistent PF/AF/ZF/SF/OF after their chained
    carry/borrow dataflow while preserving CF in the separate carry slot.
  - XADD, CMPXCHG, SCAS, and CMPS publish the same persistent RFLAGS subset
    without disturbing their existing dataflow refs.
  - BT/BTS/BTR/BTC immediate forms explicitly store old-bit CF.
  - LAHF/SAHF decode, lower, and interpret through persistent RFLAGS: LAHF
    loads SF/ZF/AF/PF/CF plus reserved bit 1 into AH while excluding OF; SAHF
    stores AH SF/ZF/AF/PF/CF back to RFLAGS while preserving OF and syncing CF.
  - W1-10 first corpus slice: interpreter adversarial table now covers
    add/sub wrap, signed overflow, ADC carry-in, SBB borrow-in, TEST zero,
    PF/AF/ZF/SF/OF/CF publication, reserved bit 1, and CF mirroring; Rust
    runtime smoke now includes LAHF, SAHF, add+LAHF, and SAHF+Jcc translation.
  - C++ decoder now accepts LAHF/SAHF: LAHF materializes the current C++
    placeholder flag model into AH as reserved bit 1 (`0x0200`) while SAHF is
    consumed as a placeholder until C++ gets persistent RFLAGS. Core smoke now
    includes LAHF/SAHF acceptance and passes with the local Debug core library.
  - W2-01 Rust behavior slice: interpreter now supports `Select` and unsigned
    div/mod enough to execute decoded RCL/RCR variable-count IR; tests cover
    RCL/RCR by CL, count=0 preservation, and CF/RFLAGS bit 0 mirroring.
  - W2-03 first atomic-CAS slices: Rust IR now has explicit `AtomicCmpxchg`,
    passes treat it as side-effecting memory, the ARM64 backend lowers it with
    `LDAXR*`/`STLXR*` plus `CLREX`, Rust `CMPXCHG` memory forms emit CAS
    instead of a `LoadMem`/`StoreMem` split, and Rust `CMPXCHG8B` now uses the
    same atomic I64 CAS path. Decoder, backend, runtime smoke, and translator
    gates are green for these slices.
  - W2-03 Rust CMPXCHG16B pair-CAS slice: Rust IR now has
    `AtomicCmpxchgPair`, optimizer passes treat it as side-effecting memory,
    the ARM64 backend lowers it with `LDAXP`/`STLXP` plus `CLREX`, Rust decoder
    `REX.W 0F C7 /1` emits a true 128-bit pair-CAS dataflow, and runtime smoke
    includes `cmpxchg16b_m128`.
  - W2-03 C++ CMPXCHG16B pair-CAS slice: C++ IR now has
    `AtomicCmpxchgPair`, the serializer/profiler/validator and memory-sensitive
    passes understand it, the ARM64 emitter/lowerer generate an
    `LDAXP`/`STLXP` retry loop with `CLREX` on compare failure, and the C++
    decoder now emits atomic pair-CAS dataflow for `48 0F C7 /1` instead of the
    old `LoadMemTSO`/`StoreMemTSO` placeholder split.
  - W2-03 C++ scalar atomic-CAS slice: C++ IR now has `AtomicCmpxchg`,
    serializers/profiler/validator/passes treat it as side-effecting memory
    with an old-value result, the ARM64 lowerer emits `LDAXR*`/`STLXR*` retry
    loops with `CLREX` on compare failure, and C++ memory `CMPXCHG` plus
    `CMPXCHG8B` now emit true CAS IR instead of `LoadMemTSO`/`StoreMemTSO`
    load/select/store splits.
  - W2-02/W2-12 C++ carry-slot foundation: `CpuStateFrame` now has stable
    `cf` and partial `rflags` slots, C++ IR has `LoadCarry`/`StoreCarry`,
    serializer/profiler/validator/passes understand the ops, and the ARM64
    lowerer reads/writes the carry slot while mirroring RFLAGS bit 0.
  - W2-02 C++ ADC/SBB carry-in slice: C++ decoder `ADC`/`SBB` register and
    memory destinations now read persistent CF, include carry-in/borrow-in in
    the computed result, publish the new CF with `StoreCarry`, and keep the old
    transient `AluFlags` bridge for non-CF flags until C++ RFLAGS parity lands.
  - W2-12 C++ partial-RFLAGS stack/AH slice: C++ IR now has `LoadRflags` and
    `StoreRflags`; the lowerer reads/writes the partial RFLAGS slot, forces
    reserved bit 1 on stores, and syncs CF; C++ `PUSHFQ`, `POPFQ`, `LAHF`, and
    `SAHF` now use that partial RFLAGS model instead of placeholder constants
    or no-ops.
  - W2-12 C++ persistent-RFLAGS publication slice: C++ IR now has
    `StoreRflagsFromNzcv` and `StoreRflagsFromBits`; serializer,
    serialization, profiler, validator, memory-sensitive passes, CSE/copy-prop,
    DCE, and flag-write elimination understand the new side-effecting ops; the
    ARM64 lowerer publishes ZF/SF/OF/PF/AF plus CF policy into the partial
    RFLAGS slot and mirrors bit 0 into the carry slot; C++ decoder routes now
    publish persistent flags for ADD/SUB/AND/CMP/TEST, ADC/SBB explicit-bit
    dataflow, XADD, CMPS, and SCAS. Decoder tests were refreshed to structural
    assertions around semantic flag publication instead of fixed statement
    counts.
  - W2-12 C++ OR/XOR logical-RFLAGS slice: C++ `WriteFlags` and `AluFlags`
    now support logical OR/XOR by materializing the logical result with
    `ORR`/`EOR` and then using `ANDS tmp,tmp,tmp` to publish N/Z while clearing
    C/V. Decoder Group 1 and register-register OR/XOR now emit persistent
    RFLAGS publication with PF and cleared CF/OF, while AF remains undefined.
  - W2-12 C++ INC/DEC RFLAGS slice: C++ Group 5 INC/DEC register forms now
    preserve CF while publishing ZF/SF/OF plus synthesized PF/AF through
    `StoreRflagsFromNzcv{Preserve,...}` and keep the transient `AluFlags`
    bridge for immediate same-block flag consumers.
  - W2-02 first ADC/SBB adversarial slice: interpreter edge table now includes
    signed overflow with carry-in, unsigned carry without signed overflow,
    signed overflow with borrow-in, no-borrow stale flag clearing, and
    borrow-to-zero ZF/PF behavior.
  - W2-04 first Group 3 MUL/IMUL width slice: one-operand byte multiply now
    writes AX only, word/dword multiply stores DX:AX / EDX:EAX high-low halves
    from an extended product, and `exec_muldiv` covers byte/word/dword plus
    existing 64-bit paths.
  - W2-04 DIV/IDIV width slice: Rust byte division now uses AX and stores
    AH:AL, word/dword division now uses DX:AX / EDX:EAX as the dividend and
    stores quotient/remainder halves, with decoder, interpreter, and
    `exec_muldiv` focal coverage. Rust and C++ backends now guard scalar
    div/mod and `WideDiv` divisor-zero before ARM64 `UDIV`/`SDIV`, guard
    unsigned qword `WideDiv` overflow when the high dividend half is at least
    the divisor, and guard signed qword `WideDiv` overflow before applying the
    result sign. Narrow quotient overflow and real guest exception delivery
    still need the next #DE slice.
  - W2-04 C++ Group 3 width slice: C++ decoder now routes `F6` byte Group 3,
    emits widened narrow MUL/IMUL products into AX or DX:AX/EDX:EAX, and emits
    narrow DIV/IDIV using AX, DX:AX, or EDX:EAX. Core smoke now confirms live
    C++ and Rust translators both accept the narrow MUL/DIV fixtures. C++ DCE
    and copy-prop now preserve/rewrite `Extend` and `Truncate` operand refs,
    which was required for the new narrowed IR to lower.
  - W2-05 first interpreter/backend-coverage slice: decoded register Group 2
    `ROL`/`ROR`/`SAR` cases execute through the translator interpreter, and the
    interpreter now supports `Sar`, `Rol`, `Ror`, `UMulHi`, `SMulHi`, `SDiv`,
    `SMod`, `Extend`, and `Truncate` ops emitted by the Rust decoder.
  - W2-05 decoder/runtime coverage slice: decoder tests now matrix
    `ROL`/`ROR`/`SHL`/`SHR`/`SAR` across register and memory destinations with
    count-one, imm8, and CL sources; runtime smoke now includes non-carry Group
    2 memory forms for backend translation.
  - W2-07 VEX parser prep slice: C5/C4 VEX parsing now exposes REX-like
    `R/X/B/W`, correct `vvvv`, mandatory-prefix `pp`, vector-length `L`, and
    map-select fields with unit coverage; VEX parsing now also stops before
    prefix-like opcode bytes such as `F2`/`F3`; BMI1 VEX dispatch is now
    covered by the implementation and C++ acceptance slices below.
  - W2-07 BMI1 implementation slice: VEX 0F38 dispatch now decodes `ANDN`,
    `BEXTR`, `BLSR`, `BLSMSK`, and `BLSI` for register and memory sources,
    publishes defined core flags through persistent CF/RFLAGS bits, executes in
    the translator interpreter, and reaches backend smoke translation.
  - W2-07 CPUID slice: Rust backend leaf 7 EBX now advertises BMI1 bit 3
    together with the existing BMI2 bit 8, with a lowerer unit test pinning the
    baked feature mask.
  - W2-07 C++ acceptance slice: C++ decoder now accepts VEX BMI1 `ANDN`,
    `BEXTR`, `BLSR`, `BLSMSK`, and `BLSI` dataflow, rejects invalid BLS group
    subops, and the core/shell smoke matrix confirms live C++ and Rust
    translators both accept the BMI1 fixtures.
  - W2-07 C++ BMI1 persistent-RFLAGS slice: C++ `ANDN`, `BLSR`, `BLSMSK`,
    `BLSI`, and `BEXTR` now publish explicit CF plus ZF/SF/OF through
    `StoreCarry` and `StoreRflagsFromBits`, mirroring the Rust decoder model.
    PF/AF remain undefined; BEXTR preserves SF.
  - W2-12 C++ BSF/BSR partial-RFLAGS slice: C++ BSF/BSR now keep the existing
    transient `CmpFlags` + `Select` flow for same-block ZF and destination
    preservation, then publish persistent ZF while preserving SF/OF/CF and
    leaving PF/AF untouched.
  - W1-08/W2-12 C++ bit-test CF slice: C++ BT/BTS/BTR/BTC immediate forms now
    compute `oldbit != 0`, store it through `StoreCarry` so persistent CF and
    RFLAGS bit 0 are synced, and keep `CmpFlags(bit, 1)` for immediate
    carry-conditional consumers. Register and memory forms are covered; memory
    immediates fold the high immediate bits into the qword address offset.
  - W1-08 Rust bit-test immediate-offset slice: Rust BT/BTS/BTR/BTC immediate
    forms now mask the bit index with `imm8 & 63` and fold memory high bits as
    `(imm8 / 64) * 8` into the qword address before load/store, matching the
    C++ decoder model.
  - W2-01/W2-05 C++ RCL/RCR carry slice: C++ RCL/RCR register-direct forms
    for imm8 and CL counts now expand through the persistent carry slot,
    preserving count-zero destination/CF behavior and writing the final CF with
    `StoreCarry`; `D1 /2,/3` register and memory count-one decoding is now
    covered too.
  - W2-05 C++ D1 non-carry shift/rotate flags slice: `ROL`, `ROR`, `SHL`,
    `SHR`, and `SAR` register and memory count-one forms now publish
    persistent CF and OF; shifts also publish ZF/SF/PF while preserving
    undefined AF, and rotates preserve undefined ZF/SF/PF/AF.
  - W2-05 C++ C1/D3 Group 2 flags slice: C++ `ROL`, `ROR`, `SHL`, `SHR`,
    and `SAR` imm8 and CL register/memory forms now mask the count, preserve
    destination and flags for count zero, publish persistent CF plus
    `StoreRflagsFromBits`, preserve undefined rotate ZF/SF/PF/AF, and preserve
    undefined OF except for count-one cases. C++ RCL/RCR memory forms for imm8
    and CL now also route through the persistent carry-slot expansion.
  - W2-12 C++ Group 3 NOT/NEG slice: `NOT` register/memory forms now decode as
    flag-free bitwise inversion, while `NEG` register/memory forms publish
    persistent CF/ZF/SF/OF/PF/AF through the existing SUB/CMP RFLAGS path.
  - W2-04 C++ IMUL memory-source slice: two-operand `0F AF /r` and
    immediate `69 /r` / `6B /r` IMUL forms now accept memory `r/m` sources
    in addition to register sources, with tests covering `[rax]` sources.
  - W2-04/W2-12 C++ MUL/IMUL flags slice: one-operand unsigned `MUL` and
    signed `IMUL`, plus two/three-operand signed `IMUL`, now publish x86 CF/OF
    through persistent carry/RFLAGS bits. Undefined PF/AF/ZF/SF are preserved.
    One-operand `MUL`/`IMUL` also accepts memory `r/m` sources.
  - W2-04 C++ DIV/IDIV memory-divisor slice: byte/word/dword/qword `DIV` and
    `IDIV` now accept memory divisors and keep the existing architectural
    dividend selection for AX, DX:AX, EDX:EAX, or explicit RDX:RAX `WideDiv`.
  - W2-04 Rust WideDiv slice: Rust IR now has `WideDiv{high,low,divisor}`
    with quotient/remainder results; Rust `DIV/IDIV r/m64` emits RDX:RAX
    wide-division IR instead of the old RAX-only `UDiv/UMod` pair, the
    reference interpreter executes unsigned and signed 128/64 cases, and the
    ARM64 backend lowers `WideDiv` through a 64-step long-division sequence for
    valid non-trapping qword cases. `TrapIf(Sigfpe)` now represents
    divisor-zero and narrow quotient-overflow checks; first-class guest `#DE`
    delivery is still pending.
  - W2-04 C++ WideDiv slice: C++ IR now has `WideDiv{high,low,divisor}` with
    quotient/remainder results, C++ `DIV/IDIV r/m64` emits RDX:RAX WideDiv IR
    instead of the old RAX-only `UDiv/UMod`/`SDiv/SMod` pair, serialization,
    validation, profiler, passes, and decoder tests cover it, and the ARM64
    backend lowers valid non-trapping qword cases through the same 64-step
    long-division shape as Rust. `TrapIf(Sigfpe)` now covers decoder-side
    divisor guards and is wired through legacy + modern serialization; guest
    `#DE` delivery still needs the runtime signal path.
  - W2-06 Rust PCMPxSTRx skeleton/flags slice: Rust IR now has
    `PcmpStrIndex`, `PcmpStrMask`, and packed `PcmpStrFlags`; legacy
    `66 0F 3A 60..63 /r imm8` decodes `PCMPESTRM/I` and `PCMPISTRM/I`
    register/memory forms into XMM0/ECX destinations and publishes
    CF/ZF/SF/OF through `StoreCarry` + `StoreRflagsFromBits` with PF/AF
    cleared. The reference interpreter executes byte/word equal-any, ranges,
    equal-each, and ordered comparisons for explicit and implicit lengths, and
    DCE/copy-prop understand the new operand refs. Tests now pin optional
    length `map_refs`, memory-source decode with extended XMM indices, strict
    legacy-prefix guards, and PCMP flag bits. C++ semantic backend parity and
    broader differential coverage remain pending.
  - W2-06 Rust backend XMM/PCMP semantic slice: Rust backend now carries
    `VecConstant`, `LoadVecReg`/`StoreVecReg`, and `LoadVec`/`StoreVec` as
    scalar low/high `u64` pairs in callee-saved host registers, uses the
    C++-compatible XMM state offset (`XMM0` at byte 144), and rebases 128-bit
    memory vector accesses through `mem_base`. `PcmpStrIndex`, `PcmpStrMask`,
    and `PcmpStrFlags` now lower through semantic helpers covering byte/word
    lanes, signed/unsigned equal-any, ranges, equal-each, equal-ordered,
    polarity, explicit lengths, implicit NUL lengths, compact masks, expanded
    masks, and packed CF/ZF/SF/OF publication. Helper calls preserve LR plus
    caller-saved value registers before `BLR`. Rust CPUID leaf 1 ECX now
    advertises SSE4.2 bit 20 so PCMPxSTRx is discoverable.
  - W2-06 C++ PCMPxSTRx IR/plumbing slice: C++ IR now has
    `PcmpStrIndex`, `PcmpStrMask`, and `PcmpStrFlags` with optional explicit
    length refs, structural equality, pretty-printing, profiler coverage,
    validator operand tracking, copy-prop rewrites, DCE/LICM pure-op handling,
    backend ref liveness, explicit backend unsupported boundaries, and legacy
    plus modern serializer payload support.
  - W2-06 C++ PCMPxSTRx decoder slice: legacy `66 0F 3A 60..63 /r imm8`
    now decodes `PCMPESTRM/I` and `PCMPISTRM/I` register/memory forms into
    `PcmpStrMask`/`PcmpStrIndex`, reads EAX/EDX explicit lengths for ESTR
    forms, handles extended XMM register indexes through REX.R/REX.B, writes
    XMM0/ECX architectural destinations, emits `PcmpStrFlags`, stores CF
    through `StoreCarry`, and clears PF/AF while publishing ZF/SF/OF through
    `StoreRflagsFromBits`.
  - W2-06 C++ PCMPxSTRx backend helper slice: C++ emitter now has stack
    push/pop for full Q registers so helper calls can preserve live Vec128 SSA
    refs. The lowerer now evaluates `PcmpStrIndex`, `PcmpStrMask`, and
    `PcmpStrFlags` through semantic helpers matching the Rust backend model
    for byte/word lanes, signed/unsigned equal-any, ranges, equal-each,
    equal-ordered, polarity, explicit lengths, implicit NUL lengths, compact
    masks, expanded masks, and packed CF/ZF/SF/OF publication. Helper-call
    lowering saves live GPR scratch refs and live FP scratch refs, passes XMM
    values as low/high `u64` arguments, and reconstructs mask results back into
    Vec128. C++ translator now advertises CPUID.1:ECX.SSE4.2 bit 20 after
    PCMPxSTRx decode/backend support; ARM64 runtime/differential execution
    coverage remains pending.
  - W2-07 C++ CPUID slice: C++ translator leaf 7 EBX now advertises BMI1 bit 3
    together with BMI2 bit 8 after the BMI1 decoder acceptance slice; AVX2
    remains deliberately clear.
  - W2-08 PCLMULQDQ slice: Rust IR/decoder/interpreter/passes/backend now
    model `VecClMul` for legacy `66 0F 3A 44 /r ib`, select source qword
    lanes from imm8 bits, execute carry-less 64x64-to-128 multiplication in
    the interpreter, and lower it through a scalar ARM64 loop. C++ IR,
    validation, serializers, profiler, pretty-printer, optimizer passes,
    legacy PCLMULQDQ decode, VEX.128 VPCLMULQDQ decode, CPUID.1:ECX bit 1,
    and backend lowering are wired; the C++ emitter uses ARM64 `PMULL` for the
    selected qword lanes. Broader runtime/differential execution coverage
    remains pending.
  - `flag_write_elim` pins flag writers for `ReadFlag`, `ReadCarryOut`,
    `CondJumpFlags`, `StoreRflagsFromNzcv`, and explicit RFLAGS publishers.
- Still open before claiming full flags/RFLAGS correctness:
  - Full C++ persistent RFLAGS parity is narrowed but not complete. C++ now has
    `cf`/partial-`rflags` storage, `LoadCarry`/`StoreCarry`,
    `LoadRflags`/`StoreRflags`, `StoreRflagsFromNzcv`,
    `StoreRflagsFromBits`, ADC/SBB CF dataflow, partial PUSHFQ/POPFQ/LAHF/SAHF,
    and persistent publication for
    ADD/SUB/AND/OR/XOR/CMP/TEST/ADC/SBB/INC/DEC/XADD/CMPS/SCAS, BSF/BSR,
    BT/BTS/BTR/BTC register/memory immediate forms, NOT/NEG, MUL/IMUL CF/OF, RCL/RCR register/memory count forms, Group 2
    `ROL`/`ROR`/`SHL`/`SHR`/`SAR` D1/C1/D3 register/memory forms, plus BMI1
    ANDN/BEXTR/BLSR/BLSMSK/BLSI. Cross-block non-CF observation and broader
    runtime/differential coverage still need coverage before claiming full
    parity.
  - ADC/SBB adversarial/differential edge corpus for carry-in, borrow-in,
    aux-carry, parity, signed overflow, zero, sign, and block-boundary
    observation.
  - Remaining Group 2 runtime/differential proof for count-zero, count-one,
    multi-count, memory, and undefined-flag preservation cases.
  - Adversarial differential corpus for carry, borrow, overflow, zero, sign,
    INC/DEC CF preservation, and wider LAHF/SAHF compositions.
  - W2-03 still needs true multi-threaded behavior/stress tests before claiming
    the entire atomic-CAS wave complete across Rust and C++.

## Execution Rules

- One wave can be split into multiple commits, but do not mix waves in one
  commit unless the test boundary requires it.
- If an item changes IR, update decoder, backend, interpreter, passes, runtime
  smoke, C++ differential, docs, and tests in the same slice.
- Do not claim "done" for a semantic feature unless there is at least one
  behavior test and one differential or cross-crate compile gate.
- Any mmap, executable memory, file descriptor, cache file, thread frame, or
  waiter must have deterministic cleanup and a test or sanitizer gate.
- Prefer code first, but leave a validation breadcrumb after each wave: exact
  command attempted, pass/fail, and blocker if interrupted.

## Wave 0 - Close The Live RFLAGS WIP

| ID | Task | Files | Acceptance |
|----|------|-------|------------|
| W0-01 | Finish compiling the current RFLAGS skeleton | `shell/prisma-{ir,backend,decoder,passes,runtime,translator}` | `cargo fmt --check`; decoder/backend/passes/runtime/translator unit tests compile |
| W0-02 | Verify `CpuStateFrame` offset migration after adding `rflags` | `shell/prisma-runtime/src/executor.rs`, `shell/prisma-backend/src/lowerer.rs`, `shell/prisma-runtime/src/dispatcher.rs` | DONE locally: layout test pins `cf=808`, `rflags=816`, `exit_reason=824`, `next_pc=832`, `mem_base=840` |
| W0-03 | Add focused lowerer tests for `LoadRflags`, `StoreRflags`, and `StoreCarry` syncing bit 0 | `shell/prisma-backend/src/lowerer.rs` | byte fixtures prove reserved bit 1 is forced and CF mirrors RFLAGS bit 0 |
| W0-04 | Add interpreter tests for `LoadRflags`/`StoreRflags` | `shell/prisma-translator/src/interp.rs` | interpreter preserves bit 1 and syncs CF |
| W0-05 | Harden `PUSHFQ`/`POPFQ` decoder tests around reserved bit 1 and CF restore | `shell/prisma-decoder/src/decode.rs` | tests prove `PUSHFQ` pushes `rflags | 2` and `POPFQ` emits `StoreRflags` before RSP advance |
| W0-06 | Refresh Rust and C++ smoke fixtures for the changed backend bytes | `shell/prisma-runtime/tests/smoke_differential.rs`, `shell/core/tests/smoke_differential.rs` | Runtime smoke green; core smoke source adjusted but local link waits on `PRISMA_CORE_LIB_DIR` |
| W0-07 | Document that current RFLAGS is partial, not full flags correctness | `docs/WORK_QUEUE.md` or this file | doc explicitly says ZF/SF/OF/PF/AF persistence is still pending |
| W0-08 | Split the live WIP into reviewable commits | git history | first commit is minimal RFLAGS spine; later commits widen decoder smoke |

## Wave 1 - Make Persistent Flags Semantically Honest

| ID | Task | Files | Acceptance |
|----|------|-------|------------|
| W1-01 | Design explicit IR for publishing NZCV/x86 flag state into persistent RFLAGS | `shell/prisma-ir/src/lib.rs`, docs RFC or code comments | optimizer sees the side effect explicitly; no hidden lowerer-only flag persistence |
| W1-02 | Add pass plumbing for the new persistent-flag publish op | `shell/prisma-passes/src/*.rs` | DCE/copy-prop/CSE/global-CSE/branch-fold handle it exhaustively |
| W1-03 | Fix `flag_write_elim` so cross-block persistent flags are not dropped | `shell/prisma-passes/src/flag_write_elim.rs` | a flag writer needed by later `PUSHFQ` or branch survives |
| W1-04 | Persist CF correctly for Add/Sub/Cmp families | decoder/backend/passes | Add uses ARM C, Sub/Cmp uses inverted ARM C where x86 borrow semantics require it |
| W1-05 | Persist ZF/SF/OF from arithmetic flag writers | backend/interpreter/tests | `PUSHFQ` after ADD/SUB/CMP exposes correct ZF/SF/OF |
| W1-06 | Synthesize PF and AF only when observable | backend/passes/tests | DONE locally for ADC/SBB and NZCV-backed ADD/SUB/CMP/TEST/logical publishers; decoder fixtures refreshed and green |
| W1-07 | Restore transient NZCV from persistent flags before cross-block condition consumers | backend/runtime/translator | DONE locally for block-entry Jcc/Select lowering and interpreter Jcc; runtime smoke covers standalone Jcc restore prefix and cmp+jcc local flags |
| W1-08 | Convert bit-test family to explicit CF storage instead of accidental `CmpFlags` behavior | `shell/prisma-decoder/src/decode.rs`, `core/src/decoder/x86_decoder.cpp` | DONE locally for Rust and C++ decoder parity: register/memory immediate BT/BTS/BTR/BTC store old-bit CF through `StoreCarry`, mask bit indexes, and fold memory qword offsets; broader adversarial runtime/differential edge coverage remains tracked under W1-10/W3-02 |
| W1-09 | Add `LAHF`/`SAHF` once the RFLAGS subset is trustworthy | decoder/backend/interpreter/tests | DONE locally: decoder exact forms and prefix guards, backend lowering gates, and interpreter byte tests prove AH/RFLAGS SF/ZF/AF/PF/CF behavior with OF preservation |
| W1-10 | Add a flags adversarial corpus | runtime/core smoke tests | IN PROGRESS locally: interpreter edge table, Rust runtime LAHF/SAHF smoke, and C++ core LAHF/SAHF acceptance smoke are green; full C++ persistent-RFLAGS semantics still pending |

## Wave 2 - Decoder And Backend ISA Grind

| ID | Task | Files | Acceptance |
|----|------|-------|------------|
| W2-01 | RCL/RCR by CL, not just count 1 | `shell/prisma-decoder`, `shell/prisma-backend`, `core` parity tests | IN PROGRESS locally: decoder/runtime fixtures existed; interpreter now executes CL/count=0 cases; C++ register and memory imm8, CL, and D1 count-one forms now read/write persistent CF; broader runtime parity still pending |
| W2-02 | Add adversarial ADC/SBB differential edge coverage | decoder/backend/runtime/core smoke tests | IN PROGRESS locally: interpreter edge table covers carry-in, borrow-in, PF/AF/OF/ZF/SF, stale flag clearing, and zero/sign/overflow cases; C++ now has `cf`/partial-`rflags`, `LoadCarry`/`StoreCarry`, ADC/SBB CF dataflow for register/memory destinations, and explicit persistent PF/AF/ZF/SF/OF publication; runtime/core differential state coverage is still pending |
| W2-03 | Implement true atomic CAS lowering for CMPXCHG/CMPXCHG8B/CMPXCHG16B | backend/runtime/tests | IN PROGRESS locally: Rust and C++ memory `CMPXCHG`/`CMPXCHG8B` emit `AtomicCmpxchg`; Rust and C++ `CMPXCHG16B` emit `AtomicCmpxchgPair`; Rust/C++ backends lower scalar CAS to `LDAXR*`/`STLXR*` and pair CAS to `LDAXP`/`STLXP`; MT behavior/stress tests still pending |
| W2-04 | Replace one-operand MUL/IMUL/DIV/IDIV placeholder math with full-width behavior where required | `core/src/decoder`, `shell/prisma-decoder`, backend | IN PROGRESS locally: Rust and C++ now fix narrow I8/I16/I32 Group 3 MUL/IMUL/DIV/IDIV dataflow, including AX, DX:AX, and EDX:EAX architectural halves; Rust and C++ qword DIV/IDIV emit explicit RDX:RAX `WideDiv` IR and ARM64-lower valid non-trapping qword cases; C++ MUL/IMUL/DIV/IDIV accept register and memory r/m operands where applicable; C++ MUL/IMUL publish CF/OF while preserving undefined flags; Rust/C++ backends guard scalar div/mod divisor-zero plus qword `WideDiv` quotient overflow; Rust/C++ `TrapIf(Sigfpe)` now models decoder-side divisor-zero and narrow quotient-overflow checks; remaining W2-04 risk is first-class guest #DE delivery plus broader runtime/differential edge coverage |
| W2-05 | Finish Group 2 memory and count forms across widths | decoder/backend/tests | IN PROGRESS locally: decoder matrix covers non-carry `ROL`/`ROR`/`SHL`/`SHR`/`SAR` register/memory forms with count-one, imm8, and CL sources; C++ RCL/RCR register/memory forms now use persistent CF; C++ D1/C1/D3 register/memory non-carry forms now publish persistent CF/OF and defined result flags while preserving count-zero and undefined flag bits; runtime smoke covers non-carry memory forms; broader runtime/differential proof still pending |
| W2-06 | PCMPxSTRx SSE4.2 string ops | decoder/backend/tests/CPUID | IN PROGRESS locally: Rust IR/decoder/interpreter/backend now cover legacy `PCMPESTRM/I` and `PCMPISTRM/I` data operands/results for selected byte/word modes, publish CF/ZF/SF/OF with PF/AF cleared, carry XMM/vector memory in the Rust backend, lower PCMP semantics through helpers, and advertise SSE4.2 in Rust CPUID; C++ IR/plumbing now represents the same PcmpStr ops with optimizer/serialization/profiler coverage; C++ decode now accepts register/memory PCMPxSTRx, explicit EAX/EDX lengths, XMM0/ECX result destinations, and flag publication; C++ backend now lowers PcmpStr index/mask/flags through semantic helpers with live GPR/FP scratch preservation; C++ CPUID SSE4.2 advertisement is now enabled; ARM64 runtime/differential execution coverage remains pending before this can be claimed honest |
| W2-07 | BMI1 batch: ANDN/BEXTR/BLSI/BLSMSK/BLSR | decoder/backend/tests/CPUID | IN PROGRESS locally: Rust VEX 0F38 decodes `ANDN`, `BEXTR`, `BLSR`, `BLSMSK`, and `BLSI` for register/memory sources with interpreter semantics, runtime smoke, and Rust CPUID leaf 7 BMI1 advertisement; C++ decoder/core smoke accepts the same BMI1 dataflow, C++ CPUID leaf 7 advertises BMI1, and C++ now publishes the supported persistent RFLAGS subset for those BMI1 forms. Remaining W2-07 risk is broader differential/runtime coverage, not basic decode/flag publication |
| W2-08 | PCLMULQDQ via ARM64 PMULL | decoder/backend/tests/CPUID | IN PROGRESS locally: Rust IR/decoder/interpreter/passes/backend now support legacy `PCLMULQDQ` with imm8 lane selection, carry-less 64x64-to-128 semantics, scalar ARM64 backend lowering, and CPUID PCLMUL advertisement; C++ IR/plumbing/serializers/profiler/pretty-printer/passes decode legacy `PCLMULQDQ` and VEX.128 `VPCLMULQDQ`, lower through ARM64 `PMULL`, and advertise CPUID.1:ECX.PCLMULQDQ. Focal Rust and C++ tests are green; broader runtime/differential execution coverage remains pending |
| W2-09 | F16C conversions | decoder/backend/tests | IN PROGRESS locally: Rust IR/decoder/interpreter/backend/CPUID cover `VCVTPH2PS`/`VCVTPS2PH` with software rounding helpers; C++ now mirrors the slice end to end — `VecF16Cvt` IR with equality/pretty-print/validator/profiler, legacy (tag 108) and modern (tag 46) serializers, DCE/copy-prop/LICM plumbing, VEX.128.66.0F38.W0 13 and VEX.128.66.0F3A.W0 1D decode with L=1/W=1 rejection, bit-exact software conversion helpers called from the ARM64 lowerer, and CPUID.1:ECX.F16C bit 29; Rust runtime smoke gained four F16C fixtures; ARM64 runtime/differential execution coverage remains pending |
| W2-10 | AVX2 integer thin spots | decoder/backend/tests | VPBROADCAST, VINSERTI128/VEXTRACTI128, variable vector shifts |
| W2-11 | x87 80-bit helper path | backend/runtime/tests | opt-in precision path exists for cases reduced-F64 cannot handle |
| W2-12 | Audit and close remaining `placeholder` comments in C++ decoder | `core/src/decoder/x86_decoder.cpp` | IN PROGRESS locally: carry-slot, partial-RFLAGS, and supported persistent-RFLAGS publication landed; ADC/SBB, PUSHFQ/POPFQ, LAHF, SAHF, ADD/SUB/AND/OR/XOR/CMP/TEST, INC/DEC, BSF/BSR, BT/BTS/BTR/BTC register/memory immediate forms, NOT/NEG, MUL/IMUL CF/OF, XADD, CMPS, and SCAS no longer rely on placeholder flag publication; each remaining decoder placeholder must still be implemented, documented as deferred, or tested as intentional |

## Wave 3 - Rust Parity And Cutover

| ID | Task | Files | Acceptance |
|----|------|-------|------------|
| W3-01 | Rust IR parity against C++ IR and Lean syntax | `shell/prisma-ir`, `ir-spec` | automated check fails on enum/layout drift |
| W3-02 | Rust decoder differential family matrix | `shell/prisma-decoder`, `shell/core/tests` | every newly supported family has C++/Rust fixture parity |
| W3-03 | Rust backend coverage for every op emitted by Rust decoder | `shell/prisma-backend` | corpus has zero unexpected `UnsupportedOp` |
| W3-04 | Rust runtime dispatcher loop with real multi-block execution | `shell/prisma-runtime` | ARM64 e2e executes multiple blocks through dispatcher |
| W3-05 | Rust direct chaining/RAS parity with C++ runtime | runtime/backend tests | direct call/ret path stats and behavior match C++ expectations |
| W3-06 | Rust syscall handler real effects instead of typed stub | `shell/prisma-runtime` | mirrors the supported C++ syscall subset |
| W3-07 | First C++ pipeline cutover to Rust through C ABI | `core`, `shell` | one stage runs Rust implementation under C++ suite with no regression |
| W3-08 | Consolidate validation script into default Rust gate | `scripts/validate-rust-workspace.ps1` | fmt, test, clippy, bridge path are one reliable command |

## Wave 4 - Linux User-Mode Runtime

| ID | Task | Files | Acceptance |
|----|------|-------|------------|
| W4-01 | `futex` WAIT/WAKE baseline | `core/src/runtime/syscalls*`, Rust runtime mirror | two guest threads synchronize without busy-spin |
| W4-02 | `futex` REQUEUE and robust futex subset | syscall/runtime tests | pthread-style patterns have coverage |
| W4-03 | `clone` guest thread creation | dispatcher/runtime/thread files | per-thread `CpuStateFrame`, TLS, stack ownership, join cleanup |
| W4-04 | `execve` translator re-entry | runtime/loader/translator | guest execs another x86 ELF and continues translating |
| W4-05 | Guest signal delivery | signal runtime | guest SIGSEGV/#UD/#DE reaches guest handler and can resume |
| W4-06 | Socket syscall family | syscall runtime | loopback echo guest-to-host and host-to-guest |
| W4-07 | `termios` completion behind current winsize partial | guest structs/syscall tests | ioctl/isatty cases match Linux behavior |
| W4-08 | Coreutils harness | `tools/coreutils`, docs | reports pass percentage against native/QEMU baseline |
| W4-09 | Syscall fuzz harness | fuzz/runtime | time-boxed fuzz finds no crash or records minimized repro |

## Wave 5 - Guest MT, TSO, And Memory Model

| ID | Task | Files | Acceptance |
|----|------|-------|------------|
| W5-01 | Real atomic lowering for LOCK/CMPXCHG/XCHG families | backend/runtime | LDAXR/STLXR or CASAL path under guest MT |
| W5-02 | Conservative vs relaxed TSO switch | runtime/backend/passes | measurable toggle with conservative default |
| W5-03 | Static TSO classifier | new pass + IR hints | classifies ST, lock-free, shared-mutable, IO, unknown |
| W5-04 | Rewrite TSO to plain where proven safe | passes/backend | unknown remains conservative |
| W5-05 | Multi-thread benchmark and regression suite | `tools/benchmarks/mt` | TSan and behavior gates green |
| W5-06 | Lean weak-memory skeleton | `ir-spec/PrismaIR` | model builds and future TSO lemmas have a home |
| W5-07 | Runtime debug assertions tied to classifier invariants | runtime/passes | debug mode catches invalid relaxed classifications |

## Wave 6 - Windows And Wine Bring-Up

| ID | Task | Files | Acceptance |
|----|------|-------|------------|
| W6-01 | Finish PE loader import binding | `shell/orchestrator/pe_loader.rs` | named and ordinal imports resolve through provider |
| W6-02 | PE TLS callbacks | PE loader tests | TLS callbacks execute in correct order |
| W6-03 | DLL forwarders | PE loader/provider | forwarded imports resolve |
| W6-04 | Wine ARM64 submodule/build glue | `third_party/wine`, build scripts | Wine builds reproducibly in expected config |
| W6-05 | `wow64cpu.dll` BTCpu stub | Wine bridge | `pBTCpuSimulate` can call Prisma dispatcher |
| W6-06 | NT syscall subset for Notepad XP | runtime/shell | documented minimal subset passes integration |
| W6-07 | Container `start`/`stop` lifecycle | `shell/orchestrator/container.rs` | create/list/start/stop/destroy with cleanup |
| W6-08 | Overlay filesystem for containers | `shell/orchestrator/fs*` | guest writes land in overlay, base remains read-only |
| W6-09 | Notepad XP end-to-end milestone | integration docs/tests | Notepad starts under Android/hardware path when dependencies exist |

## Wave 7 - Android Shell And Product Surface

| ID | Task | Files | Acceptance |
|----|------|-------|------------|
| W7-01 | Android Gradle project skeleton | `android/` | buildable APK shell |
| W7-02 | JNI bridge to Rust shell | `android`, `shell` | app can call create/list/start APIs |
| W7-03 | Container list/import/run screens | Android Compose | import `.exe`, run, and view logs |
| W7-04 | Settings and performance knobs | Android Compose/runtime | toggles map to real config |
| W7-05 | File access permissions and SAF import | Android | sample executable can be imported |
| W7-06 | X11/ANativeWindow/Vulkan surface path | Android/shell | GUI app can render a window surface |
| W7-07 | Input bridge | Android/shell | touch/mouse/gamepad events reach guest |
| W7-08 | APK signing and release pipeline | CI/Android | reproducible signed artifact |

## Wave 8 - Distributed Cache, NPU, AVF, Graphics

| ID | Task | Files | Acceptance |
|----|------|-------|------------|
| W8-01 | SHA-256 cache envelope and persistence hardening | cache/runtime | corrupt cache cannot allocate attacker-sized maps |
| W8-02 | Cache server with signed entries | `server/` | POST/GET by hash, Ed25519 signature |
| W8-03 | Client signature verification before mmap+exec | shell/runtime | unsigned or wrong-SoC entry is rejected |
| W8-04 | libp2p cache exchange | `server`, `shell` | two peers exchange same-SoC cache entry |
| W8-05 | NPU data capture and feature extraction | `tools/npu` | reproducible dataset with opcode/branch/memory features |
| W8-06 | Hot-path classifier prototype | `tools/npu`, core hook | offline model and CPU fallback path |
| W8-07 | AVF compatibility detection and VM launch spike | Android/shell | Pixel/Tensor path detects capability and launches minimal guest |
| W8-08 | Shader graph analyzer | `tools/graphics` | 20 hot-loop patterns documented |
| W8-09 | Benchmark harness against QEMU/Box64/FEX/native | `tools/benchmarks` | honest table and decision point if perf is below target |

## Wave 9 - CI, Fuzzing, Hardening, Docs

| ID | Task | Files | Acceptance |
|----|------|-------|------------|
| W9-01 | Differential corpus expansion policy | docs/tests | every opcode family includes Rust/C++ parity fixture |
| W9-02 | Nightly fuzz runs for decoder/cache/passes/backend/syscalls | CI/fuzz | time-boxed jobs with minimized repro artifacts |
| W9-03 | ASan/UBSan/TSan gates | CI/core/runtime | sanitizer jobs are stable enough to trust |
| W9-04 | Windows CTest granular discovery follow-up | `core/CMakeLists.txt` | ASCII smoke remains, granular reporting works if needed |
| W9-05 | RAII sweep for restart-sensitive resources | core/shell/server | no `mem::forget` or ownerless fd/mmap/cache file without RFC |
| W9-06 | Security review of host-guest boundary | docs/REVIEWS | findings converted to tests/fixes |
| W9-07 | Public docs glossary and architecture refresh | docs | terms and diagrams match current code |
| W9-08 | Benchmark report generation | tools/docs | Markdown/LaTeX table generation is reproducible |

## First 20 Tasks To Pull

1. W0-02 - verify runtime/backend offsets.
2. W0-06 - refresh Rust and C++ smoke fixtures around persistent RFLAGS bytes.
3. W0-08 - split the live RFLAGS WIP into reviewable commits.
4. W1-10 - add a flags adversarial corpus.
5. W2-01 - RCL/RCR by CL.
6. W2-02 - ADC/SBB adversarial differential edge coverage.
7. W2-03 - true atomic CAS lowering for CMPXCHG/CMPXCHG8B/CMPXCHG16B.
8. W2-04 - full-width one-operand MUL/IMUL/DIV/IDIV semantics.
9. W2-05 - Group 2 memory/count forms across widths.
10. W2-06 - PCMPxSTRx SSE4.2 string ops.
11. W2-07 - BMI1 batch: ANDN/BEXTR/BLSI/BLSMSK/BLSR.
12. W3-02 - Rust decoder differential matrix expansion.
13. W3-03 - Rust backend no-UnsupportedOp corpus gate.
14. W3-04 - Rust runtime dispatcher loop with real multi-block execution.
15. W4-01 - futex WAIT/WAKE.
16. W4-03 - clone guest thread creation.
17. W4-08 - coreutils harness.
18. W5-01 - real atomic CAS lowering.
19. W6-01 - PE import binding.
20. W6-02 - PE TLS callbacks.

## Notes For Parallel Agents

- Codex-safe lanes right now: W0, W1, W2, W3-02, W3-03, W9-01.
- Claude-safe lanes right now: W4, W5, W6, W8, W9.
- Avoid simultaneous edits to `shell/prisma-decoder/src/decode.rs`; queue those
  decoder waves serially.
- Runtime/syscall work can proceed in parallel with Rust decoder/backend if it
  avoids the current RFLAGS files.
