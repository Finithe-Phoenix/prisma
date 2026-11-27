//! Host-thread driver for the shared-arena pthread subset of RFC 0022.

use crate::ThreadRunControl;
use std::collections::{hash_map::Entry, HashMap, VecDeque};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{mpsc, Mutex};
use std::thread::{self, Scope};
use std::time::{Duration, Instant};

use super::ProgramImage;
use prisma_runtime::executor::InstalledBlock;
use prisma_translator::Translator;

use super::{
    arch_prctl, dispatch, gpr, BackedAddressSpace, CpuStateFrame, RunOutcome, Session,
    SyscallContext, ARCH_PRCTL, EXIT_BRANCH, EXIT_NORMAL, EXIT_SYSCALL,
};
use crate::clone_thread::{prepare_clone, PreparedClone};
use crate::syscall_dispatch::{errno, nr, LinuxThreadState};
use crate::thread_attributes::ThreadAttributes;
use crate::thread_blocking::{classify, BlockingCall};
use crate::thread_futex::{FutexAction, FutexQueues};

/// Reasons specific to the multi-thread driver that prevent a guest run from completing.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ThreadRunError {
    #[error("the guest frame must use this address space's contiguous arena")]
    ArenaRequired,
    #[error("all live guest threads are waiting on futexes")]
    FutexDeadlock,
    #[error("the host cancelled the guest run")]
    Cancelled,
    #[error("a host worker panicked")]
    HostWorkerPanicked,
}

struct GuestThread {
    tid: u32,
    pc: u64,
    identity: LinuxThreadState,
    attributes: ThreadAttributes,
}

struct CachedBlock {
    installed: InstalledBlock,
    bytes: usize,
    terminator: bool,
}

struct Process<'a> {
    image: &'a ProgramImage,
    context: &'a mut SyscallContext,
    memory: &'a mut BackedAddressSpace,
    blocks: HashMap<u64, CachedBlock>,
    ready: VecDeque<u32>,
    futex: FutexQueues,
    sleeping: Vec<(u32, Instant)>,
    live: usize,
    next_tid: u32,
    remaining: usize,
    leader: u32,
    leader_exit: Option<i32>,
    outcome: Option<Result<RunOutcome, ThreadRunError>>,
}

struct ThreadGroup<'a> {
    process: Mutex<Process<'a>>,
    control: &'a ThreadRunControl,
}

enum Action {
    Continue,
    Clone([u64; 6]),
    Wait,
    Sleep(Duration),
    Exit(i32),
    Stop(RunOutcome),
}

impl Process<'_> {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        clippy::cast_sign_loss
    )]
    fn step(
        &mut self,
        translator: &mut Translator,
        worker: &mut GuestThread,
        state: &mut CpuStateFrame,
    ) -> Action {
        self.remaining -= 1;

        // The executable image used by Session::translate_at is immutable. Each
        // installation lives until this group has joined all of its workers.
        let block = match self.blocks.entry(worker.pc) {
            Entry::Occupied(slot) => slot.into_mut(),
            Entry::Vacant(slot) => {
                let Some(block) = super::translate_image_block(self.image, translator, worker.pc)
                else {
                    return Action::Stop(RunOutcome::UnmappedPc(worker.pc));
                };
                let installed = match InstalledBlock::install(&block.code) {
                    Ok(code) => code,
                    Err(err) => return Action::Stop(RunOutcome::ExecUnavailable(err)),
                };
                slot.insert(CachedBlock {
                    installed,
                    bytes: block.guest_bytes,
                    terminator: block.ended_at_terminator,
                })
            }
        };
        state.exit_reason = EXIT_NORMAL;
        // SAFETY: the installed body came from the session translator. The
        // frame is bound to this group's live guest arena, and this worker
        // holds the process mutex throughout the call.
        if let Err(err) = unsafe { block.installed.execute(state) } {
            return Action::Stop(RunOutcome::ExecUnavailable(err));
        }
        match state.exit_reason {
            EXIT_BRANCH => {
                worker.pc = state.next_pc;
                return Action::Continue;
            }
            EXIT_SYSCALL => {}
            _ if !block.terminator => {
                worker.pc = worker.pc.wrapping_add(block.bytes as u64);
                return Action::Continue;
            }
            _ => return Action::Stop(RunOutcome::NonSyscallExit { pc: worker.pc }),
        }
        worker.pc = worker.pc.wrapping_add(block.bytes as u64);
        let number = state.gpr[gpr::RAX];
        let args = [
            state.gpr[gpr::RDI],
            state.gpr[gpr::RSI],
            state.gpr[gpr::RDX],
            state.gpr[gpr::R10],
            state.gpr[gpr::R8],
            state.gpr[gpr::R9],
        ];
        let result = match number {
            nr::CLONE => return Action::Clone(args),
            nr::EXIT => return Action::Exit(args[0] as i32 & 255),
            nr::EXIT_GROUP => return Action::Stop(RunOutcome::Exited(args[0] as i32 & 255)),
            nr::FUTEX => match self.futex.syscall(worker.tid, self.memory, args) {
                FutexAction::Return(value) => {
                    self.enqueue_woken();
                    value
                }
                FutexAction::Park => {
                    state.gpr[gpr::RAX] = 0;
                    return Action::Wait;
                }
            },
            ARCH_PRCTL => arch_prctl(state, self.memory, args[0], args[1]),
            _ => match classify(self.memory, &self.context.fds, number, args) {
                BlockingCall::Sleep(duration) => {
                    state.gpr[gpr::RAX] = 0;
                    return Action::Sleep(duration);
                }
                BlockingCall::Return(value) => value,
                BlockingCall::Immediate => {
                    self.context.swap_thread(&mut worker.identity);
                    worker.attributes.swap_with(self.context);
                    let value = catch_unwind(AssertUnwindSafe(|| {
                        dispatch(self.context, self.memory, number, args)
                    }));
                    worker.attributes.swap_with(self.context);
                    self.context.swap_thread(&mut worker.identity);
                    match value {
                        Ok(result) => result,
                        Err(payload) => std::panic::resume_unwind(payload),
                    }
                }
            },
        };
        state.gpr[gpr::RAX] = result as u64;
        Action::Continue
    }

    fn enqueue_woken(&mut self) {
        self.futex.enqueue_woken(&mut self.ready);
    }

    fn resume_sleepers(&mut self, now: Instant) -> Option<Duration> {
        self.sleeping.retain(|&(tid, deadline)| {
            if deadline <= now {
                self.ready.push_back(tid);
                false
            } else {
                true
            }
        });
        self.sleeping
            .iter()
            .map(|(_, due)| due.saturating_duration_since(now))
            .min()
    }

    fn check_deadlock(&mut self) {
        if self.live != 0 && self.outcome.is_none() && self.futex.parked_count() == self.live {
            self.outcome = Some(Err(ThreadRunError::FutexDeadlock));
        }
    }
}

impl ThreadGroup<'_> {
    fn spawn_child<'s>(
        &'s self,
        scope: &'s Scope<'s, '_>,
        p: &mut Process<'_>,
        parent: &GuestThread,
        state: &CpuStateFrame,
        args: [u64; 6],
    ) -> Result<u32, i64> {
        let tid = p.next_tid;
        let child = prepare_clone(state, p.memory, args, parent.pc, tid)?;
        let attributes = parent.attributes.for_child();
        // Do not let the child touch the arena until both TID writes have
        // succeeded. A spawn failure leaves both guest words untouched.
        let (start, ticket) = mpsc::sync_channel::<PreparedClone>(1);
        thread::Builder::new()
            .spawn_scoped(scope, move || {
                if let Ok(mut child) = ticket.recv() {
                    let mut local = GuestThread {
                        tid: child.tid,
                        pc: child.next_pc,
                        identity: LinuxThreadState::child(child.tid, child.clear_child_tid),
                        attributes,
                    };
                    self.drive(scope, &mut local, &mut child.frame);
                }
            })
            .map_err(|_| errno::EAGAIN)?;
        child.publish_tid(p.memory)?;
        p.next_tid += 1;
        p.live += 1;
        p.ready.push_back(tid);
        start
            .send(child)
            .expect("scoped worker is waiting for its launch ticket");
        Ok(tid)
    }

    fn drive<'s>(
        &'s self,
        scope: &'s Scope<'s, '_>,
        local: &mut GuestThread,
        frame: &mut CpuStateFrame,
    ) {
        let run = catch_unwind(AssertUnwindSafe(|| self.run_worker(scope, local, frame)));
        let mut p = self
            .process
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if run.is_err() {
            p.outcome = Some(Err(ThreadRunError::HostWorkerPanicked));
        }
        p.ready.retain(|&tid| tid != local.tid);
        p.futex.remove(local.tid);
        if let Some(address) = local.identity.clear_at_exit(p.memory) {
            p.futex.wake(address, 1);
            p.enqueue_woken();
        }
        p.live -= 1;
        if p.live == 0 && p.outcome.is_none() {
            p.outcome = Some(Ok(RunOutcome::Exited(p.leader_exit.unwrap_or_default())));
        }
        p.check_deadlock();
        self.control.notify_all();
    }

    #[allow(clippy::cast_sign_loss)]
    fn run_worker<'s>(
        &'s self,
        scope: &'s Scope<'s, '_>,
        local: &mut GuestThread,
        frame: &mut CpuStateFrame,
    ) {
        // Pass objects may be thread-affine. Only immutable translations and
        // their W^X installations are shared through the process cache.
        let mut translator = Translator::new();
        let mut p = self
            .process
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            let was_empty = p.ready.is_empty();
            let next_timer = p.resume_sleepers(Instant::now());
            if was_empty && !p.ready.is_empty() {
                self.control.notify_all();
            }
            if p.outcome.is_some() {
                return;
            }
            if self.control.is_cancelled() {
                p.outcome = Some(Err(ThreadRunError::Cancelled));
                self.control.notify_all();
                return;
            }
            if p.remaining == 0 {
                p.outcome = Some(Ok(RunOutcome::StepLimit));
                self.control.notify_all();
                return;
            }
            if p.ready.front() != Some(&local.tid) {
                self.control.wait(p, next_timer);
                p = self
                    .process
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                continue;
            }
            match p.step(&mut translator, local, frame) {
                Action::Continue => {}
                Action::Clone(args) => {
                    frame.gpr[gpr::RAX] = self
                        .spawn_child(scope, &mut p, local, frame, args)
                        .map_or_else(|errno| errno as u64, u64::from);
                }
                Action::Wait => {
                    p.ready.pop_front();
                    p.check_deadlock();
                    self.control.notify_all();
                    continue;
                }
                Action::Sleep(duration) if !duration.is_zero() => {
                    if let Some(deadline) = Instant::now().checked_add(duration) {
                        p.ready.pop_front();
                        p.sleeping.push((local.tid, deadline));
                        self.control.notify_all();
                        continue;
                    }
                    frame.gpr[gpr::RAX] = errno::EINVAL as u64;
                }
                Action::Sleep(_) => {}
                Action::Exit(code) => {
                    if p.leader == local.tid {
                        p.leader_exit = Some(code);
                    }
                    if p.live == 1 {
                        p.outcome = Some(Ok(RunOutcome::Exited(p.leader_exit.unwrap_or(code))));
                    }
                    return;
                }
                Action::Stop(reason) => {
                    p.outcome = Some(Ok(reason));
                    self.control.notify_all();
                    return;
                }
            }
            p.ready.pop_front();
            p.ready.push_back(local.tid);
            self.control.notify_all();
        }
    }
}

impl Session {
    /// Run a pthread-style guest using one host thread per guest thread and
    /// one shared step budget. Prepare an arena with [`Self::prepare_arena`].
    /// The process mutex serializes each guest block against arena accesses
    /// by the other threads. Futex waits and relative sleeps release it.
    /// Host I/O without a cancellable implementation returns ENOSYS in this
    /// driver. [`Session::run`] continues to provide the single-thread interface.
    ///
    /// # Safety
    /// Translated loads and stores use raw effective addresses. The caller must
    /// ensure that the guest program and its inputs only access live guest
    /// mappings with appropriate permissions, including each child stack.
    ///
    /// # Errors
    /// Returns [`ThreadRunError`] when the arena is missing, the group deadlocks,
    /// or a host worker panics.
    pub unsafe fn run_threaded(
        &mut self,
        ctx: &mut SyscallContext,
        mem: &mut BackedAddressSpace,
        state: &mut CpuStateFrame,
        max_steps: usize,
    ) -> Result<RunOutcome, ThreadRunError> {
        // SAFETY: both variants require the same guest memory guarantees.
        unsafe {
            self.run_threaded_with_control(ctx, mem, state, max_steps, &ThreadRunControl::new())
        }
    }

    /// Like [`Self::run_threaded`], with a caller-owned shutdown handle. Cancellation
    /// is observed between guest blocks and immediately interrupts parked workers.
    ///
    /// # Safety
    /// The guest and its inputs must obey the memory requirements of
    /// [`Self::run_threaded`] for the entire run.
    ///
    /// # Errors
    /// In addition to the errors from [`Self::run_threaded`], host cancellation
    /// produces [`ThreadRunError::Cancelled`].
    pub unsafe fn run_threaded_with_control(
        &mut self,
        ctx: &mut SyscallContext,
        mem: &mut BackedAddressSpace,
        state: &mut CpuStateFrame,
        max_steps: usize,
        control: &ThreadRunControl,
    ) -> Result<RunOutcome, ThreadRunError> {
        if mem.mem_base() != Some(state.mem_base) {
            return Err(ThreadRunError::ArenaRequired);
        }
        let tid = ctx.thread_tid();
        let mut local = GuestThread {
            tid,
            pc: self.entry_pc(),
            identity: LinuxThreadState::child(tid, 0),
            attributes: ThreadAttributes::capture_leader(ctx),
        };
        ctx.swap_thread(&mut local.identity);
        ctx.exit_status = None;
        let shared = ThreadGroup {
            process: Mutex::new(Process {
                image: &self.image,
                context: ctx,
                memory: mem,
                ready: VecDeque::from([tid]),
                remaining: max_steps,
                leader: tid,
                live: 1,
                blocks: HashMap::new(),
                next_tid: tid.saturating_add(1),
                futex: FutexQueues::default(),
                sleeping: Vec::new(),
                leader_exit: None,
                outcome: None,
            }),
            control,
        };
        thread::scope(|scope| shared.drive(scope, &mut local, state));
        let process = shared
            .process
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        process.context.swap_thread(&mut local.identity);
        local.attributes.swap_with(process.context);
        let outcome = process.outcome.expect("all scoped workers have exited");
        if let Ok(RunOutcome::Exited(status)) = &outcome {
            process.context.exit_status = Some(*status);
        }
        outcome
    }
}
