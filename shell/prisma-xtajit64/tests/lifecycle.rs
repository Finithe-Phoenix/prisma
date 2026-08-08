use std::ffi::c_void;
use std::sync::{Arc, Barrier, Mutex};
use std::thread;

use prisma_xtajit64::{
    current_thread_context, initialize_process, initialize_thread, provider_snapshot, InitOutcome,
    LifecycleError, ProcessInit, ProcessTerm, ProviderPhase, ThreadInit, ThreadTerm,
    STATUS_INVALID_DEVICE_STATE, STATUS_NOT_SUPPORTED, STATUS_SUCCESS,
};

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn reset_provider() {
    ProcessTerm(std::ptr::null_mut(), 0, STATUS_SUCCESS);
    ProcessTerm(std::ptr::null_mut(), 1, STATUS_SUCCESS);
}

#[test]
fn process_and_thread_init_are_idempotent() {
    let _guard = TEST_LOCK.lock().unwrap();
    reset_provider();
    assert_eq!(ThreadInit(), STATUS_INVALID_DEVICE_STATE);

    let InitOutcome::Initialized { generation } = initialize_process().unwrap() else {
        panic!("cold provider must create a generation");
    };
    assert_eq!(
        initialize_process(),
        Ok(InitOutcome::AlreadyInitialized { generation })
    );
    assert_eq!(
        initialize_thread(),
        Ok(InitOutcome::Initialized { generation })
    );
    assert_eq!(
        initialize_thread(),
        Ok(InitOutcome::AlreadyInitialized { generation })
    );
    assert_eq!(provider_snapshot().active_threads, 1);

    ThreadTerm(std::ptr::null_mut(), 0);
    ThreadTerm(std::ptr::null_mut(), 0);
    assert_eq!(provider_snapshot().active_threads, 0);
    reset_provider();
}

#[test]
fn process_term_obeys_wine_pre_and_post_call_semantics() {
    let _guard = TEST_LOCK.lock().unwrap();
    reset_provider();
    ProcessInit();
    ThreadInit();
    let generation = provider_snapshot().generation;

    ProcessTerm(std::ptr::null_mut(), 0, STATUS_SUCCESS);
    assert_eq!(provider_snapshot().phase, ProviderPhase::TerminationPending);
    assert_eq!(provider_snapshot().active_threads, 0);
    assert_eq!(
        initialize_process(),
        Err(LifecycleError::ProcessTerminating)
    );

    ProcessTerm(std::ptr::null_mut(), 1, STATUS_NOT_SUPPORTED);
    let recovered = provider_snapshot();
    assert_eq!(recovered.phase, ProviderPhase::Initialized);
    assert_eq!(recovered.generation, generation);
    assert_eq!(recovered.last_status, STATUS_NOT_SUPPORTED);
    assert_eq!(ThreadInit(), STATUS_SUCCESS);

    ProcessTerm(std::ptr::null_mut(), 0, STATUS_SUCCESS);
    ProcessTerm(std::ptr::null_mut(), 1, STATUS_SUCCESS);
    assert_eq!(provider_snapshot().phase, ProviderPhase::Cold);
}

#[test]
fn concurrent_threads_own_exactly_one_context_each() {
    const THREADS: usize = 12;

    let _guard = TEST_LOCK.lock().unwrap();
    reset_provider();
    ProcessInit();
    let generation = provider_snapshot().generation;
    let initialized = Arc::new(Barrier::new(THREADS + 1));
    let release = Arc::new(Barrier::new(THREADS + 1));

    let workers: Vec<_> = (0..THREADS)
        .map(|_| {
            let initialized = Arc::clone(&initialized);
            let release = Arc::clone(&release);
            thread::spawn(move || {
                assert_eq!(ThreadInit(), STATUS_SUCCESS);
                assert_eq!(ThreadInit(), STATUS_SUCCESS);
                assert_eq!(current_thread_context().unwrap().generation, generation);
                initialized.wait();
                release.wait();
                ThreadTerm(std::ptr::null_mut(), 0);
                ThreadTerm(std::ptr::null_mut(), 0);
            })
        })
        .collect();

    initialized.wait();
    assert_eq!(provider_snapshot().active_threads, THREADS);
    release.wait();
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(provider_snapshot().active_threads, 0);
    reset_provider();
}

#[test]
fn repeated_generations_release_all_owned_resources() {
    let _guard = TEST_LOCK.lock().unwrap();
    reset_provider();
    let mut previous_generation = provider_snapshot().generation;

    for cycle in 0..128_usize {
        assert_eq!(ProcessInit(), STATUS_SUCCESS);
        assert_eq!(ThreadInit(), STATUS_SUCCESS);
        let snapshot = provider_snapshot();
        assert!(snapshot.generation > previous_generation);
        assert_eq!(snapshot.active_threads, 1);

        let address = (0x1_0000 + cycle * 0x1000) as *mut c_void;
        prisma_xtajit64::NotifyMemoryAlloc(address, 0x1000, 0, 0, 1, STATUS_SUCCESS);
        assert_eq!(provider_snapshot().tracked_mappings, 1);

        ProcessTerm(std::ptr::null_mut(), 0, STATUS_SUCCESS);
        ProcessTerm(std::ptr::null_mut(), 1, STATUS_SUCCESS);
        ProcessTerm(std::ptr::null_mut(), 1, STATUS_SUCCESS);
        let released = provider_snapshot();
        assert_eq!(released.phase, ProviderPhase::Cold);
        assert_eq!(released.active_threads, 0);
        assert_eq!(released.tracked_mappings, 0);
        assert_eq!(released.simulation_requests, 0);
        assert_eq!(released.cache_notifications, 0);
        previous_generation = released.generation;
    }
}
