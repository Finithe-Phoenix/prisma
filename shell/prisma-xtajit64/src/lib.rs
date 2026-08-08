//! ARM64EC provider handshake for Wine 11.14's AMD64 emulation path.
//!
//! Wine's current `xtajit64` ABI calls [`ProcessInit`] and [`ThreadInit`]
//! without arguments. This crate owns one typed context per initialized host
//! thread and releases every context and mapping on termination. The simulation
//! entry points remain explicitly unsupported until F3-WN-005.

#![allow(non_snake_case)]

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::fmt;
use std::sync::{Mutex, MutexGuard, OnceLock};

pub type NtStatus = i32;
pub type WinBoolean = u8;
pub type WinBool = i32;
pub type Handle = *mut c_void;

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
    _private: [u8; 0],
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
    SimulationUnsupported,
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
    SimulationUnsupported,
}

impl LifecycleError {
    #[must_use]
    pub const fn nt_status(self) -> NtStatus {
        match self {
            Self::SimulationUnsupported => STATUS_NOT_SUPPORTED,
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
            Self::SimulationUnsupported => "xtajit64 simulation is not implemented",
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
}

/// Read-only state of the current thread context.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ThreadContextSnapshot {
    pub generation: u64,
    pub phase: ThreadPhase,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ThreadKey(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ThreadContext {
    generation: u64,
    phase: ThreadPhase,
}

#[derive(Debug)]
struct ProviderState {
    phase: ProviderPhase,
    generation: u64,
    threads: HashMap<ThreadKey, ThreadContext>,
    mappings: HashSet<usize>,
    simulation_requests: usize,
    cache_notifications: usize,
    last_status: NtStatus,
}

impl Default for ProviderState {
    fn default() -> Self {
        Self {
            phase: ProviderPhase::Cold,
            generation: 0,
            threads: HashMap::new(),
            mappings: HashSet::new(),
            simulation_requests: 0,
            cache_notifications: 0,
            last_status: STATUS_SUCCESS,
        }
    }
}

impl ProviderState {
    const fn is_running(&self) -> bool {
        matches!(
            self.phase,
            ProviderPhase::Initialized | ProviderPhase::SimulationRequested
        )
    }

    fn release_owned_resources(&mut self) {
        self.threads = HashMap::new();
        self.mappings = HashSet::new();
        self.simulation_requests = 0;
        self.cache_notifications = 0;
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
        }
    }
}

static PROVIDER: OnceLock<Mutex<ProviderState>> = OnceLock::new();

fn provider() -> &'static Mutex<ProviderState> {
    PROVIDER.get_or_init(|| Mutex::new(ProviderState::default()))
}

fn lock_provider() -> MutexGuard<'static, ProviderState> {
    provider()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
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
    let mut state = lock_provider();
    let outcome = if state.is_running() {
        let key = current_thread_key();
        let generation = state.generation;
        if let std::collections::hash_map::Entry::Vacant(entry) = state.threads.entry(key) {
            entry.insert(ThreadContext {
                generation,
                phase: ThreadPhase::Ready,
            });
            state.last_status = STATUS_SUCCESS;
            Ok(InitOutcome::Initialized { generation })
        } else {
            Ok(InitOutcome::AlreadyInitialized { generation })
        }
    } else {
        let error = phase_error(state.phase);
        state.last_status = error.nt_status();
        Err(error)
    };
    drop(state);
    outcome
}

/// Records a simulation request without pretending that execution occurred.
///
/// # Errors
///
/// Returns a lifecycle error if process/thread initialization is incomplete;
/// otherwise returns [`LifecycleError::SimulationUnsupported`] by design.
pub fn request_simulation() -> Result<(), LifecycleError> {
    let mut state = lock_provider();
    let outcome = if !state.is_running() {
        let error = phase_error(state.phase);
        state.last_status = error.nt_status();
        Err(error)
    } else if let Some(context) = state.threads.get_mut(&current_thread_key()) {
        context.phase = ThreadPhase::SimulationUnsupported;
        state.phase = ProviderPhase::SimulationRequested;
        state.simulation_requests = state.simulation_requests.saturating_add(1);
        state.last_status = STATUS_NOT_SUPPORTED;
        Err(LifecycleError::SimulationUnsupported)
    } else {
        state.last_status = STATUS_INVALID_DEVICE_STATE;
        Err(LifecycleError::ThreadNotInitialized)
    };
    drop(state);
    outcome
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
        state.threads.remove(&key);
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
    with_running_state(|state| {
        state.cache_notifications = state.cache_notifications.saturating_add(1);
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn BTCpu64IsProcessorFeaturePresent(_feature: u32) -> WinBoolean {
    0
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
    let _ = request_simulation();
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
        state.mappings.insert(address as usize);
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
            state.mappings.insert(address as usize);
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
            state.mappings.remove(&(address as usize));
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
            state.mappings.remove(&(address as usize));
        });
    }
}

/// Wine 11.14 `xtajit64.spec`: `NTSTATUS ProcessInit(void)`.
#[unsafe(no_mangle)]
pub extern "system" fn ProcessInit() -> NtStatus {
    initialize_process().map_or_else(LifecycleError::nt_status, |_| STATUS_SUCCESS)
}

/// Wine calls this before and after `NtTerminateProcess` for the current process.
#[unsafe(no_mangle)]
pub extern "system" fn ProcessTerm(_process: Handle, post_call: WinBool, status: NtStatus) {
    process_term(post_call != 0, status);
}

#[unsafe(no_mangle)]
pub extern "system" fn ResetToConsistentState(
    _exception_record: *mut ExceptionRecord,
    _amd64_context: *mut Amd64Context,
    _arm64_context: *mut Arm64NtContext,
) {
    let _ = request_simulation();
}

/// Wine 11.14 `xtajit64.spec`: `NTSTATUS ThreadInit(void)`.
#[unsafe(no_mangle)]
pub extern "system" fn ThreadInit() -> NtStatus {
    initialize_thread().map_or_else(LifecycleError::nt_status, |_| STATUS_SUCCESS)
}

/// Releases the context for the target or current thread. Repeated calls are harmless.
#[unsafe(no_mangle)]
pub extern "system" fn ThreadTerm(thread: Handle, _exit_code: i32) {
    thread_term(thread);
}

#[unsafe(no_mangle)]
pub extern "system" fn UpdateProcessorInformation(_information: *mut SystemCpuInformation) {}

// Wine treats these as transition thunks. Until F3-WN-005 they only record an
// explicit unsupported request and never claim that guest instructions ran.
#[unsafe(no_mangle)]
pub extern "system" fn ExitToX64() {
    let _ = request_simulation();
}

#[unsafe(no_mangle)]
pub extern "system" fn DispatchJump() {
    let _ = request_simulation();
}

#[unsafe(no_mangle)]
pub extern "system" fn RetToEntryThunk() {
    let _ = request_simulation();
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
        let _: extern "system" fn(*mut SystemCpuInformation) = UpdateProcessorInformation;
        let _: extern "system" fn() = ExitToX64;
        let _: extern "system" fn() = DispatchJump;
        let _: extern "system" fn() = RetToEntryThunk;
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

        ProcessTerm(std::ptr::null_mut(), 0, STATUS_SUCCESS);
        let pending = provider_snapshot();
        assert_eq!(pending.phase, ProviderPhase::TerminationPending);
        assert_eq!(pending.tracked_mappings, 0);
        assert_eq!(pending.active_threads, 0);
        let state = lock_provider();
        assert_eq!(state.threads.capacity(), 0);
        assert_eq!(state.mappings.capacity(), 0);
        drop(state);
        ProcessTerm(std::ptr::null_mut(), 1, STATUS_SUCCESS);
    }

    #[test]
    fn simulation_requires_a_thread_and_remains_explicitly_unsupported() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset();
        ProcessInit();
        assert_eq!(
            request_simulation(),
            Err(LifecycleError::ThreadNotInitialized)
        );
        ThreadInit();
        assert_eq!(
            request_simulation(),
            Err(LifecycleError::SimulationUnsupported)
        );
        assert_eq!(
            current_thread_context().unwrap().phase,
            ThreadPhase::SimulationUnsupported
        );
        assert_eq!(provider_snapshot().last_status, STATUS_NOT_SUPPORTED);
        reset();
    }
}
