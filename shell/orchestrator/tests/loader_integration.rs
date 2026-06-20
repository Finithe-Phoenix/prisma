//! Cross-module integration: the loader/linker pieces working together.
//!
//! Unit tests cover each module in isolation; these exercise the real seam
//! between them — a module mapped into the guest address space, its exports
//! registered, a program's imports resolved, and the resolved addresses checked
//! to land inside the module's mapped (and executable) region. This is the path
//! a real load drives, so it catches contract drift the unit tests miss.

use prisma_orchestrator::address_space::{AddressSpace, Protection};
use prisma_orchestrator::import_resolver::{resolve_imports, ImportError, ImportRequest};
use prisma_orchestrator::module_table::{ExportedSymbol, LoadedModule, ModuleTable};

const KERNEL32_BASE: u64 = 0x1_8000_0000;

fn kernel32() -> LoadedModule {
    LoadedModule {
        name: "KERNEL32.dll".to_owned(),
        base: KERNEL32_BASE,
        exports: vec![
            ExportedSymbol {
                name: "GetProcAddress".to_owned(),
                rva: 0x1000,
            },
            ExportedSymbol {
                name: "ExitProcess".to_owned(),
                rva: 0x2000,
            },
        ],
    }
}

#[test]
fn resolved_imports_land_inside_the_mapped_module_region() {
    // Map the module into a guest address space and register its exports.
    let mut space = AddressSpace::new();
    space
        .map(KERNEL32_BASE, 0x10000, Protection::ReadExecute, "kernel32")
        .unwrap();

    let mut table = ModuleTable::new();
    table.insert(kernel32()).unwrap();

    // Resolve a program's imports (mixed-case library names, Windows-style).
    let reqs = [
        ImportRequest::new("kernel32.dll", "GetProcAddress"),
        ImportRequest::new("KERNEL32.DLL", "ExitProcess"),
    ];
    let resolved = resolve_imports(&reqs, &table).expect("imports resolve");

    // The linker's addresses must be addresses the address space actually maps,
    // in the module's own executable region — the cross-module contract.
    for r in &resolved {
        let (region, offset) = space
            .translate(r.address)
            .expect("resolved import address must be mapped");
        assert_eq!(region.name, "kernel32");
        assert!(region.prot.is_executable());
        assert_eq!(region.base + offset, r.address);
    }
    assert_eq!(resolved[0].address, KERNEL32_BASE + 0x1000);
    assert_eq!(resolved[1].address, KERNEL32_BASE + 0x2000);
}

#[test]
fn unresolved_import_is_reported_not_silently_mapped() {
    let mut table = ModuleTable::new();
    table.insert(kernel32()).unwrap();

    let reqs = [
        ImportRequest::new("kernel32.dll", "GetProcAddress"),
        ImportRequest::new("kernel32.dll", "FunctionThatDoesNotExist"),
    ];
    // The loader must surface the missing symbol, not fabricate an address.
    assert_eq!(
        resolve_imports(&reqs, &table),
        Err(ImportError::Unresolved {
            library: "kernel32.dll".to_owned(),
            symbol: "FunctionThatDoesNotExist".to_owned(),
        })
    );
}

#[test]
fn unloaded_dependency_blocks_the_load() {
    // A program importing from a DLL that was never mapped must fail to link.
    let table = ModuleTable::new();
    let reqs = [ImportRequest::new("user32.dll", "MessageBoxW")];
    assert!(matches!(
        resolve_imports(&reqs, &table),
        Err(ImportError::Unresolved { .. })
    ));
}
