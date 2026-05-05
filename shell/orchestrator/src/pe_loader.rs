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
    I386,    // 0x014C
    X86_64,  // 0x8664
    Arm64,   // 0xAA64
    Other(u16),
}

impl PeMachine {
    fn from_u16(m: u16) -> Self {
        match m {
            0x014C => Self::I386,
            0x8664 => Self::X86_64,
            0xAA64 => Self::Arm64,
            other  => Self::Other(other),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeSection {
    pub name:             [u8; 8],
    pub virtual_size:     u32,
    pub virtual_address:  u32,
    pub raw_data_size:    u32,
    pub raw_data_offset:  u32,
    pub characteristics:  u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeImage {
    pub machine:        PeMachine,
    pub section_count:  u16,
    pub timestamp:      u32,
    pub entry_point_rva: u32,
    pub image_base:     u64,
    pub size_of_image:  u32,
    pub sections:       Vec<PeSection>,
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
}

/// Parse a PE/COFF byte slice into a `PeImage`. Read-only; the
/// caller still owns the bytes.
pub fn parse(bytes: &[u8]) -> Result<PeImage, PeError> {
    // 1. DOS header — 64 bytes. e_magic at offset 0; e_lfanew at 0x3C.
    if bytes.len() < 64 {
        return Err(PeError::Truncated(bytes.len()));
    }
    let dos_magic = u16::from_le_bytes([bytes[0], bytes[1]]);
    if dos_magic != 0x5A4D {  // 'MZ' little-endian
        return Err(PeError::BadDosMagic(dos_magic));
    }
    let e_lfanew = u32::from_le_bytes([bytes[0x3C], bytes[0x3D], bytes[0x3E], bytes[0x3F]])
        as usize;
    if e_lfanew + 24 > bytes.len() {
        return Err(PeError::LfanewOutOfRange(e_lfanew, bytes.len()));
    }

    // 2. NT signature — 4 bytes 'PE\0\0' at e_lfanew.
    let nt_magic = u32::from_le_bytes([
        bytes[e_lfanew], bytes[e_lfanew + 1],
        bytes[e_lfanew + 2], bytes[e_lfanew + 3],
    ]);
    if nt_magic != 0x0000_4550 {  // 'PE\0\0'
        return Err(PeError::BadNtMagic(nt_magic));
    }

    // 3. COFF file header — 20 bytes after NT signature.
    let coff = e_lfanew + 4;
    let machine        = u16::from_le_bytes([bytes[coff],     bytes[coff + 1]]);
    let n_sections     = u16::from_le_bytes([bytes[coff + 2], bytes[coff + 3]]);
    let timestamp      = u32::from_le_bytes([
        bytes[coff + 4], bytes[coff + 5], bytes[coff + 6], bytes[coff + 7]
    ]);
    let opt_hdr_size   = u16::from_le_bytes([bytes[coff + 16], bytes[coff + 17]]) as usize;

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
                bytes[opt + 16], bytes[opt + 17], bytes[opt + 18], bytes[opt + 19]
            ]);
            let base = u32::from_le_bytes([
                bytes[opt + 28], bytes[opt + 29], bytes[opt + 30], bytes[opt + 31]
            ]) as u64;
            let sz = u32::from_le_bytes([
                bytes[opt + 56], bytes[opt + 57], bytes[opt + 58], bytes[opt + 59]
            ]);
            (entry, base, sz)
        }
        0x020B => {
            // PE32+: image_base is u64 at offset 24.
            let entry = u32::from_le_bytes([
                bytes[opt + 16], bytes[opt + 17], bytes[opt + 18], bytes[opt + 19]
            ]);
            let mut base_buf = [0u8; 8];
            base_buf.copy_from_slice(&bytes[opt + 24 .. opt + 32]);
            let base = u64::from_le_bytes(base_buf);
            let sz = u32::from_le_bytes([
                bytes[opt + 56], bytes[opt + 57], bytes[opt + 58], bytes[opt + 59]
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
        name.copy_from_slice(&bytes[sec_off .. sec_off + 8]);
        let virtual_size    = u32::from_le_bytes(bytes[sec_off + 8 .. sec_off + 12].try_into().unwrap());
        let virtual_address = u32::from_le_bytes(bytes[sec_off + 12 .. sec_off + 16].try_into().unwrap());
        let raw_data_size   = u32::from_le_bytes(bytes[sec_off + 16 .. sec_off + 20].try_into().unwrap());
        let raw_data_offset = u32::from_le_bytes(bytes[sec_off + 20 .. sec_off + 24].try_into().unwrap());
        let characteristics = u32::from_le_bytes(bytes[sec_off + 36 .. sec_off + 40].try_into().unwrap());
        sections.push(PeSection {
            name, virtual_size, virtual_address,
            raw_data_size, raw_data_offset, characteristics,
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
    /// DOS stub (64 bytes) → e_lfanew=64 → NT magic → COFF header →
    /// optional header (240 bytes for PE32+) → 1 section. Total 408
    /// bytes.
    fn synth_minimal_pe() -> Vec<u8> {
        let mut buf = vec![0u8; 64 + 4 + 20 + 240 + 40];
        // DOS stub.
        buf[0] = b'M'; buf[1] = b'Z';
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
}
