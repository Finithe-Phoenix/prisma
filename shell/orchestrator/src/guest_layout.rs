//! Guest address-space layout for a loaded program.
//!
//! A loaded [`MappedImage`] is a flat byte buffer at one base; a running guest
//! also needs a stack, and both must live in one coherent, non-overlapping
//! virtual address space. This builds that space — the image region plus an
//! initial-thread stack — so the session can seed RSP into the stack and fetch
//! code from the image, both checked against the same [`AddressSpace`].

use crate::address_space::{AddressSpace, AddressSpaceError, Protection};
use crate::backed_address_space::BackedAddressSpace;
use crate::guest_stack::{map_default_stack, GuestStack, DEFAULT_STACK_SIZE};
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

/// The guest virtual memory of a loaded program with its **bytes**.
///
/// The byte-backed counterpart of [`GuestLayout`], built on
/// [`BackedAddressSpace`] so the address space can grow at runtime (`brk`/
/// `mmap`). This is the memory a real run loop (RFC 0019) executes against.
#[derive(Debug)]
pub struct BackedGuestLayout {
    pub space: BackedAddressSpace,
    pub image_base: u64,
    /// The value RSP is seeded with (the high end of the stack region).
    pub stack_top: u64,
}

/// `vsize` bytes of a section's content, taken from the flat mapped image at
/// virtual address `va` and zero-padded where `vsize` runs past the available
/// image bytes (a `.bss`-style zero-fill tail).
fn section_content(image: &[u8], va: usize, vsize: usize) -> Vec<u8> {
    let mut out = vec![0u8; vsize];
    if va < image.len() {
        let avail = (va + vsize).min(image.len());
        out[..avail - va].copy_from_slice(&image[va..avail]);
    }
    out
}

/// Build a byte-backed address space mapping each PE section **with its content
/// and its own protection**, plus a zeroed initial-thread stack.
///
/// The byte-backed analogue of [`layout_sections`]: where that records region
/// metadata against a separate flat image, this copies each section's bytes into
/// its own region so the space owns its memory and can be mutated/grown by a
/// running guest. Zero-virtual-size sections are skipped; overlap and overflow
/// are rejected.
///
/// # Errors
/// [`AddressSpaceError`] if a section base overflows or any mapping overlaps.
pub fn backed_layout_sections(
    img: &PeImage,
    mapped: &MappedImage,
    stack_base: u64,
) -> Result<BackedGuestLayout, AddressSpaceError> {
    let mut space = BackedAddressSpace::new();
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
        let content = section_content(
            &mapped.bytes,
            sec.virtual_address as usize,
            sec.virtual_size as usize,
        );
        space.map_with_bytes(base, &content, sec.protection())?;
    }
    // The initial-thread stack: a zeroed read-write region.
    space.map(stack_base, DEFAULT_STACK_SIZE, Protection::ReadWrite)?;
    Ok(BackedGuestLayout {
        space,
        image_base: mapped.base,
        stack_top: stack_base.saturating_add(DEFAULT_STACK_SIZE),
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

    fn two_section_image() -> PeImage {
        const EXECUTE: u32 = 0x2000_0000;
        const WRITE: u32 = 0x8000_0000;
        PeImage {
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
        }
    }

    #[test]
    fn backed_layout_installs_section_content_and_a_writable_stack() {
        let img = two_section_image();
        // A flat image with a marker at the start of each section's VA.
        let mut bytes = vec![0u8; 0x10000];
        bytes[0x1000] = 0xAA; // .text first byte
        bytes[0x2000] = 0xBB; // .data first byte
        let mapped = MappedImage {
            base: 0x1_4000_0000,
            entry_pc: 0x1_4000_1000,
            bytes,
        };
        let mut bl = backed_layout_sections(&img, &mapped, 0x2_0000_0000).expect("backed layout");

        // .text content is installed and read-only (RX): a write faults.
        assert_eq!(bl.space.read(0x1_4000_1000, 1).unwrap(), &[0xAA]);
        assert!(bl.space.write(0x1_4000_1000, &[0]).is_err());
        // .data content is installed and writable.
        assert_eq!(bl.space.read(0x1_4000_2000, 1).unwrap(), &[0xBB]);
        bl.space.write(0x1_4000_2000, &[0x99]).unwrap();
        assert_eq!(bl.space.read(0x1_4000_2000, 1).unwrap(), &[0x99]);
        // The stack is a writable region with the expected top (RSP seed).
        assert_eq!(bl.stack_top, 0x2_0000_0000 + DEFAULT_STACK_SIZE);
        bl.space.write(0x2_0000_0000, &[1, 2, 3]).unwrap();
        // Two sections + the stack.
        assert_eq!(bl.space.region_count(), 3);
    }

    #[test]
    fn section_content_zero_pads_a_bss_tail() {
        // A section whose virtual_size runs past the available image bytes is
        // zero-filled there (BSS), never reading out of the image buffer.
        let padded = super::section_content(&[1, 2, 3, 4], 2, 6);
        assert_eq!(padded, vec![3, 4, 0, 0, 0, 0]);
        // A section entirely past the image is all zeros.
        assert_eq!(super::section_content(&[1, 2], 8, 3), vec![0, 0, 0]);
    }
}
