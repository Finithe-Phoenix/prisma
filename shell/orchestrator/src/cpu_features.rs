//! Guest CPUID feature advertisement.
//!
//! The guest probes its CPU with CPUID and then freely executes whatever the
//! result advertises.
//!
//! So Prisma must never advertise a feature the translator cannot actually
//! lower — doing so makes the guest run an instruction the DBT has no rule for,
//! a hard fault rather than slow-but-correct execution. This module models the
//! feature set as a bitset and gates advertisement on the `advertised subset of
//! supported` invariant (correctness > performance).

/// One x86 ISA extension the guest can probe for. The discriminants are bit
/// positions in [`FeatureSet`], not architectural CPUID bits — the CPUID leaf
/// encoding is a separate concern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feature {
    Sse,
    Sse2,
    Sse3,
    Ssse3,
    Sse41,
    Sse42,
    Avx,
    Avx2,
    Fma,
    Bmi1,
    Bmi2,
    Popcnt,
    Lzcnt,
    Aes,
    Sha,
    Movbe,
    Crc32,
}

impl Feature {
    /// This feature's bit position in a [`FeatureSet`].
    const fn bit(self) -> u64 {
        1u64 << (self as u64)
    }
}

/// A set of advertised CPU features, as a bitset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FeatureSet {
    bits: u64,
}

impl FeatureSet {
    /// The empty set (a CPU advertising nothing beyond the baseline).
    #[must_use]
    pub const fn empty() -> Self {
        Self { bits: 0 }
    }

    /// Whether `feature` is in the set.
    #[must_use]
    pub const fn contains(self, feature: Feature) -> bool {
        self.bits & feature.bit() != 0
    }

    /// This set with `feature` added (does not mutate `self`).
    #[must_use]
    pub const fn with(self, feature: Feature) -> Self {
        Self {
            bits: self.bits | feature.bit(),
        }
    }

    /// Whether every feature in `self` is also in `other`.
    #[must_use]
    pub const fn is_subset_of(self, other: Self) -> bool {
        self.bits & other.bits == self.bits
    }

    /// The features in `self` not present in `other`.
    #[must_use]
    pub const fn difference(self, other: Self) -> Self {
        Self {
            bits: self.bits & !other.bits,
        }
    }

    /// Number of features in the set.
    #[must_use]
    pub const fn count(self) -> u32 {
        self.bits.count_ones()
    }
}

/// Build a [`FeatureSet`] from a slice of features.
#[must_use]
pub fn feature_set(features: &[Feature]) -> FeatureSet {
    let mut set = FeatureSet::empty();
    for &f in features {
        set = set.with(f);
    }
    set
}

/// The features the translator can actually lower today, mirroring the C++
/// core's coverage.
///
/// SSE..SSE4.2 + AVX/AVX2 + FMA + BMI1/2 + POPCNT/LZCNT + AES + SHA + MOVBE +
/// CRC32. Update this in lockstep when the decoder/backend gain a new family —
/// it is the single source of truth for what is safe to advertise.
#[must_use]
pub fn translatable() -> FeatureSet {
    feature_set(&[
        Feature::Sse,
        Feature::Sse2,
        Feature::Sse3,
        Feature::Ssse3,
        Feature::Sse41,
        Feature::Sse42,
        Feature::Avx,
        Feature::Avx2,
        Feature::Fma,
        Feature::Bmi1,
        Feature::Bmi2,
        Feature::Popcnt,
        Feature::Lzcnt,
        Feature::Aes,
        Feature::Sha,
        Feature::Movbe,
        Feature::Crc32,
    ])
}

/// Advertise `requested` to the guest, gated on `supported` (the set the
/// translator can lower).
///
/// Rejects if any requested feature is unsupported, reporting how many — so the
/// caller downgrades the request rather than crashing the guest on an
/// untranslatable instruction at run time.
pub const fn advertise(
    requested: FeatureSet,
    supported: FeatureSet,
) -> Result<FeatureSet, CpuFeatureError> {
    if requested.is_subset_of(supported) {
        Ok(requested)
    } else {
        Err(CpuFeatureError::Untranslatable {
            unsupported: requested.difference(supported).count(),
        })
    }
}

/// Advertise against the current translator coverage ([`translatable`]).
pub fn advertise_default(requested: FeatureSet) -> Result<FeatureSet, CpuFeatureError> {
    advertise(requested, translatable())
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CpuFeatureError {
    #[error("{unsupported} requested feature(s) are not translatable")]
    Untranslatable { unsupported: u32 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_and_with_round_trip() {
        let s = FeatureSet::empty().with(Feature::Avx2).with(Feature::Sha);
        assert!(s.contains(Feature::Avx2));
        assert!(s.contains(Feature::Sha));
        assert!(!s.contains(Feature::Aes));
        assert_eq!(s.count(), 2);
    }

    #[test]
    fn with_is_idempotent() {
        let s = FeatureSet::empty().with(Feature::Sse2).with(Feature::Sse2);
        assert_eq!(s.count(), 1);
    }

    #[test]
    fn subset_and_difference() {
        let small = feature_set(&[Feature::Sse, Feature::Sse2]);
        let big = feature_set(&[Feature::Sse, Feature::Sse2, Feature::Avx]);
        assert!(small.is_subset_of(big));
        assert!(!big.is_subset_of(small));
        assert!(small.is_subset_of(small));
        assert_eq!(big.difference(small), feature_set(&[Feature::Avx]));
        assert_eq!(small.difference(big).count(), 0);
    }

    #[test]
    fn advertise_accepts_a_supported_request() {
        let supported = feature_set(&[Feature::Sse, Feature::Sse2, Feature::Avx]);
        let req = feature_set(&[Feature::Sse2, Feature::Avx]);
        assert_eq!(advertise(req, supported), Ok(req));
    }

    #[test]
    fn advertise_rejects_an_unsupported_request_and_counts_offenders() {
        // Translator supports only SSE/SSE2; guest asks for AVX + FMA too.
        let supported = feature_set(&[Feature::Sse, Feature::Sse2]);
        let req = feature_set(&[Feature::Sse, Feature::Sse2, Feature::Avx, Feature::Fma]);
        assert_eq!(
            advertise(req, supported),
            Err(CpuFeatureError::Untranslatable { unsupported: 2 })
        );
    }

    #[test]
    fn advertise_default_accepts_the_full_translatable_set() {
        assert_eq!(advertise_default(translatable()), Ok(translatable()));
        assert_eq!(
            advertise_default(FeatureSet::empty()),
            Ok(FeatureSet::empty())
        );
    }
}
