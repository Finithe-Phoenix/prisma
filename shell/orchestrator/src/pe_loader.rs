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
//! Status: parser. Reads the DOS stub + NT headers + the section table,
//! validates magic numbers, returns a `PeImage` with
//! `(virtual_address, size, characteristics)` per section, parses the
//! import directory ([`parse_imports`] → per-DLL symbols by name/ordinal),
//! and applies base relocations ([`apply_relocations`] → rebase a mapped
//! image). No import *resolution* (binding to a loaded DLL), no execution.
//! Enough to:
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

/// One imported symbol: by name (with its hint dropped) or by ordinal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ImportSymbol {
    Name(String),
    Ordinal(u16),
}

/// One imported DLL and the symbols pulled from it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeImport {
    pub dll: String,
    pub symbols: Vec<ImportSymbol>,
}

/// One named symbol a PE/DLL exports: its name, the RVA it resolves to, and its
/// export ordinal. Resolving an import against a DLL matches the import name to
/// one of these.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeExport {
    pub name: String,
    pub rva: u32,
    pub ordinal: u16,
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
    /// `true` for PE32+ (0x020B), `false` for PE32 (0x010B). Decides the
    /// import-thunk width (8 vs 4 bytes).
    pub pe32_plus: bool,
    /// RVA + size of the import data directory (entry 1). Zero RVA means the
    /// image declares no imports.
    pub import_dir_rva: u32,
    pub import_dir_size: u32,
    /// RVA + size of the base relocation directory (entry 5). Zero RVA means
    /// the image cannot be rebased (no `.reloc`).
    pub reloc_dir_rva: u32,
    pub reloc_dir_size: u32,
    /// RVA + size of the export directory (entry 0). Zero RVA means the image
    /// exports nothing (a typical `.exe`); DLLs populate it.
    pub export_dir_rva: u32,
    pub export_dir_size: u32,
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

    #[error("size_of_image {0:#x} exceeds the loader's maximum mappable size")]
    ImageTooLarge(u32),

    #[error("import directory RVA {0:#x} does not map into any section's file data")]
    ImportRvaUnmapped(u32),

    #[error("import name/string at {0:#x} is not NUL-terminated within bounds")]
    ImportStringUnterminated(u32),

    #[error("export directory RVA {0:#x} does not map into any section's file data")]
    ExportRvaUnmapped(u32),

    #[error("relocation fixup at RVA {0:#x} lies outside the mapped image")]
    RelocOutOfImage(u32),

    #[error("relocation block at page {0:#x} has an invalid size")]
    RelocBlockTruncated(u32),

    #[error("unsupported relocation type {0}")]
    RelocUnsupportedType(u16),
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
/// Zero-filled `size_of_image` bytes at `image_base`, each section's raw data
/// copied to its virtual address. All bounds come from untrusted input, so
/// every offset is checked and `size_of_image` is capped (malformed sections
/// or an oversized image error instead of truncating or over-allocating).
/// Relocations and imports are applied separately ([`apply_relocations`] /
/// [`parse_imports`]).
pub fn map_image(img: &PeImage, file: &[u8]) -> Result<MappedImage, PeError> {
    // size_of_image is attacker-controlled (straight from the optional header),
    // so cap the up-front zero-fill: a crafted header must not be able to force
    // a multi-gigabyte allocation (DoS). 1 GiB is far beyond any image we map
    // at this stage; raise it deliberately if a real workload needs more.
    const MAX_IMAGE_SIZE: u32 = 1 << 30;
    if img.size_of_image > MAX_IMAGE_SIZE {
        return Err(PeError::ImageTooLarge(img.size_of_image));
    }
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

    // 4b. Data directories — the import directory is entry 1. PE32 places the
    //     NumberOfRvaAndSizes field at opt+92 and the array at opt+96; PE32+ at
    //     opt+108 / opt+112. Every read is guarded against both the declared
    //     optional-header size and the file length (untrusted input).
    let pe32_plus = opt_magic == 0x020B;
    let (numdirs_off, datadir_off) = if pe32_plus {
        (opt + 108, opt + 112)
    } else {
        (opt + 92, opt + 96)
    };
    let opt_end = opt + opt_hdr_size;
    let read_u32_at = |off: usize| -> Option<u32> {
        if off + 4 <= opt_end && off + 4 <= bytes.len() {
            Some(u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap()))
        } else {
            None
        }
    };
    let num_dirs = read_u32_at(numdirs_off).unwrap_or(0);
    let import_entry = datadir_off + 8; // data directory index 1
    let (import_dir_rva, import_dir_size) = if num_dirs >= 2 {
        (
            read_u32_at(import_entry).unwrap_or(0),
            read_u32_at(import_entry + 4).unwrap_or(0),
        )
    } else {
        (0, 0)
    };
    let reloc_entry = datadir_off + 5 * 8; // data directory index 5 (base reloc)
    let (reloc_dir_rva, reloc_dir_size) = if num_dirs >= 6 {
        (
            read_u32_at(reloc_entry).unwrap_or(0),
            read_u32_at(reloc_entry + 4).unwrap_or(0),
        )
    } else {
        (0, 0)
    };
    // Export directory is data directory index 0 (offset datadir_off + 0).
    let (export_dir_rva, export_dir_size) = if num_dirs >= 1 {
        (
            read_u32_at(datadir_off).unwrap_or(0),
            read_u32_at(datadir_off + 4).unwrap_or(0),
        )
    } else {
        (0, 0)
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
        pe32_plus,
        import_dir_rva,
        import_dir_size,
        reloc_dir_rva,
        reloc_dir_size,
        export_dir_rva,
        export_dir_size,
    })
}

/// Apply base relocations to a mapped image, rebasing it to `new_base`.
///
/// Walks the `.reloc` blocks and adds `new_base - image_base` to every HIGHLOW
/// (32-bit) and DIR64 (64-bit) fixup, returning the count applied. A zero delta
/// or an image with no relocation directory is a no-op. Every offset is
/// bounds-checked against the mapped image; malformed blocks error rather than
/// corrupt memory.
pub fn apply_relocations(
    img: &PeImage,
    mapped: &mut MappedImage,
    new_base: u64,
) -> Result<usize, PeError> {
    if img.reloc_dir_rva == 0 || new_base == img.image_base {
        return Ok(0);
    }
    let delta = new_base.wrapping_sub(img.image_base);
    let delta32 = (delta & 0xFFFF_FFFF) as u32;
    let image = &mut mapped.bytes;
    let dir_start = img.reloc_dir_rva as usize;
    let dir_end = dir_start
        .checked_add(img.reloc_dir_size as usize)
        .filter(|&end| end <= image.len())
        .ok_or(PeError::RelocOutOfImage(img.reloc_dir_rva))?;

    // Each block: { page_rva: u32, block_size: u32, entries: [u16] } where an
    // entry's high 4 bits are the type and its low 12 bits a page offset.
    let mut off = dir_start;
    let mut applied = 0usize;
    while off + 8 <= dir_end {
        let page_rva = u32::from_le_bytes(image[off..off + 4].try_into().unwrap());
        let block_size = u32::from_le_bytes(image[off + 4..off + 8].try_into().unwrap()) as usize;
        if block_size < 8 || off + block_size > dir_end {
            return Err(PeError::RelocBlockTruncated(page_rva));
        }
        let entries = (block_size - 8) / 2;
        for e in 0..entries {
            let eo = off + 8 + e * 2;
            let raw = u16::from_le_bytes(image[eo..eo + 2].try_into().unwrap());
            let typ = raw >> 12;
            let target_rva = page_rva.wrapping_add(u32::from(raw & 0x0FFF));
            let target = target_rva as usize;
            match typ {
                0 => {} // ABSOLUTE — padding, skip
                3 => {
                    let end = target
                        .checked_add(4)
                        .filter(|&x| x <= image.len())
                        .ok_or(PeError::RelocOutOfImage(target_rva))?;
                    let v = u32::from_le_bytes(image[target..end].try_into().unwrap());
                    image[target..end].copy_from_slice(&v.wrapping_add(delta32).to_le_bytes());
                    applied += 1;
                }
                10 => {
                    let end = target
                        .checked_add(8)
                        .filter(|&x| x <= image.len())
                        .ok_or(PeError::RelocOutOfImage(target_rva))?;
                    let v = u64::from_le_bytes(image[target..end].try_into().unwrap());
                    image[target..end].copy_from_slice(&v.wrapping_add(delta).to_le_bytes());
                    applied += 1;
                }
                _ => return Err(PeError::RelocUnsupportedType(typ)),
            }
        }
        off += block_size;
    }

    // Reflect the rebase in the image's load-address bookkeeping.
    mapped.base = new_base;
    mapped.entry_pc = new_base + u64::from(img.entry_point_rva);
    Ok(applied)
}

/// Translate a guest RVA to a file offset using the section table.
///
/// Returns `None` if the RVA falls outside every section or lands in a
/// section's purely-virtual tail (no backing file bytes).
fn rva_to_file_offset(img: &PeImage, rva: u32) -> Option<usize> {
    for sec in &img.sections {
        let span = sec.virtual_size.max(sec.raw_data_size);
        if rva >= sec.virtual_address && rva - sec.virtual_address < span {
            let delta = rva - sec.virtual_address;
            if delta < sec.raw_data_size {
                return Some(sec.raw_data_offset as usize + delta as usize);
            }
            return None;
        }
    }
    None
}

/// Read a NUL-terminated ASCII string starting at `off`.
///
/// Scans at most `max` bytes. Non-UTF-8 bytes are replaced (names are ASCII).
fn read_cstr(file: &[u8], off: usize, max: usize) -> Option<String> {
    let slice = file.get(off..)?;
    let len = slice.iter().take(max).position(|&b| b == 0)?;
    Some(String::from_utf8_lossy(&slice[..len]).into_owned())
}

/// Walk an import lookup table (ILT/IAT) at `thunk_rva` into symbols.
fn parse_thunks(img: &PeImage, file: &[u8], thunk_rva: u32) -> Result<Vec<ImportSymbol>, PeError> {
    // A crafted unterminated table is bounded by this cap rather than the
    // file size so the loop cannot spin on pathological input.
    const MAX_SYMBOLS: usize = 65_536;
    let mut off =
        rva_to_file_offset(img, thunk_rva).ok_or(PeError::ImportRvaUnmapped(thunk_rva))?;
    let mut symbols = Vec::new();
    for _ in 0..MAX_SYMBOLS {
        let (thunk, ordinal_flag, width) = if img.pe32_plus {
            let raw = file
                .get(off..off + 8)
                .ok_or(PeError::ImportRvaUnmapped(thunk_rva))?;
            (u64::from_le_bytes(raw.try_into().unwrap()), 1u64 << 63, 8)
        } else {
            let raw = file
                .get(off..off + 4)
                .ok_or(PeError::ImportRvaUnmapped(thunk_rva))?;
            (
                u64::from(u32::from_le_bytes(raw.try_into().unwrap())),
                1u64 << 31,
                4,
            )
        };
        if thunk == 0 {
            break; // null terminator ends the table
        }
        if thunk & ordinal_flag != 0 {
            symbols.push(ImportSymbol::Ordinal((thunk & 0xFFFF) as u16));
        } else {
            // Low 31 bits are an RVA to IMAGE_IMPORT_BY_NAME { hint: u16, name }.
            let by_name_rva = (thunk & 0x7FFF_FFFF) as u32;
            let name_off = rva_to_file_offset(img, by_name_rva)
                .ok_or(PeError::ImportRvaUnmapped(by_name_rva))?;
            let name = read_cstr(file, name_off + 2, 1024)
                .ok_or(PeError::ImportStringUnterminated(by_name_rva))?;
            symbols.push(ImportSymbol::Name(name));
        }
        off += width;
    }
    Ok(symbols)
}

/// Parse the PE import directory into a per-DLL symbol list.
///
/// Returns an empty vector when the image declares no imports. Every offset
/// derives from untrusted input and is bounds-checked; malformed tables error
/// rather than truncate silently.
pub fn parse_imports(img: &PeImage, file: &[u8]) -> Result<Vec<PeImport>, PeError> {
    // IMAGE_IMPORT_DESCRIPTOR is 20 bytes, terminated by an all-zero entry.
    const MAX_DLLS: usize = 8_192;
    if img.import_dir_rva == 0 {
        return Ok(Vec::new());
    }
    let mut desc_off = rva_to_file_offset(img, img.import_dir_rva)
        .ok_or(PeError::ImportRvaUnmapped(img.import_dir_rva))?;
    let mut imports = Vec::new();
    for _ in 0..MAX_DLLS {
        let desc = file
            .get(desc_off..desc_off + 20)
            .ok_or(PeError::ImportRvaUnmapped(img.import_dir_rva))?;
        let original_first_thunk = u32::from_le_bytes(desc[0..4].try_into().unwrap());
        let name_rva = u32::from_le_bytes(desc[12..16].try_into().unwrap());
        let first_thunk = u32::from_le_bytes(desc[16..20].try_into().unwrap());
        if original_first_thunk == 0 && name_rva == 0 && first_thunk == 0 {
            break; // null descriptor terminates the array
        }
        let name_off =
            rva_to_file_offset(img, name_rva).ok_or(PeError::ImportRvaUnmapped(name_rva))?;
        let dll =
            read_cstr(file, name_off, 1024).ok_or(PeError::ImportStringUnterminated(name_rva))?;
        // Prefer the import lookup table (OFT); bound DLLs may have only the IAT.
        let thunk_rva = if original_first_thunk != 0 {
            original_first_thunk
        } else {
            first_thunk
        };
        let symbols = if thunk_rva == 0 {
            Vec::new()
        } else {
            parse_thunks(img, file, thunk_rva)?
        };
        imports.push(PeImport { dll, symbols });
        desc_off += 20;
    }
    Ok(imports)
}

/// Parse the export directory into the image's named exports.
///
/// Returns an empty vector when the image exports nothing (a typical `.exe`);
/// DLLs populate it. Only named exports are returned (by-ordinal-only exports
/// are skipped). Every offset derives from untrusted input and is
/// bounds-checked; malformed tables error rather than crash or over-read.
pub fn parse_exports(img: &PeImage, file: &[u8]) -> Result<Vec<PeExport>, PeError> {
    const MAX_NAMES: u32 = 1 << 20;
    if img.export_dir_rva == 0 {
        return Ok(Vec::new());
    }
    let dir = rva_to_file_offset(img, img.export_dir_rva)
        .ok_or(PeError::ExportRvaUnmapped(img.export_dir_rva))?;
    // IMAGE_EXPORT_DIRECTORY is 40 bytes.
    let hdr = file
        .get(dir..dir + 40)
        .ok_or(PeError::ExportRvaUnmapped(img.export_dir_rva))?;
    let base = u32::from_le_bytes(hdr[16..20].try_into().unwrap());
    let num_funcs = u32::from_le_bytes(hdr[20..24].try_into().unwrap());
    let num_names = u32::from_le_bytes(hdr[24..28].try_into().unwrap());
    let funcs_rva = u32::from_le_bytes(hdr[28..32].try_into().unwrap());
    let names_rva = u32::from_le_bytes(hdr[32..36].try_into().unwrap());
    let ords_rva = u32::from_le_bytes(hdr[36..40].try_into().unwrap());
    if num_names == 0 {
        return Ok(Vec::new());
    }

    let names_off =
        rva_to_file_offset(img, names_rva).ok_or(PeError::ExportRvaUnmapped(names_rva))?;
    let ords_off = rva_to_file_offset(img, ords_rva).ok_or(PeError::ExportRvaUnmapped(ords_rva))?;
    let funcs_off =
        rva_to_file_offset(img, funcs_rva).ok_or(PeError::ExportRvaUnmapped(funcs_rva))?;

    let mut exports = Vec::new();
    for i in 0..num_names.min(MAX_NAMES) as usize {
        let sym_rva = u32::from_le_bytes(
            file.get(names_off + i * 4..names_off + i * 4 + 4)
                .ok_or(PeError::ExportRvaUnmapped(names_rva))?
                .try_into()
                .unwrap(),
        );
        let sym_off =
            rva_to_file_offset(img, sym_rva).ok_or(PeError::ExportRvaUnmapped(sym_rva))?;
        let sym_name =
            read_cstr(file, sym_off, 1024).ok_or(PeError::ImportStringUnterminated(sym_rva))?;
        let idx = u16::from_le_bytes(
            file.get(ords_off + i * 2..ords_off + i * 2 + 2)
                .ok_or(PeError::ExportRvaUnmapped(ords_rva))?
                .try_into()
                .unwrap(),
        );
        // An ordinal index past the function table is malformed — skip it.
        if u32::from(idx) >= num_funcs {
            continue;
        }
        let slot = funcs_off + idx as usize * 4;
        let rva = u32::from_le_bytes(
            file.get(slot..slot + 4)
                .ok_or(PeError::ExportRvaUnmapped(funcs_rva))?
                .try_into()
                .unwrap(),
        );
        let ordinal = (base.wrapping_add(u32::from(idx)) & 0xFFFF) as u16;
        exports.push(PeExport {
            name: sym_name,
            rva,
            ordinal,
        });
    }
    Ok(exports)
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

    #[test]
    fn map_rejects_oversized_size_of_image() {
        // A crafted 2 GiB size_of_image must error, not attempt the allocation.
        let mut buf = synth_minimal_pe();
        let opt = 64 + 4 + 20;
        buf[opt + 56..opt + 60].copy_from_slice(&0x8000_0000u32.to_le_bytes());
        let img = parse(&buf).expect("parse");
        let r = map_image(&img, &buf);
        assert!(matches!(r, Err(PeError::ImageTooLarge(0x8000_0000))));
    }

    /// PE32+ whose `.text` section (VA 0x1000) carries a hand-laid import
    /// directory: one DLL (KERNEL32.dll) importing `GetProcAddress` by name
    /// and ordinal 7. The optional header's data directory[1] points at it.
    fn synth_pe_with_imports() -> Vec<u8> {
        let mut buf = synth_minimal_pe();
        let opt = 64 + 4 + 20;
        let sec = opt + 240;
        // 16 data directories; import dir (index 1) -> RVA 0x1000, size 40.
        buf[opt + 108..opt + 112].copy_from_slice(&16u32.to_le_bytes());
        buf[opt + 120..opt + 124].copy_from_slice(&0x1000u32.to_le_bytes());
        buf[opt + 124..opt + 128].copy_from_slice(&40u32.to_le_bytes());
        // .text raw data now backs VA 0x1000 with the import payload.
        let raw_off = u32::try_from(buf.len()).unwrap();
        buf[sec + 16..sec + 20].copy_from_slice(&0x100u32.to_le_bytes()); // raw_data_size
        buf[sec + 20..sec + 24].copy_from_slice(&raw_off.to_le_bytes()); // raw_data_offset

        let mut idata = vec![0u8; 0x100];
        // IMAGE_IMPORT_DESCRIPTOR[0]: OFT=0x1040, Name=0x1060, FT=0.
        idata[0..4].copy_from_slice(&0x1040u32.to_le_bytes());
        idata[12..16].copy_from_slice(&0x1060u32.to_le_bytes());
        // [1] is the all-zero terminator (0x14..0x28).
        // ILT at 0x40 (PE32+ u64 thunks): by-name 0x1080, ordinal 7, terminator.
        idata[0x40..0x48].copy_from_slice(&0x1080u64.to_le_bytes());
        idata[0x48..0x50].copy_from_slice(&((1u64 << 63) | 7).to_le_bytes());
        // DLL name at 0x60.
        idata[0x60..0x6D].copy_from_slice(b"KERNEL32.dll\0");
        // IMAGE_IMPORT_BY_NAME at 0x80: hint(u16)=0, "GetProcAddress\0".
        idata[0x82..0x91].copy_from_slice(b"GetProcAddress\0");

        buf.extend_from_slice(&idata);
        buf
    }

    #[test]
    fn parses_import_directory() {
        let buf = synth_pe_with_imports();
        let img = parse(&buf).expect("parse");
        assert!(img.pe32_plus);
        assert_eq!(img.import_dir_rva, 0x1000);
        assert_eq!(img.import_dir_size, 40);

        let imports = parse_imports(&img, &buf).expect("imports");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].dll, "KERNEL32.dll");
        assert_eq!(
            imports[0].symbols,
            vec![
                ImportSymbol::Name("GetProcAddress".to_owned()),
                ImportSymbol::Ordinal(7),
            ]
        );
    }

    #[test]
    fn no_imports_when_directory_absent() {
        let buf = synth_minimal_pe();
        let img = parse(&buf).expect("parse");
        assert_eq!(img.import_dir_rva, 0);
        assert!(parse_imports(&img, &buf).expect("imports").is_empty());
    }

    #[test]
    fn import_rva_outside_sections_errors() {
        let buf = synth_pe_with_imports();
        let mut img = parse(&buf).expect("parse");
        img.import_dir_rva = 0xDEAD_BEEF; // maps into no section
        let r = parse_imports(&img, &buf);
        assert!(matches!(r, Err(PeError::ImportRvaUnmapped(0xDEAD_BEEF))));
    }

    #[test]
    fn unterminated_dll_name_errors() {
        let mut buf = synth_pe_with_imports();
        // Fill the DLL-name region (file offset of VA 0x1060) with non-NUL so
        // the scan never terminates within bounds.
        let img = parse(&buf).expect("parse");
        let name_off = rva_to_file_offset(&img, 0x1060).unwrap();
        // No NUL from the DLL name to EOF -> the scan can never terminate.
        for b in &mut buf[name_off..] {
            *b = b'A';
        }
        let r = parse_imports(&img, &buf);
        assert!(matches!(r, Err(PeError::ImportStringUnterminated(0x1060))));
    }

    /// PE32+ whose `.text` (VA 0x1000) holds a DIR64 pointer (`image_base` +
    /// 0x1234) at RVA 0x1000 and a one-entry `.reloc` block at RVA 0x1080
    /// targeting it. Data directory[5] points at the block.
    fn synth_pe_with_reloc() -> Vec<u8> {
        let mut buf = synth_minimal_pe();
        let opt = 64 + 4 + 20;
        let sec = opt + 240;
        // 16 data directories; base-reloc dir (index 5) -> RVA 0x1080, size 10.
        buf[opt + 108..opt + 112].copy_from_slice(&16u32.to_le_bytes());
        buf[opt + 152..opt + 156].copy_from_slice(&0x1080u32.to_le_bytes());
        buf[opt + 156..opt + 160].copy_from_slice(&10u32.to_le_bytes());
        let raw_off = u32::try_from(buf.len()).unwrap();
        buf[sec + 16..sec + 20].copy_from_slice(&0x100u32.to_le_bytes()); // raw_data_size
        buf[sec + 20..sec + 24].copy_from_slice(&raw_off.to_le_bytes()); // raw_data_offset

        let mut idata = vec![0u8; 0x100];
        // DIR64 pointer at RVA 0x1000 = image_base + 0x1234.
        idata[0..8].copy_from_slice(&(0x1_4000_0000u64 + 0x1234).to_le_bytes());
        // .reloc block at RVA 0x1080: page 0x1000, size 10, one DIR64 entry @ off 0.
        idata[0x80..0x84].copy_from_slice(&0x1000u32.to_le_bytes()); // page_rva
        idata[0x84..0x88].copy_from_slice(&10u32.to_le_bytes()); // block_size
        idata[0x88..0x8A].copy_from_slice(&(10u16 << 12).to_le_bytes()); // type=10, off=0

        buf.extend_from_slice(&idata);
        buf
    }

    #[test]
    fn applies_dir64_base_relocation() {
        let buf = synth_pe_with_reloc();
        let img = parse(&buf).expect("parse");
        assert_eq!(img.reloc_dir_rva, 0x1080);

        let mut mapped = map_image(&img, &buf).expect("map");
        let before = u64::from_le_bytes(mapped.bytes[0x1000..0x1008].try_into().unwrap());
        assert_eq!(before, 0x1_4000_1234);

        let new_base = 0x1_8000_0000u64;
        let applied = apply_relocations(&img, &mut mapped, new_base).expect("reloc");
        assert_eq!(applied, 1);
        let after = u64::from_le_bytes(mapped.bytes[0x1000..0x1008].try_into().unwrap());
        assert_eq!(after, 0x1_8000_1234);
        assert_eq!(mapped.base, new_base);
        assert_eq!(mapped.entry_pc, new_base + 0x1000);
    }

    #[test]
    fn relocation_noop_without_directory_or_delta() {
        // No reloc directory at all.
        let buf = synth_minimal_pe();
        let img = parse(&buf).expect("parse");
        let mut mapped = map_image(&img, &buf).expect("map");
        assert_eq!(
            apply_relocations(&img, &mut mapped, 0xDEAD_0000).expect("reloc"),
            0
        );
        // Directory present but new_base == image_base -> zero delta.
        let buf2 = synth_pe_with_reloc();
        let img2 = parse(&buf2).expect("parse");
        let mut m2 = map_image(&img2, &buf2).expect("map");
        assert_eq!(
            apply_relocations(&img2, &mut m2, img2.image_base).expect("reloc"),
            0
        );
    }

    #[test]
    fn malformed_reloc_block_errors() {
        let mut buf = synth_pe_with_reloc();
        let img = parse(&buf).expect("parse");
        // Corrupt block_size to 4 (< 8) at RVA 0x1084.
        let blk_off = rva_to_file_offset(&img, 0x1084).unwrap();
        buf[blk_off..blk_off + 4].copy_from_slice(&4u32.to_le_bytes());
        let mut mapped = map_image(&img, &buf).expect("map");
        let r = apply_relocations(&img, &mut mapped, 0x1_8000_0000);
        assert!(matches!(r, Err(PeError::RelocBlockTruncated(_))));
    }

    /// PE32+ whose `.text` (VA 0x1000) carries an export directory exporting one
    /// named symbol `MyFunc` at RVA 0x2000, ordinal base 1. Data directory[0]
    /// points at it.
    fn synth_pe_with_exports() -> Vec<u8> {
        let mut buf = synth_minimal_pe();
        let opt = 64 + 4 + 20;
        let sec = opt + 240;
        buf[opt + 108..opt + 112].copy_from_slice(&16u32.to_le_bytes()); // NumberOfRvaAndSizes
        buf[opt + 112..opt + 116].copy_from_slice(&0x1000u32.to_le_bytes()); // export dir RVA (idx 0)
        buf[opt + 116..opt + 120].copy_from_slice(&40u32.to_le_bytes()); // export dir size
        let raw_off = u32::try_from(buf.len()).unwrap();
        buf[sec + 16..sec + 20].copy_from_slice(&0x200u32.to_le_bytes()); // raw_data_size
        buf[sec + 20..sec + 24].copy_from_slice(&raw_off.to_le_bytes()); // raw_data_offset

        let mut d = vec![0u8; 0x200];
        // IMAGE_EXPORT_DIRECTORY at RVA 0x1000.
        d[12..16].copy_from_slice(&0x1080u32.to_le_bytes()); // Name
        d[16..20].copy_from_slice(&1u32.to_le_bytes()); // Base (ordinal base)
        d[20..24].copy_from_slice(&1u32.to_le_bytes()); // NumberOfFunctions
        d[24..28].copy_from_slice(&1u32.to_le_bytes()); // NumberOfNames
        d[28..32].copy_from_slice(&0x1040u32.to_le_bytes()); // AddressOfFunctions
        d[32..36].copy_from_slice(&0x1050u32.to_le_bytes()); // AddressOfNames
        d[36..40].copy_from_slice(&0x1060u32.to_le_bytes()); // AddressOfNameOrdinals
        d[0x40..0x44].copy_from_slice(&0x2000u32.to_le_bytes()); // functions[0] = RVA 0x2000
        d[0x50..0x54].copy_from_slice(&0x1090u32.to_le_bytes()); // names[0] -> name RVA
        d[0x60..0x62].copy_from_slice(&0u16.to_le_bytes()); // ordinals[0] = function index 0
        d[0x80..0x8A].copy_from_slice(b"MyLib.dll\0"); // DLL name (unused by us)
        d[0x90..0x97].copy_from_slice(b"MyFunc\0"); // exported name

        buf.extend_from_slice(&d);
        buf
    }

    #[test]
    fn parses_export_directory() {
        let buf = synth_pe_with_exports();
        let img = parse(&buf).expect("parse");
        assert_eq!(img.export_dir_rva, 0x1000);
        let exports = parse_exports(&img, &buf).expect("exports");
        assert_eq!(
            exports,
            vec![PeExport {
                name: "MyFunc".to_owned(),
                rva: 0x2000,
                ordinal: 1,
            }]
        );
    }

    #[test]
    fn no_exports_when_directory_absent() {
        let buf = synth_minimal_pe();
        let img = parse(&buf).expect("parse");
        assert_eq!(img.export_dir_rva, 0);
        assert!(parse_exports(&img, &buf).expect("exports").is_empty());
    }

    #[test]
    fn export_rva_outside_sections_errors() {
        let buf = synth_pe_with_exports();
        let mut img = parse(&buf).expect("parse");
        img.export_dir_rva = 0xDEAD_BEEF;
        assert!(matches!(
            parse_exports(&img, &buf),
            Err(PeError::ExportRvaUnmapped(0xDEAD_BEEF))
        ));
    }
}
