//! Loadable ARM64EC provider surface for Wine 11.14's AMD64 emulation path.
//!
//! This crate implements the ABI and lifecycle handshake only. In particular,
//! [`BeginSimulation`] records an explicit `STATUS_NOT_SUPPORTED` state and
//! does not translate or execute guest instructions. F3-WN-004/005 own the
//! context bridge and the real simulation loop.

#![allow(non_snake_case)]

use std::ffi::c_void;
use std::sync::atomic::{AtomicI32, AtomicU8, AtomicUsize, Ordering};

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ProviderPhase {
    Cold = 0,
    Initialized = 1,
    SimulationRequested = 2,
}

impl ProviderPhase {
    const fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Initialized,
            2 => Self::SimulationRequested,
            _ => Self::Cold,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProviderSnapshot {
    pub phase: ProviderPhase,
    pub active_threads: usize,
    pub tracked_mappings: usize,
    pub simulation_requests: usize,
    pub cache_notifications: usize,
    pub last_status: NtStatus,
}

static PHASE: AtomicU8 = AtomicU8::new(ProviderPhase::Cold as u8);
static ACTIVE_THREADS: AtomicUsize = AtomicUsize::new(0);
static TRACKED_MAPPINGS: AtomicUsize = AtomicUsize::new(0);
static SIMULATION_REQUESTS: AtomicUsize = AtomicUsize::new(0);
static CACHE_NOTIFICATIONS: AtomicUsize = AtomicUsize::new(0);
static LAST_STATUS: AtomicI32 = AtomicI32::new(STATUS_SUCCESS);

#[must_use]
pub fn provider_snapshot() -> ProviderSnapshot {
    ProviderSnapshot {
        phase: ProviderPhase::from_u8(PHASE.load(Ordering::Acquire)),
        active_threads: ACTIVE_THREADS.load(Ordering::Acquire),
        tracked_mappings: TRACKED_MAPPINGS.load(Ordering::Acquire),
        simulation_requests: SIMULATION_REQUESTS.load(Ordering::Acquire),
        cache_notifications: CACHE_NOTIFICATIONS.load(Ordering::Acquire),
        last_status: LAST_STATUS.load(Ordering::Acquire),
    }
}

fn reset_state() {
    ACTIVE_THREADS.store(0, Ordering::Release);
    TRACKED_MAPPINGS.store(0, Ordering::Release);
    SIMULATION_REQUESTS.store(0, Ordering::Release);
    CACHE_NOTIFICATIONS.store(0, Ordering::Release);
    LAST_STATUS.store(STATUS_SUCCESS, Ordering::Release);
    PHASE.store(ProviderPhase::Cold as u8, Ordering::Release);
}

fn initialized() -> bool {
    PHASE.load(Ordering::Acquire) != ProviderPhase::Cold as u8
}

const fn successful(status: NtStatus) -> bool {
    status >= 0
}

fn decrement_saturating(counter: &AtomicUsize) {
    let _ = counter.fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
        Some(value.saturating_sub(1))
    });
}

fn record_simulation_request() {
    SIMULATION_REQUESTS.fetch_add(1, Ordering::AcqRel);
    LAST_STATUS.store(STATUS_NOT_SUPPORTED, Ordering::Release);
    PHASE.store(ProviderPhase::SimulationRequested as u8, Ordering::Release);
}

#[unsafe(no_mangle)]
pub extern "system" fn BTCpu64FlushInstructionCache(_address: *const c_void, _size: usize) {
    CACHE_NOTIFICATIONS.fetch_add(1, Ordering::AcqRel);
}

#[unsafe(no_mangle)]
pub extern "system" fn BTCpu64IsProcessorFeaturePresent(_feature: u32) -> WinBoolean {
    0
}

#[unsafe(no_mangle)]
pub extern "system" fn BTCpu64NotifyMemoryDirty(_address: *mut c_void, _size: usize) {
    CACHE_NOTIFICATIONS.fetch_add(1, Ordering::AcqRel);
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
    record_simulation_request();
}

#[unsafe(no_mangle)]
pub extern "system" fn FlushInstructionCacheHeavy(_address: *const c_void, _size: usize) {
    CACHE_NOTIFICATIONS.fetch_add(1, Ordering::AcqRel);
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
    if !initialized() {
        return STATUS_INVALID_DEVICE_STATE;
    }
    if !address.is_null() && size != 0 {
        TRACKED_MAPPINGS.fetch_add(1, Ordering::AcqRel);
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
    if initialized() && post_call != 0 && successful(status) && !address.is_null() && size != 0 {
        TRACKED_MAPPINGS.fetch_add(1, Ordering::AcqRel);
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
    if initialized() && post_call != 0 && successful(status) && !address.is_null() {
        decrement_saturating(&TRACKED_MAPPINGS);
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
    if initialized() && post_call != 0 && successful(status) && !address.is_null() {
        decrement_saturating(&TRACKED_MAPPINGS);
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn ProcessInit() -> NtStatus {
    reset_state();
    PHASE.store(ProviderPhase::Initialized as u8, Ordering::Release);
    STATUS_SUCCESS
}

#[unsafe(no_mangle)]
pub extern "system" fn ProcessTerm(_process: Handle, _post_call: WinBool, _status: NtStatus) {
    reset_state();
}

#[unsafe(no_mangle)]
pub extern "system" fn ResetToConsistentState(
    _exception_record: *mut ExceptionRecord,
    _amd64_context: *mut Amd64Context,
    _arm64_context: *mut Arm64NtContext,
) {
    LAST_STATUS.store(STATUS_NOT_SUPPORTED, Ordering::Release);
}

#[unsafe(no_mangle)]
pub extern "system" fn ThreadInit() -> NtStatus {
    if !initialized() {
        return STATUS_INVALID_DEVICE_STATE;
    }
    ACTIVE_THREADS.fetch_add(1, Ordering::AcqRel);
    STATUS_SUCCESS
}

#[unsafe(no_mangle)]
pub extern "system" fn ThreadTerm(_thread: Handle, _exit_code: i32) {
    decrement_saturating(&ACTIVE_THREADS);
}

#[unsafe(no_mangle)]
pub extern "system" fn UpdateProcessorInformation(_information: *mut SystemCpuInformation) {}

// Wine treats these three exports as transition thunks, not ordinary APIs.
// F3-WN-003 only guarantees that their addresses are present and callable with
// the platform system ABI. Invoking one records the same explicit unsupported
// state as BeginSimulation; F3-WN-005 replaces them with real transition code.
#[unsafe(no_mangle)]
pub extern "system" fn ExitToX64() {
    record_simulation_request();
}

#[unsafe(no_mangle)]
pub extern "system" fn DispatchJump() {
    record_simulation_request();
}

#[unsafe(no_mangle)]
pub extern "system" fn RetToEntryThunk() {
    record_simulation_request();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn exported_callbacks_have_the_wine_11_14_signatures() {
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
        let _: extern "system" fn(Handle, WinBool, NtStatus) = ProcessTerm;
        let _: extern "system" fn(*mut ExceptionRecord, *mut Amd64Context, *mut Arm64NtContext) =
            ResetToConsistentState;
        let _: extern "system" fn() -> NtStatus = ThreadInit;
        let _: extern "system" fn(Handle, i32) = ThreadTerm;
        let _: extern "system" fn(*mut SystemCpuInformation) = UpdateProcessorInformation;
        let _: extern "system" fn() = ExitToX64;
        let _: extern "system" fn() = DispatchJump;
        let _: extern "system" fn() = RetToEntryThunk;
    }

    #[test]
    fn lifecycle_and_mapping_state_are_released_deterministically() {
        let _guard = TEST_LOCK.lock().unwrap();
        ProcessTerm(std::ptr::null_mut(), 0, STATUS_SUCCESS);
        assert_eq!(ThreadInit(), STATUS_INVALID_DEVICE_STATE);
        assert_eq!(ProcessInit(), STATUS_SUCCESS);
        assert_eq!(ThreadInit(), STATUS_SUCCESS);
        assert_eq!(ThreadInit(), STATUS_SUCCESS);

        let first = 0x1000_usize as *mut c_void;
        let second = 0x2000_usize as *mut c_void;
        assert_eq!(
            NotifyMapViewOfSection(
                std::ptr::null_mut(),
                first,
                std::ptr::null_mut(),
                0x1000,
                0,
                0,
            ),
            STATUS_SUCCESS
        );
        NotifyMemoryAlloc(second, 0x2000, 0, 0, 1, STATUS_SUCCESS);
        assert_eq!(provider_snapshot().tracked_mappings, 2);

        NotifyMemoryFree(second, 0, 0, 1, STATUS_SUCCESS);
        NotifyUnmapViewOfSection(first, 1, STATUS_SUCCESS);
        ThreadTerm(std::ptr::null_mut(), 0);
        assert_eq!(provider_snapshot().tracked_mappings, 0);
        assert_eq!(provider_snapshot().active_threads, 1);

        ProcessTerm(std::ptr::null_mut(), 0, STATUS_SUCCESS);
        ProcessTerm(std::ptr::null_mut(), 1, STATUS_SUCCESS);
        assert_eq!(
            provider_snapshot(),
            ProviderSnapshot {
                phase: ProviderPhase::Cold,
                active_threads: 0,
                tracked_mappings: 0,
                simulation_requests: 0,
                cache_notifications: 0,
                last_status: STATUS_SUCCESS,
            }
        );
    }

    #[test]
    fn simulation_entry_points_report_unsupported_without_claiming_execution() {
        let _guard = TEST_LOCK.lock().unwrap();
        ProcessInit();
        BeginSimulation();
        let snapshot = provider_snapshot();
        assert_eq!(snapshot.phase, ProviderPhase::SimulationRequested);
        assert_eq!(snapshot.simulation_requests, 1);
        assert_eq!(snapshot.last_status, STATUS_NOT_SUPPORTED);
        ProcessTerm(std::ptr::null_mut(), 0, STATUS_SUCCESS);
    }

    #[test]
    fn failed_or_pre_call_notifications_do_not_acquire_mapping_ownership() {
        let _guard = TEST_LOCK.lock().unwrap();
        ProcessInit();
        let address = 0x3000_usize as *mut c_void;
        NotifyMemoryAlloc(address, 0x1000, 0, 0, 0, STATUS_SUCCESS);
        NotifyMemoryAlloc(address, 0x1000, 0, 0, 1, STATUS_NOT_SUPPORTED);
        NotifyMemoryAlloc(std::ptr::null_mut(), 0x1000, 0, 0, 1, STATUS_SUCCESS);
        assert_eq!(provider_snapshot().tracked_mappings, 0);
        ProcessTerm(std::ptr::null_mut(), 0, STATUS_SUCCESS);
    }
}
