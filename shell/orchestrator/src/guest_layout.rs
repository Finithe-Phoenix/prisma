//! Guest address-space layout for a loaded program.
//!
//! A loaded [`MappedImage`] is a flat byte buffer at one base; a running guest
//! also needs a stack, and both must live in one coherent, non-overlapping
//! virtual address space. This builds that space — the image region plus an
//! initial-thread stack — so the session can seed RSP into the stack and fetch
//! code from the image, both checked against the same [`AddressSpace`].

use crate::address_space::{AddressSpace, AddressSpaceError, Protection};
use crate::guest_stack::{map_default_stack, GuestStack};
use crate::pe_loader::{MappedImage, PeImage};

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

/// Build the address space mapping each PE section with its OWN protection.
///
/// `.text` maps RX, `.data` RW, ... instead of a blanket RWX, plus the
/// initial-thread stack. This is the W^X layout: code pages are not writable and
/// data pages not executable, per the resource-discipline / W^X policy.
///
/// Zero-virtual-size sections are skipped; overlap and overflow are rejected by
/// the address space, so a malformed section table cannot produce aliasing or
/// wrapping mappings.
pub fn layout_sections(
    img: &PeImage,
    mapped: &MappedImage,
    stack_base: u64,
) -> Result<GuestLayout, AddressSpaceError> {
    let mut space = AddressSpace::new();
    for sec in &img.sections {
        if sec.virtual_size == 0 {
            continue;
        }
        let base = mapped
            .base
            .checked_add(u64::from(sec.virtual_address))
            .ok_or_else(|| {
                AddressSpaceError::Overflow(mapped.base, u64::from(sec.virtual_address))
            })?;
        space.map(
            base,
            u64::from(sec.virtual_size),
            sec.protection(),
            section_name(sec.name),
        )?;
    }
    let stack = map_default_stack(&mut space, stack_base)?;
    Ok(GuestLayout {
        space,
        image_base: mapped.base,
        stack,
    })
}

/// The printable section name (NUL-trimmed), for region diagnostics.
fn section_name(raw: [u8; 8]) -> String {
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    String::from_utf8_lossy(&raw[..end]).into_owned()
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

    use crate::pe_loader::{PeImage, PeMachine, PeSection};

    fn section(name: [u8; 8], va: u32, size: u32, chars: u32) -> PeSection {
        PeSection {
            name,
            virtual_size: size,
            virtual_address: va,
            raw_data_size: 0,
            raw_data_offset: 0,
            characteristics: chars,
        }
    }

    #[test]
    fn sections_map_with_their_own_w_xor_x_protections() {
        const EXECUTE: u32 = 0x2000_0000;
        const WRITE: u32 = 0x8000_0000;
        let img = PeImage {
            machine: PeMachine::X86_64,
            section_count: 2,
            timestamp: 0,
            entry_point_rva: 0x1000,
            image_base: 0x1_4000_0000,
            size_of_image: 0x10000,
            sections: vec![
                section(*b".text\0\0\0", 0x1000, 0x1000, EXECUTE),
                section(*b".data\0\0\0", 0x2000, 0x1000, WRITE),
            ],
            pe32_plus: true,
            import_dir_rva: 0,
            import_dir_size: 0,
            reloc_dir_rva: 0,
            reloc_dir_size: 0,
            export_dir_rva: 0,
            export_dir_size: 0,
        };
        let mapped = image_at(0x1_4000_0000, 0x10000);
        let gl = layout_sections(&img, &mapped, 0x2_0000_0000).expect("layout");

        // .text is executable but NOT writable; .data writable but NOT executable.
        let (text, _) = gl.space.translate(0x1_4000_1000).expect(".text mapped");
        assert_eq!(text.name, ".text");
        assert!(text.prot.is_executable() && !text.prot.is_writable());
        let (data, _) = gl.space.translate(0x1_4000_2000).expect(".data mapped");
        assert_eq!(data.name, ".data");
        assert!(data.prot.is_writable() && !data.prot.is_executable());
        // Two sections + the stack.
        assert_eq!(gl.space.len(), 3);
    }
}
