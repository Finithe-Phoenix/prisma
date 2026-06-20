//! CPUID leaf encoding.
//!
//! The guest executes `CPUID` and reads feature bits out of EAX/EBX/ECX/EDX.
//! This packs an advertised [`FeatureSet`] into those register values at the
//! architectural bit positions (Intel SDM Vol. 2A, CPUID), so what the guest
//! probes matches exactly what [`crate::cpu_features::advertise`] cleared as
//! translatable. Bit positions are fixed by the ISA, not chosen here.

use crate::cpu_features::{Feature, FeatureSet};

/// CPUID leaf 1 (`EAX=1`) feature registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Leaf1 {
    pub ecx: u32,
    pub edx: u32,
}

/// Encode CPUID leaf 1 (`EAX=1`) ECX/EDX feature bits from `features`.
#[must_use]
pub const fn leaf1(features: FeatureSet) -> Leaf1 {
    let mut ecx = 0u32;
    let mut edx = 0u32;
    if features.contains(Feature::Sse) {
        edx |= 1 << 25;
    }
    if features.contains(Feature::Sse2) {
        edx |= 1 << 26;
    }
    if features.contains(Feature::Sse3) {
        ecx |= 1 << 0;
    }
    if features.contains(Feature::Ssse3) {
        ecx |= 1 << 9;
    }
    if features.contains(Feature::Fma) {
        ecx |= 1 << 12;
    }
    if features.contains(Feature::Sse41) {
        ecx |= 1 << 19;
    }
    if features.contains(Feature::Sse42) {
        ecx |= 1 << 20;
    }
    if features.contains(Feature::Movbe) {
        ecx |= 1 << 22;
    }
    if features.contains(Feature::Popcnt) {
        ecx |= 1 << 23;
    }
    if features.contains(Feature::Aes) {
        ecx |= 1 << 25;
    }
    if features.contains(Feature::Avx) {
        ecx |= 1 << 28;
    }
    Leaf1 { ecx, edx }
}

/// Encode CPUID leaf 7 sub-leaf 0 (`EAX=7, ECX=0`) EBX feature bits.
#[must_use]
pub const fn leaf7_ebx(features: FeatureSet) -> u32 {
    let mut ebx = 0u32;
    if features.contains(Feature::Bmi1) {
        ebx |= 1 << 3;
    }
    if features.contains(Feature::Avx2) {
        ebx |= 1 << 5;
    }
    if features.contains(Feature::Bmi2) {
        ebx |= 1 << 8;
    }
    if features.contains(Feature::Sha) {
        ebx |= 1 << 29;
    }
    ebx
}

/// Encode extended leaf `0x8000_0001` ECX bits. LZCNT (ABM) lives here, not in
/// the standard leaves.
#[must_use]
pub const fn leaf_ext1_ecx(features: FeatureSet) -> u32 {
    let mut ecx = 0u32;
    if features.contains(Feature::Lzcnt) {
        ecx |= 1 << 5;
    }
    ecx
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu_features::feature_set;

    #[test]
    fn sse_family_lands_in_leaf1_edx_and_ecx() {
        let l = leaf1(feature_set(&[Feature::Sse, Feature::Sse2, Feature::Sse42]));
        assert_eq!(l.edx, (1 << 25) | (1 << 26)); // SSE | SSE2
        assert_eq!(l.ecx, 1 << 20); // SSE4.2
    }

    #[test]
    fn avx_and_fma_are_leaf1_ecx() {
        let l = leaf1(feature_set(&[Feature::Avx, Feature::Fma]));
        assert_eq!(l.ecx, (1 << 28) | (1 << 12));
        assert_eq!(l.edx, 0);
    }

    #[test]
    fn avx2_bmi_sha_are_leaf7_ebx() {
        let ebx = leaf7_ebx(feature_set(&[
            Feature::Avx2,
            Feature::Bmi1,
            Feature::Bmi2,
            Feature::Sha,
        ]));
        assert_eq!(ebx, (1 << 5) | (1 << 3) | (1 << 8) | (1 << 29));
    }

    #[test]
    fn lzcnt_is_extended_leaf() {
        assert_eq!(leaf_ext1_ecx(feature_set(&[Feature::Lzcnt])), 1 << 5);
        // LZCNT does not appear in the standard leaves.
        assert_eq!(leaf1(feature_set(&[Feature::Lzcnt])), Leaf1::default());
        assert_eq!(leaf7_ebx(feature_set(&[Feature::Lzcnt])), 0);
    }

    #[test]
    fn empty_feature_set_encodes_to_zero() {
        assert_eq!(leaf1(FeatureSet::empty()), Leaf1::default());
        assert_eq!(leaf7_ebx(FeatureSet::empty()), 0);
        assert_eq!(leaf_ext1_ecx(FeatureSet::empty()), 0);
    }

    #[test]
    fn features_do_not_leak_across_leaves() {
        // An AVX2 (leaf-7) request must not set any leaf-1 bit.
        let only_avx2 = feature_set(&[Feature::Avx2]);
        assert_eq!(leaf1(only_avx2), Leaf1::default());
        assert_eq!(leaf7_ebx(only_avx2), 1 << 5);
    }
}
