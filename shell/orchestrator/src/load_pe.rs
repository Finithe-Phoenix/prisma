//! `load_pe`: drive a PE through the whole load pipeline in one call.
//!
//! Ties the pieces built separately into the path a real load takes: parse the
//! headers, map the image into guest memory, apply base relocations, resolve
//! each import against the loaded-module table, and patch the resolved
//! addresses into the IAT. The result is a [`MappedImage`] whose import table is
//! bound and which is ready to hand to the translator at its entry PC.

use crate::iat_patch::{apply_iat_patches, IatError, IatPatch};
use crate::import_resolver::{resolve_imports, ImportError, ImportRequest};
use crate::module_table::ModuleTable;
use crate::pe_loader::{self, ImportSymbol, MappedImage, PeError};

/// Load `file` against the already-loaded `modules`, returning the mapped,
/// relocated, import-bound image.
///
/// Imports bind by name or by ordinal against the module table. An import that
/// cannot be resolved fails the whole load (`LoadError::Import`) instead of
/// leaving a dangling slot the guest would crash on later.
pub fn load_pe(file: &[u8], modules: &ModuleTable) -> Result<MappedImage, LoadError> {
    load_pe_with_image(file, modules).map(|(_, mapped)| mapped)
}

/// Like [`load_pe`], but also returns the parsed [`pe_loader::PeImage`].
///
/// The caller can then lay the image out PER SECTION (W^X) via
/// [`crate::guest_layout::layout_sections`] instead of a flat blanket-RWX
/// region — the section table is needed for the protections.
pub fn load_pe_with_image(
    file: &[u8],
    modules: &ModuleTable,
) -> Result<(pe_loader::PeImage, MappedImage), LoadError> {
    let img = pe_loader::parse(file)?;
    let mut mapped = pe_loader::map_image(&img, file)?;
    // Mapped at the preferred base, so relocations are an identity no-op here;
    // apply unconditionally so a future rebasing map_image stays correct.
    let base = mapped.base;
    pe_loader::apply_relocations(&img, &mut mapped, base)?;

    let imports = pe_loader::parse_imports(&img, file)?;
    let thunk_width: u32 = if img.pe32_plus { 8 } else { 4 };

    let mut patches: Vec<IatPatch> = Vec::new();
    for imp in &imports {
        for (index, sym) in imp.symbols.iter().enumerate() {
            let request = [match sym {
                ImportSymbol::Name(n) => ImportRequest::new(imp.dll.clone(), n.clone()),
                ImportSymbol::Ordinal(o) => ImportRequest::by_ordinal(imp.dll.clone(), *o),
            }];
            let resolved = resolve_imports(&request, modules)?;
            let slot_rva =
                imp.iat_slot_rva(index, thunk_width)
                    .ok_or_else(|| LoadError::IatSlotOverflow {
                        dll: imp.dll.clone(),
                    })?;
            let slot_addr = base.checked_add(u64::from(slot_rva)).ok_or_else(|| {
                LoadError::IatSlotOverflow {
                    dll: imp.dll.clone(),
                }
            })?;
            patches.push(IatPatch {
                slot_va: slot_addr,
                value: resolved[0].address,
            });
        }
    }

    apply_iat_patches(base, &mut mapped.bytes, &patches)?;
    Ok((img, mapped))
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("PE parse/map: {0}")]
    Pe(#[from] PeError),

    #[error("import resolution: {0}")]
    Import(#[from] ImportError),

    #[error("IAT patch: {0}")]
    Iat(#[from] IatError),

    #[error("IAT slot RVA overflows the image for {dll}")]
    IatSlotOverflow { dll: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal valid PE32+ with one `.text` section and no imports.
    fn minimal_pe() -> Vec<u8> {
        let mut buf = vec![0u8; 64 + 4 + 20 + 240 + 40];
        buf[0] = b'M';
        buf[1] = b'Z';
        buf[0x3C..0x40].copy_from_slice(&64u32.to_le_bytes());
        buf[64..68].copy_from_slice(b"PE\0\0");
        let coff = 68;
        buf[coff..coff + 2].copy_from_slice(&0x8664u16.to_le_bytes()); // machine
        buf[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes()); // 1 section
        buf[coff + 16..coff + 18].copy_from_slice(&240u16.to_le_bytes()); // SizeOfOptionalHeader
        let opt = coff + 20;
        buf[opt..opt + 2].copy_from_slice(&0x020Bu16.to_le_bytes()); // PE32+
        buf[opt + 16..opt + 20].copy_from_slice(&0x1000u32.to_le_bytes()); // entry RVA
        buf[opt + 24..opt + 32].copy_from_slice(&0x1_4000_0000u64.to_le_bytes()); // image_base
        buf[opt + 56..opt + 60].copy_from_slice(&0x10000u32.to_le_bytes()); // SizeOfImage
        let sec = opt + 240;
        buf[sec..sec + 5].copy_from_slice(b".text");
        buf[sec + 8..sec + 12].copy_from_slice(&0x1000u32.to_le_bytes()); // virtual_size
        buf[sec + 12..sec + 16].copy_from_slice(&0x1000u32.to_le_bytes()); // virtual_address
        buf
    }

    #[test]
    fn loads_an_import_free_pe_to_a_mapped_image() {
        let pe = minimal_pe();
        let modules = ModuleTable::new();
        let mapped = load_pe(&pe, &modules).expect("import-free PE loads");
        assert_eq!(mapped.base, 0x1_4000_0000);
        assert_eq!(mapped.entry_pc, 0x1_4000_0000 + 0x1000);
        assert_eq!(mapped.bytes.len(), 0x10000);
    }

    #[test]
    fn garbage_input_fails_as_a_pe_error() {
        let modules = ModuleTable::new();
        let err = load_pe(&[0u8; 8], &modules).unwrap_err();
        assert!(matches!(err, LoadError::Pe(_)));
    }

    #[test]
    fn load_pe_with_image_returns_the_section_table() {
        let (img, mapped) = load_pe_with_image(&minimal_pe(), &ModuleTable::new()).expect("load");
        // The parsed image exposes the sections (for per-section W^X layout),
        // and the mapped image matches the plain load_pe path.
        assert_eq!(img.sections.len(), 1);
        assert_eq!(img.sections[0].virtual_address, 0x1000);
        assert_eq!(mapped.base, 0x1_4000_0000);
        assert_eq!(mapped.entry_pc, 0x1_4000_1000);
    }
}
