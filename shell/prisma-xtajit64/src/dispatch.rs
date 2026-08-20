#[cfg(not(all(windows, target_arch = "arm64ec")))]
use std::cell::Cell;
use std::collections::{BTreeMap, VecDeque};
use std::fmt;
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
    true
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
                write!(formatter, "ARM64 execution failed at {rip:#x}: {source}")
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

#[cfg(any(test, all(windows, target_arch = "arm64ec")))]
impl DispatchError {
    pub(super) const fn diagnostic_marker(&self) -> &'static [u8] {
        match self {
            Self::InvalidLimits => b"prisma-error: invalid-limits\n",
            Self::MemoryRead { .. } => b"prisma-error: memory-read\n",
            Self::Translation { source, .. } => match source {
                TranslateError::Decode(_) => b"prisma-error: translation-decode\n",
                TranslateError::Lower(_) => b"prisma-error: translation-lower\n",
                TranslateError::Truncated { .. } => b"prisma-error: translation-truncated\n",
            },
            Self::Execution { source, .. } => match source {
                ExecError::Alloc(_) => b"prisma-error: execution-alloc\n",
                ExecError::Write => b"prisma-error: execution-write\n",
                ExecError::Protect(_) => b"prisma-error: execution-protect\n",
                ExecError::WrongArch => b"prisma-error: execution-wrong-arch\n",
                ExecError::HostStateCorruption { .. } => b"prisma-error: execution-host-state\n",
            },
            Self::UnsupportedSyscall { .. } => b"prisma-error: unsupported-syscall\n",
            Self::UnknownSyscall { .. } => b"prisma-error: unknown-syscall\n",
            Self::SyscallArguments { .. } => b"prisma-error: syscall-arguments\n",
            Self::SyscallResolution { .. } => b"prisma-error: syscall-resolution\n",
            Self::UnknownExitReason { .. } => b"prisma-error: unknown-exit-reason\n",
            Self::ContextUnavailable => b"prisma-error: context-unavailable\n",
        }
    }
}

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
    if target_rva == 0x9_b6e4 {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetModuleHandleW(name: *const u16) -> *mut std::ffi::c_void;
        }
        #[link(name = "ntdll")]
        unsafe extern "system" {
            fn LdrAddRefDll(flags: u32, module: *mut std::ffi::c_void) -> i32;
        }
        // SAFETY: RCX is the live LoadLibraryExW UTF-16 argument.
        let loaded = unsafe { GetModuleHandleW(frame.gpr[gpr::RCX] as usize as *const u16) };
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
        if add_ref_status.is_some_and(|status| status >= 0) {
            let next = complete_loaded_module_return(
                frame,
                stack,
                return_address,
                loaded as usize as u64,
            )?;
            context.store_frame(frame, next);
            return Ok(Arm64EcEntry::Returned(next));
        }
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
    /// guest threads concurrently. The executor owns `code` so production can
    /// release its heap descriptor before entering untrusted guest JIT code.
    ///
    /// # Errors
    ///
    /// Returns the real runtime allocation, protection, or architecture error.
    fn execute(
        &self,
        guest_rip: u64,
        code: Vec<u8>,
        frame: &mut CpuStateFrame,
    ) -> Result<(), ExecError>;
}

#[cfg(any(test, target_arch = "arm64ec"))]
const MAX_JIT_CACHE_BYTES: usize = 128 * 1024 * 1024;

#[cfg(any(test, target_arch = "arm64ec"))]
#[derive(Debug, Default)]
struct JitCache {
    buffers: Vec<(Vec<u8>, prisma_runtime::jit_memory::ExecBuffer)>,
    bytes: usize,
}

#[cfg(any(test, target_arch = "arm64ec"))]
impl JitCache {
    fn get(&self, code: &[u8]) -> Option<&prisma_runtime::jit_memory::ExecBuffer> {
        self.buffers
            .iter()
            .find_map(|(cached, buffer)| (cached.as_slice() == code).then_some(buffer))
    }

    fn checked_total(&self, capacity: usize) -> Result<usize, ExecError> {
        let new_total = self.bytes.checked_add(capacity).ok_or_else(|| {
            ExecError::Alloc(std::io::Error::other("ARM64EC JIT cache size overflow"))
        })?;
        if new_total > MAX_JIT_CACHE_BYTES {
            return Err(ExecError::Alloc(std::io::Error::other(
                "ARM64EC JIT cache reached its 128 MiB limit",
            )));
        }
        Ok(new_total)
    }
}

/// Per-thread owner of executable translations.
///
/// Keeping an executable page associated with one code body prevents QEMU/Wine
/// from observing a stale translation when `VirtualAlloc` reuses an address.
/// The enclosing thread context drops the whole bounded cache at `ThreadTerm`.
#[derive(Debug, Default)]
pub struct PrismaExecutor {
    #[cfg(any(test, target_arch = "arm64ec"))]
    cache: Mutex<JitCache>,
}

#[cfg(any(test, target_arch = "arm64ec"))]
impl PrismaExecutor {
    fn cached_entry(&self, code: &[u8]) -> Option<*const u8> {
        self.cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(code)
            .map(prisma_runtime::jit_memory::ExecBuffer::as_ptr)
    }

    fn publish_entry(
        &self,
        code: Vec<u8>,
        buffer: prisma_runtime::jit_memory::ExecBuffer,
    ) -> Result<*const u8, ExecError> {
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(entry) = cache
            .get(&code)
            .map(prisma_runtime::jit_memory::ExecBuffer::as_ptr)
        {
            drop(cache);
            drop(buffer);
            drop(code);
            return Ok(entry);
        }
        let new_total = match cache.checked_total(buffer.capacity()) {
            Ok(new_total) => new_total,
            Err(error) => {
                drop(cache);
                drop(buffer);
                drop(code);
                return Err(error);
            }
        };
        let entry = buffer.as_ptr();
        cache.bytes = new_total;
        cache.buffers.push((code, buffer));
        drop(cache);
        Ok(entry)
    }
}

#[cfg(target_arch = "arm64ec")]
unsafe fn execute_arm64_jit(entry: *const u8, frame: *mut CpuStateFrame) -> usize {
    let register_mask: usize;
    // SAFETY: the caller owns an executable ARM64 block and a live state
    // frame. The outer save area contains any backend ABI defect before it can
    // escape into Rust; x0 returns the x18..x29 corruption mask after every
    // original nonvolatile value has been restored.
    unsafe {
        core::arch::asm!(
            "sub sp, sp, #96",
            "stp x18, x19, [sp, #0]",
            "stp x20, x21, [sp, #16]",
            "stp x22, x23, [sp, #32]",
            "stp x24, x25, [sp, #48]",
            "stp x26, x27, [sp, #64]",
            "stp x28, x29, [sp, #80]",
            "blr {entry}",
            "mov x9, xzr",
            "ldp x10, x11, [sp, #0]",
            "cmp x18, x10",
            "cset x12, ne",
            "orr x9, x9, x12",
            "cmp x19, x11",
            "cset x12, ne",
            "orr x9, x9, x12, lsl #1",
            "ldp x10, x11, [sp, #16]",
            "cmp x20, x10",
            "cset x12, ne",
            "orr x9, x9, x12, lsl #2",
            "cmp x21, x11",
            "cset x12, ne",
            "orr x9, x9, x12, lsl #3",
            "ldp x10, x11, [sp, #32]",
            "cmp x22, x10",
            "cset x12, ne",
            "orr x9, x9, x12, lsl #4",
            "cmp x23, x11",
            "cset x12, ne",
            "orr x9, x9, x12, lsl #5",
            "ldp x10, x11, [sp, #48]",
            "cmp x24, x10",
            "cset x12, ne",
            "orr x9, x9, x12, lsl #6",
            "cmp x25, x11",
            "cset x12, ne",
            "orr x9, x9, x12, lsl #7",
            "ldp x10, x11, [sp, #64]",
            "cmp x26, x10",
            "cset x12, ne",
            "orr x9, x9, x12, lsl #8",
            "cmp x27, x11",
            "cset x12, ne",
            "orr x9, x9, x12, lsl #9",
            "ldp x10, x11, [sp, #80]",
            "cmp x28, x10",
            "cset x12, ne",
            "orr x9, x9, x12, lsl #10",
            "cmp x29, x11",
            "cset x12, ne",
            "orr x9, x9, x12, lsl #11",
            "ldp x18, x19, [sp, #0]",
            "ldp x20, x21, [sp, #16]",
            "ldp x22, x23, [sp, #32]",
            "ldp x24, x25, [sp, #48]",
            "ldp x26, x27, [sp, #64]",
            "ldp x28, x29, [sp, #80]",
            "add sp, sp, #96",
            "mov x0, x9",
            entry = in(reg) entry,
            inlateout("x0") frame => register_mask,
            clobber_abi("C"),
        );
    }
    register_mask
}

impl BlockExecutor for PrismaExecutor {
    // Keep allocation, publication, execution and cache ownership in one
    // boundary: splitting it would make non-local Wine recovery unsound.
    fn execute(
        &self,
        guest_rip: u64,
        code: Vec<u8>,
        frame: &mut CpuStateFrame,
    ) -> Result<(), ExecError> {
        #[cfg(target_arch = "arm64ec")]
        {
            use prisma_runtime::executor::wrap_block;
            use prisma_runtime::jit_memory::ExecBuffer;

            let entry = if let Some(entry) = self.cached_entry(&code) {
                drop(code);
                entry
            } else {
                let callable = wrap_block(&code);
                let mut buffer = ExecBuffer::alloc(callable.len()).map_err(ExecError::Alloc)?;
                if !buffer.write(&callable) {
                    return Err(ExecError::Write);
                }
                buffer.make_executable().map_err(ExecError::Protect)?;
                self.publish_entry(code, buffer)?
            };
            super::phase_marker(b"prisma-phase: jit-cache-ready\n");
            let sp_before: usize;
            let teb_before: usize;
            // SAFETY: these register reads establish the host-state invariants
            // checked after returning from generated code.
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
            // issue the native branch directly and pass the frame in x0. The
            // outer save area is an independent safety boundary: generated
            // code must preserve x18..x29, but a backend defect must not be
            // allowed to corrupt the Rust caller before it can be diagnosed.
            super::phase_marker(b"prisma-phase: jit-enter\n");
            let register_mask = {
                let active_jit_frame = ActiveJitFrameGuard::enter(frame, guest_rip);
                // SAFETY: `entry` and `frame` stay live for this exact invocation.
                let register_mask =
                    unsafe { execute_arm64_jit(entry, frame as *mut CpuStateFrame) };
                drop(active_jit_frame);
                register_mask
            };
            let sp_after: usize;
            let teb_after: usize;
            // SAFETY: these register reads complete the host-state invariant checks.
            unsafe {
                core::arch::asm!(
                    "mov {sp}, sp",
                    "mov {teb}, x18",
                    sp = out(reg) sp_after,
                    teb = out(reg) teb_after,
                    options(nomem, nostack, preserves_flags),
                );
            }
            super::phase_marker(b"prisma-phase: jit-returned\n");
            if register_mask != 0 {
                super::phase_marker(b"prisma-error: jit-host-state-detected\n");
                return Err(ExecError::HostStateCorruption {
                    register_mask: u16::try_from(register_mask)
                        .expect("ARM64 nonvolatile mask fits u16"),
                });
            }
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
            prisma_runtime::executor::execute_block(&code, frame)
        }
    }
}

static LIVE_RUNTIMES: AtomicUsize = AtomicUsize::new(0);

pub struct ThreadRuntime {
    cancel: AtomicBool,
    invalidate_cache: AtomicBool,
    active_dispatches: AtomicUsize,
    translation_cache: Mutex<DispatchTranslationCache>,
}

const MAX_DISPATCH_CACHE_ENTRIES: usize = 65_536;
const MAX_DISPATCH_CACHE_BYTES: usize = 32 * 1024 * 1024;

const fn persistent_dispatch_cache_enabled() -> bool {
    // Wine resumes ARM64EC execution with NtContinue, abandoning the native
    // Rust stack between x64 and EC regions. Until the persistent translation
    // owner is proven across that non-local boundary, correctness requires
    // translating from the current process bytes on every re-entry.
    !cfg!(target_arch = "arm64ec")
}

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
        self.translate_block_with_cache_policy(
            translator,
            rip,
            bytes,
            max_instructions,
            persistent_dispatch_cache_enabled(),
        )
    }

    fn translate_block_with_cache_policy(
        &self,
        translator: &mut Translator,
        rip: u64,
        bytes: &[u8],
        max_instructions: usize,
        persistent_cache: bool,
    ) -> Result<BlockTranslation, TranslateError> {
        if !persistent_cache {
            // Wine resumes each ARM64EC block through a non-local NtContinue
            // transition. Do not retain allocator-backed translator state across
            // that boundary until its preservation contract is proven.
            translator.clear_cache();
            let result = translate_dispatch_block(translator, rip, bytes, max_instructions);
            translator.clear_cache();
            return result;
        }
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
        let mut frame = context.load_frame();
        initialize_windows_segment_bases(&mut frame);
        let mut rip = context.pc_rip;
        let mut instructions = 0usize;

        for block_index in 0..limits.max_blocks {
            #[cfg(all(windows, target_arch = "arm64ec"))]
            super::phase_marker(b"prisma-phase: dispatch-block-enter\n");
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

            let bytes = memory
                .read_code(rip, limits.max_fetch_bytes)
                .map_err(|detail| DispatchError::MemoryRead { rip, detail })?;
            #[cfg(all(windows, target_arch = "arm64ec"))]
            super::phase_marker(b"prisma-phase: guest-code-read\n");
            if bytes.is_empty() {
                return Err(DispatchError::MemoryRead {
                    rip,
                    detail: "reader returned no bytes".to_owned(),
                });
            }
            // Dispatch owns the executable cache. Keep translator state
            // block-local, and avoid constructing an optimization pipeline for
            // the production one-instruction boundary.
            let mut translator = if limits.max_instructions_per_block == 1 {
                Translator::for_dispatch()
            } else {
                Translator::new()
            };
            #[cfg(all(windows, target_arch = "arm64ec"))]
            super::phase_marker(b"prisma-phase: translator-created\n");
            let block = self
                .translate_block_cached(
                    &mut translator,
                    rip,
                    &bytes,
                    limits.max_instructions_per_block,
                )
                .map_err(|source| DispatchError::Translation { rip, source })?;
            drop(translator);
            #[cfg(all(windows, target_arch = "arm64ec"))]
            super::phase_marker(b"prisma-phase: translation-ready\n");
            let block_instruction_count = block.instruction_count;
            let block_guest_bytes = block.guest_bytes;
            let block_ended_at_terminator = block.ended_at_terminator;
            frame.exit_reason = EXIT_NORMAL;
            frame.next_pc = 0;
            let execution = executor.execute(rip, block.code, &mut frame);
            #[cfg(all(windows, target_arch = "arm64ec"))]
            super::phase_marker(b"prisma-phase: dispatch-executor-returned\n");
            execution.map_err(|source| DispatchError::Execution { rip, source })?;
            instructions = instructions.saturating_add(block_instruction_count);
            let blocks = block_index + 1;

            match frame.exit_reason {
                EXIT_BRANCH => {
                    rip = frame.next_pc;
                    context.store_frame(&frame, rip);
                }
                EXIT_NORMAL if !block_ended_at_terminator => {
                    rip = rip.wrapping_add(block_guest_bytes as u64);
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
                    rip = rip.wrapping_add(block_guest_bytes as u64);
                    context.store_frame(&frame, rip);
                }
                reason => {
                    context.store_frame(&frame, rip);
                    return Err(DispatchError::UnknownExitReason { rip, reason });
                }
            }
            #[cfg(all(windows, target_arch = "arm64ec"))]
            super::phase_marker(b"prisma-phase: dispatch-block-complete\n");
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
        let (translation, ended_at_terminator) =
            translator.translate_dispatch_instruction(rip, bytes)?;
        return Ok(BlockTranslation {
            code: translation.code,
            instruction_count: 1,
            guest_bytes: translation.guest_bytes,
            ended_at_terminator,
            // The runtime resolves each exit from the live state frame. Static
            // CFG successors are intentionally unnecessary at this boundary.
            successors: Vec::new(),
        });
    }

    // Preserve arithmetic/test NZCV through its terminating Jcc by preferring
    // one fused lowering unit. If the migration backend rejects that unit
    // (notably vector-register pressure), retry with isolated cached
    // instructions; those fallback blocks contain no cross-instruction flags.
    match translator.translate_fused_block(rip, bytes, max_instructions) {
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
        // This runs immediately after guest JIT execution. Keep the failure
        // path allocation-simple so a rejected guest range reaches the typed
        // dispatcher diagnostic instead of invoking Windows error formatting.
        return Err("ReadProcessMemory rejected the guest range".to_owned());
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

    #[test]
    fn dispatch_diagnostics_are_static_and_classify_nested_failures() {
        assert_eq!(
            DispatchError::Execution {
                rip: 0x1000,
                source: ExecError::HostStateCorruption { register_mask: 1 },
            }
            .diagnostic_marker(),
            b"prisma-error: execution-host-state\n"
        );
        assert_eq!(
            DispatchError::Translation {
                rip: 0x2000,
                source: TranslateError::Truncated {
                    offset: 1,
                    consumed: 2,
                    remaining: 1,
                },
            }
            .diagnostic_marker(),
            b"prisma-error: translation-truncated\n"
        );
        assert_eq!(
            DispatchError::UnknownSyscall { rip: 0x3000, id: 7 }.diagnostic_marker(),
            b"prisma-error: unknown-syscall\n"
        );
    }

    #[test]
    fn jit_cache_keeps_linear_ownership_past_tree_split_sizes() {
        let executor = PrismaExecutor::default();
        for value in 0_u8..32 {
            let code = vec![
                value,
                value.wrapping_add(1),
                value.wrapping_add(2),
                value.wrapping_add(3),
            ];
            let mut buffer = prisma_runtime::jit_memory::ExecBuffer::alloc(code.len()).unwrap();
            assert!(buffer.write(&code));
            let entry = executor.publish_entry(code.clone(), buffer).unwrap();
            assert_eq!(executor.cached_entry(&code), Some(entry));
        }
        let cache = executor.cache.lock().unwrap();
        let entry_count = cache.buffers.len();
        let bytes = cache.bytes;
        let summed_capacity = cache
            .buffers
            .iter()
            .map(|(_, buffer)| buffer.capacity())
            .sum();
        drop(cache);
        assert_eq!(entry_count, 32);
        assert_eq!(bytes, summed_capacity);
    }

    #[test]
    fn executor_publish_deduplicates_concurrent_unlocked_misses() {
        let executor = std::sync::Arc::new(PrismaExecutor::default());
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
        let code = vec![0x11, 0x22, 0x33, 0x44];
        let workers: [_; 4] = std::array::from_fn(|_| {
            let executor = std::sync::Arc::clone(&executor);
            let barrier = std::sync::Arc::clone(&barrier);
            let code = code.clone();
            std::thread::spawn(move || {
                let mut candidate =
                    prisma_runtime::jit_memory::ExecBuffer::alloc(code.len()).unwrap();
                assert!(candidate.write(&code));
                barrier.wait();
                executor.publish_entry(code, candidate).unwrap() as usize
            })
        });
        let entries: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();

        assert!(entries.iter().all(|entry| *entry == entries[0]));
        assert_eq!(
            executor.cached_entry(&code).map(|entry| entry as usize),
            Some(entries[0])
        );
        let cache = executor.cache.lock().unwrap();
        let entry_count = cache.buffers.len();
        let bytes = cache.bytes;
        let capacity = cache.buffers[0].1.capacity();
        drop(cache);
        assert_eq!(entry_count, 1);
        assert_eq!(bytes, capacity);
    }

    #[test]
    fn jit_cache_rejects_capacity_overflow_and_budget_exhaustion() {
        let cache = JitCache::default();
        assert!(cache.checked_total(MAX_JIT_CACHE_BYTES + 1).is_err());

        let overflowed = JitCache {
            bytes: usize::MAX,
            ..JitCache::default()
        };
        assert!(overflowed.checked_total(1).is_err());
    }

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
    fn exact_boundary_dispatch_skips_unused_cfg_successors() {
        let rip = 0x1_4000_1000;
        let bytes = [0x48, 0x89, 0xd8];
        let mut dispatch_translator = Translator::new();
        let mut baseline_translator = Translator::new();

        let actual = translate_dispatch_block(&mut dispatch_translator, rip, &bytes, 1).unwrap();
        let expected = baseline_translator.translate_block(rip, &bytes, 1).unwrap();

        assert_eq!(actual.code, expected.code);
        assert_eq!(actual.instruction_count, expected.instruction_count);
        assert_eq!(actual.guest_bytes, expected.guest_bytes);
        assert_eq!(actual.ended_at_terminator, expected.ended_at_terminator);
        assert!(actual.successors.is_empty());
    }

    #[test]
    fn thread_runtime_owns_single_instruction_cache_without_translator_duplicates() {
        let runtime = ThreadRuntime::new();
        let rip = 0x1_4000_2000;
        let bytes = [0xb8, 0x2a, 0, 0, 0];
        let mut first_translator = Translator::new();
        let first = runtime
            .translate_block_cached(&mut first_translator, rip, &bytes, 1)
            .unwrap();
        assert_eq!(first_translator.stats().total(), 0);

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
        assert_eq!(second_translator.stats().total(), 0);
    }

    #[test]
    fn non_persistent_policy_retranslates_without_retaining_dispatch_entries() {
        let runtime = ThreadRuntime::new();
        let rip = 0x1_4000_2500;
        let bytes = [0xb8, 0x2a, 0, 0, 0];
        let mut first_translator = Translator::new();
        let first = runtime
            .translate_block_with_cache_policy(&mut first_translator, rip, &bytes, 1, false)
            .unwrap();
        assert_eq!(first_translator.stats().total(), 0);
        assert_eq!(first_translator.cached_count(), 0);
        assert_eq!(first_translator.cached_count(), 0);

        let mut second_translator = Translator::new();
        let second = runtime
            .translate_block_with_cache_policy(&mut second_translator, rip, &bytes, 1, false)
            .unwrap();
        assert_eq!(second, first);
        assert_eq!(second_translator.stats().total(), 0);
        assert_eq!(second_translator.cached_count(), 0);
        assert_eq!(second_translator.cached_count(), 0);
        for offset in 1..=32 {
            runtime
                .translate_block_with_cache_policy(
                    &mut second_translator,
                    rip + offset * 0x10,
                    &bytes,
                    1,
                    false,
                )
                .unwrap();
            assert_eq!(second_translator.cached_count(), 0);
        }
        let cache = runtime
            .translation_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(cache.entries.keys().next(), None);
        drop(cache);
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
    fn translates_go_systemstack_argument_setup() {
        let bytes = [
            0x31, 0xc0, 0x48, 0x89, 0x54, 0x24, 0x20, 0x88, 0x44, 0x24, 0x1f, 0x48, 0x8b, 0x05,
            0x9b, 0x97, 0xd6, 0x00, 0x48, 0x89, 0x04, 0x24, 0x48, 0x8d, 0x82, 0x20, 0x05, 0x00,
            0x00, 0x48, 0x89, 0x44, 0x24, 0x08, 0xe8, 0xa6, 0xf4, 0x03, 0x00,
        ];
        let mut translator = Translator::new();
        let optimized = translator
            .optimize_fused_block(0x1_4004_c053, &bytes, 64)
            .expect("Go systemstack setup must optimize");
        assert_eq!(optimized.instruction_count, 8);
    }

    #[test]
    fn translates_go_morestack_guard_branch() {
        let bytes = [0x49, 0x3b, 0x66, 0x10, 0x0f, 0x86, 0x25, 0x01, 0x00, 0x00];
        let mut translator = Translator::new();
        let optimized = translator
            .optimize_fused_block(0x1_4004_c960, &bytes, 64)
            .expect("Go stack guard must optimize");
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
