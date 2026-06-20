//! IAT patching: write resolved import addresses into the guest import table.
//!
//! The final dynamic-linking step. Import resolution
//! ([`crate::import_resolver`]) yields each import's guest address; the PE's
//! import descriptors say which IAT slot holds that import. This writes each
//! resolved address into its slot, in place, in the mapped image. Every slot is
//! bounds-checked against the image — the import table is untrusted input, so a
//! crafted descriptor must not be able to write outside the mapped region.

/// One IAT slot to patch: the guest virtual address of the 8-byte slot and the
/// resolved function address to store there (PE32+, so slots are 64-bit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IatPatch {
    pub slot_va: u64,
    pub value: u64,
}

/// Apply IAT patches to a mapped image laid out at `image_base`.
///
/// Each patch writes its 8-byte little-endian value at `slot_va - image_base`.
/// A slot below the base, or whose 8 bytes would run past the image end, is
/// rejected before any write — so a malformed import table cannot corrupt
/// memory outside the mapping. All-or-nothing is not guaranteed across patches;
/// callers validate the descriptor set first, and this is the last line of
/// defence.
pub fn apply_iat_patches(
    image_base: u64,
    image: &mut [u8],
    patches: &[IatPatch],
) -> Result<(), IatError> {
    for patch in patches {
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
        let end = offset.checked_add(8).ok_or(IatError::SlotOutOfRange {
            slot: patch.slot_va,
        })?;
        let dst = image.get_mut(offset..end).ok_or(IatError::SlotOutOfRange {
            slot: patch.slot_va,
        })?;
        dst.copy_from_slice(&patch.value.to_le_bytes());
    }
    Ok(())
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IatError {
    #[error("IAT slot {slot:#x} is below the image base {base:#x}")]
    SlotBelowBase { slot: u64, base: u64 },

    #[error("IAT slot {slot:#x} lies outside the mapped image")]
    SlotOutOfRange { slot: u64 },
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
            },
            IatPatch {
                slot_va: BASE + 8,
                value: 0xDEAD_BEEF,
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
}
