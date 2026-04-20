# Prisma fuzzing

Fuzzers for the x86 decoder, the IR pass pipeline, and (eventually) the
lowerer. All drivers read a raw byte buffer from either libFuzzer's
`LLVMFuzzerTestOneInput` interface or AFL++'s `__AFL_LOOP` wrapper,
feed the bytes into the target, and assert that:

1. No crash / UBSan / ASan trip.
2. Every path returns a defined result (`Decoded` or a documented
   `DecodeError` — never a panic or UB).
3. `next_ref` grows monotonically (decoder never emits with a ref
   below the caller's cursor).

## Targets

| Directory | Target | Tooling |
|---|---|---|
| `decoder/` | `decode_one` on random bytes | libFuzzer, AFL++ |
| `passes/`  | Pass pipeline on synthesized IR | libFuzzer |
| `lowerer/` | Lowerer on synthesized IR (post-pass) | libFuzzer |

Only `decoder/` exists today (F1-TC-004). The other two are
future-work tracks in the backlog (F1-TC-004 sub-items).

## Building

On macOS with Apple Clang 17 (ships libFuzzer + AddressSanitizer):

```
cmake -S . -B build -G Ninja \
      -DPRISMA_ENABLE_FUZZERS=ON \
      -DCMAKE_BUILD_TYPE=Debug \
      -DCMAKE_CXX_COMPILER=clang++
cmake --build build --target prisma_fuzz_decoder
```

The `prisma_fuzz_decoder` binary is a self-contained libFuzzer driver:

```
build/fuzz/prisma_fuzz_decoder \
    -max_len=64 \
    -artifact_prefix=artifacts/ \
    corpus/decoder/
```

## AFL++ mode

Build the same binary with `-fsanitize=fuzzer-no-link` and link
against `afl-cc` / `afl-clang++`:

```
AFL_USE_ASAN=1 afl-clang++ -fsanitize=fuzzer,address \
    -I core/include -I build/_deps/vixl_src-src/src \
    fuzz/decoder/fuzz_decoder_afl.cpp \
    build/libprisma_decoder.a build/libprisma_ir.a \
    -o build/fuzz/prisma_fuzz_decoder_afl

afl-fuzz -i corpus/decoder/ -o findings/ -- build/fuzz/prisma_fuzz_decoder_afl @@
```

## Corpus

`corpus/decoder/` seeds:
- Each known opcode as its shortest valid encoding.
- A handful of intentionally-invalid sequences (truncated REX, SIB
  with out-of-range fields) — help the fuzzer find edge paths faster.

## Nightly CI

Planned (see `BACKLOG.md::FX-CI-007`): a nightly GitHub Actions job
that runs every fuzz target for 15 minutes with ASan + UBSan and
uploads new crashes to an artifact bucket.
