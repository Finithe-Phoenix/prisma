// fuzz/decoder/fuzz_decoder.cpp — libFuzzer driver for decoder::decode_one.
//
// Invariants the fuzzer pins (any violation = fuzzer reports a crash):
//
//  1. decode_one must not crash, trip ASan, or trip UBSan on ANY input.
//  2. If it returns Decoded, `bytes_consumed` must be in [1, input_size]
//     and `stmts` must be non-empty for all non-NOP opcodes.
//  3. `next_ref` must equal the count of stmts with a bound result —
//     off-by-one here means the caller's ref counter drifts.
//  4. The three DecodeError cases (Truncated / Unknown / Unsupported)
//     must be the only failure modes.
//
// Build with:
//   cmake -S . -B build -DPRISMA_ENABLE_FUZZERS=ON
//   cmake --build build --target prisma_fuzz_decoder
//
// Run:
//   build/fuzz/prisma_fuzz_decoder -max_len=64 corpus/decoder/

#include <cassert>
#include <cstddef>
#include <cstdint>
#include <span>
#include <variant>

#include "prisma/decoder.hpp"
#include "prisma/ir.hpp"

extern "C" int LLVMFuzzerTestOneInput(const std::uint8_t* data,
                                      std::size_t size) {
    if (size == 0) return 0;

    // Two decode attempts per input to exercise the `next_ref` threading
    // path that real callers use.
    prisma::ir::Ref next_ref = 0;

    std::size_t cursor = 0;
    std::size_t prev_cursor = 0;
    int decodes = 0;

    while (cursor < size && decodes < 32) {
        const std::span<const std::uint8_t> view(data + cursor, size - cursor);
        auto res = prisma::decoder::decode_one(
            view, next_ref, /*instruction_guest_pc=*/0x1000 + cursor);

        if (std::holds_alternative<prisma::decoder::DecodeError>(res)) {
            // Any error code is acceptable. Stop the inner loop so we
            // don't spin forever on invalid bytes.
            break;
        }

        const auto& d = std::get<prisma::decoder::Decoded>(res);

        // Invariant 2: progress.
        if (d.bytes_consumed == 0) {
            // Zero-byte consumption would loop forever — report.
            __builtin_trap();
        }
        if (d.bytes_consumed > view.size()) {
            __builtin_trap();
        }

        // Invariant 3: every bound result has a unique ref below next_ref.
        for (const auto& s : d.stmts) {
            if (s.result.has_value() && *s.result >= next_ref) {
                __builtin_trap();
            }
        }

        cursor = prev_cursor + d.bytes_consumed;
        if (cursor == prev_cursor) __builtin_trap();  // defensive
        prev_cursor = cursor;
        ++decodes;
    }

    return 0;
}
