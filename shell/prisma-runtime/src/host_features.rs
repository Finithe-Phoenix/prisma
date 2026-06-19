//! Host ARM64 feature discovery.
//!
//! Mirrors C++ `prisma::runtime::host_features`. Models the ARM64 `FEAT_*`
//! capabilities the backend can opportunistically target. Detection uses
//! Linux `getauxval(AT_HWCAP/AT_HWCAP2)`; every other host (incl. the x86
//! Windows dev box) returns the all-false default, which is safe — the legacy
//! ARM64 encodings the emitter already produces work everywhere.

use std::sync::{Mutex, OnceLock};

/// ARM64 feature flags relevant to Prisma's emitter and lowering.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HostFeatures {
    /// FEAT_LSE — large-system atomics (CAS/LDADD/SWP...).
    pub feat_lse: bool,
    /// FEAT_LSE2 — unaligned single-copy-atomic load/store.
    pub feat_lse2: bool,
    /// FEAT_LRCPC — release-consistent (RCpc) load.
    pub feat_lrcpc: bool,
    /// FEAT_LRCPC2 — RCpc with immediate offset.
    pub feat_lrcpc2: bool,
    /// FEAT_FlagM — flag-manipulation (CFINV/RMIF/...).
    pub feat_flagm: bool,
    /// FEAT_FlagM2 — AXFLAG/XAFLAG.
    pub feat_flagm2: bool,
    /// FEAT_DotProd — SDOT/UDOT.
    pub feat_dotprod: bool,
    /// FEAT_CRC32 — CRC32/CRC32C.
    pub feat_crc32: bool,
    /// FEAT_SHA1.
    pub feat_sha1: bool,
    /// FEAT_SHA256.
    pub feat_sha256: bool,
    /// FEAT_AES (+PMULL).
    pub feat_aes: bool,
}

impl HostFeatures {
    /// The conservative all-false baseline (safe on any host).
    #[must_use]
    pub const fn baseline() -> Self {
        Self {
            feat_lse: false,
            feat_lse2: false,
            feat_lrcpc: false,
            feat_lrcpc2: false,
            feat_flagm: false,
            feat_flagm2: false,
            feat_dotprod: false,
            feat_crc32: false,
            feat_sha1: false,
            feat_sha256: false,
            feat_aes: false,
        }
    }
}

// Linux/aarch64 AT_HWCAP bit positions (uapi/asm/hwcap.h). Hardcoded so the
// detection path does not depend on the libc crate exposing every constant.
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
mod hwcap_bits {
    pub const HWCAP_AES: u64 = 1 << 3;
    pub const HWCAP_SHA1: u64 = 1 << 5;
    pub const HWCAP_SHA2: u64 = 1 << 6;
    pub const HWCAP_CRC32: u64 = 1 << 7;
    pub const HWCAP_ATOMICS: u64 = 1 << 8; // FEAT_LSE
    pub const HWCAP_LRCPC: u64 = 1 << 15;
    pub const HWCAP_ASIMDDP: u64 = 1 << 20; // FEAT_DotProd
    pub const HWCAP_USCAT: u64 = 1 << 25; // FEAT_LSE2
    pub const HWCAP_ILRCPC: u64 = 1 << 26; // FEAT_LRCPC2
    pub const HWCAP_FLAGM: u64 = 1 << 27;
    pub const HWCAP2_FLAGM2: u64 = 1 << 0;
}

/// Detect host features. Real detection only on Linux/aarch64; otherwise the
/// safe all-false baseline.
#[must_use]
pub fn detect() -> HostFeatures {
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        use hwcap_bits::*;
        // SAFETY: getauxval is always safe to call with AT_HWCAP/AT_HWCAP2;
        // it returns 0 for unknown entries.
        let hwcap = unsafe { libc::getauxval(libc::AT_HWCAP) };
        let hwcap2 = unsafe { libc::getauxval(libc::AT_HWCAP2) };
        HostFeatures {
            feat_lse: hwcap & HWCAP_ATOMICS != 0,
            feat_lse2: hwcap & HWCAP_USCAT != 0,
            feat_lrcpc: hwcap & HWCAP_LRCPC != 0,
            feat_lrcpc2: hwcap & HWCAP_ILRCPC != 0,
            feat_flagm: hwcap & HWCAP_FLAGM != 0,
            feat_flagm2: hwcap2 & HWCAP2_FLAGM2 != 0,
            feat_dotprod: hwcap & HWCAP_ASIMDDP != 0,
            feat_crc32: hwcap & HWCAP_CRC32 != 0,
            feat_sha1: hwcap & HWCAP_SHA1 != 0,
            feat_sha256: hwcap & HWCAP_SHA2 != 0,
            feat_aes: hwcap & HWCAP_AES != 0,
        }
    }
    #[cfg(not(all(target_os = "linux", target_arch = "aarch64")))]
    {
        HostFeatures::baseline()
    }
}

fn override_slot() -> &'static Mutex<Option<HostFeatures>> {
    static SLOT: OnceLock<Mutex<Option<HostFeatures>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// Cached host features. First call detects; later calls return the cache.
/// A test override (see [`override_for_test`]) shadows the detected values.
#[must_use]
pub fn host_features() -> HostFeatures {
    static CACHED: OnceLock<HostFeatures> = OnceLock::new();
    // Read and release the override lock before consulting the cache so the
    // mutex guard does not stay live across the `get_or_init` call.
    let override_value = *override_slot().lock().unwrap();
    if let Some(f) = override_value {
        return f;
    }
    *CACHED.get_or_init(detect)
}

/// Override the reported host features (test/bench only).
pub fn override_for_test(f: HostFeatures) {
    *override_slot().lock().unwrap() = Some(f);
}

/// Clear a previously installed test override.
pub fn clear_override() {
    *override_slot().lock().unwrap() = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_is_all_false() {
        let b = HostFeatures::baseline();
        assert!(!b.feat_lse && !b.feat_aes && !b.feat_sha256 && !b.feat_crc32);
        assert_eq!(b, HostFeatures::default());
    }

    #[test]
    fn detect_is_safe_default_off_arm_linux() {
        // On the x86 Windows/Linux dev hosts, detection must be all-false.
        #[cfg(not(all(target_os = "linux", target_arch = "aarch64")))]
        assert_eq!(detect(), HostFeatures::baseline());
    }

    #[test]
    fn override_roundtrips_then_clears() {
        let mut f = HostFeatures::baseline();
        f.feat_lse = true;
        f.feat_crc32 = true;
        override_for_test(f);
        let got = host_features();
        assert!(got.feat_lse && got.feat_crc32);
        clear_override();
        // After clearing, the override no longer shadows detection.
        assert!(override_slot().lock().unwrap().is_none());
    }
}
