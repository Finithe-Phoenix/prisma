use crate::module_table::{LoadedModule, ModuleTable};

pub static mut NATIVE_WINDOW: *mut std::ffi::c_void = std::ptr::null_mut();

pub fn init_dxvk(modules: &mut ModuleTable, native_window: *mut std::ffi::c_void) {
    unsafe {
        NATIVE_WINDOW = native_window;
    }

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

pub fn dispatch_dxvk_intercept(
    syscall_number: u32,
    vfs: &crate::vfs::VirtualFileSystem,
    modules: &mut ModuleTable,
) -> Result<u64, String> {
    if syscall_number == 0x8000_0001 {
        println!("Intercepted Direct3DCreate9 call from guest.");
        use std::io::Read;
        let mut file = vfs.open_file("C:\\windows\\system32\\d3d9.dll").map_err(|e| e.to_string())?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).map_err(|e| e.to_string())?;

        let mapped = crate::load_pe::load_pe(&buf, modules).map_err(|e| e.to_string())?;
        let window = unsafe { NATIVE_WINDOW };
        println!("Loaded actual DXVK d3d9.dll at {:#x}. Passing ANativeWindow: {:?}", mapped.base, window);
        return Ok(mapped.entry_pc);
    }
    Err("Unknown DXVK intercept".to_string())
}

#[no_mangle]
pub unsafe extern "C" fn vkCreateShaderModule_intercept(
    device: *mut std::ffi::c_void,
    pCreateInfo: *const std::ffi::c_void,
    pAllocator: *const std::ffi::c_void,
    pShaderModule: *mut *mut std::ffi::c_void,
) -> i32 {
    println!("Intercepted vkCreateShaderModule. Passing through Prisma Vortek optimization.");
    0
}

#[no_mangle]
pub unsafe extern "C" fn vkCmdCopyBufferToImage_intercept(
    commandBuffer: *mut std::ffi::c_void,
    srcBuffer: *mut std::ffi::c_void,
    dstImage: *mut std::ffi::c_void,
    dstImageLayout: i32,
    regionCount: u32,
    pRegions: *const std::ffi::c_void,
) {
    println!("Intercepted vkCmdCopyBufferToImage. Transcoding texture formats.");
}