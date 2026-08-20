//! ARM64EC provider handshake for Wine 11.14's AMD64 emulation path.
//!
//! Wine's current `xtajit64` ABI calls [`ProcessInit`] and [`ThreadInit`]
//! without arguments. This crate owns one typed context per initialized host
//! thread and releases every context and mapping on termination. The dispatch
//! bridge uses Prisma's real translator and ARM64 JIT executor; unsupported
//! Wine transition/syscall boundaries fail explicitly.

#![allow(non_snake_case)]

use std::cell::UnsafeCell;
use std::collections::BTreeMap;
use std::ffi::c_void;
use std::fmt;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

#[cfg(all(windows, target_arch = "arm64ec"))]
mod allocator;
mod dispatch;

// Wine loads xtajit64 before the initial thread owns a PE TLS block. The
// default Rust/MSVC DLL entrypoint touches TLS and cannot run at that boundary.
// This private ARM64EC entrypoint has no CRT or TLS dependency; ProcessInit
// remains the explicit provider initialization boundary defined by Wine.
#[cfg(target_arch = "arm64ec")]
core::arch::global_asm!(
    ".text",
    ".p2align 2",
    ".globl prisma_xtajit64_entry",
    "prisma_xtajit64_entry:",
    "mov w0, #1",
    "ret",
);

use dispatch::{live_runtime_count, ThreadRuntime};
pub use dispatch::{
    Arm64EcContext, BlockExecutor, DispatchError, DispatchLimits, DispatchReport, DispatchStop,
    GuestMemory, PrismaExecutor, XmmRegister,
};

pub type NtStatus = i32;
pub type WinBoolean = u8;
pub type WinBool = i32;
pub type Handle = *mut c_void;

#[cfg(all(windows, target_arch = "arm64ec"))]
fn phase_marker(message: &'static [u8]) {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetStdHandle(kind: u32) -> Handle;
        fn WriteFile(
            file: Handle,
            buffer: *const c_void,
            bytes_to_write: u32,
            bytes_written: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
    }

    const STD_ERROR_HANDLE: u32 = (-12_i32) as u32;
    let Ok(bytes_to_write) = u32::try_from(message.len()) else {
        return;
    };
    // SAFETY: both imports are direct native calls. The message is static and
    // remains live for the synchronous write; diagnostics never own the handle.
    let file = unsafe { GetStdHandle(STD_ERROR_HANDLE) };
    if file.is_null() || file.addr() == usize::MAX {
        return;
    }
    let mut bytes_written = 0_u32;
    let _ = unsafe {
        WriteFile(
            file,
            message.as_ptr().cast(),
            bytes_to_write,
            &raw mut bytes_written,
            std::ptr::null_mut(),
        )
    };
}

#[repr(C)]
pub struct ExceptionRecord {
    _private: [u8; 0],
}

#[repr(C)]
pub struct Amd64Context {
    _private: [u8; 0],
}

#[repr(C)]
pub struct Arm64NtContext {
    _private: [u8; 0],
}

#[repr(C)]
pub struct SystemCpuInformation {
    pub processor_architecture: u16,
    pub processor_level: u16,
    pub processor_revision: u16,
    pub maximum_processors: u16,
    pub processor_feature_bits: u32,
}

pub const STATUS_SUCCESS: NtStatus = 0;
pub const STATUS_NOT_SUPPORTED: NtStatus = -1_073_741_637; // 0xC00000BB
pub const STATUS_INVALID_DEVICE_STATE: NtStatus = -1_073_741_436; // 0xC0000184

/// Process-wide lifecycle observed by Wine's provider callbacks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderPhase {
    Cold,
    Initialized,
    SimulationRequested,
    TerminationPending,
}

/// State owned for one thread in the current process generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThreadPhase {
    Ready,
    Dispatching,
    Stopped,
    Failed,
}

/// Result of an idempotent process or thread initialization request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InitOutcome {
    Initialized { generation: u64 },
    AlreadyInitialized { generation: u64 },
}

/// Typed lifecycle failures translated to NTSTATUS at the exported ABI.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleError {
    ProcessNotInitialized,
    ProcessTerminating,
    ThreadNotInitialized,
    DispatchFailed,
}

impl LifecycleError {
    #[must_use]
    pub const fn nt_status(self) -> NtStatus {
        match self {
            Self::DispatchFailed => STATUS_NOT_SUPPORTED,
            Self::ProcessNotInitialized | Self::ProcessTerminating | Self::ThreadNotInitialized => {
                STATUS_INVALID_DEVICE_STATE
            }
        }
    }
}

impl fmt::Display for LifecycleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ProcessNotInitialized => "xtajit64 process is not initialized",
            Self::ProcessTerminating => "xtajit64 process termination is pending",
            Self::ThreadNotInitialized => "current xtajit64 thread is not initialized",
            Self::DispatchFailed => {
                "xtajit64 dispatch could not cross the Wine transition boundary"
            }
        })
    }
}

impl std::error::Error for LifecycleError {}

/// Read-only process state for diagnostics and lifecycle verification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProviderSnapshot {
    pub phase: ProviderPhase,
    pub generation: u64,
    pub active_threads: usize,
    pub tracked_mappings: usize,
    pub simulation_requests: usize,
    pub cache_notifications: usize,
    pub last_status: NtStatus,
    pub active_dispatches: usize,
    pub live_runtimes: usize,
    pub live_dispatch_stacks: usize,
}

/// Read-only state of the current thread context.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ThreadContextSnapshot {
    pub generation: u64,
    pub phase: ThreadPhase,
    pub last_report: Option<DispatchReport>,
    pub native_return_depth: usize,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ThreadKey(u64);

struct ThreadContext {
    generation: u64,
    phase: ThreadPhase,
    runtime: Arc<ThreadRuntime>,
    active_dispatch_calls: Arc<AtomicUsize>,
    #[cfg(all(windows, target_arch = "arm64ec"))]
    executor: Arc<PrismaExecutor>,
    last_report: Option<DispatchReport>,
    native_returns: Vec<NativeReturnFrame>,
    _dispatch_stacks: Vec<DispatchStack>,
}

struct DispatchContextLease {
    active_calls: Arc<AtomicUsize>,
}

impl DispatchContextLease {
    fn new(active_calls: Arc<AtomicUsize>) -> Self {
        active_calls.fetch_add(1, Ordering::AcqRel);
        Self { active_calls }
    }
}

impl Drop for DispatchContextLease {
    fn drop(&mut self) {
        let previous = self.active_calls.fetch_sub(1, Ordering::AcqRel);
        debug_assert_ne!(previous, 0, "dispatch call lease underflow");
    }
}

#[cfg(all(windows, target_arch = "arm64ec"))]
const DISPATCH_STACK_BYTES: usize = 1024 * 1024;
#[cfg(all(windows, target_arch = "arm64ec"))]
const MAX_NESTED_NATIVE_CALLBACKS: usize = 8;
static LIVE_DISPATCH_STACKS: AtomicUsize = AtomicUsize::new(0);
#[cfg(all(windows, target_arch = "arm64ec"))]
static THREAD_INIT_COUNT: AtomicUsize = AtomicUsize::new(0);

#[cfg(all(windows, target_arch = "arm64ec"))]
pub(crate) fn thread_init_count() -> usize {
    THREAD_INIT_COUNT.load(Ordering::Acquire)
}
/// Dedicated native stack for Rust translation and JIT orchestration.
///
/// Wine's x64 context uses the CHPE emulator stack as guest RSP. Running the
/// translator on that same stack lets legitimate guest writes above RSP
/// overwrite active Rust frames. Each Wine thread therefore owns a disjoint
/// native stack for the duration of its provider lifecycle.
struct DispatchStack {
    #[cfg(all(windows, target_arch = "arm64ec"))]
    base: *mut c_void,
    #[cfg(all(windows, target_arch = "arm64ec"))]
    size: usize,
    #[cfg(all(windows, target_arch = "arm64ec"))]
    previous_bounds: Option<StackBounds>,
}

#[cfg(all(windows, target_arch = "arm64ec"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct StackBounds {
    base: usize,
    limit: usize,
}

#[cfg(all(windows, target_arch = "arm64ec"))]
fn current_teb_stack_bounds() -> StackBounds {
    let teb: usize;
    // SAFETY: Windows ARM64/ARM64EC reserves x18 for the current TEB. The
    // first NT_TIB fields are StackBase at +0x08 and StackLimit at +0x10.
    unsafe {
        core::arch::asm!(
            "mov {teb}, x18",
            teb = out(reg) teb,
            options(nomem, nostack, preserves_flags),
        );
        StackBounds {
            base: (teb.wrapping_add(0x08) as *const usize).read(),
            limit: (teb.wrapping_add(0x10) as *const usize).read(),
        }
    }
}

#[cfg(all(windows, target_arch = "arm64ec"))]
unsafe fn set_teb_stack_bounds(bounds: StackBounds) {
    let teb: usize;
    // SAFETY: the caller owns the current thread and supplies bounds for the
    // stack on which it is about to execute. These are the current TEB fields.
    unsafe {
        core::arch::asm!(
            "mov {teb}, x18",
            teb = out(reg) teb,
            options(nomem, nostack, preserves_flags),
        );
        (teb.wrapping_add(0x08) as *mut usize).write(bounds.base);
        (teb.wrapping_add(0x10) as *mut usize).write(bounds.limit);
    }
}

#[cfg(all(windows, target_arch = "arm64ec"))]
unsafe impl Send for DispatchStack {}

impl DispatchStack {
    #[allow(clippy::unnecessary_wraps)] // Allocation is fallible on ARM64EC only.
    fn allocate() -> Result<Self, LifecycleError> {
        #[cfg(all(windows, target_arch = "arm64ec"))]
        {
            const MEM_COMMIT: u32 = 0x1000;
            const MEM_RESERVE: u32 = 0x2000;
            const PAGE_READWRITE: u32 = 0x04;
            unsafe extern "system" {
                fn VirtualAlloc(
                    address: *mut c_void,
                    size: usize,
                    allocation_type: u32,
                    protection: u32,
                ) -> *mut c_void;
            }
            // SAFETY: a null address asks Windows for a fresh private region.
            let base = unsafe {
                VirtualAlloc(
                    core::ptr::null_mut(),
                    DISPATCH_STACK_BYTES,
                    MEM_RESERVE | MEM_COMMIT,
                    PAGE_READWRITE,
                )
            };
            if base.is_null() {
                std::eprintln!("prisma: dispatch stack allocation failed");
                return Err(LifecycleError::DispatchFailed);
            }
            LIVE_DISPATCH_STACKS.fetch_add(1, Ordering::AcqRel);
            return Ok(Self {
                base,
                size: DISPATCH_STACK_BYTES,
                previous_bounds: None,
            });
        }
        #[cfg(not(all(windows, target_arch = "arm64ec")))]
        {
            LIVE_DISPATCH_STACKS.fetch_add(1, Ordering::AcqRel);
            Ok(Self {})
        }
    }

    #[cfg(all(windows, target_arch = "arm64ec"))]
    fn top(&self) -> usize {
        (self.base as usize + self.size) & !0xf
    }

    #[cfg(all(windows, target_arch = "arm64ec"))]
    fn bounds(&self) -> StackBounds {
        StackBounds {
            base: self.top(),
            limit: self.base as usize,
        }
    }
}

impl Drop for DispatchStack {
    fn drop(&mut self) {
        #[cfg(all(windows, target_arch = "arm64ec"))]
        {
            const MEM_RELEASE: u32 = 0x8000;
            unsafe extern "system" {
                fn VirtualFree(address: *mut c_void, size: usize, free_type: u32) -> i32;
            }
            if !self.base.is_null() {
                // SAFETY: this is the exact base returned by VirtualAlloc;
                // MEM_RELEASE requires a zero size and releases the full region.
                let released = unsafe { VirtualFree(self.base, 0, MEM_RELEASE) };
                debug_assert_ne!(released, 0, "failed to release dispatch stack");
                self.base = core::ptr::null_mut();
            }
        }
        LIVE_DISPATCH_STACKS.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NativeReturnFrame {
    continuation: u64,
    stack: u64,
    context: Arm64EcContext,
}

/// Native ARM64EC registers that an x64 call must preserve for its EC caller.
///
/// These values must be captured by the naked exit helper before Rust gets a
/// chance to use callee-saved registers for its own frame. `RtlCaptureContext`
/// called from the Rust helper can otherwise observe the helper's temporaries
/// instead of the values owned by the ARM64EC caller.
#[cfg(any(test, all(windows, target_arch = "arm64ec")))]
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct NativeNonvolatileRegisters {
    x19: u64,
    x20: u64,
    x21: u64,
    x22: u64,
    x25: u64,
    x26: u64,
    x27: u64,
    fp: u64,
}

#[cfg(any(test, all(windows, target_arch = "arm64ec")))]
#[repr(C, align(16))]
struct TransitionSaveArea {
    simd: [XmmRegister; 8],
    native: NativeNonvolatileRegisters,
    arguments: [u64; 4],
    target: u64,
    continuation: u64,
    stack: u64,
    stack_argument_area: u64,
    stack_argument_size: u64,
    reserved: u64,
}

#[cfg(any(test, all(windows, target_arch = "arm64ec")))]
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct EntryReturnSaveArea {
    simd_nonvolatile: [XmmRegister; 10],
    native: NativeNonvolatileRegisters,
    arguments: [u64; 4],
    return_address: u64,
    native_rax: u64,
    stack: u64,
    reserved: u64,
}

struct ProviderState {
    phase: ProviderPhase,
    generation: u64,
    threads: BTreeMap<ThreadKey, ThreadContext>,
    retired_threads: Vec<ThreadContext>,
    #[cfg(all(windows, target_arch = "arm64ec"))]
    executor: Option<Arc<PrismaExecutor>>,
    mappings: Vec<usize>,
    simulation_requests: usize,
    cache_notifications: usize,
    last_status: NtStatus,
}

impl Default for ProviderState {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderState {
    const fn new() -> Self {
        Self {
            phase: ProviderPhase::Cold,
            generation: 0,
            threads: BTreeMap::new(),
            retired_threads: Vec::new(),
            #[cfg(all(windows, target_arch = "arm64ec"))]
            executor: None,
            mappings: Vec::new(),
            simulation_requests: 0,
            cache_notifications: 0,
            last_status: STATUS_SUCCESS,
        }
    }

    const fn is_running(&self) -> bool {
        matches!(
            self.phase,
            ProviderPhase::Initialized | ProviderPhase::SimulationRequested
        )
    }

    fn release_owned_resources(&mut self) {
        self.reap_retired_threads();
        for context in self.threads.values().chain(&self.retired_threads) {
            context.runtime.cancel();
        }
        for (_, context) in std::mem::take(&mut self.threads) {
            self.retire_thread(context);
        }
        #[cfg(all(windows, target_arch = "arm64ec"))]
        {
            self.executor = None;
        }
        self.mappings = Vec::new();
        self.simulation_requests = 0;
        self.cache_notifications = 0;
    }

    fn retire_thread(&mut self, context: ThreadContext) {
        if context.active_dispatch_calls.load(Ordering::Acquire) == 0
            && context.runtime.active_dispatches() == 0
        {
            drop(context);
        } else {
            self.retired_threads.push(context);
        }
    }

    fn reap_retired_threads(&mut self) {
        self.retired_threads.retain(|context| {
            context.active_dispatch_calls.load(Ordering::Acquire) != 0
                || context.runtime.active_dispatches() != 0
        });
    }

    fn track_mapping(&mut self, address: usize) {
        // Keep this collection linear. Wine invokes these callbacks across
        // ARM64EC transition frames, and the Rust B-tree root-split path is
        // not safe at that boundary. Mapping counts are small and this avoids
        // publishing tree-node pointers while preserving exact ownership.
        if !self.mappings.contains(&address) {
            self.mappings.push(address);
        }
    }

    fn untrack_mapping(&mut self, address: usize) {
        if let Some(index) = self.mappings.iter().position(|entry| *entry == address) {
            self.mappings.swap_remove(index);
        }
    }

    fn snapshot(&self) -> ProviderSnapshot {
        ProviderSnapshot {
            phase: self.phase,
            generation: self.generation,
            active_threads: self.threads.len(),
            tracked_mappings: self.mappings.len(),
            simulation_requests: self.simulation_requests,
            cache_notifications: self.cache_notifications,
            last_status: self.last_status,
            active_dispatches: self
                .threads
                .values()
                .chain(&self.retired_threads)
                .map(|context| context.runtime.active_dispatches())
                .sum(),
            live_runtimes: live_runtime_count(),
            live_dispatch_stacks: LIVE_DISPATCH_STACKS.load(Ordering::Acquire),
        }
    }
}

struct SpinMutex<T> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}

impl<T> SpinMutex<T> {
    const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }

    fn lock(&self) -> SpinGuard<'_, T> {
        loop {
            if self
                .locked
                .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return SpinGuard { mutex: self };
            }
            while self.locked.load(Ordering::Relaxed) {
                std::hint::spin_loop();
            }
        }
    }
}

// SAFETY: `locked` grants exclusive access to `value`, and `T: Send` permits
// moving its ownership between threads while the guard is held.
unsafe impl<T: Send> Sync for SpinMutex<T> {}

struct SpinGuard<'a, T> {
    mutex: &'a SpinMutex<T>,
}

impl<T> Deref for SpinGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: the guard exists only after acquiring `locked` exclusively.
        unsafe { &*self.mutex.value.get() }
    }
}

impl<T> DerefMut for SpinGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: this guard is the sole owner while `locked` is true.
        unsafe { &mut *self.mutex.value.get() }
    }
}

impl<T> Drop for SpinGuard<'_, T> {
    fn drop(&mut self) {
        self.mutex.locked.store(false, Ordering::Release);
    }
}

static PROVIDER: SpinMutex<ProviderState> = SpinMutex::new(ProviderState::new());

fn lock_provider() -> SpinGuard<'static, ProviderState> {
    PROVIDER.lock()
}

/// Returns a consistent snapshot of the process provider state.
#[must_use]
pub fn provider_snapshot() -> ProviderSnapshot {
    lock_provider().snapshot()
}

/// Returns the context owned by the calling thread.
///
/// # Errors
///
/// Returns a typed lifecycle error when the process or current thread has not
/// completed the corresponding Wine initialization callback.
pub fn current_thread_context() -> Result<ThreadContextSnapshot, LifecycleError> {
    let state = lock_provider();
    if !state.is_running() {
        return Err(phase_error(state.phase));
    }
    state
        .threads
        .get(&current_thread_key())
        .map(|context| ThreadContextSnapshot {
            generation: context.generation,
            phase: context.phase,
            last_report: context.last_report,
            native_return_depth: context.native_returns.len(),
        })
        .ok_or(LifecycleError::ThreadNotInitialized)
}

/// Initializes the process generation without resetting an already-live provider.
///
/// # Errors
///
/// Returns [`LifecycleError::ProcessTerminating`] while a two-phase Wine
/// termination callback is in progress.
pub fn initialize_process() -> Result<InitOutcome, LifecycleError> {
    let mut state = lock_provider();
    let outcome = match state.phase {
        ProviderPhase::Cold => {
            state.release_owned_resources();
            state.generation = state.generation.wrapping_add(1).max(1);
            state.phase = ProviderPhase::Initialized;
            state.last_status = STATUS_SUCCESS;
            Ok(InitOutcome::Initialized {
                generation: state.generation,
            })
        }
        ProviderPhase::Initialized | ProviderPhase::SimulationRequested => {
            Ok(InitOutcome::AlreadyInitialized {
                generation: state.generation,
            })
        }
        ProviderPhase::TerminationPending => {
            state.last_status = STATUS_INVALID_DEVICE_STATE;
            Err(LifecycleError::ProcessTerminating)
        }
    };
    drop(state);
    outcome
}

/// Creates one context for the calling thread, or returns its existing generation.
///
/// # Errors
///
/// Returns a typed lifecycle error when the process is cold or terminating.
pub fn initialize_thread() -> Result<InitOutcome, LifecycleError> {
    let key = current_thread_key();
    let generation = {
        let mut state = lock_provider();
        if !state.is_running() {
            let error = phase_error(state.phase);
            state.last_status = error.nt_status();
            return Err(error);
        }
        if state.threads.contains_key(&key) {
            return Ok(InitOutcome::AlreadyInitialized {
                generation: state.generation,
            });
        }
        state.generation
    };

    // Wine invokes ThreadInit from inside its syscall callback. Constructing
    // the owned stack, Arc allocations, and JIT cache while holding PROVIDER
    // can re-enter provider notification exports and spin forever on the same
    // non-reentrant lock. Build the complete candidate before reacquiring it.
    let candidate = ThreadContext {
        generation,
        phase: ThreadPhase::Ready,
        runtime: Arc::new(ThreadRuntime::new()),
        active_dispatch_calls: Arc::new(AtomicUsize::new(0)),
        #[cfg(all(windows, target_arch = "arm64ec"))]
        executor: Arc::new(PrismaExecutor::default()),
        last_report: None,
        native_returns: Vec::new(),
        _dispatch_stacks: vec![DispatchStack::allocate()?],
    };

    let mut state = lock_provider();
    if !state.is_running() || state.generation != generation {
        let error = phase_error(state.phase);
        state.last_status = error.nt_status();
        drop(state);
        drop(candidate);
        return Err(error);
    }
    if state.threads.contains_key(&key) {
        drop(state);
        drop(candidate);
        return Ok(InitOutcome::AlreadyInitialized { generation });
    }
    #[cfg(all(windows, target_arch = "arm64ec"))]
    let candidate = {
        let mut candidate = candidate;
        if let Some(executor) = state.executor.as_ref() {
            candidate.executor = Arc::clone(executor);
        } else {
            state.executor = Some(Arc::clone(&candidate.executor));
        }
        candidate
    };
    state.threads.insert(key, candidate);
    state.last_status = STATUS_SUCCESS;
    drop(state);
    Ok(InitOutcome::Initialized { generation })
}

/// Translate and execute mapped Wine guest code for the calling thread.
///
/// This public bridge makes the memory and execution boundaries injectable for
/// tests and embedders. The exported [`BeginSimulation`] supplies the real
/// current-process reader and Prisma JIT executor.
///
/// # Errors
///
/// Returns a typed context, memory, translation, execution, or unsupported
/// boundary error without reporting synthetic guest progress.
pub fn dispatch_context<M: GuestMemory, E: BlockExecutor>(
    context: &mut Arm64EcContext,
    memory: &M,
    executor: &E,
    limits: DispatchLimits,
) -> Result<DispatchReport, DispatchError> {
    let key = current_thread_key();
    let mut state = lock_provider();
    if !state.is_running() {
        let error = phase_error(state.phase);
        state.last_status = error.nt_status();
        drop(state);
        return Err(DispatchError::ContextUnavailable);
    }
    let Some(thread) = state.threads.get_mut(&key) else {
        state.last_status = STATUS_INVALID_DEVICE_STATE;
        drop(state);
        return Err(DispatchError::ContextUnavailable);
    };
    thread.phase = ThreadPhase::Dispatching;
    let runtime = Arc::clone(&thread.runtime);
    let dispatch_call = DispatchContextLease::new(Arc::clone(&thread.active_dispatch_calls));
    drop(state);

    let result = runtime.dispatch(context, memory, executor, limits);
    let mut state = lock_provider();
    if state.is_running() {
        if let Some(thread) = state.threads.get_mut(&key) {
            if let Ok(report) = &result {
                thread.last_report = Some(*report);
                thread.phase = ThreadPhase::Stopped;
            } else {
                thread.phase = ThreadPhase::Failed;
            }
            state.last_status = if result.is_ok() {
                STATUS_SUCCESS
            } else {
                STATUS_NOT_SUPPORTED
            };
        }
        state.phase = ProviderPhase::SimulationRequested;
        state.simulation_requests = state.simulation_requests.saturating_add(1);
    }
    drop(dispatch_call);
    state.reap_retired_threads();
    drop(state);
    result
}

#[cfg(any(test, all(windows, target_arch = "arm64ec")))]
// Ownership is intentionally transferred into the per-thread LIFO; borrowing
// would require a second large copy and obscure which layer releases it.
#[allow(clippy::large_types_passed_by_value)]
fn push_native_return(frame: NativeReturnFrame) -> Result<(), LifecycleError> {
    let mut state = lock_provider();
    if !state.is_running() {
        return Err(phase_error(state.phase));
    }
    let thread = state
        .threads
        .get_mut(&current_thread_key())
        .ok_or(LifecycleError::ThreadNotInitialized)?;
    thread.native_returns.push(frame);
    drop(state);
    Ok(())
}

#[cfg(any(test, all(windows, target_arch = "arm64ec")))]
fn pop_native_return() -> Result<NativeReturnFrame, LifecycleError> {
    let mut state = lock_provider();
    if !state.is_running() {
        return Err(phase_error(state.phase));
    }
    let frame = state
        .threads
        .get_mut(&current_thread_key())
        .ok_or(LifecycleError::ThreadNotInitialized)?
        .native_returns
        .pop()
        .ok_or(LifecycleError::DispatchFailed)?;
    drop(state);
    Ok(frame)
}

fn is_native_return_continuation(rip: u64) -> bool {
    let state = lock_provider();
    state
        .threads
        .get(&current_thread_key())
        .and_then(|thread| thread.native_returns.last())
        .is_some_and(|frame| frame.continuation == rip)
}

#[cfg(all(windows, target_arch = "arm64ec"))]
fn run_simulation(context: &mut Arm64EcContext) -> ! {
    phase_marker(b"prisma-phase: dispatch-loop-enter\n");
    let mut first_dispatch = true;
    loop {
        let executor = {
            let state = lock_provider();
            let Some(thread) = state.threads.get(&current_thread_key()) else {
                drop(state);
                record_failed_dispatch();
                // SAFETY: the current thread has no owned execution context.
                unsafe { dispatch::terminate_current_process(STATUS_NOT_SUPPORTED) }
            };
            Arc::clone(&thread.executor)
        };
        let result = dispatch_context(
            context,
            &dispatch::ProcessMemory,
            executor.as_ref(),
            DispatchLimits::default(),
        );
        if first_dispatch {
            phase_marker(b"prisma-phase: first-dispatch-returned\n");
            first_dispatch = false;
        }
        // `resume_wine_context` abandons this Rust stack. Release the temporary
        // strong reference first; ThreadContext remains the cache owner.
        drop(executor);
        match result {
            Ok(DispatchReport {
                stop: DispatchStop::BlockLimit,
                ..
            }) => {}
            Ok(DispatchReport {
                stop: DispatchStop::NativeTransitionRequired,
                ..
            }) => {
                // SAFETY: `context` is Wine's current-thread CHPE context,
                // synchronized immediately before this non-returning boundary.
                restore_dispatch_stack_bounds_or_terminate();
                unsafe { dispatch::resume_wine_context(context) }
            }
            Ok(DispatchReport {
                stop: DispatchStop::NativeReturnRequired,
                ..
            }) => {
                let native_rax = context.x8_rax;
                let Ok(frame) = pop_native_return() else {
                    record_failed_dispatch();
                    unsafe { dispatch::terminate_current_process(STATUS_NOT_SUPPORTED) }
                };
                restore_native_return_context(context, frame, native_rax);
                restore_dispatch_stack_bounds_or_terminate();
                unsafe { dispatch::resume_wine_context(context) }
            }
            Ok(_) => {
                phase_marker(b"prisma-error: unexpected-dispatch-stop\n");
                record_failed_dispatch();
                // SAFETY: a cancelled context cannot safely cross back through
                // KiUserEmulationDispatcher.
                unsafe { dispatch::terminate_current_process(STATUS_NOT_SUPPORTED) }
            }
            Err(error) => {
                phase_marker(error.diagnostic_marker());
                record_failed_dispatch();
                // SAFETY: a failed context cannot safely cross back through
                // KiUserEmulationDispatcher.
                unsafe { dispatch::terminate_current_process(STATUS_NOT_SUPPORTED) }
            }
        }
    }
}

#[cfg(all(windows, target_arch = "arm64ec"))]
fn current_dispatch_stack_top() -> Result<usize, LifecycleError> {
    let key = current_thread_key();
    let (depth, needs_stack) = {
        let state = lock_provider();
        if !state.is_running() {
            return Err(phase_error(state.phase));
        }
        let thread = state
            .threads
            .get(&key)
            .ok_or(LifecycleError::ThreadNotInitialized)?;
        let depth = thread
            ._dispatch_stacks
            .iter()
            .take_while(|stack| stack.previous_bounds.is_some())
            .count();
        if depth > MAX_NESTED_NATIVE_CALLBACKS {
            std::eprintln!("prisma: native callback nesting limit exceeded");
            return Err(LifecycleError::DispatchFailed);
        }
        (depth, thread._dispatch_stacks.len() == depth)
    };

    // VirtualAlloc is observable by Wine, which can synchronously call the
    // provider's memory-notification exports. Never hold PROVIDER while asking
    // the OS for a new stack or that reentrant callback deadlocks on the mutex.
    let stack = if needs_stack {
        Some(DispatchStack::allocate()?)
    } else {
        None
    };
    let mut state = lock_provider();
    if !state.is_running() {
        return Err(phase_error(state.phase));
    }
    let thread = state
        .threads
        .get_mut(&key)
        .ok_or(LifecycleError::ThreadNotInitialized)?;
    if thread._dispatch_stacks.len() == depth {
        thread
            ._dispatch_stacks
            .push(stack.ok_or(LifecycleError::DispatchFailed)?);
    }
    let stack = thread
        ._dispatch_stacks
        .get_mut(depth)
        .ok_or(LifecycleError::DispatchFailed)?;
    if stack.previous_bounds.is_some() {
        return Err(LifecycleError::DispatchFailed);
    }
    let previous = current_teb_stack_bounds();
    let bounds = stack.bounds();
    stack.previous_bounds = Some(previous);
    // SAFETY: the next operation switches SP to this exact owned allocation.
    unsafe { set_teb_stack_bounds(bounds) };
    Ok(stack.top())
}

#[cfg(all(windows, target_arch = "arm64ec"))]
fn restore_dispatch_stack_bounds() -> Result<(), LifecycleError> {
    let key = current_thread_key();
    let previous = {
        let mut state = lock_provider();
        let thread = state
            .threads
            .get_mut(&key)
            .ok_or(LifecycleError::ThreadNotInitialized)?;
        thread
            ._dispatch_stacks
            .iter_mut()
            .rev()
            .find_map(|stack| stack.previous_bounds.take())
            .ok_or(LifecycleError::DispatchFailed)?
    };
    // SAFETY: the caller restores these bounds immediately before transferring
    // control back to the native stack from which the transition originated.
    unsafe { set_teb_stack_bounds(previous) };
    Ok(())
}

#[cfg(all(windows, target_arch = "arm64ec"))]
fn restore_dispatch_stack_bounds_or_terminate() {
    if let Err(error) = restore_dispatch_stack_bounds() {
        std::eprintln!("prisma: dispatch stack restoration failed: {error}");
        record_failed_dispatch();
        // SAFETY: returning with mismatched TEB stack metadata is unsafe.
        unsafe { dispatch::terminate_current_process(STATUS_NOT_SUPPORTED) }
    }
}

#[cfg(all(windows, target_arch = "arm64ec"))]
unsafe extern "system" fn dispatch_stack_entry(context: *mut Arm64EcContext) -> ! {
    phase_marker(b"prisma-phase: dispatch-stack-enter\n");
    // SAFETY: the stack-switch thunk receives Wine's live, thread-owned context.
    let Some(context) = (unsafe { context.as_mut() }) else {
        record_failed_dispatch();
        // SAFETY: no valid context exists to resume.
        unsafe { dispatch::terminate_current_process(STATUS_NOT_SUPPORTED) }
    };
    run_simulation(context)
}

#[cfg(all(windows, target_arch = "arm64ec"))]
fn run_simulation_on_dispatch_stack(context: &mut Arm64EcContext) -> ! {
    let stack_top = match current_dispatch_stack_top() {
        Ok(stack_top) => stack_top,
        Err(error) => {
            std::eprintln!("prisma: dispatch stack activation failed: {error}");
            record_failed_dispatch();
            // SAFETY: no owned alternate stack exists for safe translation.
            unsafe { dispatch::terminate_current_process(STATUS_NOT_SUPPORTED) }
        }
    };
    phase_marker(b"prisma-phase: dispatch-stack-ready\n");
    let entry = dispatch_stack_entry as *const () as usize;
    // SAFETY: stack zero owns the initial simulation. Each nested native-to-x64
    // callback increments `native_returns` and therefore receives a distinct
    // owned stack. Resetting that depth's stack cannot overwrite the native
    // frames or callback arguments that remain live on the previous depth.
    // This function never returns, so abandoning its old-stack prologue is safe.
    unsafe {
        core::arch::asm!(
            "mov sp, {stack}",
            "br {entry}",
            in("x0") context as *mut Arm64EcContext,
            stack = in(reg) stack_top,
            entry = in(reg) entry,
            options(noreturn),
        )
    }
}

fn record_failed_dispatch() {
    let mut state = lock_provider();
    let key = current_thread_key();
    if let Some(thread) = state.threads.get_mut(&key) {
        thread.phase = ThreadPhase::Failed;
    }
    if state.is_running() {
        state.phase = ProviderPhase::SimulationRequested;
        state.simulation_requests = state.simulation_requests.saturating_add(1);
    }
    state.last_status = STATUS_NOT_SUPPORTED;
}

fn phase_error(phase: ProviderPhase) -> LifecycleError {
    if phase == ProviderPhase::TerminationPending {
        LifecycleError::ProcessTerminating
    } else {
        LifecycleError::ProcessNotInitialized
    }
}

fn process_term(post_call: bool, status: NtStatus) {
    let mut state = lock_provider();
    if !post_call {
        if state.phase != ProviderPhase::Cold {
            state.release_owned_resources();
            state.phase = ProviderPhase::TerminationPending;
            state.last_status = STATUS_SUCCESS;
        }
        return;
    }

    if successful(status) {
        state.release_owned_resources();
        state.phase = ProviderPhase::Cold;
        state.last_status = STATUS_SUCCESS;
    } else if state.phase == ProviderPhase::TerminationPending {
        state.phase = ProviderPhase::Initialized;
        state.last_status = status;
    }
}

fn thread_term(handle: Handle) {
    let mut state = lock_provider();
    if !state.is_running() {
        return;
    }
    if let Some(key) = thread_key_from_handle(handle) {
        if let Some(context) = state.threads.remove(&key) {
            context.runtime.cancel();
            state.retire_thread(context);
        }
    }
}

const fn successful(status: NtStatus) -> bool {
    status >= 0
}

fn with_running_state(action: impl FnOnce(&mut ProviderState)) {
    let mut state = lock_provider();
    if state.is_running() {
        action(&mut state);
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn BTCpu64FlushInstructionCache(_address: *const c_void, _size: usize) {
    let runtimes = {
        let mut state = lock_provider();
        if !state.is_running() {
            return;
        }
        state.cache_notifications = state.cache_notifications.saturating_add(1);
        state
            .threads
            .values()
            .map(|context| Arc::clone(&context.runtime))
            .collect::<Vec<_>>()
    };
    for runtime in runtimes {
        runtime.clear_cache();
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn BTCpu64IsProcessorFeaturePresent(feature: u32) -> WinBoolean {
    u8::from(matches!(
        feature,
        2 | 3 | 6 | 8 | 10 | 12 | 13 | 14 | 23 | 32 | 36 | 37 | 38
    ))
}

#[unsafe(no_mangle)]
pub extern "system" fn BTCpu64NotifyMemoryDirty(_address: *mut c_void, _size: usize) {
    with_running_state(|state| {
        state.cache_notifications = state.cache_notifications.saturating_add(1);
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn BTCpu64NotifyReadFile(
    _handle: Handle,
    _buffer: *mut c_void,
    _size: usize,
    _post_call: WinBool,
    _status: NtStatus,
) {
}

#[unsafe(no_mangle)]
pub extern "system" fn BeginSimulation() {
    #[cfg(all(windows, target_arch = "arm64ec"))]
    {
        phase_marker(b"prisma-phase: begin-simulation-enter\n");
        // SAFETY: Wine invokes this callback after installing the current
        // thread's CHPE v2 CPU area and owns the context for the call duration.
        let context = unsafe { dispatch::current_wine_context() };
        let context = match context {
            Ok(context) => context,
            Err(_) => {
                record_failed_dispatch();
                // SAFETY: returning would execute Wine's deliberate `brk #1`.
                unsafe { dispatch::terminate_current_process(STATUS_NOT_SUPPORTED) }
            }
        };
        phase_marker(b"prisma-phase: begin-simulation-context-ready\n");
        run_simulation_on_dispatch_stack(context)
    }
    #[cfg(not(all(windows, target_arch = "arm64ec")))]
    record_failed_dispatch();
}

#[unsafe(no_mangle)]
pub extern "system" fn FlushInstructionCacheHeavy(address: *const c_void, size: usize) {
    BTCpu64FlushInstructionCache(address, size);
}

#[unsafe(no_mangle)]
pub extern "system" fn NotifyMapViewOfSection(
    _section: *mut c_void,
    address: *mut c_void,
    _unknown: *mut c_void,
    size: usize,
    _allocation_type: u32,
    _protection: u32,
) -> NtStatus {
    let mut state = lock_provider();
    if !state.is_running() {
        state.last_status = STATUS_INVALID_DEVICE_STATE;
        return STATUS_INVALID_DEVICE_STATE;
    }
    if !address.is_null() && size != 0 {
        state.track_mapping(address as usize);
    }
    STATUS_SUCCESS
}

#[unsafe(no_mangle)]
pub extern "system" fn NotifyMemoryAlloc(
    address: *mut c_void,
    size: usize,
    _allocation_type: u32,
    _protection: u32,
    post_call: WinBool,
    status: NtStatus,
) {
    if post_call != 0 && successful(status) && !address.is_null() && size != 0 {
        with_running_state(|state| {
            state.track_mapping(address as usize);
        });
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn NotifyMemoryFree(
    address: *mut c_void,
    _size: usize,
    _free_type: u32,
    post_call: WinBool,
    status: NtStatus,
) {
    if post_call != 0 && successful(status) && !address.is_null() {
        with_running_state(|state| {
            state.untrack_mapping(address as usize);
        });
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn NotifyMemoryProtect(
    _address: *mut c_void,
    _size: usize,
    _new_protection: u32,
    _post_call: WinBool,
    _status: NtStatus,
) {
}

#[unsafe(no_mangle)]
pub extern "system" fn NotifyUnmapViewOfSection(
    address: *mut c_void,
    post_call: WinBool,
    status: NtStatus,
) {
    if post_call != 0 && successful(status) && !address.is_null() {
        with_running_state(|state| {
            state.untrack_mapping(address as usize);
        });
    }
}

/// Wine 11.14 `xtajit64.spec`: `NTSTATUS ProcessInit(void)`.
#[unsafe(no_mangle)]
pub extern "system" fn ProcessInit() -> NtStatus {
    #[cfg(all(windows, target_arch = "arm64ec"))]
    phase_marker(b"prisma-phase: process-init-enter\n");
    let status = initialize_process().map_or_else(LifecycleError::nt_status, |_| STATUS_SUCCESS);
    #[cfg(all(windows, target_arch = "arm64ec"))]
    phase_marker(b"prisma-phase: process-init-exit\n");
    status
}

/// Wine calls this before and after `NtTerminateProcess` for the current process.
#[unsafe(no_mangle)]
pub extern "system" fn ProcessTerm(_process: Handle, post_call: WinBool, status: NtStatus) {
    process_term(post_call != 0, status);
}

#[cfg(all(windows, target_arch = "arm64ec"))]
unsafe extern "system" fn reset_to_consistent_state_active(
    _exception_record: *mut ExceptionRecord,
    amd64_context: *mut Amd64Context,
    _arm64_context: *mut Arm64NtContext,
) {
    // SAFETY: Wine passes the AMD64 view embedded at offset zero in its full
    // ARM64EC context. The active JIT guard is thread-local and only exposes a
    // live frame while translated code is executing on this same thread.
    // Native ARM64EC exceptions legitimately arrive without an active Prisma
    // frame; in that case there is no translated state to synchronize.
    let _ =
        unsafe { dispatch::reset_active_exception_context(amd64_context.cast::<Arm64EcContext>()) };
}

#[cfg(all(windows, target_arch = "arm64ec"))]
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ResetToConsistentState(
    _exception_record: *mut ExceptionRecord,
    _amd64_context: *mut Amd64Context,
    _arm64_context: *mut Arm64NtContext,
) {
    core::arch::naked_asm!(
        "ldr x16, [x18, #0x1788]",
        "cbz x16, 2f",
        "ldrb w16, [x16]",
        "cbnz w16, 1f",
        "2: ret",
        "1: b \"#{active}\"",
        active = sym reset_to_consistent_state_active,
    )
}

#[cfg(not(all(windows, target_arch = "arm64ec")))]
#[unsafe(no_mangle)]
pub extern "system" fn ResetToConsistentState(
    _exception_record: *mut ExceptionRecord,
    amd64_context: *mut Amd64Context,
    _arm64_context: *mut Arm64NtContext,
) {
    // SAFETY: test callers provide either a valid context or null.
    let _ =
        unsafe { dispatch::reset_active_exception_context(amd64_context.cast::<Arm64EcContext>()) };
}

/// Wine 11.14 `xtajit64.spec`: `NTSTATUS ThreadInit(void)`.
#[unsafe(no_mangle)]
pub extern "system" fn ThreadInit() -> NtStatus {
    #[cfg(all(windows, target_arch = "arm64ec"))]
    phase_marker(b"prisma-phase: thread-init-enter\n");
    let status = initialize_thread().map_or_else(LifecycleError::nt_status, |_| STATUS_SUCCESS);
    #[cfg(all(windows, target_arch = "arm64ec"))]
    if successful(status) {
        THREAD_INIT_COUNT.fetch_add(1, Ordering::AcqRel);
    }
    #[cfg(all(windows, target_arch = "arm64ec"))]
    phase_marker(b"prisma-phase: thread-init-exit\n");
    status
}

/// Releases the context for the target or current thread. Repeated calls are harmless.
#[unsafe(no_mangle)]
pub extern "system" fn ThreadTerm(thread: Handle, _exit_code: i32) {
    thread_term(thread);
}

#[unsafe(no_mangle)]
/// Updates Wine's processor description to match the emulated AMD64 CPU.
///
/// # Safety
///
/// `information` must be null or point to a writable `SystemCpuInformation`
/// supplied by Wine for the duration of this call.
pub unsafe extern "system" fn UpdateProcessorInformation(information: *mut SystemCpuInformation) {
    const PROCESSOR_ARCHITECTURE_AMD64: u16 = 9;

    let Some(information) = (unsafe { information.as_mut() }) else {
        return;
    };
    information.processor_architecture = PROCESSOR_ARCHITECTURE_AMD64;
    information.processor_level = 21;
    information.processor_revision = 1;
}

#[cfg(all(windows, target_arch = "arm64ec"))]
unsafe fn transition_context_or_terminate() -> &'static mut Arm64EcContext {
    // SAFETY: the transition thunk is running on the thread that owns Wine's
    // CHPE area and its ContextAmd64 allocation.
    match unsafe { dispatch::current_wine_transition_context() } {
        Ok(context) => context,
        Err(_) => {
            record_failed_dispatch();
            // SAFETY: no context exists to resume safely.
            unsafe { dispatch::terminate_current_process(STATUS_NOT_SUPPORTED) }
        }
    }
}

#[cfg(all(windows, target_arch = "arm64ec"))]
unsafe fn start_x64_transition(
    context: &mut Arm64EcContext,
    target: u64,
    stack: u64,
    arguments: [u64; 4],
) -> ! {
    context.x0_rcx = arguments[0];
    context.x1_rdx = arguments[1];
    context.x2_r8 = arguments[2];
    context.x3_r9 = arguments[3];
    context.sp_rsp = stack;
    context.pc_rip = target;
    // SAFETY: this thread owns the CHPE area and is transferring its context
    // from native execution to Prisma's x64 simulation loop.
    if unsafe { dispatch::set_simulation_active(true) }.is_err() {
        record_failed_dispatch();
        // SAFETY: the CHPE ownership transfer could not be completed.
        unsafe { dispatch::terminate_current_process(STATUS_NOT_SUPPORTED) }
    }
    run_simulation_on_dispatch_stack(context)
}

#[cfg(all(windows, target_arch = "arm64ec"))]
unsafe extern "system" fn exit_to_x64_transition(save_area: *const TransitionSaveArea) -> ! {
    // SAFETY: the naked thunk preserves the incoming register and stack values
    // and tail-branches here on the owning Wine thread.
    let context = unsafe { transition_context_or_terminate() };
    // SAFETY: RtlCaptureContext fills Wine's exact hybrid context layout and
    // supplies the remaining live state. The naked helper's explicit save is
    // applied below because this Rust frame may itself use nonvolatile GPRs.
    unsafe { dispatch::capture_native_context(context) };
    // SAFETY: the naked thunk owns one aligned save area for this non-returning
    // helper invocation, and it remains live until the values are copied.
    let saved = unsafe { &*save_area };
    let [rcx, rdx, r8, r9] = saved.arguments;
    let target = saved.target;
    let continuation = saved.continuation;
    let stack = saved.stack;
    restore_native_nonvolatile_registers(context, saved.native);
    for index in 0..8 {
        // SAFETY: the naked thunk passes a 16-byte-aligned eight-register save
        // area that remains live until this non-returning helper transfers it.
        let value = saved.simd[index];
        let _ = context.set_xmm(index, value);
    }
    let Some(return_slot) = stack.checked_sub(8) else {
        record_failed_dispatch();
        // SAFETY: an invalid native stack cannot be resumed.
        unsafe { dispatch::terminate_current_process(STATUS_NOT_SUPPORTED) }
    };
    let frame = NativeReturnFrame {
        continuation,
        stack,
        context: *context,
    };
    let arguments = [rcx, rdx, r8, r9];
    // SAFETY: `return_slot` is the eight-byte slot immediately below the live
    // Wine stack pointer captured by this non-returning transition thunk.
    let write_result = unsafe { dispatch::write_current_process_u64(return_slot, continuation) };
    if write_result.is_err() || push_native_return(frame).is_err() {
        record_failed_dispatch();
        // SAFETY: the synthetic cross-ISA return frame was not installed in
        // both the guest stack and the provider-owned native return stack.
        unsafe { dispatch::terminate_current_process(STATUS_NOT_SUPPORTED) }
    }
    // SAFETY: context ownership and the synthetic x64 return address are live;
    // RetToEntryThunk restores `frame` instead of returning through this Rust
    // stack, which Wine abandons on each NtContinue context transfer.
    unsafe { start_x64_transition(context, target, return_slot, arguments) }
}

#[cfg(all(windows, target_arch = "arm64ec"))]
unsafe extern "system" fn dispatch_jump_transition(save_area: *const TransitionSaveArea) -> ! {
    // SAFETY: the naked thunk passes the live native register aliases exactly.
    let context = unsafe { transition_context_or_terminate() };
    // SAFETY: see `exit_to_x64_transition`; no synthetic return is needed for
    // this tail jump.
    unsafe { dispatch::capture_native_context(context) };
    // SAFETY: see `exit_to_x64_transition`.
    let saved = unsafe { &*save_area };
    let [rcx, rdx, r8, r9] = saved.arguments;
    let target = saved.target;
    let stack = saved.stack;
    restore_native_nonvolatile_registers(context, saved.native);
    for index in 0..8 {
        // SAFETY: see `exit_to_x64_transition`; the save area is owned by this
        // abandoned native transition frame until context transfer completes.
        let value = saved.simd[index];
        let _ = context.set_xmm(index, value);
    }
    unsafe { start_x64_transition(context, target, stack, [rcx, rdx, r8, r9]) }
}

#[cfg(all(windows, target_arch = "arm64ec"))]
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ExitToX64() -> ! {
    core::arch::naked_asm!(
        "sub sp, sp, #0x110",
        "stp q0, q1, [sp, #0x00]",
        "stp q2, q3, [sp, #0x20]",
        "stp q4, q5, [sp, #0x40]",
        "stp q6, q7, [sp, #0x60]",
        "stp x19, x20, [sp, #0x80]",
        "stp x21, x22, [sp, #0x90]",
        "stp x25, x26, [sp, #0xa0]",
        "stp x27, x29, [sp, #0xb0]",
        "stp x0, x1, [sp, #0xc0]",
        "stp x2, x3, [sp, #0xd0]",
        "stp x9, x30, [sp, #0xe0]",
        "add x10, sp, #0x110",
        "str x10, [sp, #0xf0]",
        "stp x4, x5, [sp, #0xf8]",
        "str xzr, [sp, #0x108]",
        "mov x0, sp",
        "b \"#{transition}\"",
        transition = sym exit_to_x64_transition,
    )
}

#[cfg(not(all(windows, target_arch = "arm64ec")))]
#[unsafe(no_mangle)]
pub extern "system" fn ExitToX64() {
    record_failed_dispatch();
}

#[cfg(all(windows, target_arch = "arm64ec"))]
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DispatchJump() -> ! {
    core::arch::naked_asm!(
        "sub sp, sp, #0x110",
        "stp q0, q1, [sp, #0x00]",
        "stp q2, q3, [sp, #0x20]",
        "stp q4, q5, [sp, #0x40]",
        "stp q6, q7, [sp, #0x60]",
        "stp x19, x20, [sp, #0x80]",
        "stp x21, x22, [sp, #0x90]",
        "stp x25, x26, [sp, #0xa0]",
        "stp x27, x29, [sp, #0xb0]",
        "stp x0, x1, [sp, #0xc0]",
        "stp x2, x3, [sp, #0xd0]",
        "str x9, [sp, #0xe0]",
        "str xzr, [sp, #0xe8]",
        "add x10, sp, #0x110",
        "str x10, [sp, #0xf0]",
        "stp x4, x5, [sp, #0xf8]",
        "str xzr, [sp, #0x108]",
        "mov x0, sp",
        "b \"#{transition}\"",
        transition = sym dispatch_jump_transition,
    )
}

#[cfg(not(all(windows, target_arch = "arm64ec")))]
#[unsafe(no_mangle)]
pub extern "system" fn DispatchJump() {
    record_failed_dispatch();
}

#[cfg(all(windows, target_arch = "arm64ec"))]
unsafe extern "system" fn ret_to_entry_transition(save_area: *const EntryReturnSaveArea) -> ! {
    // SAFETY: the naked return thunk owns this aligned area until this
    // non-returning helper resumes the x64 context.
    let saved = unsafe { &*save_area };
    let return_address = saved.return_address;
    let native_rax = saved.native_rax;
    // SAFETY: the dispatcher thunk runs on the Wine thread that owns this area.
    let context = unsafe { transition_context_or_terminate() };
    if return_address == 0 {
        let Ok(frame) = pop_native_return() else {
            record_failed_dispatch();
            // SAFETY: there is no matching native continuation to restore.
            unsafe { dispatch::terminate_current_process(STATUS_NOT_SUPPORTED) }
        };
        restore_native_return_context(context, frame, native_rax);
        // SAFETY: the x64 result/nonvolatile state remains synchronized.
        unsafe { dispatch::resume_wine_context(context) }
    }

    // SAFETY: the naked transition captured the live stack before entering
    // Rust. RtlCaptureContext synchronizes the remaining native state.
    unsafe { dispatch::capture_native_context(context) };
    restore_entry_return_context(context, saved);
    let stack = saved.stack;
    let arguments = saved.arguments;
    // SAFETY: ARM64EC's dispatch-ret ABI supplies the popped x64 return in LR.
    unsafe { start_x64_transition(context, return_address, stack, arguments) }
}

#[cfg(all(windows, target_arch = "arm64ec"))]
fn ret_to_entry_thunk_address() -> u64 {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleA(name: *const u8) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const u8) -> *const c_void;
    }

    // SAFETY: both names are static NUL-terminated strings and xtajit64 is the
    // currently executing provider module.
    let module = unsafe { GetModuleHandleA(c"xtajit64.dll".as_ptr().cast()) };
    let address = if module.is_null() {
        std::ptr::null()
    } else {
        // SAFETY: the module is loaded and the export name is static.
        unsafe { GetProcAddress(module, c"RetToEntryThunk".as_ptr().cast()) }
    };
    match u64::try_from(address as usize) {
        Ok(address) if address != 0 => address,
        _ => {
            record_failed_dispatch();
            // SAFETY: a synthetic x64 return requires the native export.
            unsafe { dispatch::terminate_current_process(STATUS_NOT_SUPPORTED) }
        }
    }
}

#[cfg(any(test, all(windows, target_arch = "arm64ec")))]
// The frame is popped by value from the owned LIFO and consumed exactly once.
#[allow(clippy::large_types_passed_by_value)]
fn restore_native_return_context(
    context: &mut Arm64EcContext,
    frame: NativeReturnFrame,
    native_rax: u64,
) {
    *context = frame.context;
    context.pc_rip = frame.continuation;
    context.sp_rsp = frame.stack;
    context.tail.arm64_lr = frame.continuation;
    context.x8_rax = native_rax;
}

#[cfg(any(test, all(windows, target_arch = "arm64ec")))]
fn restore_native_nonvolatile_registers(
    context: &mut Arm64EcContext,
    saved: NativeNonvolatileRegisters,
) {
    context.x19_r12 = saved.x19;
    context.x20_r13 = saved.x20;
    context.x21_r14 = saved.x21;
    context.x22_r15 = saved.x22;
    context.x25_rsi = saved.x25;
    context.x26_rdi = saved.x26;
    context.x27_rbx = saved.x27;
    context.fp_rbp = saved.fp;
}

#[cfg(any(test, all(windows, target_arch = "arm64ec")))]
fn restore_entry_return_context(context: &mut Arm64EcContext, saved: &EntryReturnSaveArea) {
    restore_native_nonvolatile_registers(context, saved.native);
    for (index, value) in saved.simd_nonvolatile.iter().copied().enumerate() {
        let restored = context.set_xmm(index + 6, value);
        debug_assert!(restored, "entry return XMM index must be representable");
    }
    context.x0_rcx = saved.arguments[0];
    context.x1_rdx = saved.arguments[1];
    context.x2_r8 = saved.arguments[2];
    context.x3_r9 = saved.arguments[3];
    context.x8_rax = saved.native_rax;
    context.sp_rsp = saved.stack;
}

#[cfg(all(windows, target_arch = "arm64ec"))]
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RetToEntryThunk() -> ! {
    core::arch::naked_asm!(
        "sub sp, sp, #0x120",
        "stp q6, q7, [sp, #0x00]",
        "stp q8, q9, [sp, #0x20]",
        "stp q10, q11, [sp, #0x40]",
        "stp q12, q13, [sp, #0x60]",
        "stp q14, q15, [sp, #0x80]",
        "stp x19, x20, [sp, #0xa0]",
        "stp x21, x22, [sp, #0xb0]",
        "stp x25, x26, [sp, #0xc0]",
        "stp x27, x29, [sp, #0xd0]",
        "stp x0, x1, [sp, #0xe0]",
        "stp x2, x3, [sp, #0xf0]",
        "stp x30, x8, [sp, #0x100]",
        "add x9, sp, #0x120",
        "str x9, [sp, #0x110]",
        "str xzr, [sp, #0x118]",
        "mov x0, sp",
        "b \"#{transition}\"",
        transition = sym ret_to_entry_transition,
    )
}

#[cfg(not(all(windows, target_arch = "arm64ec")))]
#[unsafe(no_mangle)]
pub extern "system" fn RetToEntryThunk() {
    record_failed_dispatch();
}

#[cfg(windows)]
fn current_thread_key() -> ThreadKey {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        #[link_name = "GetCurrentThreadId"]
        fn get_current_thread_id() -> u32;
    }

    // SAFETY: GetCurrentThreadId has no preconditions and returns a value.
    ThreadKey(u64::from(unsafe { get_current_thread_id() }))
}

#[cfg(not(windows))]
fn current_thread_key() -> ThreadKey {
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_THREAD_KEY: AtomicU64 = AtomicU64::new(1);
    thread_local! {
        static THREAD_KEY: ThreadKey = ThreadKey(NEXT_THREAD_KEY.fetch_add(1, Ordering::Relaxed));
    }
    THREAD_KEY.with(|key| *key)
}

#[cfg(windows)]
fn thread_key_from_handle(handle: Handle) -> Option<ThreadKey> {
    const CURRENT_THREAD_PSEUDO_HANDLE: isize = -2;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        #[link_name = "GetThreadId"]
        fn get_thread_id(thread: Handle) -> u32;
    }

    if handle.is_null() || handle as isize == CURRENT_THREAD_PSEUDO_HANDLE {
        return Some(current_thread_key());
    }

    // SAFETY: Wine supplies a thread handle. A zero result is treated as an
    // invalid/unresolvable handle and does not release another thread's state.
    let id = unsafe { get_thread_id(handle) };
    (id != 0).then(|| ThreadKey(u64::from(id)))
}

#[cfg(not(windows))]
// Match the Windows handle-resolution contract so shared lifecycle code can
// exercise both success and failure-shaped paths on host test platforms.
#[allow(clippy::unnecessary_wraps)]
fn thread_key_from_handle(_handle: Handle) -> Option<ThreadKey> {
    Some(current_thread_key())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn reset() {
        ProcessTerm(std::ptr::null_mut(), 0, STATUS_SUCCESS);
        ProcessTerm(std::ptr::null_mut(), 1, STATUS_SUCCESS);
    }

    #[test]
    fn exported_callbacks_match_wine_11_14_xtajit64_signatures() {
        let _guard = TEST_LOCK.lock().unwrap();
        let _: extern "system" fn(*const c_void, usize) = BTCpu64FlushInstructionCache;
        let _: extern "system" fn(u32) -> WinBoolean = BTCpu64IsProcessorFeaturePresent;
        let _: extern "system" fn(*mut c_void, usize) = BTCpu64NotifyMemoryDirty;
        let _: extern "system" fn(Handle, *mut c_void, usize, WinBool, NtStatus) =
            BTCpu64NotifyReadFile;
        let _: extern "system" fn() = BeginSimulation;
        let _: extern "system" fn(*const c_void, usize) = FlushInstructionCacheHeavy;
        let _: extern "system" fn(
            *mut c_void,
            *mut c_void,
            *mut c_void,
            usize,
            u32,
            u32,
        ) -> NtStatus = NotifyMapViewOfSection;
        let _: extern "system" fn(*mut c_void, usize, u32, u32, WinBool, NtStatus) =
            NotifyMemoryAlloc;
        let _: extern "system" fn(*mut c_void, usize, u32, WinBool, NtStatus) = NotifyMemoryFree;
        let _: extern "system" fn(*mut c_void, usize, u32, WinBool, NtStatus) = NotifyMemoryProtect;
        let _: extern "system" fn(*mut c_void, WinBool, NtStatus) = NotifyUnmapViewOfSection;
        let _: extern "system" fn() -> NtStatus = ProcessInit;
        let _: extern "system" fn() -> NtStatus = ThreadInit;
        let _: extern "system" fn(Handle, WinBool, NtStatus) = ProcessTerm;
        let _: extern "system" fn(Handle, i32) = ThreadTerm;
        let _: extern "system" fn(*mut ExceptionRecord, *mut Amd64Context, *mut Arm64NtContext) =
            ResetToConsistentState;
        let _: unsafe extern "system" fn(*mut SystemCpuInformation) = UpdateProcessorInformation;
        let _: extern "system" fn() = ExitToX64;
        let _: extern "system" fn() = DispatchJump;
        let _: extern "system" fn() = RetToEntryThunk;
    }

    #[test]
    fn processor_information_reports_the_emulated_amd64_cpu() {
        let mut information = SystemCpuInformation {
            processor_architecture: 12,
            processor_level: 0,
            processor_revision: 0,
            maximum_processors: 20,
            processor_feature_bits: 0x1234_5678,
        };

        // SAFETY: the callback receives a valid writable Wine structure.
        unsafe { UpdateProcessorInformation(&raw mut information) };

        assert_eq!(information.processor_architecture, 9);
        assert_eq!(information.processor_level, 21);
        assert_eq!(information.processor_revision, 1);
        assert_eq!(information.maximum_processors, 20);
        assert_eq!(information.processor_feature_bits, 0x1234_5678);
        assert_eq!(std::mem::size_of::<SystemCpuInformation>(), 12);
    }

    #[test]
    fn mapping_ownership_is_unique_and_released_on_pre_termination() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset();
        assert_eq!(ProcessInit(), STATUS_SUCCESS);
        let address = 0x1000_usize as *mut c_void;
        NotifyMemoryAlloc(address, 0x1000, 0, 0, 1, STATUS_SUCCESS);
        NotifyMemoryAlloc(address, 0x1000, 0, 0, 1, STATUS_SUCCESS);
        assert_eq!(provider_snapshot().tracked_mappings, 1);

        for index in 1..32_usize {
            let distinct = (0x1000 + index * 0x1000) as *mut c_void;
            NotifyMemoryAlloc(distinct, 0x1000, 0, 0, 1, STATUS_SUCCESS);
        }
        assert_eq!(provider_snapshot().tracked_mappings, 32);

        for index in (1..32_usize).step_by(2) {
            let distinct = (0x1000 + index * 0x1000) as *mut c_void;
            NotifyMemoryFree(distinct, 0x1000, 0, 1, STATUS_SUCCESS);
        }
        assert_eq!(provider_snapshot().tracked_mappings, 16);

        ProcessTerm(std::ptr::null_mut(), 0, STATUS_SUCCESS);
        let pending = provider_snapshot();
        assert_eq!(pending.phase, ProviderPhase::TerminationPending);
        assert_eq!(pending.tracked_mappings, 0);
        assert_eq!(pending.active_threads, 0);
        let state = lock_provider();
        assert!(state.threads.is_empty());
        assert!(state.mappings.is_empty());
        drop(state);
        ProcessTerm(std::ptr::null_mut(), 1, STATUS_SUCCESS);
    }

    #[test]
    fn native_exception_without_active_jit_frame_preserves_thread_state() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset();
        assert_eq!(ProcessInit(), STATUS_SUCCESS);
        assert_eq!(ThreadInit(), STATUS_SUCCESS);
        assert_eq!(current_thread_context().unwrap().phase, ThreadPhase::Ready);

        ResetToConsistentState(
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );

        assert_eq!(current_thread_context().unwrap().phase, ThreadPhase::Ready);
        ThreadTerm(std::ptr::null_mut(), 0);
        ProcessTerm(std::ptr::null_mut(), 0, STATUS_SUCCESS);
        ProcessTerm(std::ptr::null_mut(), 1, STATUS_SUCCESS);
    }

    #[test]
    fn native_return_frames_are_lifo_and_released_with_the_thread() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset();
        assert_eq!(ProcessInit(), STATUS_SUCCESS);
        assert_eq!(ThreadInit(), STATUS_SUCCESS);
        let first = NativeReturnFrame {
            continuation: 0x1000,
            stack: 0x2000,
            context: Arm64EcContext::default(),
        };
        let second = NativeReturnFrame {
            continuation: 0x3000,
            stack: 0x4000,
            context: Arm64EcContext::default(),
        };
        push_native_return(first).unwrap();
        push_native_return(second).unwrap();
        assert_eq!(current_thread_context().unwrap().native_return_depth, 2);
        assert!(is_native_return_continuation(second.continuation));
        assert!(!is_native_return_continuation(first.continuation));
        assert_eq!(pop_native_return().unwrap(), second);
        assert!(is_native_return_continuation(first.continuation));
        assert_eq!(pop_native_return().unwrap(), first);
        assert!(!is_native_return_continuation(first.continuation));
        assert!(pop_native_return().is_err());

        push_native_return(first).unwrap();
        let runtime = {
            let state = lock_provider();
            Arc::downgrade(
                &state
                    .threads
                    .get(&current_thread_key())
                    .expect("current thread must own a runtime")
                    .runtime,
            )
        };
        ProcessTerm(std::ptr::null_mut(), 0, STATUS_SUCCESS);
        assert_eq!(provider_snapshot().active_threads, 0);
        assert!(runtime.upgrade().is_none());
        ProcessTerm(std::ptr::null_mut(), 1, STATUS_SUCCESS);
    }

    #[test]
    fn native_return_restores_full_context_and_x64_result() {
        let mut saved = Arm64EcContext {
            x0_rcx: 0x1010,
            x1_rdx: 0x1111,
            x2_r8: 0x1212,
            x3_r9: 0x1313,
            x27_rbx: 0x1717,
            fp_rbp: 0x1818,
            x19_r12: 0x1919,
            x20_r13: 0x2020,
            x21_r14: 0x2121,
            x22_r15: 0x2222,
            x25_rsi: 0x2525,
            x26_rdi: 0x2626,
            ..Arm64EcContext::default()
        };
        saved.tail.arm64_x9 = 0xfefe;
        saved.tail.arm64_lr = 0xaaaa;
        let frame = NativeReturnFrame {
            continuation: 0x1234,
            stack: 0x5678,
            context: saved,
        };
        let mut live = Arm64EcContext::default();
        restore_native_return_context(&mut live, frame, 0xdead_beef);
        assert_eq!(live.pc_rip, 0x1234);
        assert_eq!(live.sp_rsp, 0x5678);
        assert_eq!(live.tail.arm64_lr, 0x1234);
        assert_eq!(live.x8_rax, 0xdead_beef);
        assert_eq!(live.x0_rcx, 0x1010);
        assert_eq!(live.x1_rdx, 0x1111);
        assert_eq!(live.x2_r8, 0x1212);
        assert_eq!(live.x3_r9, 0x1313);
        assert_eq!(live.x27_rbx, 0x1717);
        assert_eq!(live.fp_rbp, 0x1818);
        assert_eq!(live.x19_r12, 0x1919);
        assert_eq!(live.x20_r13, 0x2020);
        assert_eq!(live.x21_r14, 0x2121);
        assert_eq!(live.x22_r15, 0x2222);
        assert_eq!(live.x25_rsi, 0x2525);
        assert_eq!(live.x26_rdi, 0x2626);
        assert_eq!(live.tail.arm64_x9, 0xfefe);
    }

    #[test]
    fn transition_save_restores_every_arm64ec_nonvolatile_gpr() {
        assert_eq!(std::mem::size_of::<TransitionSaveArea>(), 0x110);
        assert_eq!(std::mem::offset_of!(TransitionSaveArea, native), 0x80);
        assert_eq!(std::mem::offset_of!(TransitionSaveArea, arguments), 0xc0);
        assert_eq!(std::mem::offset_of!(TransitionSaveArea, target), 0xe0);
        assert_eq!(std::mem::offset_of!(TransitionSaveArea, continuation), 0xe8);
        assert_eq!(std::mem::offset_of!(TransitionSaveArea, stack), 0xf0);
        assert_eq!(
            std::mem::offset_of!(TransitionSaveArea, stack_argument_area),
            0xf8
        );
        assert_eq!(
            std::mem::offset_of!(TransitionSaveArea, stack_argument_size),
            0x100
        );
        let saved = NativeNonvolatileRegisters {
            x19: 0x19,
            x20: 0x20,
            x21: 0x21,
            x22: 0x22,
            x25: 0x25,
            x26: 0x26,
            x27: 0x27,
            fp: 0x29,
        };
        let mut context = Arm64EcContext::default();
        restore_native_nonvolatile_registers(&mut context, saved);
        assert_eq!(context.x19_r12, saved.x19);
        assert_eq!(context.x20_r13, saved.x20);
        assert_eq!(context.x21_r14, saved.x21);
        assert_eq!(context.x22_r15, saved.x22);
        assert_eq!(context.x25_rsi, saved.x25);
        assert_eq!(context.x26_rdi, saved.x26);
        assert_eq!(context.x27_rbx, saved.x27);
        assert_eq!(context.fp_rbp, saved.fp);
    }

    #[test]
    fn entry_return_save_restores_x64_nonvolatile_state_and_exact_layout() {
        assert_eq!(std::mem::size_of::<EntryReturnSaveArea>(), 0x120);
        assert_eq!(
            std::mem::offset_of!(EntryReturnSaveArea, simd_nonvolatile),
            0
        );
        assert_eq!(std::mem::offset_of!(EntryReturnSaveArea, native), 0xa0);
        assert_eq!(std::mem::offset_of!(EntryReturnSaveArea, arguments), 0xe0);
        assert_eq!(
            std::mem::offset_of!(EntryReturnSaveArea, return_address),
            0x100
        );
        assert_eq!(std::mem::offset_of!(EntryReturnSaveArea, native_rax), 0x108);
        assert_eq!(std::mem::offset_of!(EntryReturnSaveArea, stack), 0x110);

        let mut saved = EntryReturnSaveArea {
            native: NativeNonvolatileRegisters {
                x19: 0x19,
                x20: 0x20,
                x21: 0x21,
                x22: 0x22,
                x25: 0x25,
                x26: 0x26,
                x27: 0x27,
                fp: 0x29,
            },
            arguments: [0x10, 0x11, 0x12, 0x13],
            native_rax: 0x88,
            stack: 0x1000,
            ..EntryReturnSaveArea::default()
        };
        for (index, register) in saved.simd_nonvolatile.iter_mut().enumerate() {
            let index = u64::try_from(index).unwrap();
            *register = XmmRegister {
                low: 0x6000 + index,
                high: 0xf000 + index,
            };
        }

        let mut context = Arm64EcContext::default();
        restore_entry_return_context(&mut context, &saved);
        assert_eq!(context.x0_rcx, 0x10);
        assert_eq!(context.x1_rdx, 0x11);
        assert_eq!(context.x2_r8, 0x12);
        assert_eq!(context.x3_r9, 0x13);
        assert_eq!(context.x8_rax, 0x88);
        assert_eq!(context.sp_rsp, 0x1000);
        assert_eq!(context.x19_r12, 0x19);
        assert_eq!(context.x20_r13, 0x20);
        assert_eq!(context.x21_r14, 0x21);
        assert_eq!(context.x22_r15, 0x22);
        assert_eq!(context.x25_rsi, 0x25);
        assert_eq!(context.x26_rdi, 0x26);
        assert_eq!(context.x27_rbx, 0x27);
        assert_eq!(context.fp_rbp, 0x29);
        for index in 0..10 {
            assert_eq!(context.xmm(index + 6), Some(saved.simd_nonvolatile[index]));
        }
    }

    #[test]
    fn simulation_requires_a_thread_and_reports_missing_host_context() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset();
        ProcessInit();
        BeginSimulation();
        assert_eq!(provider_snapshot().last_status, STATUS_NOT_SUPPORTED);
        ThreadInit();
        BeginSimulation();
        assert_eq!(current_thread_context().unwrap().phase, ThreadPhase::Failed);
        assert_eq!(provider_snapshot().last_status, STATUS_NOT_SUPPORTED);
        reset();
    }
}
