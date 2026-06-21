//! Import resolution: bind a PE's imports to loaded modules' exports.
//!
//! A PE lists the `(library, symbol)` pairs it needs; resolving them against the
//! [`ModuleTable`] yields the guest addresses the loader patches into the import
//! address table (IAT). Resolution fails loudly on the first unresolved import —
//! patching a bogus address would instead crash the guest much later at an
//! unrelated call site, so the loader must abort at load time.

use crate::module_table::ModuleTable;

/// What an import binds to within its library: a name or an export ordinal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportKind {
    /// Bind by exported name.
    Name(String),
    /// Bind by export ordinal (the import table carries no name).
    Ordinal(u16),
}

impl ImportKind {
    /// A display string for diagnostics: the name, or `#<ordinal>`.
    fn display(&self) -> String {
        match self {
            Self::Name(n) => n.clone(),
            Self::Ordinal(o) => format!("#{o}"),
        }
    }
}

/// One import an image requests from a named library, by name or by ordinal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportRequest {
    pub library: String,
    pub kind: ImportKind,
}

impl ImportRequest {
    /// An import bound by exported name.
    pub fn new(library: impl Into<String>, symbol: impl Into<String>) -> Self {
        Self {
            library: library.into(),
            kind: ImportKind::Name(symbol.into()),
        }
    }

    /// An import bound by export ordinal.
    pub fn by_ordinal(library: impl Into<String>, ordinal: u16) -> Self {
        Self {
            library: library.into(),
            kind: ImportKind::Ordinal(ordinal),
        }
    }
}

/// A resolved import: the request plus the guest virtual address it binds to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedImport {
    pub library: String,
    pub symbol: String,
    pub address: u64,
}

/// Resolve every import against the loaded-module table — the input to IAT
/// patching.
///
/// Fails on the FIRST unresolved `(library, symbol)`: the loader must abort
/// rather than patch a bogus address, which would crash the guest later at an
/// unrelated call site. Library names match case-insensitively (via the table);
/// symbol names are exact.
pub fn resolve_imports(
    requests: &[ImportRequest],
    table: &ModuleTable,
) -> Result<Vec<ResolvedImport>, ImportError> {
    let mut resolved = Vec::with_capacity(requests.len());
    for req in requests {
        let module = table.get(&req.library);
        let address = match &req.kind {
            ImportKind::Name(name) => module.and_then(|m| m.resolve(name)),
            ImportKind::Ordinal(ordinal) => module.and_then(|m| m.resolve_ordinal(*ordinal)),
        }
        .ok_or_else(|| ImportError::Unresolved {
            library: req.library.clone(),
            symbol: req.kind.display(),
        })?;
        resolved.push(ResolvedImport {
            library: req.library.clone(),
            symbol: req.kind.display(),
            address,
        });
    }
    Ok(resolved)
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ImportError {
    #[error("unresolved import {library}!{symbol}")]
    Unresolved { library: String, symbol: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module_table::{ExportedSymbol, LoadedModule};

    fn table() -> ModuleTable {
        let mut t = ModuleTable::new();
        t.insert(LoadedModule {
            name: "KERNEL32.dll".to_owned(),
            base: 0x1_8000_0000,
            exports: vec![
                ExportedSymbol {
                    name: "GetProcAddress".to_owned(),
                    rva: 0x1000,
                    ordinal: 1,
                },
                ExportedSymbol {
                    name: "ExitProcess".to_owned(),
                    rva: 0x2000,
                    ordinal: 2,
                },
            ],
        })
        .unwrap();
        t
    }

    #[test]
    fn resolves_all_imports_to_addresses() {
        let reqs = [
            ImportRequest::new("kernel32.dll", "GetProcAddress"),
            ImportRequest::new("KERNEL32.DLL", "ExitProcess"),
        ];
        let resolved = resolve_imports(&reqs, &table()).expect("all resolve");
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].address, 0x1_8000_1000);
        assert_eq!(resolved[1].address, 0x1_8000_2000);
    }

    #[test]
    fn empty_request_set_resolves_to_empty() {
        assert_eq!(resolve_imports(&[], &table()), Ok(Vec::new()));
    }

    #[test]
    fn fails_on_first_unresolved_symbol() {
        let reqs = [
            ImportRequest::new("kernel32.dll", "GetProcAddress"),
            ImportRequest::new("kernel32.dll", "NoSuchFunction"),
        ];
        assert_eq!(
            resolve_imports(&reqs, &table()),
            Err(ImportError::Unresolved {
                library: "kernel32.dll".to_owned(),
                symbol: "NoSuchFunction".to_owned(),
            })
        );
    }

    #[test]
    fn fails_on_unloaded_library() {
        let reqs = [ImportRequest::new("user32.dll", "MessageBoxA")];
        assert_eq!(
            resolve_imports(&reqs, &table()),
            Err(ImportError::Unresolved {
                library: "user32.dll".to_owned(),
                symbol: "MessageBoxA".to_owned(),
            })
        );
    }

    #[test]
    fn resolves_an_import_by_ordinal() {
        let reqs = [ImportRequest::by_ordinal("kernel32.dll", 2)];
        let resolved = resolve_imports(&reqs, &table()).expect("ordinal resolves");
        assert_eq!(resolved[0].address, 0x1_8000_2000); // ExitProcess @ ordinal 2
        assert_eq!(resolved[0].symbol, "#2");
    }

    #[test]
    fn unresolved_ordinal_is_reported() {
        let reqs = [ImportRequest::by_ordinal("kernel32.dll", 99)];
        assert_eq!(
            resolve_imports(&reqs, &table()),
            Err(ImportError::Unresolved {
                library: "kernel32.dll".to_owned(),
                symbol: "#99".to_owned(),
            })
        );
    }
}
