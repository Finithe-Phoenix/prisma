use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use prisma_runtime::executor::{
    gpr, CpuStateFrame, ExecError, EXIT_BRANCH, EXIT_NORMAL, EXIT_SYSCALL,
};
use prisma_translator::{TranslateError, Translator};

/// Wine 11.14's ARM64EC context prefix, through the AMD64 instruction pointer.
///
/// The unusual ARM register names are the documented ARM64EC aliases for the
/// corresponding AMD64 registers. The remainder of `ARM64EC_NT_CONTEXT` is not
/// accessed by the initial integer dispatch bridge.
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
}

impl Arm64EcContext {
    fn load_frame(&self) -> CpuStateFrame {
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
        frame
    }

    fn store_frame(&mut self, frame: &CpuStateFrame, rip: u64) {
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
                    context.store_frame(&frame, rip);
                    return Err(DispatchError::UnsupportedSyscall { rip });
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
impl GuestMemory for ProcessMemory {
    fn read_code(&self, rip: u64, max_len: usize) -> Result<Vec<u8>, String> {
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

        let mut bytes = vec![0_u8; max_len];
        let mut read = 0usize;
        // SAFETY: the destination is a valid allocation of `max_len` bytes;
        // ReadProcessMemory validates the source range in the current process.
        let ok = unsafe {
            ReadProcessMemory(
                GetCurrentProcess(),
                rip as *const std::ffi::c_void,
                bytes.as_mut_ptr().cast(),
                max_len,
                &raw mut read,
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        bytes.truncate(read);
        Ok(bytes)
    }
}

#[cfg(all(windows, target_arch = "arm64ec"))]
pub unsafe fn current_wine_context() -> Result<&'static mut Arm64EcContext, DispatchError> {
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
    if area.is_null() {
        return Err(DispatchError::ContextUnavailable);
    }
    // SAFETY: the non-null CPU area belongs to the current Wine thread.
    let context = unsafe { (*area).context_amd64 };
    // SAFETY: Wine owns this context and serializes its use on this thread.
    unsafe { context.as_mut() }.ok_or(DispatchError::ContextUnavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    #[test]
    fn context_prefix_matches_wine_11_14_offsets() {
        assert_eq!(offset_of!(Arm64EcContext, context_flags), 0x30);
        assert_eq!(offset_of!(Arm64EcContext, e_flags), 0x44);
        assert_eq!(offset_of!(Arm64EcContext, x8_rax), 0x78);
        assert_eq!(offset_of!(Arm64EcContext, sp_rsp), 0x98);
        assert_eq!(offset_of!(Arm64EcContext, pc_rip), 0xf8);
        assert_eq!(size_of::<Arm64EcContext>(), 0x100);
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
        let mut frame = context.load_frame();
        frame.gpr[gpr::RAX] = 99;
        context.store_frame(&frame, 0x5678);
        assert_eq!(context.x8_rax, 99);
        assert_eq!(context.x0_rcx, 2);
        assert_eq!(context.x22_r15, 15);
        assert_eq!(context.e_flags, 0x203);
        assert_eq!(context.pc_rip, 0x5678);
    }
}
