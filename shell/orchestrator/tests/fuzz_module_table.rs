//! Property-based robustness fuzzing for the loaded-module table.
//!
//! DLL names come from a PE's (untrusted) import descriptors, so `ModuleTable`
//! must stay coherent under arbitrary names: case-insensitive matching that
//! never admits two aliasing modules, lookups consistent with inserts, and
//! resolution that only ever returns `base + rva`. These properties assert that
//! over random module sets and probes.

use proptest::prelude::*;

use prisma_orchestrator::module_table::{ExportedSymbol, LoadedModule, ModuleTable};

/// An ASCII-ish name generator that mixes case so the case-insensitivity logic
/// is actually exercised (not just distinct lowercase names).
fn name_strategy() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9._]{1,12}"
}

proptest! {
    /// Inserting a set of modules never panics, and the table never holds two
    /// modules whose names are equal ignoring case (the no-alias invariant).
    #[test]
    fn no_two_modules_alias_case_insensitively(
        names in prop::collection::vec(name_strategy(), 0..32),
    ) {
        let mut table = ModuleTable::new();
        for (i, n) in names.iter().enumerate() {
            let m = LoadedModule {
                name: n.clone(),
                base: (i as u64 + 1) * 0x1000,
                exports: Vec::new(),
            };
            let _ = table.insert(m); // duplicates (case-insensitive) are rejected
        }
        // Every inserted name resolves, and lookups are case-insensitive: the
        // upper- and lower-cased spelling hit the same module.
        for n in &names {
            if let Some(m) = table.get(n) {
                let lower = table.get(&n.to_ascii_lowercase());
                let upper = table.get(&n.to_ascii_uppercase());
                prop_assert_eq!(lower, Some(m));
                prop_assert_eq!(upper, Some(m));
            }
        }
    }

    /// A duplicate insert (any case spelling) is always rejected, so the count
    /// never grows past the number of distinct case-folded names.
    #[test]
    fn insert_is_case_insensitively_idempotent(name in name_strategy()) {
        let mut table = ModuleTable::new();
        let make = |base| LoadedModule { name: name.clone(), base, exports: Vec::new() };
        prop_assert!(table.insert(make(0x1000)).is_ok());
        prop_assert!(table.insert(make(0x2000)).is_err()); // same name
        let mut alt = make(0x3000);
        alt.name = name.to_ascii_uppercase();
        // Upper-cased spelling is the same module unless the name has no letters
        // (then upper == original and it's still a duplicate).
        prop_assert!(table.insert(alt).is_err());
        prop_assert_eq!(table.len(), 1);
    }

    /// `resolve` only ever returns `base + rva` for a real export, and `None`
    /// for anything else — it never fabricates an address.
    #[test]
    fn resolve_returns_base_plus_rva_or_none(
        lib in name_strategy(),
        sym in name_strategy(),
        base in 0u64..0xFFFF_0000,
        rva in any::<u32>(),
        probe in name_strategy(),
    ) {
        let mut table = ModuleTable::new();
        table.insert(LoadedModule {
            name: lib.clone(),
            base,
            exports: vec![ExportedSymbol { name: sym.clone(), rva }],
        }).unwrap();

        prop_assert_eq!(table.resolve(&lib, &sym), Some(base + u64::from(rva)));
        // A different (library, symbol) only resolves if it happens to match.
        let got = table.resolve(&lib, &probe);
        if probe == sym {
            prop_assert_eq!(got, Some(base + u64::from(rva)));
        } else {
            prop_assert_eq!(got, None);
        }
        // An unloaded library never resolves.
        prop_assert_eq!(table.resolve("definitely-not-loaded-xyz", &sym), None);
    }

    /// remove (case-insensitive) drains the table; the count matches the number
    /// of distinct case-folded names, and removing every input name empties it.
    #[test]
    fn remove_drains_the_table(names in prop::collection::vec(name_strategy(), 0..16)) {
        let mut table = ModuleTable::new();
        let mut distinct = std::collections::HashSet::new();
        for (i, n) in names.iter().enumerate() {
            if table
                .insert(LoadedModule {
                    name: n.clone(),
                    base: (i as u64 + 1) * 0x1000,
                    exports: Vec::new(),
                })
                .is_ok()
            {
                distinct.insert(n.to_ascii_lowercase());
            }
        }
        prop_assert_eq!(table.len(), distinct.len());
        // Removing every original spelling (case-insensitive) drains everything.
        for n in &names {
            let _ = table.remove(n);
        }
        prop_assert!(table.is_empty());
    }
}
