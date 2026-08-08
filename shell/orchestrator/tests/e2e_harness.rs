use prisma_orchestrator::module_table::{LoadedModule, ModuleTable};
use prisma_orchestrator::vfs::VirtualFileSystem;
use std::fs;
use tempfile::tempdir;

#[test]
fn e2e_pipeline_initialization() {
    let base_dir = tempdir().unwrap();
    let overlay_dir = tempdir().unwrap();

    let vfs = VirtualFileSystem::new(base_dir.path(), overlay_dir.path());
    let mut table = ModuleTable::new();

    let module = LoadedModule {
        name: "dummy.dll".to_owned(),
        base: 0x1000_0000,
        exports: vec![],
    };

    table.insert(module).unwrap();

    assert_eq!(table.len(), 1);

    // Simulate loading a dummy PE block by checking VFS
    let test_file_path = base_dir.path().join("dummy.exe");
    fs::write(&test_file_path, b"MZ dummy PE").unwrap();

    let resolved = vfs.resolve_path(r"C:\dummy.exe").unwrap();
    assert!(resolved.exists());
    assert_eq!(fs::read_to_string(resolved).unwrap(), "MZ dummy PE");
}
