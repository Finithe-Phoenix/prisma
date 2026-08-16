use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;

use prisma_runtime::executor::{CpuStateFrame, ExecError, EXIT_BRANCH, EXIT_NORMAL, EXIT_SYSCALL};
use prisma_xtajit64::{
    dispatch_context, provider_snapshot, Arm64EcContext, BlockExecutor, DispatchError,
    DispatchLimits, DispatchStop, GuestMemory, ProcessInit, ProcessTerm, ThreadInit, ThreadTerm,
    XmmRegister, STATUS_SUCCESS,
};

static TEST_LOCK: Mutex<()> = Mutex::new(());

struct FixtureMemory {
    base: u64,
    bytes: Vec<u8>,
}

impl GuestMemory for FixtureMemory {
    fn read_code(&self, rip: u64, max_len: usize) -> Result<Vec<u8>, String> {
        let offset = usize::try_from(rip.checked_sub(self.base).ok_or("RIP below fixture")?)
            .map_err(|_| "RIP outside host usize")?;
        let bytes = self.bytes.get(offset..).ok_or("RIP beyond fixture")?;
        Ok(bytes[..bytes.len().min(max_len)].to_vec())
    }
}

struct RecordingExecutor {
    calls: Arc<AtomicUsize>,
    saw_code: Arc<AtomicBool>,
    exit_reason: u64,
    next_pc: u64,
    rax: u64,
}

impl BlockExecutor for RecordingExecutor {
    fn execute(
        &self,
        _guest_rip: u64,
        code: &[u8],
        frame: &mut CpuStateFrame,
    ) -> Result<(), ExecError> {
        self.saw_code.fetch_or(!code.is_empty(), Ordering::SeqCst);
        self.calls.fetch_add(1, Ordering::SeqCst);
        frame.gpr[0] = self.rax;
        frame.exit_reason = self.exit_reason;
        frame.next_pc = self.next_pc;
        Ok(())
    }
}

fn reset() {
    ProcessTerm(std::ptr::null_mut(), 0, STATUS_SUCCESS);
    ProcessTerm(std::ptr::null_mut(), 1, STATUS_SUCCESS);
}

fn start() {
    reset();
    assert_eq!(ProcessInit(), STATUS_SUCCESS);
    assert_eq!(ThreadInit(), STATUS_SUCCESS);
}

const fn limits(max_blocks: usize) -> DispatchLimits {
    DispatchLimits {
        max_blocks,
        max_fetch_bytes: 16,
        max_instructions_per_block: 1,
    }
}

#[test]
fn translates_one_real_block_and_commits_registers_and_fallthrough_rip() {
    let _guard = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    start();
    let calls = Arc::new(AtomicUsize::new(0));
    let saw_code = Arc::new(AtomicBool::new(false));
    // MOV EAX, imm32 is fixture input, not a production demo byte path.
    let memory = FixtureMemory {
        base: 0x1000,
        bytes: vec![0xb8, 0x2a, 0, 0, 0],
    };
    let executor = RecordingExecutor {
        calls: Arc::clone(&calls),
        saw_code: Arc::clone(&saw_code),
        exit_reason: EXIT_NORMAL,
        next_pc: 0,
        rax: 42,
    };
    let mut context = Arm64EcContext {
        pc_rip: 0x1000,
        ..Arm64EcContext::default()
    };

    let report = dispatch_context(&mut context, &memory, &executor, limits(1)).unwrap();
    assert_eq!(report.stop, DispatchStop::BlockLimit);
    assert_eq!(report.blocks, 1);
    assert_eq!(report.instructions, 1);
    assert_eq!(context.pc_rip, 0x1005);
    assert_eq!(context.x8_rax, 42);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(saw_code.load(Ordering::SeqCst));
    reset();
}

#[test]
fn branch_exit_updates_rip_and_obeys_bound() {
    let _guard = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    start();
    let memory = FixtureMemory {
        base: 0x2000,
        bytes: vec![0x90, 0x90],
    };
    let executor = RecordingExecutor {
        calls: Arc::new(AtomicUsize::new(0)),
        saw_code: Arc::new(AtomicBool::new(false)),
        exit_reason: EXIT_BRANCH,
        next_pc: 0x2001,
        rax: 0,
    };
    let mut context = Arm64EcContext {
        pc_rip: 0x2000,
        ..Arm64EcContext::default()
    };
    let report = dispatch_context(&mut context, &memory, &executor, limits(1)).unwrap();
    assert_eq!(report.stop, DispatchStop::BlockLimit);
    assert_eq!(context.pc_rip, 0x2001);
    reset();
}

struct XmmExecutor;

impl BlockExecutor for XmmExecutor {
    fn execute(
        &self,
        guest_rip: u64,
        _code: &[u8],
        frame: &mut CpuStateFrame,
    ) -> Result<(), ExecError> {
        assert_eq!(guest_rip, 0x2800);
        assert_eq!(frame.xmm(3), Some([0x3c; 16]));
        assert!(frame.set_xmm(5, [0xa5; 16]));
        frame.exit_reason = EXIT_NORMAL;
        Ok(())
    }
}

#[test]
fn dispatch_synchronizes_xmm_state_with_the_wine_context() {
    let _guard = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    start();
    let memory = FixtureMemory {
        base: 0x2800,
        bytes: vec![0x90],
    };
    let mut context = Arm64EcContext {
        pc_rip: 0x2800,
        ..Arm64EcContext::default()
    };
    assert!(context.set_xmm(
        3,
        XmmRegister {
            low: 0x3c3c_3c3c_3c3c_3c3c,
            high: 0x3c3c_3c3c_3c3c_3c3c,
        }
    ));

    let report = dispatch_context(&mut context, &memory, &XmmExecutor, limits(1)).unwrap();
    assert_eq!(report.stop, DispatchStop::BlockLimit);
    assert_eq!(
        context.xmm(5),
        Some(XmmRegister {
            low: 0xa5a5_a5a5_a5a5_a5a5,
            high: 0xa5a5_a5a5_a5a5_a5a5,
        })
    );
    assert!(!context.set_xmm(16, XmmRegister::default()));
    assert_eq!(context.xmm(16), None);
    reset();
}

#[test]
fn syscall_and_memory_failures_are_typed_and_never_report_success() {
    let _guard = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    start();
    let memory = FixtureMemory {
        base: 0x3000,
        bytes: vec![0x0f, 0x05],
    };
    let executor = RecordingExecutor {
        calls: Arc::new(AtomicUsize::new(0)),
        saw_code: Arc::new(AtomicBool::new(false)),
        exit_reason: EXIT_SYSCALL,
        next_pc: 0,
        rax: 0,
    };
    let mut context = Arm64EcContext {
        pc_rip: 0x3000,
        ..Arm64EcContext::default()
    };
    assert!(matches!(
        dispatch_context(&mut context, &memory, &executor, limits(1)),
        Err(DispatchError::UnsupportedSyscall { rip: 0x3000 })
    ));

    context.pc_rip = 0x4000;
    assert!(matches!(
        dispatch_context(&mut context, &memory, &executor, limits(1)),
        Err(DispatchError::MemoryRead { rip: 0x4000, .. })
    ));
    reset();
}

struct CoordinatedExecutor {
    first_call: AtomicBool,
    entered: Arc<Barrier>,
    release: Arc<Barrier>,
}

impl BlockExecutor for CoordinatedExecutor {
    fn execute(
        &self,
        _guest_rip: u64,
        _code: &[u8],
        frame: &mut CpuStateFrame,
    ) -> Result<(), ExecError> {
        if !self.first_call.swap(true, Ordering::AcqRel) {
            self.entered.wait();
            self.release.wait();
        }
        frame.exit_reason = EXIT_BRANCH;
        frame.next_pc = 0x5000;
        Ok(())
    }
}

struct ConcurrentRipExecutor {
    entered: Barrier,
    seen: Mutex<Vec<u64>>,
}

impl BlockExecutor for ConcurrentRipExecutor {
    fn execute(
        &self,
        guest_rip: u64,
        _code: &[u8],
        frame: &mut CpuStateFrame,
    ) -> Result<(), ExecError> {
        self.seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(guest_rip);
        self.entered.wait();
        frame.exit_reason = EXIT_NORMAL;
        Ok(())
    }
}

#[test]
fn concurrent_dispatches_keep_their_own_guest_rip() {
    let _guard = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    reset();
    assert_eq!(ProcessInit(), STATUS_SUCCESS);
    let executor = Arc::new(ConcurrentRipExecutor {
        entered: Barrier::new(2),
        seen: Mutex::new(Vec::new()),
    });

    let workers = [0x6000, 0x7000].map(|guest_rip| {
        let executor = Arc::clone(&executor);
        thread::spawn(move || {
            assert_eq!(ThreadInit(), STATUS_SUCCESS);
            let memory = FixtureMemory {
                base: guest_rip,
                bytes: vec![0x90],
            };
            let mut context = Arm64EcContext {
                pc_rip: guest_rip,
                ..Arm64EcContext::default()
            };
            dispatch_context(&mut context, &memory, executor.as_ref(), limits(1)).unwrap()
        })
    });

    for worker in workers {
        assert_eq!(worker.join().unwrap().stop, DispatchStop::BlockLimit);
    }
    let mut seen = executor
        .seen
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    seen.sort_unstable();
    assert_eq!(seen, vec![0x6000, 0x7000]);
    reset();
}

#[test]
fn process_term_cancels_an_active_dispatch() {
    let _guard = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    reset();
    assert_eq!(ProcessInit(), STATUS_SUCCESS);
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let worker = {
        let entered = Arc::clone(&entered);
        let release = Arc::clone(&release);
        thread::spawn(move || {
            assert_eq!(ThreadInit(), STATUS_SUCCESS);
            let memory = FixtureMemory {
                base: 0x5000,
                bytes: vec![0x90],
            };
            let executor = CoordinatedExecutor {
                first_call: AtomicBool::new(false),
                entered,
                release,
            };
            let mut context = Arm64EcContext {
                pc_rip: 0x5000,
                ..Arm64EcContext::default()
            };
            dispatch_context(&mut context, &memory, &executor, limits(1_000)).unwrap()
        })
    };

    entered.wait();
    assert_eq!(provider_snapshot().active_dispatches, 1);
    ProcessTerm(std::ptr::null_mut(), 0, STATUS_SUCCESS);
    release.wait();
    let report = worker.join().unwrap();
    assert_eq!(report.stop, DispatchStop::Cancelled);
    assert_eq!(report.blocks, 1);
    assert_eq!(provider_snapshot().live_runtimes, 0);
    assert_eq!(
        provider_snapshot().phase,
        prisma_xtajit64::ProviderPhase::TerminationPending
    );
    ProcessTerm(std::ptr::null_mut(), 1, STATUS_SUCCESS);
}

#[test]
fn thread_term_and_restart_drop_every_runtime() {
    let _guard = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    start();
    assert_eq!(provider_snapshot().live_runtimes, 1);
    ThreadTerm(std::ptr::null_mut(), 0);
    assert_eq!(provider_snapshot().live_runtimes, 0);
    assert_eq!(ThreadInit(), STATUS_SUCCESS);
    assert_eq!(provider_snapshot().live_runtimes, 1);
    reset();
    assert_eq!(provider_snapshot().live_runtimes, 0);

    for _ in 0..32 {
        start();
        assert_eq!(provider_snapshot().live_runtimes, 1);
        reset();
        assert_eq!(provider_snapshot().live_runtimes, 0);
    }
}
