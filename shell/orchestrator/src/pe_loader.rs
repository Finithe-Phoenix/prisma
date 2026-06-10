//! PE/COFF loader skeleton — Fase 2+ Wine-integration prep.
//!
//! When Prisma loads a Windows `.exe` we need to:
//!
//!   1. Parse the PE/COFF headers — DOS stub → NT headers → section
//!      table.
//!   2. Map each section into guest virtual memory at its preferred
//!      base + relocations resolved.
//!   3. Resolve imports via Wine's loader path, which calls back into
//!      this orchestrator for `LoadLibrary` / `GetProcAddress`
//!      indirection.
//!   4. Hand the entry-point guest PC to the C++ Translator + Dispatcher
//!      (`core/build/prisma_run` is the moral equivalent today).
//!
//! Status: parser-only skeleton. Reads the DOS stub + NT headers + the
//! section table, validates magic numbers, returns a `PeImage` with
//! `(virtual_address, size, characteristics)` per section. No
//! relocation, no imports, no execution. Enough to:
//!
//!   * Round-trip the PE shape through serde (the Android side will
//!     display it before launching).
//!   * Bail safely on malformed inputs without UB.
//!   * Pin the public API so the Wine + Translator integration in
//!     Fase 4 has a stable C-ABI surface to call.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PeMachine {
    I386,   // 0x014C
    X86_64, // 0x8664
    Arm64,  // 0xAA64
    Other(u16),
}

impl PeMachine {
    const fn from_u16(m: u16) -> Self {
        match m {
            0x014C => Self::I386,
            0x8664 => Self::X86_64,
            0xAA64 => Self::Arm64,
            other => Self::Other(other),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeSection {
    pub name: [u8; 8],
    pub virtual_size: u32,
    pub virtual_address: u32,
    pub raw_data_size: u32,
    pub raw_data_offset: u32,
    pub characteristics: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeImage {
    pub machine: PeMachine,
    pub section_count: u16,
    pub timestamp: u32,
    pub entry_point_rva: u32,
    pub image_base: u64,
    pub size_of_image: u32,
    pub sections: Vec<PeSection>,
}

#[derive(Debug, Error)]
pub enum PeError {
    #[error("file too small to be a PE ({0} bytes)")]
    Truncated(usize),

    #[error("invalid DOS magic: expected 'MZ', got {0:#06x}")]
    BadDosMagic(u16),

    #[error("e_lfanew points outside the file ({0} ≥ {1})")]
    LfanewOutOfRange(usize, usize),

    #[error("invalid NT magic: expected 'PE\\0\\0', got {0:#010x}")]
    BadNtMagic(u32),

    #[error("unsupported optional header magic: {0:#06x}")]
    UnsupportedOptHeader(u16),

    #[error("section {0} maps outside the image (va {1:#x} + {2:#x} > {3:#x})")]
    SectionOutOfImage(usize, u32, u32, u32),

    #[error("section {0} raw data lies outside the file ({1:#x} + {2:#x} > {3:#x})")]
    SectionRawOutOfFile(usize, u32, u32, usize),

    #[error("entry point RVA {0:#x} lies outside the image ({1:#x})")]
    EntryOutOfImage(u32, u32),

    #[error("image_base + size_of_image wraps the 64-bit address space")]
    ImageWrapsAddressSpace,
}

/// A PE image laid out flat in guest virtual memory: `bytes[i]` is the
/// byte at guest address `base + i`. This is what the C++ dispatcher
/// fetches guest code from (via `prisma-core`'s `GuestImage`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedImage {
    pub base: u64,
    pub entry_pc: u64,
    pub bytes: Vec<u8>,
}

/// Lay out a parsed PE in guest virtual memory.
///
/// Zero-filled `size_of_image` bytes at `image_base`, each section's
/// raw data copied to its virtual address. No relocations, no imports
/// — the Fase 2 hybrid path loads fully-linked position-dependent
/// images. All bounds come from untrusted input, so every offset is
/// checked; malformed sections error instead of truncating silently.
pub fn map_image(img: &PeImage, file: &[u8]) -> Result<MappedImage, PeError> {
    if img.entry_point_rva >= img.size_of_image {
        return Err(PeError::EntryOutOfImage(
            img.entry_point_rva,
            img.size_of_image,
        ));
    }
    if img
        .image_base
        .checked_add(u64::from(img.size_of_image))
        .is_none()
    {
        return Err(PeError::ImageWrapsAddressSpace);
    }

    let mut bytes = vec![0u8; img.size_of_image as usize];
    for (idx, sec) in img.sections.iter().enumerate() {
        if sec.raw_data_size == 0 {
            continue; // purely virtual section (.bss-style)
        }
        let va_end = sec
            .virtual_address
            .checked_add(sec.raw_data_size)
            .filter(|&end| end <= img.size_of_image)
            .ok_or(PeError::SectionOutOfImage(
                idx,
                sec.virtual_address,
                sec.raw_data_size,
                img.size_of_image,
            ))?;
        let raw_end = sec
            .raw_data_offset
            .checked_add(sec.raw_data_size)
            .map(|end| end as usize)
            .filter(|&end| end <= file.len())
            .ok_or(PeError::SectionRawOutOfFile(
                idx,
                sec.raw_data_offset,
                sec.raw_data_size,
                file.len(),
            ))?;
        bytes[sec.virtual_address as usize..va_end as usize]
            .copy_from_slice(&file[sec.raw_data_offset as usize..raw_end]);
    }

    Ok(MappedImage {
        base: img.image_base,
        entry_pc: img.image_base + u64::from(img.entry_point_rva),
        bytes,
    })
}

/// Parse a PE/COFF byte slice into a `PeImage`. Read-only; the
/// caller still owns the bytes.
pub fn parse(bytes: &[u8]) -> Result<PeImage, PeError> {
    // 1. DOS header — 64 bytes. e_magic at offset 0; e_lfanew at 0x3C.
    if bytes.len() < 64 {
        return Err(PeError::Truncated(bytes.len()));
    }
    let dos_magic = u16::from_le_bytes([bytes[0], bytes[1]]);
    if dos_magic != 0x5A4D {
        // 'MZ' little-endian
        return Err(PeError::BadDosMagic(dos_magic));
    }
    let e_lfanew =
        u32::from_le_bytes([bytes[0x3C], bytes[0x3D], bytes[0x3E], bytes[0x3F]]) as usize;
    if e_lfanew + 24 > bytes.len() {
        return Err(PeError::LfanewOutOfRange(e_lfanew, bytes.len()));
    }

    // 2. NT signature — 4 bytes 'PE\0\0' at e_lfanew.
    let nt_magic = u32::from_le_bytes([
        bytes[e_lfanew],
        bytes[e_lfanew + 1],
        bytes[e_lfanew + 2],
        bytes[e_lfanew + 3],
    ]);
    if nt_magic != 0x0000_4550 {
        // 'PE\0\0'
        return Err(PeError::BadNtMagic(nt_magic));
    }

    // 3. COFF file header — 20 bytes after NT signature.
    let coff = e_lfanew + 4;
    let machine = u16::from_le_bytes([bytes[coff], bytes[coff + 1]]);
    let n_sections = u16::from_le_bytes([bytes[coff + 2], bytes[coff + 3]]);
    let timestamp = u32::from_le_bytes([
        bytes[coff + 4],
        bytes[coff + 5],
        bytes[coff + 6],
        bytes[coff + 7],
    ]);
    let opt_hdr_size = u16::from_le_bytes([bytes[coff + 16], bytes[coff + 17]]) as usize;

    // 4. Optional header — variable size; first 2 bytes carry magic
    //    (0x010B = PE32, 0x020B = PE32+).
    let opt = coff + 20;
    if opt + opt_hdr_size > bytes.len() {
        return Err(PeError::Truncated(bytes.len()));
    }
    let opt_magic = u16::from_le_bytes([bytes[opt], bytes[opt + 1]]);
    let (entry_rva, image_base, size_of_image) = match opt_magic {
        0x010B => {
            // PE32: image_base is u32 at offset 28.
            let entry = u32::from_le_bytes([
                bytes[opt + 16],
                bytes[opt + 17],
                bytes[opt + 18],
                bytes[opt + 19],
            ]);
            let base = u64::from(u32::from_le_bytes([
                bytes[opt + 28],
                bytes[opt + 29],
                bytes[opt + 30],
                bytes[opt + 31],
            ]));
            let sz = u32::from_le_bytes([
                bytes[opt + 56],
                bytes[opt + 57],
                bytes[opt + 58],
                bytes[opt + 59],
            ]);
            (entry, base, sz)
        }
        0x020B => {
            // PE32+: image_base is u64 at offset 24.
            let entry = u32::from_le_bytes([
                bytes[opt + 16],
                bytes[opt + 17],
                bytes[opt + 18],
                bytes[opt + 19],
            ]);
            let mut base_buf = [0u8; 8];
            base_buf.copy_from_slice(&bytes[opt + 24..opt + 32]);
            let base = u64::from_le_bytes(base_buf);
            let sz = u32::from_le_bytes([
                bytes[opt + 56],
                bytes[opt + 57],
                bytes[opt + 58],
                bytes[opt + 59],
            ]);
            (entry, base, sz)
        }
        m => return Err(PeError::UnsupportedOptHeader(m)),
    };

    // 5. Section table — each entry is 40 bytes, n_sections of them.
    let mut sec_off = opt + opt_hdr_size;
    let mut sections = Vec::with_capacity(n_sections as usize);
    for _ in 0..n_sections {
        if sec_off + 40 > bytes.len() {
            return Err(PeError::Truncated(bytes.len()));
        }
        let mut name = [0u8; 8];
        name.copy_from_slice(&bytes[sec_off..sec_off + 8]);
        let virtual_size = u32::from_le_bytes(bytes[sec_off + 8..sec_off + 12].try_into().unwrap());
        let virtual_address =
            u32::from_le_bytes(bytes[sec_off + 12..sec_off + 16].try_into().unwrap());
        let raw_data_size =
            u32::from_le_bytes(bytes[sec_off + 16..sec_off + 20].try_into().unwrap());
        let raw_data_offset =
            u32::from_le_bytes(bytes[sec_off + 20..sec_off + 24].try_into().unwrap());
        let characteristics =
            u32::from_le_bytes(bytes[sec_off + 36..sec_off + 40].try_into().unwrap());
        sections.push(PeSection {
            name,
            virtual_size,
            virtual_address,
            raw_data_size,
            raw_data_offset,
            characteristics,
        });
        sec_off += 40;
    }

    Ok(PeImage {
        machine: PeMachine::from_u16(machine),
        section_count: n_sections,
        timestamp,
        entry_point_rva: entry_rva,
        image_base,
        size_of_image,
        sections,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds the smallest possible PE32+ that `parse` accepts:
    /// DOS stub (64 bytes) → `e_lfanew=64` → NT magic → COFF header →
    /// optional header (240 bytes for PE32+) → 1 section. Total 408
    /// bytes.
    fn synth_minimal_pe() -> Vec<u8> {
        let mut buf = vec![0u8; 64 + 4 + 20 + 240 + 40];
        // DOS stub.
        buf[0] = b'M';
        buf[1] = b'Z';
        buf[0x3C..0x40].copy_from_slice(&64u32.to_le_bytes());
        // NT magic 'PE\0\0'.
        buf[64..68].copy_from_slice(b"PE\0\0");
        // COFF: machine = X86_64 (0x8664), 1 section.
        let coff = 64 + 4;
        buf[coff..coff + 2].copy_from_slice(&0x8664u16.to_le_bytes());
        buf[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes());
        buf[coff + 16..coff + 18].copy_from_slice(&240u16.to_le_bytes()); // SizeOfOptionalHeader
                                                                          // Optional header: PE32+ magic, image_base = 0x140000000.
        let opt = coff + 20;
        buf[opt..opt + 2].copy_from_slice(&0x020Bu16.to_le_bytes());
        buf[opt + 16..opt + 20].copy_from_slice(&0x1000u32.to_le_bytes()); // entry RVA
        buf[opt + 24..opt + 32].copy_from_slice(&0x1_4000_0000u64.to_le_bytes());
        buf[opt + 56..opt + 60].copy_from_slice(&0x10000u32.to_le_bytes()); // SizeOfImage
                                                                            // Section: ".text\0\0\0", VA=0x1000, size=0x1000.
        let sec = opt + 240;
        buf[sec..sec + 5].copy_from_slice(b".text");
        buf[sec + 8..sec + 12].copy_from_slice(&0x1000u32.to_le_bytes());
        buf[sec + 12..sec + 16].copy_from_slice(&0x1000u32.to_le_bytes());
        buf
    }

    #[test]
    fn parse_minimal_pe32_plus() {
        let buf = synth_minimal_pe();
        let img = parse(&buf).expect("parse");
        assert_eq!(img.machine, PeMachine::X86_64);
        assert_eq!(img.section_count, 1);
        assert_eq!(img.entry_point_rva, 0x1000);
        assert_eq!(img.image_base, 0x1_4000_0000);
        assert_eq!(img.sections.len(), 1);
        assert_eq!(&img.sections[0].name[..5], b".text");
    }

    #[test]
    fn rejects_bad_dos_magic() {
        let mut buf = synth_minimal_pe();
        buf[0] = b'Z';
        let r = parse(&buf);
        assert!(matches!(r, Err(PeError::BadDosMagic(_))));
    }

    #[test]
    fn rejects_bad_nt_magic() {
        let mut buf = synth_minimal_pe();
        buf[64] = b'Q';
        let r = parse(&buf);
        assert!(matches!(r, Err(PeError::BadNtMagic(_))));
    }

    #[test]
    fn rejects_truncated_input() {
        let buf = vec![0u8; 32];
        let r = parse(&buf);
        assert!(matches!(r, Err(PeError::Truncated(32))));
    }

    #[test]
    fn rejects_unknown_opt_header() {
        let mut buf = synth_minimal_pe();
        let opt = 64 + 4 + 20;
        buf[opt..opt + 2].copy_from_slice(&0x9999u16.to_le_bytes());
        let r = parse(&buf);
        assert!(matches!(r, Err(PeError::UnsupportedOptHeader(_))));
    }

    /// Synth PE with `code` appended as .text raw data at VA 0x1000.
    fn synth_pe_with_code(code: &[u8]) -> Vec<u8> {
        let mut buf = synth_minimal_pe();
        let sec = 64 + 4 + 20 + 240;
        let raw_off = u32::try_from(buf.len()).unwrap();
        let raw_size = u32::try_from(code.len()).unwrap();
        buf[sec + 16..sec + 20].copy_from_slice(&raw_size.to_le_bytes());
        buf[sec + 20..sec + 24].copy_from_slice(&raw_off.to_le_bytes());
        buf.extend_from_slice(code);
        buf
    }

    #[test]
    fn maps_section_raw_data_to_virtual_address() {
        let code = [0x48u8, 0x31, 0xC0, 0xC3]; // xor rax,rax ; ret
        let buf = synth_pe_with_code(&code);
        let img = parse(&buf).expect("parse");
        let mapped = map_image(&img, &buf).expect("map");

        assert_eq!(mapped.base, 0x1_4000_0000);
        assert_eq!(mapped.entry_pc, 0x1_4000_1000);
        assert_eq!(mapped.bytes.len(), 0x10000);
        assert_eq!(&mapped.bytes[0x1000..0x1000 + code.len()], &code);
        // Outside the section stays zero-filled.
        assert!(mapped.bytes[..0x1000].iter().all(|&b| b == 0));
    }

    #[test]
    fn map_rejects_section_past_image_end() {
        let buf = synth_pe_with_code(&[0xC3]);
        let mut img = parse(&buf).expect("parse");
        img.sections[0].virtual_address = img.size_of_image;
        let r = map_image(&img, &buf);
        assert!(matches!(r, Err(PeError::SectionOutOfImage(0, _, _, _))));
    }

    #[test]
    fn map_rejects_raw_data_past_file_end() {
        let buf = synth_pe_with_code(&[0xC3]);
        let mut img = parse(&buf).expect("parse");
        img.sections[0].raw_data_size = 0x4000;
        let r = map_image(&img, &buf);
        assert!(matches!(r, Err(PeError::SectionRawOutOfFile(0, _, _, _))));
    }

    #[test]
    fn map_rejects_entry_outside_image() {
        let buf = synth_pe_with_code(&[0xC3]);
        let mut img = parse(&buf).expect("parse");
        img.entry_point_rva = img.size_of_image;
        let r = map_image(&img, &buf);
        assert!(matches!(r, Err(PeError::EntryOutOfImage(_, _))));
    }
}
