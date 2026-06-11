// prisma/host_features.hpp — F1-RT-016 / F1-RT-017 ARM64 host features.
//
// A small, boolean-flag description of what the host ARM64 CPU supports.
// Populated once per process via `detect_host_features()`. Downstream
// emitter / runtime code consults the result to pick between legacy and
// modern encodings (e.g. load/store exclusive vs. LSE `cas`).
//
// Detection backends:
//   * macOS (Apple Silicon): `sysctlbyname("hw.optional.arm.FEAT_*")`.
//   * Linux: `getauxval(AT_HWCAP)` / HWCAP2 bits from <asm/hwcap.h>.
//   * Other hosts (Windows on ARM, BSDs, emulators) fall through to
//     "all features disabled" — safe default that forces the legacy
//     code paths to emit.
//
// The struct is intentionally value-typed and trivially copyable so it
// can be copied into worker threads without locking. The "detected once"
// process lifetime is captured by a singleton accessor in the .cpp.

#pragma once

namespace prisma::runtime {

struct HostFeatures {
    // FEAT_LSE — Large System Extensions (v8.1): `cas`, `casp`, `ldadd`,
    //            etc. Makes atomic RMW one instruction instead of
    //            LDAXR/STLXR loops.
    bool feat_lse{false};

    // FEAT_LSE2 — v8.4 relaxation of LSE's alignment requirement for
    //             128-bit unpaired load/store.
    bool feat_lse2{false};

    // FEAT_LRCPC / FEAT_LRCPC2 — Release-consistent processor
    //             consistency. Enables `ldapr` as a cheaper acquire
    //             load than `ldar`. LRCPC2 adds register-offset forms.
    bool feat_lrcpc{false};
    bool feat_lrcpc2{false};

    // FEAT_FlagM / FEAT_FlagM2 — direct manipulation of NZCV. Useful
    //             when lowering x86 flag-setting patterns.
    bool feat_flagm{false};
    bool feat_flagm2{false};

    // FEAT_DotProd — SDOT / UDOT. Relevant when future NEON paths land.
    bool feat_dotprod{false};

    // FEAT_CRC32 — hardware CRC32 (common since v8.0 in practice, but
    //             the bit exists to cover minimal cores).
    bool feat_crc32{false};

    // FEAT_SHA1 / FEAT_SHA256 — ARMv8 crypto extensions backing the
    //             SHA-NI lowering (F2-IR-060). Both are optional even
    //             on v8.0 cores (e.g. some Raspberry Pi SoCs lack
    //             them), so the guest CPUID SHA bit is advertised only
    //             when both are present.
    bool feat_sha1{false};
    bool feat_sha256{false};
};

// Returns a reference to the process-wide detected feature set. First
// call performs detection; subsequent calls return the cached result.
// Thread-safe under the usual static-local initialisation guarantee.
[[nodiscard]] const HostFeatures& host_features();

// For tests: force a specific feature set, bypassing detection. Useful
// for exercising both legacy and LSE code paths on a single host. The
// override persists until `clear_host_features_override()` is called.
// Not thread-safe — set during test setup on the main thread.
void override_host_features_for_test(HostFeatures f) noexcept;
void clear_host_features_override() noexcept;

}  // namespace prisma::runtime
