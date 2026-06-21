//! Guest address-space layout for a loaded program.
//!
//! A loaded [`MappedImage`] is a flat byte buffer at one base; a running guest
//! also needs a stack, and both must live in one coherent, non-overlapping
//! virtual address space. This builds that space — the image region plus an
//! initial-thread stack — so the session can seed RSP into the stack and fetch
//! code from the image, both checked against the same [`AddressSpace`].

use crate::address_space::{AddressSpace, AddressSpaceError, Protection};
use crate::guest_stack::{map_default_stack, GuestStack};
use crate::pe_loader::MappedImage;

/// The guest virtual memory of a loaded program: the address space, the image
/// base, and the initial-thread stack.
#[derive(Debug)]
pub struct GuestLayout {
    pub space: AddressSpace,
    pub image_base: u64,
    pub stack: GuestStack,
}

/// Build the address space for `image` plus an initial-thread stack based at
/// `stack_base`.
///
/// The image is mapped read/write/execute (a single flat region for now;
/// per-section protections are a later refinement) and the stack read/write.
/// Overlap and overflow are rejected by the address space, so the stack can
/// never alias the image.
pub fn layout(image: &MappedImage, stack_base: u64) -> Result<GuestLayout, AddressSpaceError> {
    // usize -> u64 is lossless on every supported host; clamp the impossible
    // wider case so the address space's own overflow check rejects it.
    let image_size = u64::try_from(image.bytes.len()).unwrap_or(u64::MAX);
    let mut space = AddressSpace::new();
    space.map(
        image.base,
        image_size,
        Protection::ReadWriteExecute,
        "image",
    )?;
    let stack = map_default_stack(&mut space, stack_base)?;
    Ok(GuestLayout {
        space,
        image_base: image.base,
        stack,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guest_stack::DEFAULT_STACK_SIZE;

    fn image_at(base: u64, size: usize) -> MappedImage {
        MappedImage {
            base,
            entry_pc: base + 0x1000,
            bytes: vec![0u8; size],
        }
    }

    #[test]
    fn image_and_stack_coexist_in_one_space() {
        let image = image_at(0x1_4000_0000, 0x10000);
        let gl = layout(&image, 0x2_0000_0000).expect("layout");
        // Both regions are present and translate.
        let (img_region, _) = gl.space.translate(0x1_4000_0000).expect("image mapped");
        assert_eq!(img_region.name, "image");
        assert!(img_region.prot.is_executable());
        let (stk_region, _) = gl.space.translate(0x2_0000_0000).expect("stack mapped");
        assert_eq!(stk_region.name, "stack");
        assert!(stk_region.prot.is_writable());
        // The stack top is the value RSP gets seeded with.
        assert_eq!(gl.stack.top, 0x2_0000_0000 + DEFAULT_STACK_SIZE);
        assert_eq!(gl.space.len(), 2);
    }

    #[test]
    fn a_stack_overlapping_the_image_is_rejected() {
        let image = image_at(0x40_0000, 0x0010_0000);
        // Base the stack inside the image span.
        assert!(matches!(
            layout(&image, 0x44_0000),
            Err(AddressSpaceError::Overlap { .. })
        ));
    }
}
