#[cfg(not(all(windows, target_arch = "arm64ec")))]
use std::cell::Cell;
use std::collections::{BTreeMap, VecDeque};
use std::fmt;
#[cfg(all(windows, target_arch = "arm64ec"))]
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use prisma_runtime::executor::{
    gpr, CpuStateFrame, ExecError, EXIT_BRANCH, EXIT_NORMAL, EXIT_SYSCALL, XMM_REGISTER_COUNT,
};
use prisma_translator::{BlockTranslation, TranslateError, Translator};

#[cfg(not(all(windows, target_arch = "arm64ec")))]
thread_local! {
    static ACTIVE_JIT_FRAME: Cell<*mut CpuStateFrame> = const { Cell::new(std::ptr::null_mut()) };
    static ACTIVE_JIT_RIP: Cell<u64> = const { Cell::new(0) };
}

#[cfg(any(test, all(windows, target_arch = "arm64ec")))]
struct ActiveJitFrameGuard {
    #[cfg(not(all(windows, target_arch = "arm64ec")))]
    previous_frame: *mut CpuStateFrame,
    #[cfg(not(all(windows, target_arch = "arm64ec")))]
    previous_rip: u64,
}

#[cfg(any(test, all(windows, target_arch = "arm64ec")))]
impl ActiveJitFrameGuard {
    fn enter(frame: &mut CpuStateFrame, rip: u64) -> Self {
        #[cfg(all(windows, target_arch = "arm64ec"))]
        unsafe {
            if let Ok(area) = current_wine_cpu_area() {
                area.emulator_data[0] = (frame as *mut CpuStateFrame).cast();
                area.emulator_data[1] = rip as usize as *mut std::ffi::c_void;
            }
        }
        #[cfg(not(all(windows, target_arch = "arm64ec")))]
        let previous_frame = ACTIVE_JIT_FRAME.replace(frame);
        #[cfg(not(all(windows, target_arch = "arm64ec")))]
        let previous_rip = ACTIVE_JIT_RIP.replace(rip);
        Self {
            #[cfg(not(all(windows, target_arch = "arm64ec")))]
            previous_frame,
            #[cfg(not(all(windows, target_arch = "arm64ec")))]
            previous_rip,
        }
    }
}

#[cfg(any(test, all(windows, target_arch = "arm64ec")))]
impl Drop for ActiveJitFrameGuard {
    fn drop(&mut self) {
        #[cfg(all(windows, target_arch = "arm64ec"))]
        unsafe {
            if let Ok(area) = current_wine_cpu_area() {
                // Prisma never nests generated blocks on one Wine thread. The
                // frame is valid only for the dynamic extent of the `blr`; a
                // native exception after that boundary must not observe stale
                // guest state restored from an earlier provider callback.
                area.emulator_data[0] = std::ptr::null_mut();
                area.emulator_data[1] = std::ptr::null_mut();
            }
        }
        #[cfg(not(all(windows, target_arch = "arm64ec")))]
        {
            ACTIVE_JIT_RIP.set(self.previous_rip);
            ACTIVE_JIT_FRAME.set(self.previous_frame);
        }
    }
}

// The private module's parent owns Wine's exception callbacks and is the only
// caller allowed to consume this thread-local publication.
#[allow(clippy::redundant_pub_crate)]
pub(super) unsafe fn reset_active_exception_context(context: *mut Arm64EcContext) -> bool {
    #[cfg(all(windows, target_arch = "arm64ec"))]
    let (frame, block_rip) = unsafe {
        match current_wine_cpu_area() {
            Ok(area) => {
                let active = (
                    area.emulator_data[0].cast::<CpuStateFrame>(),
                    area.emulator_data[1] as usize as u64,
                );
                // Wine continues through its exception dispatcher after this
                // callback and can abandon the JIT Rust stack non-locally.
                // Consume the per-thread publication here instead of relying
                // on `ActiveJitFrameGuard::drop` to run on that path.
                area.emulator_data[0] = std::ptr::null_mut();
                area.emulator_data[1] = std::ptr::null_mut();
                active
            }
            Err(_) => (std::ptr::null_mut(), 0),
        }
    };
    #[cfg(not(all(windows, target_arch = "arm64ec")))]
    let frame = ACTIVE_JIT_FRAME.replace(std::ptr::null_mut());
    #[cfg(not(all(windows, target_arch = "arm64ec")))]
    let block_rip = ACTIVE_JIT_RIP.replace(0);
    if frame.is_null() || context.is_null() {
        return false;
    }
    // The JIT publishes a boundary before every fused x64 instruction. Fall
    // back to the block start only if a fault precedes the first marker.
    let precise_rip = unsafe { (*frame).next_pc };
    let rip = if precise_rip != 0 {
        precise_rip
    } else {
        block_rip
    };
    if rip == 0 {
        return false;
    }
    // SAFETY: the guard owns a live dispatch-stack frame for this exact thread,
    // while Wine owns the mutable exception context for this callback.
    unsafe { (*context).store_frame(&*frame, rip) };
    #[cfg(all(windows, target_arch = "arm64ec"))]
    if RESET_CAPTURED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        std::eprintln!(
            "prisma-reset-capture: block={block_rip:#x} precise={precise_rip:#x} restored={rip:#x} exit={} next={:#x}",
            unsafe { (*frame).exit_reason },
            unsafe { (*frame).next_pc },
        );
    }
    true
}

// The current ARM64EC bring-up contains targeted diagnostics that must not sit
// on the hot block-dispatch path. Keep them compiled for the next fault probe,
// but silent during the real throughput run; all temporary probes are removed
// before the Phase 1 gate is committed.
#[cfg(all(windows, target_arch = "arm64ec"))]
macro_rules! eprintln {
    ($($argument:tt)*) => {{
        if false {
            std::eprintln!($($argument)*);
        }
    }};
}

#[cfg(any(test, all(windows, target_arch = "arm64ec")))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct WineSyscallEntry {
    name: &'static [u8],
    argument_bytes: u16,
}

#[cfg(any(test, all(windows, target_arch = "arm64ec")))]
include!(concat!(env!("OUT_DIR"), "/wine_syscalls.rs"));

#[cfg(any(test, all(windows, target_arch = "arm64ec")))]
const MAX_WIN64_SYSCALL_ARGUMENTS: usize = 16;

/// One 128-bit register in Wine's AMD64-compatible context layout.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct XmmRegister {
    pub low: u64,
    pub high: u64,
}

/// Wine-owned tail of `ARM64EC_NT_CONTEXT`, starting at offset `0x100`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Arm64EcContextTail {
    x87_prefix: [u8; 0x20],
    pub arm64_lr: u64,
    arm64_x16_0: u16,
    arm64_alias_reserved_0: [u8; 6],
    arm64_x6: u64,
    arm64_x16_1: u16,
    arm64_alias_reserved_1: [u8; 6],
    arm64_x7: u64,
    arm64_x16_2: u16,
    arm64_alias_reserved_2: [u8; 6],
    pub arm64_x9: u64,
    arm64_x16_3: u16,
    arm64_alias_reserved_3: [u8; 6],
    arm64_x10: u64,
    arm64_x17_0: u16,
    arm64_alias_reserved_4: [u8; 6],
    arm64_x11: u64,
    arm64_x17_1: u16,
    arm64_alias_reserved_5: [u8; 6],
    arm64_x12: u64,
    arm64_x17_2: u16,
    arm64_alias_reserved_6: [u8; 6],
    arm64_x15: u64,
    arm64_x17_3: u16,
    arm64_alias_reserved_7: [u8; 6],
    xmm: [XmmRegister; XMM_REGISTER_COUNT],
    xsave_reserved: [[u8; 0x20]; 3],
    vector_register: [XmmRegister; 26],
    vector_control: u64,
    debug_control: u64,
    last_branch_to_rip: u64,
    last_branch_from_rip: u64,
    last_exception_to_rip: u64,
    last_exception_from_rip: u64,
}

impl XmmRegister {
    fn from_bytes(bytes: [u8; 16]) -> Self {
        let mut low = [0_u8; 8];
        let mut high = [0_u8; 8];
        low.copy_from_slice(&bytes[..8]);
        high.copy_from_slice(&bytes[8..]);
        Self {
            low: u64::from_le_bytes(low),
            high: u64::from_le_bytes(high),
        }
    }

    fn to_bytes(self) -> [u8; 16] {
        let mut bytes = [0_u8; 16];
        bytes[..8].copy_from_slice(&self.low.to_le_bytes());
        bytes[8..].copy_from_slice(&self.high.to_le_bytes());
        bytes
    }
}

/// Wine 11.14's complete `ARM64EC_NT_CONTEXT` byte layout.
///
/// The unusual ARM register names are the documented ARM64EC aliases for the
/// corresponding AMD64 registers. Prisma synchronizes all integer, control and
/// XMM state used by translated code and preserves the remaining Wine-owned
/// context bytes in place.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Arm64EcContext {
    pub p1_home: u64,
    pub p2_home: u64,
    pub p3_home: u64,
    pub p4_home: u64,
    pub p5_home: u64,
    pub p6_home: u64,
    pub context_flags: u32,
    pub mx_csr: u32,
    pub seg_cs: u16,
    pub seg_ds: u16,
    pub seg_es: u16,
    pub seg_fs: u16,
    pub seg_gs: u16,
    pub seg_ss: u16,
    pub e_flags: u32,
    pub dr0: u64,
    pub dr1: u64,
    pub dr2: u64,
    pub dr3: u64,
    pub dr6: u64,
    pub dr7: u64,
    pub x8_rax: u64,
    pub x0_rcx: u64,
    pub x1_rdx: u64,
    pub x27_rbx: u64,
    pub sp_rsp: u64,
    pub fp_rbp: u64,
    pub x25_rsi: u64,
    pub x26_rdi: u64,
    pub x2_r8: u64,
    pub x3_r9: u64,
    pub x4_r10: u64,
    pub x5_r11: u64,
    pub x19_r12: u64,
    pub x20_r13: u64,
    pub x21_r14: u64,
    pub x22_r15: u64,
    pub pc_rip: u64,
    pub tail: Arm64EcContextTail,
}

impl Arm64EcContext {
    /// Returns one AMD64-visible XMM register from Wine's live context.
    #[must_use]
    pub fn xmm(&self, index: usize) -> Option<XmmRegister> {
        self.tail.xmm.get(index).copied()
    }

    /// Updates one AMD64-visible XMM register in Wine's live context.
    pub fn set_xmm(&mut self, index: usize, value: XmmRegister) -> bool {
        let Some(destination) = self.tail.xmm.get_mut(index) else {
            return false;
        };
        *destination = value;
        true
    }

    /// Copies Wine's AMD64-visible register state into the stable Prisma frame.
    #[must_use]
    pub fn load_frame(&self) -> CpuStateFrame {
        let mut frame = CpuStateFrame::default();
        frame.gpr[gpr::RAX] = self.x8_rax;
        frame.gpr[gpr::RCX] = self.x0_rcx;
        frame.gpr[gpr::RDX] = self.x1_rdx;
        frame.gpr[gpr::RBX] = self.x27_rbx;
        frame.gpr[gpr::RSP] = self.sp_rsp;
        frame.gpr[gpr::RBP] = self.fp_rbp;
        frame.gpr[gpr::RSI] = self.x25_rsi;
        frame.gpr[gpr::RDI] = self.x26_rdi;
        frame.gpr[gpr::R8] = self.x2_r8;
        frame.gpr[gpr::R9] = self.x3_r9;
        frame.gpr[gpr::R10] = self.x4_r10;
        frame.gpr[gpr::R11] = self.x5_r11;
        frame.gpr[gpr::R12] = self.x19_r12;
        frame.gpr[gpr::R13] = self.x20_r13;
        frame.gpr[gpr::R14] = self.x21_r14;
        frame.gpr[gpr::R15] = self.x22_r15;
        frame.rflags = u64::from(self.e_flags);
        frame.cf = frame.rflags & 1;
        for (index, value) in self.tail.xmm.iter().copied().enumerate() {
            let _ = frame.set_xmm(index, value.to_bytes());
        }
        frame
    }

    /// Commits Prisma's translated register state back to Wine's live context.
    pub fn store_frame(&mut self, frame: &CpuStateFrame, rip: u64) {
        self.x8_rax = frame.gpr[gpr::RAX];
        self.x0_rcx = frame.gpr[gpr::RCX];
        self.x1_rdx = frame.gpr[gpr::RDX];
        self.x27_rbx = frame.gpr[gpr::RBX];
        self.sp_rsp = frame.gpr[gpr::RSP];
        self.fp_rbp = frame.gpr[gpr::RBP];
        self.x25_rsi = frame.gpr[gpr::RSI];
        self.x26_rdi = frame.gpr[gpr::RDI];
        self.x2_r8 = frame.gpr[gpr::R8];
        self.x3_r9 = frame.gpr[gpr::R9];
        self.x4_r10 = frame.gpr[gpr::R10];
        self.x5_r11 = frame.gpr[gpr::R11];
        self.x19_r12 = frame.gpr[gpr::R12];
        self.x20_r13 = frame.gpr[gpr::R13];
        self.x21_r14 = frame.gpr[gpr::R14];
        self.x22_r15 = frame.gpr[gpr::R15];
        #[allow(clippy::cast_possible_truncation)]
        {
            // AMD64 EFLAGS is exactly the low 32 bits of the runtime RFLAGS.
            self.e_flags = frame.rflags as u32;
        }
        self.pc_rip = rip;
        for (index, destination) in self.tail.xmm.iter_mut().enumerate() {
            if let Some(value) = frame.xmm(index) {
                *destination = XmmRegister::from_bytes(value);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DispatchLimits {
    pub max_blocks: usize,
    pub max_fetch_bytes: usize,
    pub max_instructions_per_block: usize,
}

impl Default for DispatchLimits {
    fn default() -> Self {
        Self {
            // Cancellation is checked on every block, so a long-lived
            // dispatch can retain its translator/cache without weakening
            // shutdown responsiveness.
            max_blocks: 1_000_000,
            max_fetch_bytes: 4_096,
            // Keep fusion deliberately shallow while the exact GuestPc state
            // barrier is validated against the Go runtime under Wine.
            max_instructions_per_block: 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DispatchStop {
    Cancelled,
    BlockLimit,
    NativeTransitionRequired,
    NativeReturnRequired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DispatchReport {
    pub stop: DispatchStop,
    pub blocks: usize,
    pub instructions: usize,
    pub rip: u64,
}

#[derive(Debug)]
pub enum DispatchError {
    InvalidLimits,
    MemoryRead { rip: u64, detail: String },
    Translation { rip: u64, source: TranslateError },
    Execution { rip: u64, source: ExecError },
    UnsupportedSyscall { rip: u64 },
    UnknownSyscall { rip: u64, id: u64 },
    SyscallArguments { rip: u64, id: u64, detail: String },
    SyscallResolution { rip: u64, id: u64, name: String },
    UnknownExitReason { rip: u64, reason: u64 },
    ContextUnavailable,
}

impl fmt::Display for DispatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimits => formatter.write_str("dispatch limits must all be non-zero"),
            Self::MemoryRead { rip, detail } => {
                write!(formatter, "cannot read guest code at {rip:#x}: {detail}")
            }
            Self::Translation { rip, source } => {
                write!(formatter, "translation failed at {rip:#x}: {source}")
            }
            Self::Execution { rip, source } => {
                write!(formatter, "ARM64 execution failed at {rip:#x}: {source:?}")
            }
            Self::UnsupportedSyscall { rip } => write!(
                formatter,
                "Win64 syscall dispatch is not connected for block at {rip:#x}"
            ),
            Self::UnknownSyscall { rip, id } => {
                write!(
                    formatter,
                    "unknown Wine 11.14 Win64 syscall {id:#x} at {rip:#x}"
                )
            }
            Self::SyscallArguments { rip, id, detail } => write!(
                formatter,
                "cannot marshal Wine Win64 syscall {id:#x} at {rip:#x}: {detail}"
            ),
            Self::SyscallResolution { rip, id, name } => write!(
                formatter,
                "cannot resolve Wine Win64 syscall {id:#x} ({name}) at {rip:#x}"
            ),
            Self::UnknownExitReason { rip, reason } => {
                write!(formatter, "block at {rip:#x} returned exit reason {reason}")
            }
            Self::ContextUnavailable => formatter
                .write_str("Wine ARM64EC CPU area is unavailable on this target or calling thread"),
        }
    }
}

impl std::error::Error for DispatchError {}

/// Safe source of already-mapped Wine guest code.
pub trait GuestMemory {
    /// Return at most `max_len` bytes starting at `rip`.
    ///
    /// # Errors
    ///
    /// Returns a diagnostic when the mapped guest range cannot be read.
    fn read_code(&self, rip: u64, max_len: usize) -> Result<Vec<u8>, String>;

    /// Read an exact already-mapped guest data range.
    ///
    /// # Errors
    ///
    /// Returns a diagnostic when the mapped range cannot be read completely.
    fn read_data(&self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        let _ = (address, length);
        Err("guest data reads are unavailable for this memory source".to_owned())
    }
}

#[cfg(any(test, all(windows, target_arch = "arm64ec")))]
fn wine_syscall_entry(id: u64) -> Option<&'static WineSyscallEntry> {
    usize::try_from(id)
        .ok()
        .and_then(|index| WINE_SYSCALLS.get(index))
}

#[cfg(any(test, all(windows, target_arch = "arm64ec")))]
fn marshal_win64_syscall_arguments<M: GuestMemory>(
    memory: &M,
    frame: &CpuStateFrame,
    argument_bytes: u16,
) -> Result<Vec<u64>, String> {
    let argument_count = usize::from(argument_bytes / 8);
    if argument_bytes % 8 != 0 || argument_count > MAX_WIN64_SYSCALL_ARGUMENTS {
        return Err(format!("unsupported argument byte count {argument_bytes}"));
    }

    let mut arguments = Vec::with_capacity(argument_count);
    let register_arguments = [
        frame.gpr[gpr::R10],
        frame.gpr[gpr::RDX],
        frame.gpr[gpr::R8],
        frame.gpr[gpr::R9],
    ];
    arguments.extend_from_slice(&register_arguments[..argument_count.min(4)]);

    let stack_argument_count = argument_count.saturating_sub(4);
    if stack_argument_count != 0 {
        let stack_start = frame.gpr[gpr::RSP]
            .checked_add(0x28)
            .ok_or_else(|| "RSP overflow while locating Win64 stack arguments".to_owned())?;
        let byte_count = stack_argument_count
            .checked_mul(8)
            .ok_or_else(|| "Win64 stack argument size overflow".to_owned())?;
        let bytes = memory.read_data(stack_start, byte_count)?;
        if bytes.len() != byte_count {
            return Err(format!(
                "short Win64 stack read: expected {byte_count} bytes, got {}",
                bytes.len()
            ));
        }
        for chunk in bytes.chunks_exact(8) {
            let mut value = [0_u8; 8];
            value.copy_from_slice(chunk);
            arguments.push(u64::from_le_bytes(value));
        }
    }
    Ok(arguments)
}

#[cfg(all(windows, target_arch = "arm64ec"))]
fn dispatch_win64_syscall<M: GuestMemory>(
    memory: &M,
    frame: &CpuStateFrame,
    rip: u64,
) -> Result<i32, DispatchError> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleA(name: *const u8) -> *mut std::ffi::c_void;
        fn GetProcAddress(
            module: *mut std::ffi::c_void,
            name: *const u8,
        ) -> *const std::ffi::c_void;
    }

    let id = frame.gpr[gpr::RAX];
    let entry = wine_syscall_entry(id).ok_or(DispatchError::UnknownSyscall { rip, id })?;
    let arguments = marshal_win64_syscall_arguments(memory, frame, entry.argument_bytes)
        .map_err(|detail| DispatchError::SyscallArguments { rip, id, detail })?;
    if SYSCALL_TRACE_COUNT.fetch_add(1, Ordering::AcqRel) < 64 {
        let name = String::from_utf8_lossy(&entry.name[..entry.name.len() - 1]);
        std::eprintln!("prisma-syscall-checkpoint: {name}");
    }
    // SAFETY: both byte strings are statically generated NUL-terminated ASCII.
    let module = unsafe { GetModuleHandleA(c"ntdll.dll".as_ptr().cast()) };
    let address = if module.is_null() {
        std::ptr::null()
    } else {
        // SAFETY: `entry.name` is a generated NUL-terminated ASCII export name.
        unsafe { GetProcAddress(module, entry.name.as_ptr()) }
    };
    if address.is_null() {
        let name = String::from_utf8_lossy(&entry.name[..entry.name.len() - 1]).into_owned();
        return Err(DispatchError::SyscallResolution { rip, id, name });
    }
    // SAFETY: the export address and exact ABI argument list were validated
    // against the generated Wine table above.
    Ok(unsafe { invoke_win64_syscall(address, &arguments) })
}

#[cfg(not(all(windows, target_arch = "arm64ec")))]
const fn dispatch_win64_syscall<M: GuestMemory>(
    _memory: &M,
    _frame: &CpuStateFrame,
    rip: u64,
) -> Result<i32, DispatchError> {
    Err(DispatchError::UnsupportedSyscall { rip })
}

#[cfg(all(windows, target_arch = "arm64ec"))]
fn is_arm64ec_code(address: u64) -> bool {
    if is_main_amd64_image_address(address) {
        return false;
    }

    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlIsEcCode(address: usize) -> u8;
    }

    usize::try_from(address)
        .ok()
        // SAFETY: Wine's ntdll validates the address through its EC bitmap and
        // does not dereference the target page.
        .is_some_and(|address| unsafe { RtlIsEcCode(address) != 0 })
}

#[cfg(all(windows, target_arch = "arm64ec"))]
fn is_main_amd64_image_address(address: u64) -> bool {
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlPcToFileHeader(
            pc: *const std::ffi::c_void,
            base: *mut *mut std::ffi::c_void,
        ) -> *mut std::ffi::c_void;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut std::ffi::c_void;
    }

    let Ok(pc) = usize::try_from(address) else {
        return false;
    };
    let mut module_base = std::ptr::null_mut();
    // SAFETY: both calls only query loader metadata for the live process.
    let mapped_base =
        unsafe { RtlPcToFileHeader(pc as *const std::ffi::c_void, &raw mut module_base) };
    // SAFETY: a null module name requests the process executable.
    let main_module = unsafe { GetModuleHandleW(std::ptr::null()) };
    !mapped_base.is_null() && module_base == main_module
}

#[cfg(any(test, all(windows, target_arch = "arm64ec")))]
fn arm64ec_stack_argument_base(stack: u64) -> Result<u64, DispatchError> {
    stack
        .checked_add(8)
        .ok_or_else(|| DispatchError::MemoryRead {
            rip: stack,
            detail: "x64 stack overflow while entering ARM64EC".to_owned(),
        })
}

#[cfg(all(windows, target_arch = "arm64ec"))]
enum Arm64EcEntry {
    Native,
    Returned(u64),
}

#[cfg(any(test, all(windows, target_arch = "arm64ec")))]
fn complete_loaded_module_return(
    frame: &mut CpuStateFrame,
    stack: u64,
    return_address: u64,
    module: u64,
) -> Result<u64, DispatchError> {
    frame.gpr[gpr::RAX] = module;
    frame.gpr[gpr::RSP] = arm64ec_stack_argument_base(stack)?;
    Ok(return_address)
}

#[cfg(any(test, all(windows, target_arch = "arm64ec")))]
const fn loaded_module_reuse_flags(flags: u64) -> bool {
    const LOAD_LIBRARY_SEARCH_SYSTEM32: u64 = 0x0000_0800;
    flags == 0 || flags == LOAD_LIBRARY_SEARCH_SYSTEM32
}

#[cfg(all(windows, target_arch = "arm64ec"))]
fn prepare_arm64ec_entry<M: GuestMemory>(
    context: &mut Arm64EcContext,
    frame: &mut CpuStateFrame,
    memory: &M,
    target: u64,
) -> Result<Arm64EcEntry, DispatchError> {
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlPcToFileHeader(
            pc: *const std::ffi::c_void,
            base: *mut *mut std::ffi::c_void,
        ) -> *mut std::ffi::c_void;
    }
    let return_thunk = crate::ret_to_entry_thunk_address();
    if target == return_thunk {
        context.store_frame(frame, target);
        context.tail.arm64_lr = 0;
        return Ok(Arm64EcEntry::Native);
    }

    let stack = frame.gpr[gpr::RSP];
    let return_bytes = memory
        .read_data(stack, 8)
        .map_err(|detail| DispatchError::MemoryRead { rip: stack, detail })?;
    let return_address = u64::from_le_bytes(return_bytes.as_slice().try_into().map_err(|_| {
        DispatchError::MemoryRead {
            rip: stack,
            detail: "short x64 return-address read".to_owned(),
        }
    })?);
    let metadata_address = target.checked_sub(4).ok_or(DispatchError::MemoryRead {
        rip: target,
        detail: "ARM64EC entry metadata underflow".to_owned(),
    })?;
    let metadata =
        memory
            .read_data(metadata_address, 4)
            .map_err(|detail| DispatchError::MemoryRead {
                rip: metadata_address,
                detail,
            })?;
    let raw_offset = i32::from_le_bytes(metadata.as_slice().try_into().map_err(|_| {
        DispatchError::MemoryRead {
            rip: metadata_address,
            detail: "short ARM64EC entry-metadata read".to_owned(),
        }
    })?);
    let entry_offset = i64::from(raw_offset & !3);
    let entry = target.wrapping_add_signed(entry_offset);
    let stack_arguments = arm64ec_stack_argument_base(stack)?;
    frame.gpr[gpr::RSP] = stack_arguments;
    context.store_frame(frame, entry);
    // ARM64EC entry thunks use x4 as a pointer to the x64 home/stack argument
    // area. With the return address consumed, [x4 + 0x20] is the fifth x64
    // argument at the original [rsp + 0x28]. R10 is volatile across the call.
    context.x4_r10 = stack_arguments;
    context.tail.arm64_lr = return_address;
    context.tail.arm64_x9 = target;
    eprintln!(
        "prisma-native-entry-boundary: target={target:#x} entry={entry:#x} rdi={:#x} stack={:#x} thread_inits={}",
        context.x26_rdi,
        context.sp_rsp,
        crate::thread_init_count()
    );
    std::eprintln!("prisma-native-target-before-module-query: target={target:#x}");
    let mut module_base = std::ptr::null_mut();
    // SAFETY: the routine only queries loader metadata for this mapped address.
    let _ = unsafe {
        RtlPcToFileHeader(
            target as usize as *const std::ffi::c_void,
            &raw mut module_base,
        )
    };
    let module_base_address = module_base as usize as u64;
    let target_rva = target.wrapping_sub(module_base_address);
    if NATIVE_TARGET_DIAGNOSTICS.fetch_add(1, Ordering::AcqRel)
        < NATIVE_TARGET_TRACE_LIMIT.load(Ordering::Acquire)
    {
        std::eprintln!(
            "prisma-native-target: base={module_base_address:#x} rva={target_rva:#x} return={return_address:#x}"
        );
    }
    if target_rva == 0x9_b6e4 {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetModuleHandleW(name: *const u16) -> *mut std::ffi::c_void;
        }
        #[link(name = "ntdll")]
        unsafe extern "system" {
            fn LdrAddRefDll(flags: u32, module: *mut std::ffi::c_void) -> i32;
        }
        let mut units = Vec::new();
        for offset in (0..520_u64).step_by(2) {
            let Ok(bytes) = memory.read_data(frame.gpr[gpr::RCX].wrapping_add(offset), 2) else {
                break;
            };
            let Some(unit) = <[u8; 2]>::try_from(bytes.as_slice())
                .ok()
                .map(u16::from_le_bytes)
            else {
                break;
            };
            if unit == 0 {
                break;
            }
            units.push(unit);
        }
        let path = String::from_utf16_lossy(&units);
        let basename = path.rsplit(['\\', '/']).next().unwrap_or(&path);
        // SAFETY: RCX is the live LoadLibraryExW UTF-16 argument; the decoder
        // above bounded and validated reads from the same NUL-terminated span.
        let loaded = unsafe { GetModuleHandleW(frame.gpr[gpr::RCX] as usize as *const u16) };
        std::eprintln!(
            "prisma-loader-checkpoint: requested-module={basename} already-loaded={}",
            !loaded.is_null()
        );
        let kernelbase = "kernelbase.dll"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let module_matches = module_base == unsafe { GetModuleHandleW(kernelbase.as_ptr()) };
        let file_is_null = frame.gpr[gpr::RDX] == 0;
        let flags_allow_reuse = loaded_module_reuse_flags(frame.gpr[gpr::R8]);
        let add_ref_status =
            if module_matches && file_is_null && flags_allow_reuse && !loaded.is_null() {
                // SAFETY: `loaded` is a live module handle returned on this thread.
                Some(unsafe { LdrAddRefDll(0, loaded) })
            } else {
                None
            };
        std::eprintln!(
            "prisma-loader-checkpoint: module-match={module_matches} file-null={file_is_null} flags-allow-reuse={flags_allow_reuse} addref-success={}",
            add_ref_status.is_some_and(|status| status >= 0)
        );
        if add_ref_status.is_some_and(|status| status >= 0) {
            let next = complete_loaded_module_return(
                frame,
                stack,
                return_address,
                loaded as usize as u64,
            )?;
            context.store_frame(frame, next);
            std::eprintln!("prisma-loader-checkpoint: reused-loaded-module=true");
            POST_LOADER_TRACE_REMAINING.store(128, Ordering::Release);
            return Ok(Arm64EcEntry::Returned(next));
        }
    }
    if (0x1_4004_b660..0x1_4004_b7c0).contains(&return_address) {
        let stack_args = memory.read_data(stack.wrapping_add(0x28), 16).ok();
        eprintln!(
            "prisma-newosproc-native: return={return_address:#x} target={target:#x} rva={target_rva:#x} rcx={:#x} rdx={:#x} r8_start={:#x} r9_param={:#x} stack_args={stack_args:02x?}",
            frame.gpr[gpr::RCX],
            frame.gpr[gpr::RDX],
            frame.gpr[gpr::R8],
            frame.gpr[gpr::R9],
        );
    }
    let api = match target_rva {
        0x10_21b0 => Some("GetThreadContext"),
        0x10_3ed0 => Some("ResumeThread"),
        0x10_4600 => Some("SetThreadContext"),
        0x10_4ba0 => Some("SuspendThread"),
        _ => None,
    };
    if let Some(api) = api {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetCurrentThreadId() -> u32;
        }
        // SAFETY: GetCurrentThreadId has no preconditions.
        let tid = unsafe { GetCurrentThreadId() };
        eprintln!(
            "prisma-thread-api: tid={tid} api={api} handle={:#x} arg1={:#x} rsp={:#x} return={return_address:#x}",
            frame.gpr[gpr::RCX],
            frame.gpr[gpr::RDX],
            frame.gpr[gpr::RSP],
        );
    }
    Ok(Arm64EcEntry::Native)
}

#[cfg(not(all(windows, target_arch = "arm64ec")))]
const fn is_arm64ec_code(_address: u64) -> bool {
    false
}

#[cfg(all(windows, target_arch = "arm64ec"))]
fn initialize_windows_segment_bases(frame: &mut CpuStateFrame) {
    let teb: u64;
    // SAFETY: Windows ARM64 reserves x18 for the current TEB. Wine exposes the
    // same TEB to AMD64 code through GS, matching Windows' x64 ABI.
    unsafe { core::arch::asm!("mov {}, x18", out(reg) teb) };
    frame.gs_base = teb;
}

#[cfg(not(all(windows, target_arch = "arm64ec")))]
const fn initialize_windows_segment_bases(_frame: &mut CpuStateFrame) {}

#[cfg(all(windows, target_arch = "arm64ec"))]
unsafe fn invoke_win64_syscall(address: *const std::ffi::c_void, arguments: &[u64]) -> i32 {
    macro_rules! call {
        ($($index:tt),*) => {{
            // SAFETY: `address` is an ntdll Nt* export selected from Wine's
            // exact 11.14 syscall table. Each match arm casts it to the exact
            // argument count recorded by that same table.
            let function: unsafe extern "system" fn($(call!(@ty $index)),*) -> i32 =
                unsafe { core::mem::transmute(address) };
            unsafe { function($(arguments[$index]),*) }
        }};
        (@ty $index:tt) => { u64 };
    }

    match arguments.len() {
        0 => call!(),
        1 => call!(0),
        2 => call!(0, 1),
        3 => call!(0, 1, 2),
        4 => call!(0, 1, 2, 3),
        5 => call!(0, 1, 2, 3, 4),
        6 => call!(0, 1, 2, 3, 4, 5),
        7 => call!(0, 1, 2, 3, 4, 5, 6),
        8 => call!(0, 1, 2, 3, 4, 5, 6, 7),
        9 => call!(0, 1, 2, 3, 4, 5, 6, 7, 8),
        10 => call!(0, 1, 2, 3, 4, 5, 6, 7, 8, 9),
        11 => call!(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10),
        12 => call!(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11),
        13 => call!(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12),
        14 => call!(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13),
        15 => call!(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14),
        16 => call!(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15),
        _ => unreachable!("argument count was validated before invocation"),
    }
}

/// Execution boundary, injectable so loop semantics remain testable off ARM64.
pub trait BlockExecutor {
    /// Execute translated ARM64 code against the guest frame.
    ///
    /// `guest_rip` belongs to this exact invocation. Implementations must not
    /// recover it from process-global state because Wine can dispatch several
    /// guest threads concurrently.
    ///
    /// # Errors
    ///
    /// Returns the real runtime allocation, protection, or architecture error.
    fn execute(
        &self,
        guest_rip: u64,
        code: &[u8],
        frame: &mut CpuStateFrame,
    ) -> Result<(), ExecError>;
}

#[cfg(target_arch = "arm64ec")]
const MAX_JIT_CACHE_BYTES: usize = 128 * 1024 * 1024;

#[cfg(target_arch = "arm64ec")]
#[derive(Debug, Default)]
struct JitCache {
    buffers: BTreeMap<Vec<u8>, prisma_runtime::jit_memory::ExecBuffer>,
    bytes: usize,
}

/// Per-thread owner of executable translations.
///
/// Keeping an executable page associated with one code body prevents QEMU/Wine
/// from observing a stale translation when `VirtualAlloc` reuses an address.
/// The enclosing thread context drops the whole bounded cache at `ThreadTerm`.
#[derive(Debug, Default)]
pub struct PrismaExecutor {
    #[cfg(target_arch = "arm64ec")]
    cache: Mutex<JitCache>,
}

impl BlockExecutor for PrismaExecutor {
    // Keep allocation, publication, execution and cache ownership in one
    // boundary: splitting it would make non-local Wine recovery unsound.
    #[allow(clippy::too_many_lines)]
    fn execute(
        &self,
        guest_rip: u64,
        code: &[u8],
        frame: &mut CpuStateFrame,
    ) -> Result<(), ExecError> {
        #[cfg(target_arch = "arm64ec")]
        {
            use prisma_runtime::executor::wrap_block;
            use prisma_runtime::jit_memory::ExecBuffer;

            let callable = wrap_block(code);
            let entry = {
                let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
                if let Some(buffer) = cache.buffers.get(code) {
                    buffer.as_ptr()
                } else {
                    let mut buffer = ExecBuffer::alloc(callable.len()).map_err(ExecError::Alloc)?;
                    if !buffer.write(&callable) {
                        return Err(ExecError::Write);
                    }
                    buffer.make_executable().map_err(ExecError::Protect)?;
                    let new_total =
                        cache.bytes.checked_add(buffer.capacity()).ok_or_else(|| {
                            ExecError::Alloc(std::io::Error::other(
                                "ARM64EC JIT cache size overflow",
                            ))
                        })?;
                    if new_total > MAX_JIT_CACHE_BYTES {
                        return Err(ExecError::Alloc(std::io::Error::other(
                            "ARM64EC JIT cache reached its 128 MiB limit",
                        )));
                    }
                    let entry = buffer.as_ptr();
                    cache.bytes = new_total;
                    cache.buffers.insert(code.to_vec(), buffer);
                    entry
                }
            };
            if guest_rip == 0x1_4005_6E49 {
                eprintln!(
                    "prisma-dfcc-jit: body={:02x?} callable={:02x?}",
                    code, callable
                );
            }
            if guest_rip == 0x1_4002_0AA9 {
                eprintln!(
                    "prisma-rip-store-probe: jit={:p} frame={:p} mem_base={:#x} callable={} body={}",
                    entry,
                    frame,
                    frame.mem_base,
                    callable.len(),
                    code.len(),
                );
            }
            eprintln!(
                "prisma: JIT base={:p} frame={:p} bytes={}",
                entry,
                frame,
                callable.len()
            );
            let probe_rip = guest_rip;
            if !FAULT_CAPTURED.load(Ordering::Acquire) {
                PRISMA_FAULT_SNAPSHOT[0].store(probe_rip, Ordering::Release);
                PRISMA_FAULT_SNAPSHOT[1].store(entry as usize as u64, Ordering::Release);
                PRISMA_FAULT_SNAPSHOT[2].store(
                    frame as *mut CpuStateFrame as usize as u64,
                    Ordering::Release,
                );
                PRISMA_FAULT_SNAPSHOT[3].store(frame.mem_base, Ordering::Release);
                PRISMA_FAULT_SNAPSHOT[4].store(code.len() as u64, Ordering::Release);
                PRISMA_FAULT_SNAPSHOT[5].store(callable.len() as u64, Ordering::Release);
                for (index, value) in frame.gpr.iter().copied().enumerate() {
                    PRISMA_FAULT_SNAPSHOT[index + 6].store(value, Ordering::Release);
                }
                PRISMA_FAULT_SNAPSHOT[22].store(frame.rflags, Ordering::Release);
                PRISMA_FAULT_SNAPSHOT[23].store(frame.gs_base, Ordering::Release);
            }
            let sp_before: usize;
            let teb_before: usize;
            let publish_slot = (probe_rip == 0x1_4006_83CB).then(|| {
                frame.gpr[gpr::RAX]
                    .wrapping_add(frame.gpr[gpr::RDX])
                    .wrapping_add(0x8b0)
            });
            // SAFETY: these register reads are side-effect free diagnostics.
            unsafe {
                core::arch::asm!(
                    "mov {sp}, sp",
                    "mov {teb}, x18",
                    sp = out(reg) sp_before,
                    teb = out(reg) teb_before,
                    options(nomem, nostack, preserves_flags),
                );
            }
            // SAFETY: `wrap_block` emits native ARM64 with the Prisma
            // state-frame prologue and epilogue. A Rust ARM64EC indirect call
            // would route this anonymous JIT page through the x64 dispatcher;
            // issue the native branch directly and pass the frame in x0.
            let _active_jit_frame = ActiveJitFrameGuard::enter(frame, probe_rip);
            unsafe {
                core::arch::asm!(
                    "blr {entry}",
                    entry = in(reg) entry,
                    in("x0") frame as *mut CpuStateFrame,
                    clobber_abi("C"),
                );
            }
            if let Some(slot) = publish_slot {
                let host_address = frame.mem_base.wrapping_add(slot) as *const u64;
                // SAFETY: the translated block has just written this mapped guest slot.
                let value = unsafe { host_address.read_volatile() };
                if value >= (1_u64 << 32) {
                    PUBLISH_WATCH_ADDRESS.store(slot, Ordering::Release);
                    PUBLISH_WATCH_VALUE.store(value, Ordering::Release);
                }
            }
            let watched_address = PUBLISH_WATCH_ADDRESS.load(Ordering::Acquire);
            let watched_value = PUBLISH_WATCH_VALUE.load(Ordering::Acquire);
            let watched_current = if watched_address == 0 {
                0
            } else {
                let host_address = frame.mem_base.wrapping_add(watched_address) as *const u64;
                // SAFETY: the watch address was established from a successful guest write.
                unsafe { host_address.read_volatile() }
            };
            if watched_value >= (1_u64 << 32)
                && watched_current == (watched_value & u64::from(u32::MAX))
                && FAULT_CAPTURED
                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            {
                PRISMA_FAULT_SNAPSHOT[0].store(probe_rip, Ordering::Release);
                PRISMA_FAULT_SNAPSHOT[1].store(entry as usize as u64, Ordering::Release);
                PRISMA_FAULT_SNAPSHOT[2].store(
                    frame as *mut CpuStateFrame as usize as u64,
                    Ordering::Release,
                );
                PRISMA_FAULT_SNAPSHOT[3].store(watched_address, Ordering::Release);
                PRISMA_FAULT_SNAPSHOT[4].store(watched_value, Ordering::Release);
                PRISMA_FAULT_SNAPSHOT[5].store(callable.len() as u64, Ordering::Release);
                PRISMA_FAULT_SNAPSHOT[24].store(probe_rip, Ordering::Release);
                PRISMA_FAULT_SNAPSHOT[25].store(watched_current, Ordering::Release);
                PRISMA_FAULT_SNAPSHOT[26].store(0x5741_5443, Ordering::Release);
            }
            let sp_after: usize;
            let teb_after: usize;
            // SAFETY: these register reads are side-effect free diagnostics.
            unsafe {
                core::arch::asm!(
                    "mov {sp}, sp",
                    "mov {teb}, x18",
                    sp = out(reg) sp_after,
                    teb = out(reg) teb_after,
                    options(nomem, nostack, preserves_flags),
                );
            }
            eprintln!(
                "prisma: JIT host state sp={sp_before:#x}->{sp_after:#x} teb={teb_before:#x}->{teb_after:#x}"
            );
            assert_eq!(
                sp_before, sp_after,
                "JIT corrupted the native stack pointer"
            );
            assert_eq!(
                teb_before, teb_after,
                "JIT corrupted the Windows TEB register"
            );
            Ok(())
        }
        #[cfg(not(target_arch = "arm64ec"))]
        {
            let _ = guest_rip;
            prisma_runtime::executor::execute_block(code, frame)
        }
    }
}

static LIVE_RUNTIMES: AtomicUsize = AtomicUsize::new(0);
#[cfg(all(windows, target_arch = "arm64ec"))]
static MORESTACK_TRACE_STARTED: AtomicBool = AtomicBool::new(false);
#[cfg(all(windows, target_arch = "arm64ec"))]
static MORESTACK_TRACE_REMAINING: AtomicUsize = AtomicUsize::new(0);
#[cfg(all(windows, target_arch = "arm64ec"))]
static SCAN_TRACE_COUNT: AtomicUsize = AtomicUsize::new(0);
#[cfg(all(windows, target_arch = "arm64ec"))]
static LAST_GUEST_RIP: AtomicU64 = AtomicU64::new(0);
#[cfg(all(windows, target_arch = "arm64ec"))]
static EXPECTED_CHECK_GROWTH: AtomicBool = AtomicBool::new(false);
#[cfg(all(windows, target_arch = "arm64ec"))]
static FIRST_RUNTIME_THROW_REPORTED: AtomicBool = AtomicBool::new(false);
#[cfg(all(windows, target_arch = "arm64ec"))]
static RUNTIME_CHECK_PATH: AtomicUsize = AtomicUsize::new(0);
#[cfg(all(windows, target_arch = "arm64ec"))]
static SYSCALL_TRACE_COUNT: AtomicUsize = AtomicUsize::new(0);
#[cfg(all(windows, target_arch = "arm64ec"))]
static POST_LOADER_TRACE_REMAINING: AtomicUsize = AtomicUsize::new(0);
#[cfg(all(windows, target_arch = "arm64ec"))]
static NATIVE_TARGET_DIAGNOSTICS: AtomicUsize = AtomicUsize::new(0);
#[cfg(all(windows, target_arch = "arm64ec"))]
static NATIVE_TARGET_TRACE_LIMIT: AtomicUsize = AtomicUsize::new(512);
#[cfg(all(windows, target_arch = "arm64ec"))]
static FAULT_CAPTURED: AtomicBool = AtomicBool::new(false);
#[cfg(all(windows, target_arch = "arm64ec"))]
static PUBLISH_WATCH_ADDRESS: AtomicU64 = AtomicU64::new(0);
#[cfg(all(windows, target_arch = "arm64ec"))]
static PUBLISH_WATCH_VALUE: AtomicU64 = AtomicU64::new(0);
#[cfg(all(windows, target_arch = "arm64ec"))]
#[unsafe(no_mangle)]
pub static PRISMA_FAULT_SNAPSHOT: [AtomicU64; 48] = [const { AtomicU64::new(0) }; 48];
#[cfg(all(windows, target_arch = "arm64ec"))]
static BAD_GO_M_REPORTED: AtomicBool = AtomicBool::new(false);
#[cfg(all(windows, target_arch = "arm64ec"))]
static TSTART_DIAGNOSTICS: AtomicUsize = AtomicUsize::new(0);
#[cfg(all(windows, target_arch = "arm64ec"))]
static NEWOSPROC_DIAGNOSTICS: AtomicUsize = AtomicUsize::new(0);
#[cfg(all(windows, target_arch = "arm64ec"))]
static ALLOCM_OBJECT_DIAGNOSTICS: AtomicUsize = AtomicUsize::new(0);
#[cfg(all(windows, target_arch = "arm64ec"))]
static MALG_OBJECT_DIAGNOSTICS: AtomicUsize = AtomicUsize::new(0);
#[cfg(all(windows, target_arch = "arm64ec"))]
static ALLOCM_MALG_DIAGNOSTICS: AtomicUsize = AtomicUsize::new(0);
#[cfg(all(windows, target_arch = "arm64ec"))]
static FIRST_CHANCE_DIAGNOSTICS: AtomicUsize = AtomicUsize::new(0);
#[cfg(all(windows, target_arch = "arm64ec"))]
static RESET_CAPTURED: AtomicBool = AtomicBool::new(false);

#[cfg(all(windows, target_arch = "arm64ec"))]
pub fn last_guest_rip() -> u64 {
    LAST_GUEST_RIP.load(Ordering::Acquire)
}

pub struct ThreadRuntime {
    cancel: AtomicBool,
    invalidate_cache: AtomicBool,
    active_dispatches: AtomicUsize,
    translation_cache: Mutex<DispatchTranslationCache>,
}

const MAX_DISPATCH_CACHE_ENTRIES: usize = 65_536;
const MAX_DISPATCH_CACHE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug)]
struct CachedDispatchTranslation {
    source: Vec<u8>,
    block: BlockTranslation,
    bytes: usize,
    generation: u64,
}

#[derive(Debug, Default)]
struct DispatchTranslationCache {
    entries: BTreeMap<u64, CachedDispatchTranslation>,
    insertion_order: VecDeque<(u64, u64)>,
    bytes: usize,
    next_generation: u64,
}

impl DispatchTranslationCache {
    fn get(&self, rip: u64, bytes: &[u8]) -> Option<BlockTranslation> {
        let entry = self.entries.get(&rip)?;
        bytes
            .starts_with(&entry.source)
            .then(|| entry.block.clone())
    }

    fn insert(&mut self, rip: u64, bytes: &[u8], block: &BlockTranslation) {
        let Some(source) = bytes.get(..block.guest_bytes) else {
            return;
        };
        let entry_bytes = source.len().saturating_add(block.code.len());
        if entry_bytes > MAX_DISPATCH_CACHE_BYTES {
            return;
        }

        if let Some(previous) = self.entries.remove(&rip) {
            self.bytes = self.bytes.saturating_sub(previous.bytes);
        }
        self.next_generation = self.next_generation.wrapping_add(1);
        let generation = self.next_generation;
        self.entries.insert(
            rip,
            CachedDispatchTranslation {
                source: source.to_vec(),
                block: block.clone(),
                bytes: entry_bytes,
                generation,
            },
        );
        self.insertion_order.push_back((rip, generation));
        self.bytes = self.bytes.saturating_add(entry_bytes);
        self.evict_to_limits();
    }

    fn evict_to_limits(&mut self) {
        while self.entries.len() > MAX_DISPATCH_CACHE_ENTRIES
            || self.bytes > MAX_DISPATCH_CACHE_BYTES
        {
            let Some((rip, generation)) = self.insertion_order.pop_front() else {
                self.clear();
                break;
            };
            if self.entries.get(&rip).map(|entry| entry.generation) == Some(generation) {
                if let Some(entry) = self.entries.remove(&rip) {
                    self.bytes = self.bytes.saturating_sub(entry.bytes);
                }
            }
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.insertion_order.clear();
        self.bytes = 0;
    }
}

impl ThreadRuntime {
    pub fn new() -> Self {
        LIVE_RUNTIMES.fetch_add(1, Ordering::AcqRel);
        Self {
            cancel: AtomicBool::new(false),
            invalidate_cache: AtomicBool::new(false),
            active_dispatches: AtomicUsize::new(0),
            translation_cache: Mutex::new(DispatchTranslationCache::default()),
        }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }

    pub fn active_dispatches(&self) -> usize {
        self.active_dispatches.load(Ordering::Acquire)
    }

    pub fn clear_cache(&self) {
        self.invalidate_cache.store(true, Ordering::Release);
        self.translation_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    fn translate_block_cached(
        &self,
        translator: &mut Translator,
        rip: u64,
        bytes: &[u8],
        max_instructions: usize,
    ) -> Result<BlockTranslation, TranslateError> {
        if max_instructions == 1 {
            let cached = self
                .translation_cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(rip, bytes);
            if let Some(block) = cached {
                return Ok(block);
            }
        }

        let block = translate_dispatch_block(translator, rip, bytes, max_instructions)?;
        if max_instructions == 1 {
            self.translation_cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(rip, bytes, &block);
        }
        Ok(block)
    }

    // This loop is the state-machine boundary for one guest thread. Helpers
    // may compute transitions, but ownership and cleanup remain visible here.
    #[allow(clippy::too_many_lines)]
    pub fn dispatch<M: GuestMemory, E: BlockExecutor>(
        &self,
        context: &mut Arm64EcContext,
        memory: &M,
        executor: &E,
        limits: DispatchLimits,
    ) -> Result<DispatchReport, DispatchError> {
        if limits.max_blocks == 0
            || limits.max_fetch_bytes == 0
            || limits.max_instructions_per_block == 0
        {
            return Err(DispatchError::InvalidLimits);
        }

        let _lease = DispatchLease::new(self);
        #[cfg(all(windows, target_arch = "arm64ec"))]
        eprintln!("prisma: runtime lease acquired");
        // `Translator` contains non-Send pass objects. Wine dispatch is
        // thread-affine, so ownership remains on this stack and is dropped
        // before returning across the provider boundary.
        let mut translator = Translator::new();
        #[cfg(all(windows, target_arch = "arm64ec"))]
        eprintln!("prisma: translator created");
        let mut frame = context.load_frame();
        initialize_windows_segment_bases(&mut frame);
        let mut rip = context.pc_rip;
        let mut instructions = 0usize;
        #[cfg(all(windows, target_arch = "arm64ec"))]
        let mut rdi_history = [(0_u64, 0_u64, 0_u64); 64];
        #[cfg(all(windows, target_arch = "arm64ec"))]
        let mut rdi_history_count = 0_usize;
        #[cfg(all(windows, target_arch = "arm64ec"))]
        let mut rax_history = [(0_u64, 0_u64, 0_u64); 64];
        #[cfg(all(windows, target_arch = "arm64ec"))]
        let mut rax_history_count = 0_usize;
        #[cfg(all(windows, target_arch = "arm64ec"))]
        eprintln!(
            "prisma: frame loaded rip={rip:#x} rsp={:#x}",
            frame.gpr[gpr::RSP]
        );

        for block_index in 0..limits.max_blocks {
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if rip == 0x1_4008_4280 && !FIRST_RUNTIME_THROW_REPORTED.swap(true, Ordering::AcqRel) {
                let caller = memory
                    .read_data(frame.gpr[gpr::RSP], 8)
                    .ok()
                    .and_then(|bytes| <[u8; 8]>::try_from(bytes.as_slice()).ok())
                    .map(u64::from_le_bytes);
                std::eprintln!("prisma-runtime-init-probe: first_throw_caller={caller:x?}");
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if let Some(failure) = match rip {
                0x1_4006_2685 => Some("cas1"),
                0x1_4006_266F => Some("cas2"),
                0x1_4006_265E => Some("cas3"),
                0x1_4006_264D => Some("cas4"),
                _ => None,
            } {
                std::eprintln!("prisma-runtime-init-probe: atomic_check_failure={failure}");
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if rip == 0x1_4008_9D20 {
                let caller = memory
                    .read_data(frame.gpr[gpr::RSP], 8)
                    .ok()
                    .and_then(|bytes| <[u8; 8]>::try_from(bytes.as_slice()).ok())
                    .map(u64::from_le_bytes);
                std::eprintln!("prisma-runtime-init-probe: morestack_caller={caller:x?}");
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if rip == 0x1_4008_ce60 {
                let faulting_rip = LAST_GUEST_RIP.load(Ordering::Acquire);
                if faulting_rip != 0x1_4002_0609
                    && PUBLISH_WATCH_ADDRESS.load(Ordering::Acquire) != 0
                    && FAULT_CAPTURED
                        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok()
                {
                    let exception_pointers = frame.gpr[gpr::RCX];
                    let read_u64 = |address| {
                        memory
                            .read_data(address, 8)
                            .ok()
                            .and_then(|bytes| <[u8; 8]>::try_from(bytes.as_slice()).ok())
                            .map(u64::from_le_bytes)
                    };
                    let read_u32 = |address| {
                        memory
                            .read_data(address, 4)
                            .ok()
                            .and_then(|bytes| <[u8; 4]>::try_from(bytes.as_slice()).ok())
                            .map(u32::from_le_bytes)
                    };
                    let exception_record = read_u64(exception_pointers).unwrap_or_default();
                    PRISMA_FAULT_SNAPSHOT[24].store(faulting_rip, Ordering::Release);
                    PRISMA_FAULT_SNAPSHOT[25].store(rip, Ordering::Release);
                    PRISMA_FAULT_SNAPSHOT[26].store(
                        u64::from(read_u32(exception_record).unwrap_or_default()),
                        Ordering::Release,
                    );
                    PRISMA_FAULT_SNAPSHOT[27].store(
                        read_u64(exception_record.wrapping_add(16)).unwrap_or_default(),
                        Ordering::Release,
                    );
                    PRISMA_FAULT_SNAPSHOT[28].store(
                        read_u64(exception_record.wrapping_add(32)).unwrap_or_default(),
                        Ordering::Release,
                    );
                    PRISMA_FAULT_SNAPSHOT[29].store(
                        read_u64(exception_record.wrapping_add(40)).unwrap_or_default(),
                        Ordering::Release,
                    );
                    for (index, value) in frame.gpr.iter().copied().enumerate() {
                        PRISMA_FAULT_SNAPSHOT[index + 30].store(value, Ordering::Release);
                    }
                    PRISMA_FAULT_SNAPSHOT[46].store(frame.rflags, Ordering::Release);
                    PRISMA_FAULT_SNAPSHOT[47].store(frame.gs_base, Ordering::Release);
                    std::eprintln!(
                        "prisma-fault-capture: guest={faulting_rip:#x} handler={rip:#x} code={:#x} exception_address={:#x} access_kind={:#x} access_address={:#x}",
                        read_u32(exception_record).unwrap_or_default(),
                        read_u64(exception_record.wrapping_add(16)).unwrap_or_default(),
                        read_u64(exception_record.wrapping_add(32)).unwrap_or_default(),
                        read_u64(exception_record.wrapping_add(40)).unwrap_or_default(),
                    );
                }
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if matches!(
                rip,
                0x1_4002_20ed
                    | 0x1_4002_2175
                    | 0x1_4002_21a5
                    | 0x1_4002_247c
                    | 0x1_4002_2506
                    | 0x1_4002_252e
            ) {
                let read_u64 = |address| {
                    memory
                        .read_data(address, 8)
                        .ok()
                        .and_then(|bytes| <[u8; 8]>::try_from(bytes.as_slice()).ok())
                        .map(u64::from_le_bytes)
                };
                match rip {
                    0x1_4002_20ed if frame.gpr[gpr::RAX] == 0x1c8 => {
                        let mcache = frame.gpr[gpr::RSI];
                        let index = frame.gpr[gpr::R10];
                        let slot = mcache
                            .wrapping_add(index.wrapping_mul(8))
                            .wrapping_add(0x30);
                        eprintln!(
                            "prisma-small-noheader-slot: mcache={mcache:#x} index={index:#x} slot={slot:#x} span={:x?}",
                            read_u64(slot),
                        );
                    }
                    0x1_4002_2175 if frame.gpr[gpr::RAX] == 0x1c8 => {
                        eprintln!(
                            "prisma-small-noheader-object: span={:#x} object={:#x}",
                            frame.gpr[gpr::R10],
                            frame.gpr[gpr::R13],
                        );
                    }
                    0x1_4002_21a5 => {
                        eprintln!(
                            "prisma-small-noheader-nextfree-return: object={:#x} span={:#x} rsp={:#x}",
                            frame.gpr[gpr::RAX],
                            frame.gpr[gpr::RBX],
                            frame.gpr[gpr::RSP],
                        );
                    }
                    0x1_4002_247c if frame.gpr[gpr::RAX] == 0x7f8 => {
                        let mcache = frame.gpr[gpr::RSI];
                        let index = frame.gpr[gpr::R9];
                        let slot = mcache
                            .wrapping_add(index.wrapping_mul(8))
                            .wrapping_add(0x30);
                        eprintln!(
                            "prisma-small-header-slot: mcache={mcache:#x} index={index:#x} slot={slot:#x} span={:x?}",
                            read_u64(slot),
                        );
                    }
                    0x1_4002_2506 if frame.gpr[gpr::RAX] == 0x7f8 => {
                        eprintln!(
                            "prisma-small-header-object: span={:#x} object={:#x}",
                            frame.gpr[gpr::R9],
                            frame.gpr[gpr::R12],
                        );
                    }
                    0x1_4002_252e => {
                        eprintln!(
                            "prisma-small-header-nextfree-return: object={:#x} span={:#x} rsp={:#x}",
                            frame.gpr[gpr::RAX],
                            frame.gpr[gpr::RBX],
                            frame.gpr[gpr::RSP],
                        );
                    }
                    _ => {}
                }
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if matches!(rip, 0x1_4002_2ca0 | 0x1_4008_1dc0) {
                let read_u64 = |address| {
                    memory
                        .read_data(address, 8)
                        .ok()
                        .and_then(|bytes| <[u8; 8]>::try_from(bytes.as_slice()).ok())
                        .map(u64::from_le_bytes)
                };
                #[link(name = "kernel32")]
                unsafe extern "system" {
                    fn GetCurrentThreadId() -> u32;
                }
                // SAFETY: GetCurrentThreadId has no preconditions.
                let tid = unsafe { GetCurrentThreadId() };
                if rip == 0x1_4002_2ca0 {
                    let caller = read_u64(frame.gpr[gpr::RSP]);
                    if matches!(caller, Some(0x1_4005_55b1 | 0x1_4005_b525)) {
                        let type_address = frame.gpr[gpr::RAX];
                        let size = read_u64(type_address);
                        eprintln!(
                            "prisma-newobject-entry: tid={tid} caller={caller:x?} type={type_address:#x} size={size:x?} rsp={:#x}",
                            frame.gpr[gpr::RSP],
                        );
                    }
                } else {
                    let newobject_caller = read_u64(frame.gpr[gpr::RSP].wrapping_add(0x28));
                    if matches!(newobject_caller, Some(0x1_4005_55b1 | 0x1_4005_b525)) {
                        eprintln!(
                            "prisma-mallocgc-entry: tid={tid} caller={newobject_caller:x?} size={:#x} type={:#x} needzero={:#x} rsp={:#x}",
                            frame.gpr[gpr::RAX],
                            frame.gpr[gpr::RBX],
                            frame.gpr[gpr::RCX],
                            frame.gpr[gpr::RSP],
                        );
                    }
                }
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if matches!(rip, 0x1_4005_55b1 | 0x1_4005_b525 | 0x1_4005_55ef) {
                #[link(name = "kernel32")]
                unsafe extern "system" {
                    fn GetCurrentThreadId() -> u32;
                }
                // SAFETY: GetCurrentThreadId has no preconditions.
                let tid = unsafe { GetCurrentThreadId() };
                match rip {
                    0x1_4005_55b1
                        if ALLOCM_OBJECT_DIAGNOSTICS.fetch_add(1, Ordering::AcqRel) < 12 =>
                    {
                        eprintln!(
                            "prisma-allocm-newobject-return: tid={tid} object={:#x} rsp={:#x}",
                            frame.gpr[gpr::RAX],
                            frame.gpr[gpr::RSP],
                        );
                    }
                    0x1_4005_b525
                        if MALG_OBJECT_DIAGNOSTICS.fetch_add(1, Ordering::AcqRel) < 12 =>
                    {
                        eprintln!(
                            "prisma-malg-newobject-return: tid={tid} object={:#x} rsp={:#x}",
                            frame.gpr[gpr::RAX],
                            frame.gpr[gpr::RSP],
                        );
                    }
                    0x1_4005_55ef
                        if ALLOCM_MALG_DIAGNOSTICS.fetch_add(1, Ordering::AcqRel) < 12 =>
                    {
                        let mp = memory
                            .read_data(frame.gpr[gpr::RSP].wrapping_add(0x30), 8)
                            .ok()
                            .and_then(|bytes| <[u8; 8]>::try_from(bytes.as_slice()).ok())
                            .map(u64::from_le_bytes);
                        eprintln!(
                            "prisma-allocm-malg-return: tid={tid} mp={mp:x?} g0={:#x} rsp={:#x}",
                            frame.gpr[gpr::RAX],
                            frame.gpr[gpr::RSP],
                        );
                    }
                    _ => {}
                }
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if rip == 0x1_4004_b660 && NEWOSPROC_DIAGNOSTICS.fetch_add(1, Ordering::AcqRel) < 8 {
                let mp = frame.gpr[gpr::RAX];
                let g0 = memory
                    .read_data(mp, 8)
                    .ok()
                    .and_then(|bytes| <[u8; 8]>::try_from(bytes.as_slice()).ok())
                    .map(u64::from_le_bytes);
                let g0_m = g0.and_then(|g0| {
                    memory
                        .read_data(g0.wrapping_add(0x30), 8)
                        .ok()
                        .and_then(|bytes| <[u8; 8]>::try_from(bytes.as_slice()).ok())
                        .map(u64::from_le_bytes)
                });
                eprintln!(
                    "prisma-newosproc-entry: rax_mp={mp:#x} g0={g0:x?} g0_m={g0_m:x?} rbx={:#x} rcx={:#x} rdx={:#x} rsp={:#x}",
                    frame.gpr[gpr::RBX],
                    frame.gpr[gpr::RCX],
                    frame.gpr[gpr::RDX],
                    frame.gpr[gpr::RSP],
                );
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if rip == 0x1_4008_d180 && TSTART_DIAGNOSTICS.fetch_add(1, Ordering::AcqRel) < 8 {
                let mp = frame.gpr[gpr::RCX];
                let g0 = memory
                    .read_data(mp, 8)
                    .ok()
                    .and_then(|bytes| <[u8; 8]>::try_from(bytes.as_slice()).ok())
                    .map(u64::from_le_bytes);
                let g0_m = g0.and_then(|g0| {
                    memory
                        .read_data(g0.wrapping_add(0x30), 8)
                        .ok()
                        .and_then(|bytes| <[u8; 8]>::try_from(bytes.as_slice()).ok())
                        .map(u64::from_le_bytes)
                });
                let mp_bytes = memory.read_data(mp, 32).ok();
                let g0_bytes = g0.and_then(|g0| memory.read_data(g0, 64).ok());
                eprintln!(
                    "prisma-tstart-entry: mp={mp:#x} g0={g0:x?} g0_m={g0_m:x?} rsp={:#x} mp_bytes={mp_bytes:02x?} g0_bytes={g0_bytes:02x?}",
                    frame.gpr[gpr::RSP],
                );
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if (0x1_4000_0000..0x1_4100_0000).contains(&rip)
                && !BAD_GO_M_REPORTED.load(Ordering::Acquire)
            {
                let g = frame.gpr[gpr::R14];
                if g != 0 {
                    let direct_m = memory
                        .read_data(g.wrapping_add(0x30), 8)
                        .ok()
                        .and_then(|bytes| <[u8; 8]>::try_from(bytes.as_slice()).ok())
                        .map(u64::from_le_bytes);
                    if direct_m == Some(g.wrapping_add(8))
                        && !BAD_GO_M_REPORTED.swap(true, Ordering::AcqRel)
                    {
                        let writer_rip = LAST_GUEST_RIP.load(Ordering::Acquire);
                        let writer_bytes = memory.read_code(writer_rip, 64).ok();
                        eprintln!(
                            "prisma-go-m-corruption: writer_rip={writer_rip:#x} writer_bytes={writer_bytes:02x?} next_rip={rip:#x} g={g:#x} bad_m={:#x} rsp={:#x} rax={:#x} rbx={:#x} rcx={:#x} rdx={:#x}",
                            direct_m.unwrap_or_default(),
                            frame.gpr[gpr::RSP],
                            frame.gpr[gpr::RAX],
                            frame.gpr[gpr::RBX],
                            frame.gpr[gpr::RCX],
                            frame.gpr[gpr::RDX],
                        );
                    }
                }
            }
            if self.cancel.load(Ordering::Acquire) {
                context.store_frame(&frame, rip);
                return Ok(DispatchReport {
                    stop: DispatchStop::Cancelled,
                    blocks: block_index,
                    instructions,
                    rip,
                });
            }
            if self.invalidate_cache.swap(false, Ordering::AcqRel) {
                translator.clear_cache();
                self.translation_cache
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clear();
            }
            if is_arm64ec_code(rip) {
                if crate::is_native_return_continuation(rip) {
                    context.store_frame(&frame, rip);
                    return Ok(DispatchReport {
                        stop: DispatchStop::NativeReturnRequired,
                        blocks: block_index,
                        instructions,
                        rip,
                    });
                }
                #[cfg(all(windows, target_arch = "arm64ec"))]
                match prepare_arm64ec_entry(context, &mut frame, memory, rip)? {
                    Arm64EcEntry::Native => {}
                    Arm64EcEntry::Returned(next) => {
                        rip = next;
                        continue;
                    }
                }
                #[cfg(not(all(windows, target_arch = "arm64ec")))]
                context.store_frame(&frame, rip);
                return Ok(DispatchReport {
                    stop: DispatchStop::NativeTransitionRequired,
                    blocks: block_index,
                    instructions,
                    rip,
                });
            }

            #[cfg(all(windows, target_arch = "arm64ec"))]
            if POST_LOADER_TRACE_REMAINING
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                std::eprintln!("prisma-post-loader-block: {rip:#x}");
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            eprintln!("prisma: reading block rip={rip:#x}");
            let bytes = match memory.read_code(rip, limits.max_fetch_bytes) {
                Ok(bytes) => bytes,
                Err(detail) => {
                    #[cfg(all(windows, target_arch = "arm64ec"))]
                    std::eprintln!(
                        "prisma-flow-probe: unreadable={rip:#x} previous={:#x} exit={} next={:#x} rsp={:#x}",
                        LAST_GUEST_RIP.load(Ordering::Acquire),
                        frame.exit_reason,
                        frame.next_pc,
                        frame.gpr[gpr::RSP],
                    );
                    #[cfg(all(windows, target_arch = "arm64ec"))]
                    {
                        let retained = rdi_history_count.min(rdi_history.len());
                        let first = rdi_history_count.saturating_sub(retained);
                        for sequence in first..rdi_history_count {
                            let (writer, before, after) = rdi_history[sequence % rdi_history.len()];
                            std::eprintln!(
                                "prisma-rdi-history: sequence={sequence} writer={writer:#x} before={before:#x} after={after:#x}"
                            );
                        }
                        let retained = rax_history_count.min(rax_history.len());
                        let first = rax_history_count.saturating_sub(retained);
                        for sequence in first..rax_history_count {
                            let (writer, before, after) = rax_history[sequence % rax_history.len()];
                            std::eprintln!(
                                "prisma-rax-history: sequence={sequence} writer={writer:#x} before={before:#x} after={after:#x}"
                            );
                        }
                    }
                    #[cfg(all(windows, target_arch = "arm64ec"))]
                    eprintln!(
                        "prisma: unreadable next rip={rip:#x} last_block={:#x} rsp={:#x} rax={:#x} rcx={:#x} rdx={:#x} rbx={:#x} rsi={:#x} rdi={:#x} r8={:#x} r9={:#x} r10={:#x} r11={:#x}",
                        LAST_GUEST_RIP.load(Ordering::Acquire),
                        frame.gpr[gpr::RSP],
                        frame.gpr[gpr::RAX],
                        frame.gpr[gpr::RCX],
                        frame.gpr[gpr::RDX],
                        frame.gpr[gpr::RBX],
                        frame.gpr[gpr::RSI],
                        frame.gpr[gpr::RDI],
                        frame.gpr[gpr::R8],
                        frame.gpr[gpr::R9],
                        frame.gpr[gpr::R10],
                        frame.gpr[gpr::R11],
                    );
                    return Err(DispatchError::MemoryRead { rip, detail });
                }
            };
            #[cfg(all(windows, target_arch = "arm64ec"))]
            eprintln!(
                "prisma: read {} bytes head={:02x?}",
                bytes.len(),
                &bytes[..bytes.len().min(32)]
            );
            if bytes.is_empty() {
                return Err(DispatchError::MemoryRead {
                    rip,
                    detail: "reader returned no bytes".to_owned(),
                });
            }
            let block = self
                .translate_block_cached(
                    &mut translator,
                    rip,
                    &bytes,
                    limits.max_instructions_per_block,
                )
                .map_err(|source| DispatchError::Translation { rip, source })?;
            #[cfg(all(windows, target_arch = "arm64ec"))]
            eprintln!(
                "prisma: translated instructions={} guest_bytes={}",
                block.instruction_count, block.guest_bytes
            );
            frame.exit_reason = EXIT_NORMAL;
            frame.next_pc = 0;
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if rip == 0x1_4005_6E49 {
                eprintln!(
                    "prisma-56e49-guest: instructions={} guest_bytes={} ended={} head={:02x?}",
                    block.instruction_count,
                    block.guest_bytes,
                    block.ended_at_terminator,
                    &bytes[..bytes.len().min(40)],
                );
                frame.next_pc = 0xD1A6_005E_6E49_0001;
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if rip == 0x1_4008_B545 {
                let read_u64 = |address| {
                    memory
                        .read_data(address, 8)
                        .ok()
                        .and_then(|bytes| <[u8; 8]>::try_from(bytes.as_slice()).ok())
                        .map(u64::from_le_bytes)
                };
                let m_g0 = read_u64(frame.gpr[gpr::R8]);
                let direct_g_m = read_u64(frame.gpr[gpr::RDI].wrapping_add(0x30));
                eprintln!(
                    "prisma-g0-select-before: r8_m={:#x} direct_g_m={direct_g_m:x?} m_g0={m_g0:x?} rdi_g={:#x} rsi={:#x} r14={:#x} rsp={:#x}",
                    frame.gpr[gpr::R8],
                    frame.gpr[gpr::RDI],
                    frame.gpr[gpr::RSI],
                    frame.gpr[gpr::R14],
                    frame.gpr[gpr::RSP],
                );
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if rip == 0x1_4008_B583 {
                let read_u64 = |address| {
                    memory
                        .read_data(address, 8)
                        .ok()
                        .and_then(|bytes| <[u8; 8]>::try_from(bytes.as_slice()).ok())
                        .map(u64::from_le_bytes)
                };
                let tls_offset = read_u64(0x1_40df_f650);
                let g = tls_offset.and_then(|offset| read_u64(frame.gs_base.wrapping_add(offset)));
                let rdi_stack = read_u64(frame.gpr[gpr::RDI].wrapping_add(8));
                let stack_delta = read_u64(frame.gpr[gpr::RSP]);
                let restore_rsp = rdi_stack
                    .zip(stack_delta)
                    .map(|(stack_high, delta)| stack_high.wrapping_sub(delta));
                let restore_frame =
                    restore_rsp.and_then(|address| memory.read_data(address, 32).ok());
                let g0_frame = memory.read_data(frame.gpr[gpr::RSP], 32).ok();
                let saved_g = read_u64(frame.gpr[gpr::RSP].wrapping_add(8));
                let saved_stack_high = saved_g.and_then(|g| read_u64(g.wrapping_add(8)));
                let actual_restore_rsp = saved_stack_high
                    .zip(stack_delta)
                    .map(|(stack_high, delta)| stack_high.wrapping_sub(delta));
                let actual_restore_frame =
                    actual_restore_rsp.and_then(|address| memory.read_data(address, 32).ok());
                eprintln!(
                    "prisma-8b583-before: insns={} bytes={} ended={} rsp={:#x} rdi={:#x} rsi={:#x} rdx={:#x} gs={:#x} mem_base={:#x} tls={tls_offset:x?} g={g:x?} rdi_stack={rdi_stack:x?} delta={stack_delta:x?} restore_rsp={restore_rsp:x?} restore_frame={restore_frame:02x?} g0_frame={g0_frame:02x?} saved_g={saved_g:x?} saved_stack_high={saved_stack_high:x?} actual_restore_rsp={actual_restore_rsp:x?} actual_restore_frame={actual_restore_frame:02x?}",
                    block.instruction_count,
                    block.guest_bytes,
                    block.ended_at_terminator,
                    frame.gpr[gpr::RSP],
                    frame.gpr[gpr::RDI],
                    frame.gpr[gpr::RSI],
                    frame.gpr[gpr::RDX],
                    frame.gs_base,
                    frame.mem_base,
                );
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if rip == 0x1_4008_B55F {
                let stack = memory.read_data(frame.gpr[gpr::RSP], 64).ok();
                let rdi_stack = memory
                    .read_data(frame.gpr[gpr::RDI].wrapping_add(8), 8)
                    .ok()
                    .and_then(|bytes| <[u8; 8]>::try_from(bytes.as_slice()).ok())
                    .map(u64::from_le_bytes);
                let rsi_sched_sp = memory
                    .read_data(frame.gpr[gpr::RSI].wrapping_add(0x38), 8)
                    .ok()
                    .and_then(|bytes| <[u8; 8]>::try_from(bytes.as_slice()).ok())
                    .map(u64::from_le_bytes);
                eprintln!(
                    "prisma-asmcgocall-entry: rip={rip:#x} rsp={:#x} rbp={:#x} rax={:#x} rbx={:#x} rcx={:#x} rdx={:#x} rdi={:#x} rsi_g0={:#x} rsi_sched_sp={rsi_sched_sp:x?} r8_m={:#x} r14={:#x} rdi_stack={rdi_stack:x?} stack={stack:02x?}",
                    frame.gpr[gpr::RSP],
                    frame.gpr[gpr::RBP],
                    frame.gpr[gpr::RAX],
                    frame.gpr[gpr::RBX],
                    frame.gpr[gpr::RCX],
                    frame.gpr[gpr::RDX],
                    frame.gpr[gpr::RDI],
                    frame.gpr[gpr::RSI],
                    frame.gpr[gpr::R8],
                    frame.gpr[gpr::R14],
                );
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if (0x1_4006_d900..=0x1_4006_e200).contains(&rip) {
                let source = memory
                    .read_data(frame.gpr[gpr::RAX], 16)
                    .ok()
                    .map(|bytes| bytes.into_iter().take(16).collect::<Vec<_>>());
                eprintln!(
                    "prisma: symtab before rip={rip:#x} rax={:#x} rbx={:#x} rcx={:#x} rdx={:#x} rdi={:#x} rsi={:#x} r8={:#x} rflags={:#x} source={source:02x?}",
                    frame.gpr[gpr::RAX],
                    frame.gpr[gpr::RBX],
                    frame.gpr[gpr::RCX],
                    frame.gpr[gpr::RDX],
                    frame.gpr[gpr::RDI],
                    frame.gpr[gpr::RSI],
                    frame.gpr[gpr::R8],
                    frame.rflags,
                );
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if rip == 0x1_4004_c009 {
                let m = frame.gpr[gpr::RDX];
                let field_e0 = memory.read_data(m.wrapping_add(0xe0), 4).ok();
                let field_340 = memory.read_data(m.wrapping_add(0x340), 8).ok();
                let callback = memory.read_data(0x1_40db_5800, 8).ok();
                eprintln!(
                    "prisma: c009 state rdx={m:#x} rax={:#x} rcx={:#x} rbx={:#x} e0={field_e0:02x?} f340={field_340:02x?} callback={callback:02x?}",
                    frame.gpr[gpr::RAX],
                    frame.gpr[gpr::RCX],
                    frame.gpr[gpr::RBX],
                );
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if rip == 0x1_4006_5ec0 {
                #[link(name = "ntdll")]
                unsafe extern "system" {
                    fn RtlPcToFileHeader(
                        pc: *const std::ffi::c_void,
                        base: *mut *mut std::ffi::c_void,
                    ) -> *mut std::ffi::c_void;
                }
                let record = frame.gpr[gpr::RAX];
                let context = frame.gpr[gpr::RBX];
                let code = memory.read_data(record, 4).ok();
                let address = memory.read_data(record.wrapping_add(0x10), 8).ok();
                let information = memory.read_data(record.wrapping_add(0x20), 16).ok();
                let native_rip = memory.read_data(context.wrapping_add(0xf8), 8).ok();
                let as_u64 = |bytes: &Option<Vec<u8>>| {
                    bytes
                        .as_ref()
                        .and_then(|bytes| <[u8; 8]>::try_from(bytes.as_slice()).ok())
                        .map(u64::from_le_bytes)
                };
                let native_rip_value = as_u64(&native_rip).unwrap_or_default();
                let fault_address = information
                    .as_ref()
                    .and_then(|bytes| bytes.get(8..16))
                    .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
                    .map(u64::from_le_bytes)
                    .unwrap_or_default();
                let mut rip_base = std::ptr::null_mut();
                let mut fault_base = std::ptr::null_mut();
                // SAFETY: both calls only query loader metadata for diagnostic addresses.
                unsafe {
                    RtlPcToFileHeader(
                        native_rip_value as usize as *const std::ffi::c_void,
                        &raw mut rip_base,
                    );
                    RtlPcToFileHeader(
                        fault_address as usize as *const std::ffi::c_void,
                        &raw mut fault_base,
                    );
                }
                let fault_probe = memory.read_data(fault_address, 1);
                if FIRST_CHANCE_DIAGNOSTICS.fetch_add(1, Ordering::Relaxed) < 3 {
                    let jit_window = native_rip_value
                        .checked_sub(0x80)
                        .and_then(|start| memory.read_data(start, 0x100).ok());
                    eprintln!(
                        "prisma: first-chance last_guest={:#x} record={record:#x} context={context:#x} code={code:02x?} address={address:02x?} info={information:02x?} rip={native_rip:02x?} rip_base={rip_base:p} fault={fault_address:#x} fault_base={fault_base:p} probe={fault_probe:?} jit_window={jit_window:02x?}",
                        LAST_GUEST_RIP.load(Ordering::Acquire),
                    );
                }
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if rip == 0x1_4004_a867 {
                eprintln!(
                    "prisma: zero-loop rdx={:#x} rsi={:#x} rsp={:#x}",
                    frame.gpr[gpr::RDX],
                    frame.gpr[gpr::RSI],
                    frame.gpr[gpr::RSP]
                );
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if matches!(rip, 0x1_4008_6c58 | 0x1_4008_6c69) {
                let count = SCAN_TRACE_COUNT.fetch_add(1, Ordering::Relaxed);
                if count < 8 || count.is_power_of_two() {
                    eprintln!(
                        "prisma-scan-probe: count={count} rip={rip:#x} rbx={:#x} r9={:#x} rsi={:#x} rdx={:#x} r8={:#x} rdi={:#x}",
                        frame.gpr[gpr::RBX],
                        frame.gpr[gpr::R9],
                        frame.gpr[gpr::RSI],
                        frame.gpr[gpr::RDX],
                        frame.gpr[gpr::R8],
                        frame.gpr[gpr::RDI],
                    );
                }
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if rip == 0x1_4008_bd00 {
                let caller = memory
                    .read_data(frame.gpr[gpr::RSP], 8)
                    .ok()
                    .and_then(|bytes| {
                        <[u8; 8]>::try_from(bytes.as_slice())
                            .ok()
                            .map(u64::from_le_bytes)
                    });
                if caller == Some(0x1_4006_e12f)
                    && !MORESTACK_TRACE_STARTED.swap(true, Ordering::AcqRel)
                {
                    MORESTACK_TRACE_REMAINING.store(200, Ordering::Release);
                    eprintln!(
                        "prisma: tracing failed morestack entry rsp={:#x} r14={:#x} caller={caller:x?}",
                        frame.gpr[gpr::RSP],
                        frame.gpr[gpr::R14]
                    );
                }
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if rip == 0x1_4004_c960 {
                let guard_address = frame.gpr[gpr::R14].wrapping_add(0x10);
                let guard = memory.read_data(guard_address, 8).ok().and_then(|bytes| {
                    <[u8; 8]>::try_from(bytes.as_slice())
                        .ok()
                        .map(u64::from_le_bytes)
                });
                let return_address =
                    memory
                        .read_data(frame.gpr[gpr::RSP], 8)
                        .ok()
                        .and_then(|bytes| {
                            <[u8; 8]>::try_from(bytes.as_slice())
                                .ok()
                                .map(u64::from_le_bytes)
                        });
                eprintln!(
                    "prisma: morestack check rsp={:#x} r14={:#x} guard={guard:x?} return={return_address:x?}",
                    frame.gpr[gpr::RSP],
                    frame.gpr[gpr::R14]
                );
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if MORESTACK_TRACE_REMAINING
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                eprintln!(
                    "prisma: morestack-trace rip={rip:#x} rsp={:#x} rax={:#x} rbx={:#x} rcx={:#x} rdx={:#x} r14={:#x}",
                    frame.gpr[gpr::RSP],
                    frame.gpr[gpr::RAX],
                    frame.gpr[gpr::RBX],
                    frame.gpr[gpr::RCX],
                    frame.gpr[gpr::RDX],
                    frame.gpr[gpr::R14],
                );
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if rip == 0x1_4001_5c3e {
                eprintln!(
                    "prisma: rep-movsq before rcx={:#x} rsi={:#x} rdi={:#x} rsp={:#x} r14={:#x}",
                    frame.gpr[gpr::RCX],
                    frame.gpr[gpr::RSI],
                    frame.gpr[gpr::RDI],
                    frame.gpr[gpr::RSP],
                    frame.gpr[gpr::R14],
                );
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if matches!(
                rip,
                0x1_4002_0c25
                    | 0x1_4005_c860
                    | 0x1_4005_d3b7
                    | 0x1_4005_d3e5
                    | 0x1_4002_2060
                    | 0x1_4002_20cf
            ) {
                let read_u64 = |address| {
                    memory
                        .read_data(address, 8)
                        .ok()
                        .and_then(|bytes| <[u8; 8]>::try_from(bytes.as_slice()).ok())
                        .map(u64::from_le_bytes)
                };
                let m = read_u64(frame.gpr[gpr::R14].wrapping_add(0x30));
                let mcache = m.and_then(|m| read_u64(m.wrapping_add(0xa0)));
                let mcache_inner = mcache.and_then(|mcache| read_u64(mcache.wrapping_add(0x38)));
                std::eprintln!(
                    "prisma-mcache-probe-before: rip={rip:#x} rax={:#x} rsi={:#x} g={:#x} m={m:x?} mcache={mcache:x?} inner={mcache_inner:x?} global={:x?}",
                    frame.gpr[gpr::RAX],
                    frame.gpr[gpr::RSI],
                    frame.gpr[gpr::R14],
                    read_u64(0x1_40df_f500),
                );
                if rip == 0x1_4005_c860 {
                    let active_p = m.and_then(|m| read_u64(m.wrapping_add(0xa0)));
                    let return_address = read_u64(frame.gpr[gpr::RSP]);
                    std::eprintln!(
                        "prisma-proc-destroy-entry: candidate={:#x} active={active_p:x?} return={return_address:x?}",
                        frame.gpr[gpr::RAX],
                    );
                }
                if rip == 0x1_4005_d3b7 {
                    let read_u32 = |address| {
                        memory
                            .read_data(address, 4)
                            .ok()
                            .and_then(|bytes| <[u8; 4]>::try_from(bytes.as_slice()).ok())
                            .map(u32::from_le_bytes)
                    };
                    std::eprintln!(
                        "prisma-procresize-loop-before: old={:x?} nprocs={:x?}",
                        read_u32(frame.gpr[gpr::RSP].wrapping_add(0x44)),
                        read_u32(frame.gpr[gpr::RSP].wrapping_add(0x130)),
                    );
                }
                if rip == 0x1_4005_d3e5 {
                    let read_u32 = |address| {
                        memory
                            .read_data(address, 4)
                            .ok()
                            .and_then(|bytes| <[u8; 4]>::try_from(bytes.as_slice()).ok())
                            .map(u32::from_le_bytes)
                    };
                    std::eprintln!(
                        "prisma-procresize-branch-before: old={:x?} nprocs={:#x}",
                        read_u32(frame.gpr[gpr::RSP].wrapping_add(0x44)),
                        frame.gpr[gpr::RAX] as u32,
                    );
                }
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            eprintln!("prisma: executing ARM64 block");
            #[cfg(all(windows, target_arch = "arm64ec"))]
            LAST_GUEST_RIP.store(rip, Ordering::Release);
            #[cfg(all(windows, target_arch = "arm64ec"))]
            let rdi_before = frame.gpr[gpr::RDI];
            #[cfg(all(windows, target_arch = "arm64ec"))]
            let rax_before = frame.gpr[gpr::RAX];
            #[cfg(all(windows, target_arch = "arm64ec"))]
            let return_target_before = (rip == 0x1_4006_257B).then(|| {
                memory
                    .read_data(frame.gpr[gpr::RSP], 8)
                    .ok()
                    .and_then(|bytes| <[u8; 8]>::try_from(bytes.as_slice()).ok())
                    .map(u64::from_le_bytes)
            });
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if rip == 0x1_4006_2360 {
                let read_g = |offset: u64| {
                    memory
                        .read_data(frame.gpr[gpr::R14].wrapping_add(offset), 8)
                        .ok()
                        .and_then(|bytes| <[u8; 8]>::try_from(bytes.as_slice()).ok())
                        .map(u64::from_le_bytes)
                };
                let low = read_g(0);
                let high = read_g(8);
                let guard = read_g(0x10);
                let expected = guard.is_some_and(|guard| frame.gpr[gpr::RSP] <= guard);
                std::eprintln!(
                    "prisma-runtime-init-probe: stack_in_bounds={} stack_below_low={} stack_has_frame_margin={}",
                    low.zip(high).is_some_and(|(low, high)| (low..=high).contains(&frame.gpr[gpr::RSP])),
                    low.is_some_and(|low| frame.gpr[gpr::RSP] < low),
                    guard.is_some_and(|guard| frame.gpr[gpr::RSP] > guard)
                );
                EXPECTED_CHECK_GROWTH.store(expected, Ordering::Release);
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            {
                let checkpoint = if rip == 0x1_4006_2360 {
                    Some((1, "entry"))
                } else if rip == 0x1_4006_257B {
                    Some((2, "normal-return"))
                } else if (0x1_4006_257C..0x1_4006_2697).contains(&rip) {
                    Some((3, "failure-path"))
                } else if (0x1_4006_2697..0x1_4006_26B0).contains(&rip) {
                    Some((4, "stack-growth"))
                } else {
                    None
                };
                if let Some((state, label)) = checkpoint {
                    if RUNTIME_CHECK_PATH.swap(state, Ordering::AcqRel) != state {
                        std::eprintln!("prisma-runtime-check-path: {label}");
                    }
                }
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            let morestack_guard_expected = (rip == 0x1_4004_C960).then(|| {
                memory
                    .read_data(frame.gpr[gpr::R14].wrapping_add(0x10), 8)
                    .ok()
                    .and_then(|bytes| <[u8; 8]>::try_from(bytes.as_slice()).ok())
                    .map(u64::from_le_bytes)
                    .is_some_and(|guard| frame.gpr[gpr::RSP] <= guard)
            });
            executor
                .execute(rip, &block.code, &mut frame)
                .map_err(|source| DispatchError::Execution { rip, source })?;
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if matches!(rip, 0x1_4002_0c25 | 0x1_4005_d3b7 | 0x1_4005_d3e5) {
                let global = memory
                    .read_data(0x1_40df_f500, 8)
                    .ok()
                    .and_then(|bytes| <[u8; 8]>::try_from(bytes.as_slice()).ok())
                    .map(u64::from_le_bytes);
                std::eprintln!(
                    "prisma-mcache-probe-after: rip={rip:#x} rax={:#x} global={global:x?}",
                    frame.gpr[gpr::RAX],
                );
                if rip == 0x1_4005_d3b7 {
                    std::eprintln!(
                        "prisma-procresize-loop-after: exit={} next={:#x}",
                        frame.exit_reason,
                        frame.next_pc,
                    );
                }
                if rip == 0x1_4005_d3e5 {
                    std::eprintln!(
                        "prisma-procresize-branch-after: exit={} next={:#x}",
                        frame.exit_reason,
                        frame.next_pc,
                    );
                }
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if let Some(expected_taken) = morestack_guard_expected {
                let actual_taken = frame.next_pc == 0x1_4004_CA8F;
                std::eprintln!(
                    "prisma-runtime-init-probe: stack_guard_branch_matches={}",
                    expected_taken == actual_taken
                );
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if let Some(expected) = return_target_before {
                std::eprintln!(
                    "prisma-runtime-init-probe: return_published_target={} return_marked_branch={}",
                    expected == Some(frame.next_pc),
                    frame.exit_reason == EXIT_BRANCH
                );
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if rip == 0x1_4006_2364 {
                let actual_taken = frame.next_pc == 0x1_4006_2697;
                std::eprintln!(
                    "prisma-runtime-init-probe: check_guard_branch_matches={}",
                    actual_taken == EXPECTED_CHECK_GROWTH.load(Ordering::Acquire)
                );
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if frame.gpr[gpr::RDI] != rdi_before {
                rdi_history[rdi_history_count % rdi_history.len()] =
                    (rip, rdi_before, frame.gpr[gpr::RDI]);
                rdi_history_count = rdi_history_count.saturating_add(1);
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if frame.gpr[gpr::RAX] != rax_before {
                rax_history[rax_history_count % rax_history.len()] =
                    (rip, rax_before, frame.gpr[gpr::RAX]);
                rax_history_count = rax_history_count.saturating_add(1);
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if rip == 0x1_4005_6E49 {
                eprintln!(
                    "prisma-56e49-exit: reason={} next={:#x} rsp={:#x} code={:02x?}",
                    frame.exit_reason,
                    frame.next_pc,
                    frame.gpr[gpr::RSP],
                    block.code,
                );
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if rip == 0x1_4008_B545 {
                let selected_sched_sp = memory
                    .read_data(frame.gpr[gpr::RSI].wrapping_add(0x38), 8)
                    .ok()
                    .and_then(|bytes| <[u8; 8]>::try_from(bytes.as_slice()).ok())
                    .map(u64::from_le_bytes);
                eprintln!(
                    "prisma-g0-select-after: next={:#x} rsi_g0={:#x} selected_sched_sp={selected_sched_sp:x?} r8_m={:#x} rdi_g={:#x}",
                    frame.next_pc,
                    frame.gpr[gpr::RSI],
                    frame.gpr[gpr::R8],
                    frame.gpr[gpr::RDI],
                );
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if rip == 0x1_4008_B583 {
                let stack = memory.read_data(frame.gpr[gpr::RSP], 32).ok();
                let popped_frame = memory
                    .read_data(frame.gpr[gpr::RSP].wrapping_sub(16), 16)
                    .ok();
                eprintln!(
                    "prisma-8b583-after: reason={} next={:#x} rsp={:#x} rbp={:#x} rcx={:#x} rdi={:#x} rsi={:#x} popped_frame={popped_frame:02x?} stack={stack:02x?}",
                    frame.exit_reason,
                    frame.next_pc,
                    frame.gpr[gpr::RSP],
                    frame.gpr[gpr::RBP],
                    frame.gpr[gpr::RCX],
                    frame.gpr[gpr::RDI],
                    frame.gpr[gpr::RSI],
                );
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if (0x1_4006_d900..=0x1_4006_e200).contains(&rip) {
                eprintln!(
                    "prisma: symtab after rip={rip:#x} next={:#x} rax={:#x} rbx={:#x} rcx={:#x} rdx={:#x} rdi={:#x} rsi={:#x} r8={:#x} rflags={:#x}",
                    frame.next_pc,
                    frame.gpr[gpr::RAX],
                    frame.gpr[gpr::RBX],
                    frame.gpr[gpr::RCX],
                    frame.gpr[gpr::RDX],
                    frame.gpr[gpr::RDI],
                    frame.gpr[gpr::RSI],
                    frame.gpr[gpr::R8],
                    frame.rflags,
                );
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            eprintln!(
                "prisma: block exit reason={} rsp={:#x} rax={:#x} next={:#x}",
                frame.exit_reason,
                frame.gpr[gpr::RSP],
                frame.gpr[gpr::RAX],
                frame.next_pc
            );
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if rip == 0x1_4004_c960 {
                eprintln!(
                    "prisma: morestack result next={:#x} rflags={:#x}",
                    frame.next_pc, frame.rflags
                );
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if rip == 0x1_4001_5c3e {
                eprintln!(
                    "prisma: rep-movsq after rcx={:#x} rsi={:#x} rdi={:#x} next={:#x}",
                    frame.gpr[gpr::RCX],
                    frame.gpr[gpr::RSI],
                    frame.gpr[gpr::RDI],
                    frame.next_pc,
                );
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if let Ok(top) = memory.read_data(frame.gpr[gpr::RSP], 8) {
                if let Ok(bytes) = <[u8; 8]>::try_from(top.as_slice()) {
                    eprintln!("prisma: stack top={:#x}", u64::from_le_bytes(bytes));
                }
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if (0x1_4008_74c0..=0x1_4008_75a5).contains(&rip)
                || matches!(
                    rip,
                    0x1_4004_c053 | 0x1_4008_b520 | 0x1_4008_b538 | 0x1_4008_b545 | 0x1_4008_b5a7
                )
            {
                if let Ok(stack) = memory.read_data(frame.gpr[gpr::RSP], 32) {
                    let words = stack
                        .chunks_exact(8)
                        .map(|word| u64::from_le_bytes(word.try_into().expect("eight bytes")))
                        .collect::<Vec<_>>();
                    eprintln!("prisma: target stack rip={rip:#x} words={words:x?}");
                }
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            if (0x1_4004_a940..=0x1_4004_a980).contains(&rip) {
                let slot = memory.read_data(0x1_40db_5800, 8).ok().and_then(|bytes| {
                    <[u8; 8]>::try_from(bytes.as_slice())
                        .ok()
                        .map(u64::from_le_bytes)
                });
                eprintln!(
                    "prisma: init slot rip={rip:#x} rax={:#x} slot={slot:x?}",
                    frame.gpr[gpr::RAX]
                );
            }
            instructions = instructions.saturating_add(block.instruction_count);
            let blocks = block_index + 1;

            match frame.exit_reason {
                EXIT_BRANCH => {
                    rip = frame.next_pc;
                    context.store_frame(&frame, rip);
                }
                EXIT_NORMAL if !block.ended_at_terminator => {
                    rip = rip.wrapping_add(block.guest_bytes as u64);
                    context.store_frame(&frame, rip);
                }
                EXIT_NORMAL => {
                    context.store_frame(&frame, rip);
                    return Ok(DispatchReport {
                        stop: DispatchStop::NativeTransitionRequired,
                        blocks,
                        instructions,
                        rip,
                    });
                }
                EXIT_SYSCALL => {
                    let status = dispatch_win64_syscall(memory, &frame, rip)?;
                    frame.gpr[gpr::RAX] = u64::from(u32::from_ne_bytes(status.to_ne_bytes()));
                    rip = rip.wrapping_add(block.guest_bytes as u64);
                    context.store_frame(&frame, rip);
                }
                reason => {
                    context.store_frame(&frame, rip);
                    return Err(DispatchError::UnknownExitReason { rip, reason });
                }
            }
        }

        Ok(DispatchReport {
            stop: DispatchStop::BlockLimit,
            blocks: limits.max_blocks,
            instructions,
            rip,
        })
    }
}

fn translate_dispatch_block(
    translator: &mut Translator,
    rip: u64,
    bytes: &[u8],
    max_instructions: usize,
) -> Result<BlockTranslation, TranslateError> {
    if max_instructions == 1 {
        return translator.translate_block(rip, bytes, 1);
    }

    #[cfg(all(windows, target_arch = "arm64ec"))]
    if rip == 0x1_4004_c009 {
        eprintln!("prisma: diagnostic translate begin c009");
    }
    #[cfg(all(windows, target_arch = "arm64ec"))]
    if rip == 0x1_4008_9a41 {
        if let Ok(block) = translator.optimize_fused_block(rip, bytes, max_instructions) {
            eprintln!("prisma: diagnostic IR {:#?}", block.func);
        }
    }
    // Preserve arithmetic/test NZCV through its terminating Jcc by preferring
    // one fused lowering unit. If the migration backend rejects that unit
    // (notably vector-register pressure), retry with isolated cached
    // instructions; those fallback blocks contain no cross-instruction flags.
    let fused = translator.translate_fused_block(rip, bytes, max_instructions);
    #[cfg(all(windows, target_arch = "arm64ec"))]
    if rip == 0x1_4004_c009 {
        eprintln!("prisma: diagnostic translate end c009 ok={}", fused.is_ok());
    }
    match fused {
        Ok(block) => Ok(block),
        Err(TranslateError::Lower(_)) => translator.translate_block(rip, bytes, max_instructions),
        Err(error) => Err(error),
    }
}

impl Drop for ThreadRuntime {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        LIVE_RUNTIMES.fetch_sub(1, Ordering::AcqRel);
    }
}

struct DispatchLease<'a>(&'a ThreadRuntime);

impl<'a> DispatchLease<'a> {
    fn new(runtime: &'a ThreadRuntime) -> Self {
        runtime.active_dispatches.fetch_add(1, Ordering::AcqRel);
        Self(runtime)
    }
}

impl Drop for DispatchLease<'_> {
    fn drop(&mut self) {
        self.0.active_dispatches.fetch_sub(1, Ordering::AcqRel);
    }
}

pub fn live_runtime_count() -> usize {
    LIVE_RUNTIMES.load(Ordering::Acquire)
}

#[cfg(all(windows, target_arch = "arm64ec"))]
pub struct ProcessMemory;

#[cfg(all(windows, target_arch = "arm64ec"))]
pub(super) fn read_current_process_memory(address: u64, length: usize) -> Result<Vec<u8>, String> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
        fn ReadProcessMemory(
            process: *mut std::ffi::c_void,
            base: *const std::ffi::c_void,
            buffer: *mut std::ffi::c_void,
            size: usize,
            read: *mut usize,
        ) -> i32;
    }

    let mut bytes = vec![0_u8; length];
    let mut read = 0usize;
    // SAFETY: the destination is a valid allocation of `max_len` bytes;
    // ReadProcessMemory validates the source range in the current process.
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const std::ffi::c_void,
            bytes.as_mut_ptr().cast(),
            length,
            &raw mut read,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    bytes.truncate(read);
    Ok(bytes)
}

#[cfg(all(windows, target_arch = "arm64ec"))]
/// Writes one synthetic x64 return address into the current ARM64EC stack.
///
/// # Safety
///
/// `address` must identify eight writable bytes owned by the calling thread's
/// live Wine stack for the duration of this call.
pub unsafe fn write_current_process_u64(address: u64, value: u64) -> Result<(), String> {
    let address = usize::try_from(address)
        .map_err(|_| "stack address does not fit the native pointer width".to_owned())?;
    if address == 0 {
        return Err("stack address is null".to_owned());
    }
    // SAFETY: the caller supplies a live writable stack slot. The x64 stack is
    // only 8-byte aligned, so use an unaligned store explicitly.
    unsafe { (address as *mut u64).write_unaligned(value) };
    Ok(())
}

#[cfg(all(windows, target_arch = "arm64ec"))]
impl GuestMemory for ProcessMemory {
    fn read_code(&self, rip: u64, max_len: usize) -> Result<Vec<u8>, String> {
        read_current_process_memory(rip, max_len)
    }

    fn read_data(&self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        let bytes = read_current_process_memory(address, length)?;
        if bytes.len() != length {
            return Err(format!(
                "short process-memory read: expected {length} bytes, got {}",
                bytes.len()
            ));
        }
        Ok(bytes)
    }
}

#[cfg(any(test, all(windows, target_arch = "arm64ec")))]
#[repr(C)]
struct ChpeV2CpuAreaInfo {
    in_simulation: u8,
    in_syscall_callback: u8,
    padding: [u8; 6],
    emulator_stack_base: u64,
    emulator_stack_limit: u64,
    context_amd64: *mut Arm64EcContext,
    suspend_doorbell: *mut u32,
    loading_module_modflag: u64,
    emulator_data: [*mut std::ffi::c_void; 4],
    emulator_data_inline: u64,
}

#[cfg(any(test, all(windows, target_arch = "arm64ec")))]
unsafe fn context_from_cpu_area<'a>(
    area: *mut ChpeV2CpuAreaInfo,
) -> Result<&'a mut Arm64EcContext, DispatchError> {
    // SAFETY: the caller guarantees that a non-null pointer refers to Wine's
    // current-thread CHPE area for the duration of the returned borrow.
    let area = unsafe { area.as_mut() }.ok_or(DispatchError::ContextUnavailable)?;
    if area.in_simulation == 0 {
        return Err(DispatchError::ContextUnavailable);
    }
    // SAFETY: Wine owns and thread-serializes ContextAmd64 while simulation is
    // active. A null context is rejected without dereferencing it.
    unsafe { area.context_amd64.as_mut() }.ok_or(DispatchError::ContextUnavailable)
}

#[cfg(all(windows, target_arch = "arm64ec"))]
unsafe fn current_wine_cpu_area() -> Result<&'static mut ChpeV2CpuAreaInfo, DispatchError> {
    const CHPE_V2_CPU_AREA_OFFSET: usize = 0x1788;
    let teb: usize;
    // SAFETY: Windows ARM64 reserves x18 for the current TEB.
    unsafe { core::arch::asm!("mov {}, x18", out(reg) teb) };
    if teb == 0 {
        return Err(DispatchError::ContextUnavailable);
    }
    // SAFETY: Wine uses the Windows 64-bit TEB layout and stores the CHPE v2
    // area pointer at offset 0x1788 before invoking BeginSimulation.
    let area = unsafe {
        (teb.checked_add(CHPE_V2_CPU_AREA_OFFSET)
            .ok_or(DispatchError::ContextUnavailable)? as *const *mut ChpeV2CpuAreaInfo)
            .read()
    };
    // SAFETY: the pointer was loaded from the calling thread's TEB.
    unsafe { area.as_mut() }.ok_or(DispatchError::ContextUnavailable)
}

#[cfg(all(windows, target_arch = "arm64ec"))]
pub unsafe fn current_wine_context() -> Result<&'static mut Arm64EcContext, DispatchError> {
    // SAFETY: the caller is Wine's current simulation thread.
    let area = unsafe { current_wine_cpu_area()? };
    // SAFETY: the pointer was obtained from the current TEB, and Wine keeps the
    // area and context live for the non-returning simulation transition.
    unsafe { context_from_cpu_area(area) }
}

#[cfg(all(windows, target_arch = "arm64ec"))]
pub unsafe fn current_wine_transition_context() -> Result<&'static mut Arm64EcContext, DispatchError>
{
    // SAFETY: transition callbacks run on the thread that owns this CHPE area.
    let area = unsafe { current_wine_cpu_area()? };
    // SAFETY: Wine owns and serializes ContextAmd64 across the transition.
    unsafe { area.context_amd64.as_mut() }.ok_or(DispatchError::ContextUnavailable)
}

#[cfg(all(windows, target_arch = "arm64ec"))]
pub unsafe fn capture_native_context(context: &mut Arm64EcContext) {
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlCaptureContext(context: *mut Arm64EcContext);
    }

    // SAFETY: the destination is Wine's writable 0x4d0-byte hybrid context.
    unsafe { RtlCaptureContext(context) };
}

#[cfg(all(windows, target_arch = "arm64ec"))]
pub unsafe fn set_simulation_active(active: bool) -> Result<(), DispatchError> {
    // SAFETY: transition callbacks run on the thread that owns this CHPE area.
    let area = unsafe { current_wine_cpu_area()? };
    area.in_simulation = u8::from(active);
    Ok(())
}

#[cfg(all(windows, target_arch = "arm64ec"))]
pub unsafe fn resume_wine_context(context: &mut Arm64EcContext) -> ! {
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtContinue(context: *mut Arm64EcContext, alertable: i32) -> i32;
    }

    // SAFETY: the CHPE area belongs to this thread. Clearing the flag hands
    // context ownership back to Wine before the non-returning continuation.
    let _ = unsafe { set_simulation_active(false) };

    // SAFETY: `context` is Wine's live AMD64-compatible context. NtContinue
    // either resumes x64 through KiUserEmulationDispatcher or restores an EC
    // target natively; success does not return.
    eprintln!(
        "prisma: resume native rip={:#x} rsp={:#x} lr={:#x} x9={:#x} flags={:#x}",
        context.pc_rip,
        context.sp_rsp,
        context.tail.arm64_lr,
        context.tail.arm64_x9,
        context.context_flags
    );
    let status = unsafe { NtContinue(context, 0) };
    // SAFETY: reaching this path means continuation failed. Terminating the
    // exact current Wine process prevents execution of KiUserEmulationDispatcher's
    // deliberate `brk #1` with a partially transferred context.
    // SAFETY: continuation failed and the current process cannot safely resume.
    unsafe { terminate_current_process(status) }
}

#[cfg(all(windows, target_arch = "arm64ec"))]
pub unsafe fn terminate_current_process(status: i32) -> ! {
    const CURRENT_PROCESS_PSEUDO_HANDLE: isize = -1;

    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtTerminateProcess(process: *mut std::ffi::c_void, status: i32) -> i32;
    }

    // SAFETY: the pseudo-handle identifies only the calling Wine process.
    let _ = unsafe {
        NtTerminateProcess(
            CURRENT_PROCESS_PSEUDO_HANDLE as *mut std::ffi::c_void,
            status,
        )
    };
    std::process::abort()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    struct StackMemory {
        address: u64,
        bytes: Vec<u8>,
    }

    impl GuestMemory for StackMemory {
        fn read_code(&self, _rip: u64, _max_len: usize) -> Result<Vec<u8>, String> {
            Err("code read is not part of this fixture".to_owned())
        }

        fn read_data(&self, address: u64, length: usize) -> Result<Vec<u8>, String> {
            if address != self.address || length != self.bytes.len() {
                return Err("unexpected stack range".to_owned());
            }
            Ok(self.bytes.clone())
        }
    }

    #[test]
    fn generated_wine_11_14_syscall_table_is_dense_and_exact() {
        assert_eq!(WINE_SYSCALLS.len(), 264);
        assert_eq!(
            wine_syscall_entry(0x0f),
            Some(&WineSyscallEntry {
                name: b"NtClose\0",
                argument_bytes: 8,
            })
        );
        assert_eq!(
            wine_syscall_entry(0x55),
            Some(&WineSyscallEntry {
                name: b"NtCreateFile\0",
                argument_bytes: 88,
            })
        );
        assert!(wine_syscall_entry(264).is_none());
        assert!(wine_syscall_entry(u64::MAX).is_none());
        assert_eq!(
            WINE_SYSCALLS.iter().map(|entry| entry.argument_bytes).max(),
            Some(128)
        );
    }

    #[test]
    fn win64_syscall_arguments_use_r10_registers_then_rsp_shadow_space() {
        let mut frame = CpuStateFrame::default();
        frame.gpr[gpr::R10] = 10;
        frame.gpr[gpr::RDX] = 20;
        frame.gpr[gpr::R8] = 30;
        frame.gpr[gpr::R9] = 40;
        frame.gpr[gpr::RSP] = 0x2000;
        let stack_values = [50_u64, 60, 70];
        let bytes = stack_values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        let memory = StackMemory {
            address: 0x2028,
            bytes,
        };

        let arguments = marshal_win64_syscall_arguments(&memory, &frame, 7 * 8).unwrap();
        assert_eq!(arguments, [10, 20, 30, 40, 50, 60, 70]);
        assert!(marshal_win64_syscall_arguments(&memory, &frame, 17 * 8).is_err());
    }

    #[test]
    fn context_prefix_matches_wine_11_14_offsets() {
        assert_eq!(offset_of!(Arm64EcContext, context_flags), 0x30);
        assert_eq!(offset_of!(Arm64EcContext, e_flags), 0x44);
        assert_eq!(offset_of!(Arm64EcContext, x8_rax), 0x78);
        assert_eq!(offset_of!(Arm64EcContext, sp_rsp), 0x98);
        assert_eq!(offset_of!(Arm64EcContext, pc_rip), 0xf8);
        assert_eq!(offset_of!(Arm64EcContext, tail), 0x100);
        assert_eq!(offset_of!(Arm64EcContextTail, arm64_lr), 0x20);
        assert_eq!(offset_of!(Arm64EcContextTail, arm64_x9), 0x50);
        assert_eq!(offset_of!(Arm64EcContextTail, xmm), 0xa0);
        assert_eq!(offset_of!(Arm64EcContextTail, vector_register), 0x200);
        assert_eq!(
            offset_of!(Arm64EcContextTail, last_exception_from_rip),
            0x3c8
        );
        assert_eq!(size_of::<Arm64EcContext>(), 0x4d0);
        assert_eq!(offset_of!(ChpeV2CpuAreaInfo, context_amd64), 0x18);
        assert_eq!(size_of::<ChpeV2CpuAreaInfo>(), 0x58);
    }

    #[test]
    fn active_jit_frame_restores_the_wine_exception_context() {
        let mut frame = CpuStateFrame::default();
        frame.gpr[gpr::RAX] = 0x1_0000_1234;
        frame.gpr[gpr::R8] = 0x2_0000_5678;
        frame.gpr[gpr::RSP] = 0x3_0000_0000;
        frame.rflags = 0x246;
        frame.next_pc = 0x1_4000_2010;
        let mut context = Arm64EcContext::default();

        {
            let _guard = ActiveJitFrameGuard::enter(&mut frame, 0x1_4000_2000);
            // SAFETY: both test-owned objects remain live for the callback.
            assert!(unsafe { reset_active_exception_context(&raw mut context) });
            // Recovery consumes the publication because Wine can abandon the
            // Rust JIT stack after returning from this callback.
            assert!(!unsafe { reset_active_exception_context(&raw mut context) });
        }

        assert_eq!(context.x8_rax, frame.gpr[gpr::RAX]);
        assert_eq!(context.x2_r8, frame.gpr[gpr::R8]);
        assert_eq!(context.sp_rsp, frame.gpr[gpr::RSP]);
        assert_eq!(context.e_flags, u32::try_from(frame.rflags).unwrap());
        assert_eq!(context.pc_rip, frame.next_pc);
        // SAFETY: no guard is active after leaving the scope.
        assert!(!unsafe { reset_active_exception_context(&raw mut context) });
    }

    #[test]
    fn active_jit_frame_falls_back_to_the_block_start_before_first_marker() {
        let mut frame = CpuStateFrame::default();
        let mut context = Arm64EcContext::default();
        let block_start = 0x1_4000_3000;

        {
            let _guard = ActiveJitFrameGuard::enter(&mut frame, block_start);
            // SAFETY: both test-owned objects remain live for the callback.
            assert!(unsafe { reset_active_exception_context(&raw mut context) });
        }

        assert_eq!(context.pc_rip, block_start);
    }

    #[test]
    fn default_wine_dispatch_keeps_exact_guest_instruction_boundaries() {
        assert_eq!(DispatchLimits::default().max_blocks, 1_000_000);
        assert_eq!(DispatchLimits::default().max_instructions_per_block, 1);
    }

    #[test]
    fn exact_boundary_dispatch_uses_the_non_fused_translation_path() {
        let rip = 0x1_4000_1000;
        let bytes = [0x48, 0x89, 0xd8];
        let mut dispatch_translator = Translator::new();
        let mut baseline_translator = Translator::new();

        let actual = translate_dispatch_block(&mut dispatch_translator, rip, &bytes, 1).unwrap();
        let expected = baseline_translator.translate_block(rip, &bytes, 1).unwrap();

        assert_eq!(actual, expected);
    }

    #[test]
    fn thread_runtime_reuses_and_explicitly_clears_single_instruction_translations() {
        let runtime = ThreadRuntime::new();
        let rip = 0x1_4000_2000;
        let bytes = [0xb8, 0x2a, 0, 0, 0];
        let mut first_translator = Translator::new();
        let first = runtime
            .translate_block_cached(&mut first_translator, rip, &bytes, 1)
            .unwrap();
        assert_eq!(first_translator.stats().cache_misses, 1);

        let mut second_translator = Translator::new();
        let second = runtime
            .translate_block_cached(&mut second_translator, rip, &bytes, 1)
            .unwrap();
        assert_eq!(second, first);
        assert_eq!(second_translator.stats().total(), 0);

        runtime.clear_cache();
        let third = runtime
            .translate_block_cached(&mut second_translator, rip, &bytes, 1)
            .unwrap();
        assert_eq!(third, first);
        assert_eq!(second_translator.stats().cache_misses, 1);
    }

    #[test]
    fn dispatch_cache_replaces_stale_guest_bytes_at_the_same_rip() {
        let runtime = ThreadRuntime::new();
        let rip = 0x1_4000_3000;
        let mut translator = Translator::new();
        let first = runtime
            .translate_block_cached(&mut translator, rip, &[0xb8, 0x2a, 0, 0, 0], 1)
            .unwrap();
        let replacement = runtime
            .translate_block_cached(&mut translator, rip, &[0xb8, 0x2b, 0, 0, 0], 1)
            .unwrap();

        assert_ne!(replacement.code, first.code);
        let cache = runtime
            .translation_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(
            cache.entries.get(&rip).unwrap().source,
            [0xb8, 0x2b, 0, 0, 0]
        );
        drop(cache);
    }

    #[test]
    fn single_register_move_keeps_sequential_control_flow() {
        let mut translator = Translator::new();
        let block = translate_dispatch_block(
            &mut translator,
            0x1_4008_99e3,
            &[0x48, 0x89, 0xf3, 0x48, 0x83, 0xec, 0x28],
            1,
        )
        .expect("register move must translate");
        assert_eq!(block.instruction_count, 1);
        assert_eq!(block.guest_bytes, 3);
        assert!(!block.ended_at_terminator);
    }

    #[test]
    fn arm64ec_stack_argument_area_starts_after_the_x64_return_address() {
        assert_eq!(arm64ec_stack_argument_base(0x1000).unwrap(), 0x1008);
        assert!(matches!(
            arm64ec_stack_argument_base(u64::MAX),
            Err(DispatchError::MemoryRead { rip: u64::MAX, .. })
        ));
    }

    #[test]
    fn loaded_module_return_updates_only_result_stack_and_control_flow() {
        let mut frame = CpuStateFrame::default();
        frame.gpr[gpr::RCX] = 0x1111;
        frame.gpr[gpr::RDX] = 0x2222;
        frame.gpr[gpr::RSP] = 0x3000;
        frame.rflags = 0x246;

        let next = complete_loaded_module_return(&mut frame, 0x3000, 0x4000, 0x5000)
            .expect("valid x64 stack must accept the native return");

        assert_eq!(next, 0x4000);
        assert_eq!(frame.gpr[gpr::RAX], 0x5000);
        assert_eq!(frame.gpr[gpr::RSP], 0x3008);
        assert_eq!(frame.gpr[gpr::RCX], 0x1111);
        assert_eq!(frame.gpr[gpr::RDX], 0x2222);
        assert_eq!(frame.rflags, 0x246);
    }

    #[test]
    fn loaded_module_reuse_accepts_only_normal_image_search_modes() {
        assert!(loaded_module_reuse_flags(0));
        assert!(loaded_module_reuse_flags(0x800));
        assert!(!loaded_module_reuse_flags(0x2));
        assert!(!loaded_module_reuse_flags(0x20));
        assert!(!loaded_module_reuse_flags(0x8000_0000));
    }

    #[test]
    fn dispatch_translation_falls_back_under_vector_pressure() {
        let bytes = [
            0x66, 0x0f, 0x7f, 0x44, 0x24, 0x20, // movdqa [rsp+20h], xmm0
            0x66, 0x0f, 0x7f, 0x4c, 0x24, 0x30, // movdqa [rsp+30h], xmm1
            0x66, 0x0f, 0x7f, 0x54, 0x24, 0x40, // movdqa [rsp+40h], xmm2
            0x66, 0x0f, 0x7f, 0x5c, 0x24, 0x50, // movdqa [rsp+50h], xmm3
        ];
        let mut translator = Translator::new();
        let block = translate_dispatch_block(&mut translator, 0x1_4000_168a, &bytes, 64)
            .expect("isolated instruction lowering should handle all four stores");
        assert_eq!(block.instruction_count, 4);
        assert_eq!(block.guest_bytes, bytes.len());
        assert!(!block.ended_at_terminator);
        assert!(!block.code.is_empty());
    }

    #[test]
    fn translates_go_runtime_write_compare_branch_block() {
        let bytes = [
            0x48, 0x89, 0x9a, 0x30, 0x05, 0x00, 0x00, 0x83, 0xba, 0xe0, 0x00, 0x00, 0x00, 0x00,
            0x74, 0x3a,
        ];
        let mut translator = Translator::new();
        let block = translate_dispatch_block(&mut translator, 0x1_4004_c009, &bytes, 64)
            .expect("Go runtime block must translate without host exceptions");
        assert_eq!(block.instruction_count, 3);
        assert_eq!(block.guest_bytes, bytes.len());
        assert!(block.ended_at_terminator);
    }

    #[test]
    fn diagnoses_go_systemstack_argument_setup() {
        let bytes = [
            0x31, 0xc0, 0x48, 0x89, 0x54, 0x24, 0x20, 0x88, 0x44, 0x24, 0x1f, 0x48, 0x8b, 0x05,
            0x9b, 0x97, 0xd6, 0x00, 0x48, 0x89, 0x04, 0x24, 0x48, 0x8d, 0x82, 0x20, 0x05, 0x00,
            0x00, 0x48, 0x89, 0x44, 0x24, 0x08, 0xe8, 0xa6, 0xf4, 0x03, 0x00,
        ];
        let mut translator = Translator::new();
        let optimized = translator
            .optimize_fused_block(0x1_4004_c053, &bytes, 64)
            .expect("Go systemstack setup must optimize");
        eprintln!("{:#?}", optimized.func);
        assert_eq!(optimized.instruction_count, 8);
    }

    #[test]
    fn diagnoses_go_morestack_guard_branch() {
        let bytes = [0x49, 0x3b, 0x66, 0x10, 0x0f, 0x86, 0x25, 0x01, 0x00, 0x00];
        let mut translator = Translator::new();
        let optimized = translator
            .optimize_fused_block(0x1_4004_c960, &bytes, 64)
            .expect("Go stack guard must optimize");
        eprintln!("{:#?}", optimized.func);
        assert_eq!(optimized.instruction_count, 2);
    }

    #[test]
    fn context_round_trip_preserves_integer_registers_and_rip() {
        let mut context = Arm64EcContext {
            x8_rax: 1,
            x0_rcx: 2,
            x22_r15: 15,
            e_flags: 0x203,
            pc_rip: 0x1234,
            ..Arm64EcContext::default()
        };
        context.tail.xmm = [XmmRegister {
            low: 0x1122_3344_5566_7788,
            high: 0x99aa_bbcc_ddee_ff00,
        }; XMM_REGISTER_COUNT];
        let mut frame = context.load_frame();
        frame.gpr[gpr::RAX] = 99;
        assert!(frame.set_xmm(7, [0x5a; 16]));
        context.store_frame(&frame, 0x5678);
        assert_eq!(context.x8_rax, 99);
        assert_eq!(context.x0_rcx, 2);
        assert_eq!(context.x22_r15, 15);
        assert_eq!(context.e_flags, 0x203);
        assert_eq!(context.pc_rip, 0x5678);
        assert_eq!(context.tail.xmm[0].low, 0x1122_3344_5566_7788);
        assert_eq!(context.tail.xmm[7], XmmRegister::from_bytes([0x5a; 16]));
    }

    #[test]
    fn cpu_area_requires_active_simulation_and_returns_exact_context() {
        let mut context = Arm64EcContext {
            pc_rip: 0x1234,
            ..Arm64EcContext::default()
        };
        let mut area = ChpeV2CpuAreaInfo {
            in_simulation: 0,
            in_syscall_callback: 0,
            padding: [0; 6],
            emulator_stack_base: 0,
            emulator_stack_limit: 0,
            context_amd64: &raw mut context,
            suspend_doorbell: std::ptr::null_mut(),
            loading_module_modflag: 0,
            emulator_data: [std::ptr::null_mut(); 4],
            emulator_data_inline: 0,
        };
        // SAFETY: `area` and `context` are live and uniquely owned here.
        assert!(unsafe { context_from_cpu_area(&raw mut area) }.is_err());
        area.in_simulation = 1;
        // SAFETY: active area and its unique context remain live for the borrow.
        let bound = unsafe { context_from_cpu_area(&raw mut area) }.unwrap();
        bound.pc_rip = 0x5678;
        assert_eq!(context.pc_rip, 0x5678);
    }
}
