use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use prisma_runtime::executor::{
    gpr, CpuStateFrame, ExecError, EXIT_BRANCH, EXIT_NORMAL, EXIT_SYSCALL, XMM_REGISTER_COUNT,
};
use prisma_translator::{TranslateError, Translator};

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
    x87_and_arm_aliases: [[u8; 0x20]; 5],
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
            max_blocks: 4_096,
            max_fetch_bytes: 4_096,
            max_instructions_per_block: 256,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DispatchStop {
    Cancelled,
    BlockLimit,
    NativeTransitionRequired,
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
    /// # Errors
    ///
    /// Returns the real runtime allocation, protection, or architecture error.
    fn execute(&self, code: &[u8], frame: &mut CpuStateFrame) -> Result<(), ExecError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PrismaExecutor;

impl BlockExecutor for PrismaExecutor {
    fn execute(&self, code: &[u8], frame: &mut CpuStateFrame) -> Result<(), ExecError> {
        #[cfg(target_arch = "arm64ec")]
        {
            use prisma_runtime::executor::wrap_block;
            use prisma_runtime::jit_memory::ExecBuffer;

            let callable = wrap_block(code);
            let mut buffer = ExecBuffer::alloc(callable.len()).map_err(ExecError::Alloc)?;
            if !buffer.write(&callable) {
                return Err(ExecError::Write);
            }
            buffer.make_executable().map_err(ExecError::Protect)?;
            // SAFETY: `wrap_block` emits the Prisma state-frame prologue and
            // epilogue. ARM64EC uses native ARM64 instructions and passes the
            // first pointer argument in x0, matching that generated ABI.
            let entry: extern "C" fn(*mut CpuStateFrame) =
                unsafe { core::mem::transmute(buffer.as_ptr()) };
            entry(frame);
            Ok(())
        }
        #[cfg(not(target_arch = "arm64ec"))]
        prisma_runtime::executor::execute_block(code, frame)
    }
}

static LIVE_RUNTIMES: AtomicUsize = AtomicUsize::new(0);

pub struct ThreadRuntime {
    cancel: AtomicBool,
    invalidate_cache: AtomicBool,
    active_dispatches: AtomicUsize,
}

impl ThreadRuntime {
    pub fn new() -> Self {
        LIVE_RUNTIMES.fetch_add(1, Ordering::AcqRel);
        Self {
            cancel: AtomicBool::new(false),
            invalidate_cache: AtomicBool::new(false),
            active_dispatches: AtomicUsize::new(0),
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
    }

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
        // `Translator` contains non-Send pass objects. Wine dispatch is
        // thread-affine, so ownership remains on this stack and is dropped
        // before returning across the provider boundary.
        let mut translator = Translator::new();
        let mut frame = context.load_frame();
        let mut rip = context.pc_rip;
        let mut instructions = 0usize;

        for block_index in 0..limits.max_blocks {
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
            }

            let bytes = memory
                .read_code(rip, limits.max_fetch_bytes)
                .map_err(|detail| DispatchError::MemoryRead { rip, detail })?;
            if bytes.is_empty() {
                return Err(DispatchError::MemoryRead {
                    rip,
                    detail: "reader returned no bytes".to_owned(),
                });
            }
            let block = translator
                .translate_fused_block(rip, &bytes, limits.max_instructions_per_block)
                .map_err(|source| DispatchError::Translation { rip, source })?;
            frame.exit_reason = EXIT_NORMAL;
            frame.next_pc = 0;
            executor
                .execute(&block.code, &mut frame)
                .map_err(|source| DispatchError::Execution { rip, source })?;
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
fn read_current_process_memory(address: u64, length: usize) -> Result<Vec<u8>, String> {
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
pub unsafe fn current_wine_context() -> Result<&'static mut Arm64EcContext, DispatchError> {
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
    // SAFETY: the pointer was obtained from the current TEB, and Wine keeps the
    // area and context live for the non-returning simulation transition.
    unsafe { context_from_cpu_area(area) }
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
