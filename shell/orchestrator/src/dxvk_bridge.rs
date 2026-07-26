use crate::module_table::{LoadedModule, ModuleTable};

pub fn init_dxvk(modules: &mut ModuleTable, native_window: *mut std::ffi::c_void) {
    let mut d3d9_mem = vec![0u8; 4096];
    let d3d9 = LoadedModule::create_synthetic(
        "d3d9.dll",
        &["Direct3DCreate9", "Direct3DCreate9Ex"],
        0x7FFA_0000,
        &mut d3d9_mem,
    );
    modules.insert(d3d9).unwrap();

    let mut dxgi_mem = vec![0u8; 4096];
    let dxgi = LoadedModule::create_synthetic(
        "dxgi.dll",
        &["CreateDXGIFactory", "CreateDXGIFactory1"],
        0x7FF9_0000,
        &mut dxgi_mem,
    );
    modules.insert(dxgi).unwrap();

    let mut d3d11_mem = vec![0u8; 4096];
    let d3d11 = LoadedModule::create_synthetic(
        "d3d11.dll",
        &["D3D11CreateDevice", "D3D11CreateDeviceAndSwapChain"],
        0x7FF8_0000,
        &mut d3d11_mem,
    );
    modules.insert(d3d11).unwrap();

    println!("Prisma Orchestrator (DXVK): Bootstrap complete. Surface pointer: {:?}", native_window);
}
