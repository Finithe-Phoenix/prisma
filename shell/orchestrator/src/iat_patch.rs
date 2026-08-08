//! IAT patching: write resolved import addresses into the guest import table.
//!
//! The final dynamic-linking step. Import resolution
//! ([`crate::import_resolver`]) yields each import's guest address; the PE's
//! import descriptors say which IAT slot holds that import. This writes each
//! resolved address into its slot, in place, in the mapped image. Every slot is
//! bounds-checked against the image — the import table is untrusted input, so a
//! crafted descriptor must not be able to write outside the mapped region.

/// Width of one import-address-table slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IatWidth {
    Pe32,
    Pe32Plus,
}

impl IatWidth {
    const fn bytes(self) -> usize {
        match self {
            Self::Pe32 => 4,
            Self::Pe32Plus => 8,
        }
    }
}

/// One IAT slot to patch: the guest virtual address, resolved function address,
/// and the width selected by the image's optional-header format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IatPatch {
    pub slot_va: u64,
    pub value: u64,
    pub width: IatWidth,
}

/// Apply IAT patches to a mapped image laid out at `image_base`.
///
/// Each patch writes a 4-byte PE32 or 8-byte PE32+ little-endian value at
/// `slot_va - image_base`. Every patch is validated before the first write, so
/// malformed input cannot leave a partially modified image.
pub fn apply_iat_patches(
    image_base: u64,
    image: &mut [u8],
    patches: &[IatPatch],
) -> Result<(), IatError> {
    let mut ranges = Vec::with_capacity(patches.len());
    for patch in patches {
        if patch.width == IatWidth::Pe32 && patch.value > u64::from(u32::MAX) {
            return Err(IatError::ValueOutOfRange {
                slot: patch.slot_va,
                value: patch.value,
            });
        }
        let offset = patch
            .slot_va
            .checked_sub(image_base)
            .ok_or(IatError::SlotBelowBase {
                slot: patch.slot_va,
                base: image_base,
            })?;
        let offset = usize::try_from(offset).map_err(|_| IatError::SlotOutOfRange {
            slot: patch.slot_va,
        })?;
        let end = offset
            .checked_add(patch.width.bytes())
            .ok_or(IatError::SlotOutOfRange {
                slot: patch.slot_va,
            })?;
        if end > image.len() {
            return Err(IatError::SlotOutOfRange {
                slot: patch.slot_va,
            });
        }
        ranges.push((offset, end));
    }

    for (patch, (offset, end)) in patches.iter().zip(ranges) {
        let bytes = patch.value.to_le_bytes();
        image[offset..end].copy_from_slice(&bytes[..patch.width.bytes()]);
    }
    Ok(())
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IatError {
    #[error("IAT slot {slot:#x} is below the image base {base:#x}")]
    SlotBelowBase { slot: u64, base: u64 },

    #[error("IAT slot {slot:#x} lies outside the mapped image")]
    SlotOutOfRange { slot: u64 },

    #[error("IAT value {value:#x} does not fit the PE32 slot at {slot:#x}")]
    ValueOutOfRange { slot: u64, value: u64 },
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u64 = 0x1_4000_0000;

    #[test]
    fn writes_little_endian_addresses_into_slots() {
        let mut image = vec![0u8; 32];
        let patches = [
            IatPatch {
                slot_va: BASE,
                value: 0x1122_3344_5566_7788,
                width: IatWidth::Pe32Plus,
            },
            IatPatch {
                slot_va: BASE + 8,
                value: 0xDEAD_BEEF,
                width: IatWidth::Pe32Plus,
            },
        ];
        apply_iat_patches(BASE, &mut image, &patches).expect("patch");
        assert_eq!(&image[0..8], &0x1122_3344_5566_7788u64.to_le_bytes());
        assert_eq!(&image[8..16], &0xDEAD_BEEFu64.to_le_bytes());
        // Untouched tail stays zero.
        assert!(image[16..].iter().all(|&b| b == 0));
    }

    #[test]
    fn rejects_slot_below_base() {
        let mut image = vec![0u8; 16];
        let patches = [IatPatch {
            slot_va: BASE - 8,
            value: 1,
            width: IatWidth::Pe32Plus,
        }];
        assert!(matches!(
            apply_iat_patches(BASE, &mut image, &patches),
            Err(IatError::SlotBelowBase { .. })
        ));
        // Nothing was written.
        assert!(image.iter().all(|&b| b == 0));
    }

    #[test]
    fn rejects_slot_running_past_image_end() {
        let mut image = vec![0u8; 16];
        // Slot starts at offset 12; its 8 bytes would reach offset 20 > 16.
        let patches = [IatPatch {
            slot_va: BASE + 12,
            value: 1,
            width: IatWidth::Pe32Plus,
        }];
        assert!(matches!(
            apply_iat_patches(BASE, &mut image, &patches),
            Err(IatError::SlotOutOfRange { .. })
        ));
    }

    #[test]
    fn last_byte_slot_is_accepted() {
        let mut image = vec![0u8; 16];
        // Slot at offset 8 occupies bytes 8..16 exactly — in bounds.
        let patches = [IatPatch {
            slot_va: BASE + 8,
            value: 0xAABB,
            width: IatWidth::Pe32Plus,
        }];
        apply_iat_patches(BASE, &mut image, &patches).expect("in-bounds");
        assert_eq!(&image[8..16], &0xAABBu64.to_le_bytes());
    }

    #[test]
    fn empty_patch_set_is_a_noop() {
        let mut image = vec![0xFFu8; 8];
        apply_iat_patches(BASE, &mut image, &[]).expect("noop");
        assert!(image.iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn pe32_writes_four_bytes_without_touching_the_next_slot() {
        let mut image = vec![0xAA; 12];
        let patches = [IatPatch {
            slot_va: BASE + 4,
            value: 0x1122_3344,
            width: IatWidth::Pe32,
        }];
        apply_iat_patches(BASE, &mut image, &patches).expect("patch");
        assert_eq!(&image[4..8], &0x1122_3344u32.to_le_bytes());
        assert_eq!(&image[8..12], &[0xAA; 4]);
    }

    #[test]
    fn validates_every_patch_before_modifying_the_image() {
        let mut image = vec![0xAA; 16];
        let patches = [
            IatPatch {
                slot_va: BASE,
                value: 1,
                width: IatWidth::Pe32Plus,
            },
            IatPatch {
                slot_va: BASE + 12,
                value: 2,
                width: IatWidth::Pe32Plus,
            },
        ];
        assert!(matches!(
            apply_iat_patches(BASE, &mut image, &patches),
            Err(IatError::SlotOutOfRange { .. })
        ));
        assert_eq!(image, vec![0xAA; 16]);
    }

    #[test]
    fn rejects_a_pe32_address_that_would_truncate() {
        let mut image = vec![0; 4];
        let patches = [IatPatch {
            slot_va: BASE,
            value: u64::from(u32::MAX) + 1,
            width: IatWidth::Pe32,
        }];
        assert!(matches!(
            apply_iat_patches(BASE, &mut image, &patches),
            Err(IatError::ValueOutOfRange { .. })
        ));
        assert_eq!(image, vec![0; 4]);
    }
}
