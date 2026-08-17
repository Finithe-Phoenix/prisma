#![cfg(target_os = "android")]

use jni::objects::{JClass, JObject, JString, JValue};
use jni::JNIEnv;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

static TERMINAL_STDIN: Mutex<Option<VecDeque<String>>> = Mutex::new(None);
use crate::address_space::Protection;
use crate::backed_address_space::BackedAddressSpace;
use crate::guest_layout::populate_backed;
use crate::load_pe::load_pe_with_image;
use crate::module_table::ModuleTable;
use jni::sys::jint;
use prisma_runtime::executor::{self, execute_block};
use prisma_runtime::guest_thread::GuestThread;
use prisma_runtime::peb::Peb;
use prisma_runtime::teb::Teb;
use prisma_translator::Translator;
use std::fs;

#[no_mangle]
pub extern "system" fn Java_dev_prismaemu_app_OrchestratorJni_runExecutable(
    mut env: JNIEnv,
    _class: JClass,
    path: JString,
) -> jint {
    let path_str: String = match env.get_string(&path) {
        Ok(s) => s.into(),
        Err(_) => return -1, // JNI Error
    };

    // Load the file
    let file_bytes = match fs::read(&path_str) {
        Ok(bytes) => bytes,
        Err(_) => return -2, // File not found or read error
    };

    let mut modules = ModuleTable::new();

    // Create synthetic kernel32
    let kernel32_base = 0x7FFE_0000;
    let mut kernel32_mem = vec![0u8; 4096];
    let kernel32 = crate::module_table::LoadedModule::create_synthetic(
        "kernel32.dll",
        &["LoadLibraryA", "CreateFileW"],
        kernel32_base,
        &mut kernel32_mem,
    );
    modules.insert(kernel32).unwrap();

    // Create synthetic ntdll
    let ntdll_base = 0x7FFD_0000;
    let mut ntdll_mem = vec![0u8; 4096];
    let ntdll = crate::module_table::LoadedModule::create_synthetic(
        "ntdll.dll",
        &["NtAllocateVirtualMemory"],
        ntdll_base,
        &mut ntdll_mem,
    );
    modules.insert(ntdll).unwrap();

    // Create synthetic user32
    let user32_base = 0x7FFC_0000;
    let mut user32_mem = vec![0u8; 4096];
    let user32 = crate::module_table::LoadedModule::create_synthetic(
        "user32.dll",
        &[
            "RegisterClassExW",
            "CreateWindowExW",
            "ShowWindow",
            "UpdateWindow",
            "GetMessageW",
            "TranslateMessage",
            "DispatchMessageW",
            "DefWindowProcW",
        ],
        user32_base,
        &mut user32_mem,
    );
    modules.insert(user32).unwrap();

    // Create synthetic gdi32
    let gdi32_base = 0x7FFB_0000;
    let mut gdi32_mem = vec![0u8; 4096];
    let gdi32 = crate::module_table::LoadedModule::create_synthetic(
        "gdi32.dll",
        &[
            "BeginPaint",
            "EndPaint",
            "GetDC",
            "ReleaseDC",
            "TextOutW",
            "FillRect",
            "CreateSolidBrush",
        ],
        gdi32_base,
        &mut gdi32_mem,
    );
    modules.insert(gdi32).unwrap();

    let (img, mapped) = match load_pe_with_image(&file_bytes, &modules) {
        Ok(res) => res,
        Err(e) => {
            println!(
                "Prisma Orchestrator (JNI): Failed to load PE {}: {}",
                path_str, e
            );
            return -3;
        }
    };

    println!(
        "Prisma Orchestrator (JNI): Successfully mapped {} at base {:#x}. Entry PC: {:#x}",
        path_str, mapped.base, mapped.entry_pc
    );

    // 1. Initialize the Memory Arena (RFC 0020)
    // 512 MiB window at 0x1_0000_0000
    let arena_size = 512 * 1024 * 1024;
    let window_base = 0x1_0000_0000;
    let mut space = match BackedAddressSpace::with_arena(window_base, arena_size) {
        Ok(s) => s,
        Err(e) => {
            println!(
                "Prisma Orchestrator (JNI): Failed to allocate GuestArena: {}",
                e
            );
            return -4;
        }
    };

    // 2. Map PE Sections and Stack
    // Stack is 2 MiB from the top of the arena
    let stack_base = window_base + arena_size as u64 - (2 * 1024 * 1024);
    if let Err(e) = populate_backed(&mut space, &img, &mapped, stack_base) {
        println!(
            "Prisma Orchestrator (JNI): Failed to populate memory layout: {:?}",
            e
        );
        return -5;
    }

    // 2b. Map Synthetic DLL Trampolines
    space
        .map(kernel32_base, 4096, Protection::ReadExecute)
        .unwrap();
    space.write(kernel32_base, &kernel32_mem).unwrap();

    space
        .map(ntdll_base, 4096, Protection::ReadExecute)
        .unwrap();
    space.write(ntdll_base, &ntdll_mem).unwrap();

    space
        .map(user32_base, 4096, Protection::ReadExecute)
        .unwrap();
    space.write(user32_base, &user32_mem).unwrap();

    space
        .map(gdi32_base, 4096, Protection::ReadExecute)
        .unwrap();
    space.write(gdi32_base, &gdi32_mem).unwrap();

    // 3. Setup PEB and TEB
    let peb_addr = window_base + arena_size as u64 - (4 * 1024 * 1024);
    let teb_addr = window_base + arena_size as u64 - (5 * 1024 * 1024);

    // Map their regions
    space.map(peb_addr, 4096, Protection::ReadWrite).unwrap();
    space.map(teb_addr, 4096, Protection::ReadWrite).unwrap();

    let peb = Peb::with_image_base(mapped.base);
    space.write(peb_addr, &peb.to_bytes()).unwrap();

    let teb = Teb {
        addr: teb_addr,
        stack_base: stack_base + crate::guest_stack::DEFAULT_STACK_SIZE, // top
        stack_limit: stack_base,                                         // bottom
        peb: peb_addr,
    };
    space.write(teb_addr, &teb.to_bytes()).unwrap();

    // 4. Initialize the CPU State Frame (GuestThread)
    let mut state = GuestThread::with_teb(teb.stack_base, teb_addr);
    state.next_pc = mapped.entry_pc;
    state.mem_base = space.mem_base().unwrap();

    // 5. Initialize the JIT Translator
    let mut translator = Translator::new();

    println!(
        "Prisma Orchestrator (JNI): Ignition! Starting JIT execution loop inside 512MiB Arena..."
    );

    // 6. Execution Loop
    let mut step_count = 0;
    let max_steps = 200; // Extended safety test run

    loop {
        let pc = state.next_pc;

        // Read up to 128 bytes from the current region for the JIT to translate
        let mut chunk = 128;
        let mut code_slice_res = space.read(pc, chunk);
        while code_slice_res.is_err() && chunk > 15 {
            chunk -= 16;
            code_slice_res = space.read(pc, chunk);
        }

        let guest_bytes = match code_slice_res {
            Ok(b) => b,
            Err(_) => match space.read(pc, 15) {
                Ok(b) => b,
                Err(e) => {
                    println!(
                        "Prisma Orchestrator (JNI): Memory fault (Fetch) at PC {:#x}: {:?}",
                        pc, e
                    );
                    break;
                }
            },
        };

        // Translate the block
        let translation = match translator.translate_block(pc, guest_bytes, 50) {
            Ok(t) => t,
            Err(e) => {
                println!(
                    "Prisma Orchestrator (JNI): Translation failed at PC {:#x}: {:?}",
                    pc, e
                );
                break;
            }
        };

        if translation.code.is_empty() {
            println!(
                "Prisma Orchestrator (JNI): No code generated at PC {:#x}",
                pc
            );
            break;
        }

        // Execute the ARM64 block (on aarch64 this runs natively, on x86 it skips and returns WrongArch)
        if let Err(e) = execute_block(&translation.code, &mut state) {
            println!("Prisma Orchestrator (JNI): Execution error: {:?}", e);
            break;
        }

        // Resolve next block
        if state.exit_reason == executor::EXIT_BRANCH {
            // next_pc is already set by the block via CSEL/etc
            state.exit_reason = executor::EXIT_NORMAL;
        } else if state.exit_reason == executor::EXIT_SYSCALL {
            // Read RAX for syscall number (guest_thread::GPR_RAX is index 0)
            let syscall_number = state.gpr[0] as u32;
            if syscall_number >= 0x8000_0000 {
                println!(
                    "Prisma Orchestrator (JNI): Guest executed WIN32 hypercall: {:#x}",
                    syscall_number
                );
                // Here we would call dispatch_win32
                if let Ok(stub_str) = env.new_string("StubText") {
                    let _ = env.call_static_method(
                        "dev/prismaemu/app/Win32Renderer",
                        "drawText",
                        "(Ljava/lang/String;II)V",
                        &[
                            jni::objects::JValue::Object(stub_str.as_ref()),
                            jni::objects::JValue::Int(0),
                            jni::objects::JValue::Int(0),
                        ],
                    );
                }
            } else {
                println!(
                    "Prisma Orchestrator (JNI): Guest executed POSIX syscall: {}",
                    syscall_number
                );
            }
            state.exit_reason = executor::EXIT_NORMAL;
            // Advance PC past syscall instruction (2 bytes)
            state.next_pc = pc + 2;
        } else {
            // Normal fallthrough
            state.next_pc = pc.wrapping_add(translation.guest_bytes as u64);
        }

        step_count += 1;
        if step_count >= max_steps {
            println!(
                "Prisma Orchestrator (JNI): Reached safety test limit of {} steps",
                max_steps
            );
            break;
        }
    }

    println!("Prisma Orchestrator (JNI): Execution loop terminated successfully.");
    0
}

#[no_mangle]
pub extern "system" fn Java_dev_prismaemu_app_OrchestratorJni_setSurface(
    mut env: JNIEnv,
    _class: JClass,
    surface: jni::objects::JObject,
) {
    let raw_env = env.get_raw() as *mut _;
    let raw_surface = surface.as_raw() as *mut _;
    let native_window = unsafe { ndk_sys::ANativeWindow_fromSurface(raw_env, raw_surface) };

    let mut modules = crate::module_table::ModuleTable::new();
    crate::dxvk_bridge::init_dxvk(&mut modules, native_window as *mut std::ffi::c_void);
}

#[no_mangle]
pub extern "system" fn Java_dev_prismaemu_app_OrchestratorJni_spawnTerminalProcess(
    mut env: jni::JNIEnv,
    _class: JClass,
    callback_obj: JObject,
) {
    {
        let mut queue = TERMINAL_STDIN.lock().unwrap();
        if queue.is_none() {
            *queue = Some(VecDeque::new());
        }
    }

    let jvm = env.get_java_vm().expect("Failed to get JavaVM");
    let callback_global = env
        .new_global_ref(callback_obj)
        .expect("Failed to create global ref");

    thread::spawn(move || {
        let mut env = jvm
            .attach_current_thread()
            .expect("Failed to attach current thread");

        let initial_msg = env.new_string("Prisma Terminal v1.0\n$ ").unwrap();
        let _ = env.call_method(
            &callback_global,
            "onTerminalOutput",
            "(Ljava/lang/String;)V",
            &[JValue::Object(initial_msg.as_ref())],
        );

        loop {
            thread::sleep(Duration::from_millis(100));

            let input_opt = {
                let mut queue = TERMINAL_STDIN.lock().unwrap();
                if let Some(q) = queue.as_mut() {
                    q.pop_front()
                } else {
                    None
                }
            };

            if let Some(input) = input_opt {
                let echo = format!("{}\n$ ", input);
                let echo_msg = env.new_string(echo).unwrap();
                let _ = env.call_method(
                    &callback_global,
                    "onTerminalOutput",
                    "(Ljava/lang/String;)V",
                    &[JValue::Object(echo_msg.as_ref())],
                );
            }
        }
    });
}

#[no_mangle]
pub extern "system" fn Java_dev_prismaemu_app_OrchestratorJni_sendTerminalInput(
    mut env: jni::JNIEnv,
    _class: JClass,
    input: JString,
) {
    let input_str: String = match env.get_string(&input) {
        Ok(s) => s.into(),
        Err(_) => return,
    };

    let mut queue_lock = TERMINAL_STDIN.lock().unwrap();
    if let Some(q) = queue_lock.as_mut() {
        q.push_back(input_str);
    }
}
