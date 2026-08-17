use std::ffi::CStr;
use std::os::raw::{c_char, c_void};

/// The main entry point called by Wine when it needs to simulate x86 code.
/// In the future, this will translate the x86 context and pass it to prisma-translator.
#[no_mangle]
pub extern "system" fn CpuSimulate(_context: *mut c_void) {
    // Placeholder: Start JIT execution loop.
    // E.g., `prisma_translator::dispatch(context)`
}

/// Called by Wine when a new DLL is mapped into memory.
#[no_mangle]
pub extern "system" fn CpuNotifyDllLoad(path: *const c_char) {
    if path.is_null() {
        return;
    }
    let _c_str = unsafe { CStr::from_ptr(path) };
    // Placeholder: Inform translation cache about the new executable region.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dll_load_null() {
        // Should not panic.
        CpuNotifyDllLoad(std::ptr::null());
    }
}
