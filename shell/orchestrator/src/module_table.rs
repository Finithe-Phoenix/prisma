//! Loaded-module table: the dynamic linker's view of what is mapped.
//!
//! When the loader maps a PE it records the module here; resolving an import
//! `(library, symbol)` is then a lookup against the already-loaded modules.
//! Windows DLL names match case-insensitively (`KERNEL32.dll` == `kernel32.DLL`),
//! so the table keys on a normalised name. Teardown is explicit (`remove` /
//! `clear`) so an unloaded module leaves nothing behind across a restart, per
//! the resource-discipline clause.

use std::collections::HashMap;

use thiserror::Error;

/// One symbol a module exports: its name and its RVA (offset from the module's
/// load base).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportedSymbol {
    pub name: String,
    pub rva: u32,
}

/// A module mapped into the guest: its name, load base, and exports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedModule {
    pub name: String,
    pub base: u64,
    pub exports: Vec<ExportedSymbol>,
}

impl LoadedModule {
    /// Resolve an exported symbol to its guest virtual address (`base + rva`).
    /// Export names are case-sensitive (unlike module names). Returns `None`
    /// if the symbol is not exported or the address would overflow.
    #[must_use]
    pub fn resolve(&self, symbol: &str) -> Option<u64> {
        let export = self.exports.iter().find(|e| e.name == symbol)?;
        self.base.checked_add(u64::from(export.rva))
    }
}

/// Normalise a module name for case-insensitive matching (Windows semantics).
fn normalize(name: &str) -> String {
    name.to_ascii_lowercase()
}

/// The set of currently loaded modules, keyed by normalised name.
#[derive(Debug, Default)]
pub struct ModuleTable {
    modules: HashMap<String, LoadedModule>,
}

impl ModuleTable {
    #[must_use]
    pub fn new() -> Self {
        Self {
            modules: HashMap::new(),
        }
    }

    /// Register a loaded module. Rejects a second module with the same name
    /// (case-insensitive) — the loader must unload the old one first, so the
    /// table never silently shadows a mapping.
    pub fn insert(&mut self, module: LoadedModule) -> Result<(), ModuleError> {
        let key = normalize(&module.name);
        if self.modules.contains_key(&key) {
            return Err(ModuleError::AlreadyLoaded(module.name));
        }
        self.modules.insert(key, module);
        Ok(())
    }

    /// Look a module up by name (case-insensitive).
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&LoadedModule> {
        self.modules.get(&normalize(name))
    }

    /// Resolve an import `(library, symbol)` to its guest virtual address — the
    /// dynamic-linking step. `None` if the library is not loaded or the symbol
    /// is not exported.
    #[must_use]
    pub fn resolve(&self, library: &str, symbol: &str) -> Option<u64> {
        self.get(library)?.resolve(symbol)
    }

    /// Unload a module, returning it. Explicit teardown: the mapping's
    /// bookkeeping is released deterministically.
    pub fn remove(&mut self, name: &str) -> Option<LoadedModule> {
        self.modules.remove(&normalize(name))
    }

    /// Unload every module at once — used on container teardown.
    pub fn clear(&mut self) {
        self.modules.clear();
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.modules.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ModuleError {
    #[error("module already loaded: {0}")]
    AlreadyLoaded(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kernel32() -> LoadedModule {
        LoadedModule {
            name: "KERNEL32.dll".to_owned(),
            base: 0x1_8000_0000,
            exports: vec![
                ExportedSymbol {
                    name: "GetProcAddress".to_owned(),
                    rva: 0x1000,
                },
                ExportedSymbol {
                    name: "LoadLibraryA".to_owned(),
                    rva: 0x2000,
                },
            ],
        }
    }

    #[test]
    fn resolve_adds_rva_to_base() {
        let m = kernel32();
        assert_eq!(m.resolve("GetProcAddress"), Some(0x1_8000_1000));
        assert_eq!(m.resolve("LoadLibraryA"), Some(0x1_8000_2000));
        assert_eq!(m.resolve("NoSuchSymbol"), None);
    }

    #[test]
    fn module_lookup_is_case_insensitive() {
        let mut t = ModuleTable::new();
        t.insert(kernel32()).unwrap();
        // Same module, different case spellings all resolve.
        assert!(t.get("KERNEL32.dll").is_some());
        assert!(t.get("kernel32.dll").is_some());
        assert!(t.get("Kernel32.DLL").is_some());
        assert!(t.get("user32.dll").is_none());
    }

    #[test]
    fn resolve_import_end_to_end() {
        let mut t = ModuleTable::new();
        t.insert(kernel32()).unwrap();
        assert_eq!(
            t.resolve("kernel32.DLL", "GetProcAddress"),
            Some(0x1_8000_1000)
        );
        assert_eq!(t.resolve("kernel32.dll", "Missing"), None);
        assert_eq!(t.resolve("absent.dll", "GetProcAddress"), None);
    }

    #[test]
    fn insert_rejects_case_insensitive_duplicate() {
        let mut t = ModuleTable::new();
        t.insert(kernel32()).unwrap();
        let mut dup = kernel32();
        dup.name = "kernel32.dll".to_owned();
        assert!(matches!(t.insert(dup), Err(ModuleError::AlreadyLoaded(_))));
    }

    #[test]
    fn remove_and_clear_release_modules() {
        let mut t = ModuleTable::new();
        t.insert(kernel32()).unwrap();
        // remove matches case-insensitively and returns the module.
        let m = t.remove("Kernel32.DLL").expect("removed");
        assert_eq!(m.name, "KERNEL32.dll");
        assert!(t.is_empty());
        // After removal it can be inserted again (no stale shadow).
        t.insert(kernel32()).unwrap();
        assert_eq!(t.len(), 1);
        t.clear();
        assert!(t.is_empty());
    }

    #[test]
    fn resolve_rejects_overflowing_address() {
        let m = LoadedModule {
            name: "x.dll".to_owned(),
            base: u64::MAX,
            exports: vec![ExportedSymbol {
                name: "f".to_owned(),
                rva: 1,
            }],
        };
        assert_eq!(m.resolve("f"), None);
    }
}
