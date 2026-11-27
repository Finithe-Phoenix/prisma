//! Linux guest clone/futex acceptance tests through the normal session run loop.
//! No host scheduling order is assumed: the producer retries WAKE until one
//! waiter consumes it, then joins on the guest child's clear_child_tid word.

#[path = "support/static_elf.rs"]
mod static_elf;

use prisma_orchestrator::module_table::ModuleTable;
use prisma_runtime::executor::{gpr, ExecError};
use prisma_session::{PreparedRun, RunOutcome, Session, ThreadRunControl, ThreadRunError};

const STACK: u64 = 0x1_4002_0000;
const SHARED: u64 = STACK + 0x100;
const CHILD_SP: u64 = STACK + 0x8000;
const CHILD_TLS: u64 = STACK + 0x9000;

// CLONE_VM | FS | FILES | SIGHAND | THREAD | SETTLS | PARENT_SETTID |
// CHILD_CLEARTID | CHILD_SETTID. Initial frame supplies the clone arguments.
const CLONE_FLAGS: u64 =
    0x100 | 0x200 | 0x400 | 0x800 | 0x10000 | 0x80000 | 0x100000 | 0x200000 | 0x1000000;

fn pe_with_code(code: &[u8]) -> Vec<u8> {
    let mut buf = vec![0u8; 64 + 4 + 20 + 240 + 40];
    buf[0] = b'M';
    buf[1] = b'Z';
    buf[0x3C..0x40].copy_from_slice(&64u32.to_le_bytes());
    buf[64..68].copy_from_slice(b"PE\0\0");
    let coff = 68;
    buf[coff..coff + 2].copy_from_slice(&0x8664u16.to_le_bytes());
    buf[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes());
    buf[coff + 16..coff + 18].copy_from_slice(&240u16.to_le_bytes());
    let opt = coff + 20;
    buf[opt..opt + 2].copy_from_slice(&0x020Bu16.to_le_bytes());
    buf[opt + 16..opt + 20].copy_from_slice(&0x1000u32.to_le_bytes());
    buf[opt + 24..opt + 32].copy_from_slice(&0x1_4000_0000u64.to_le_bytes());
    buf[opt + 56..opt + 60].copy_from_slice(&0x10000u32.to_le_bytes());
    let sec = opt + 240;
    buf[sec..sec + 5].copy_from_slice(b".text");
    buf[sec + 8..sec + 12].copy_from_slice(&0x1000u32.to_le_bytes());
    buf[sec + 12..sec + 16].copy_from_slice(&0x1000u32.to_le_bytes());
    let raw_off = u32::try_from(buf.len()).unwrap();
    buf[sec + 16..sec + 20].copy_from_slice(&u32::try_from(code.len()).unwrap().to_le_bytes());
    buf[sec + 20..sec + 24].copy_from_slice(&raw_off.to_le_bytes());
    buf.extend_from_slice(code);
    buf
}

fn prepare(session: &Session) -> PreparedRun {
    let mut run = session.prepare_arena(STACK, &[b"threads"], &[]).unwrap();
    run.mem.write(SHARED + 44, &u32::MAX.to_le_bytes()).unwrap();
    run.state.gpr[gpr::RBX] = SHARED;
    run.state.gpr[gpr::RDI] = CLONE_FLAGS;
    run.state.gpr[gpr::RSI] = CHILD_SP;
    run.state.gpr[gpr::RDX] = SHARED + 12; // parent_tid
    run.state.gpr[gpr::R10] = SHARED + 16; // child_tid
    run.state.gpr[gpr::R8] = CHILD_TLS;
    run
}

// Child waits on a shared word and consumes the value published before WAKE.
// Parent joins via the child TID and returns that consumed value.
#[rustfmt::skip]
const PRODUCER_CONSUMER: &[u8] = &[
    // entry:
    0xb8, 0xba, 0x00, 0x00, 0x00,        // mov eax,0xba
    0x0f, 0x05,                          // syscall
    0x89, 0x43, 0x34,                    // mov [rbx+0x34],eax
    0xb8, 0x38, 0x00, 0x00, 0x00,        // mov eax,0x38
    0x0f, 0x05,                          // syscall
    0x85, 0xc0,                          // test eax,eax
    0x74, 0x55,                          // je child
    0x89, 0x43, 0x28,                    // mov [rbx+0x28],eax
    0xb8, 0x2a, 0x00, 0x00, 0x00,        // mov eax,0x2a
    0x89, 0x43, 0x04,                    // mov [rbx+0x4],eax
    // retry_wake:
    0x48, 0x89, 0xdf,                    // mov rdi,rbx
    0xbe, 0x81, 0x00, 0x00, 0x00,        // mov esi,0x81
    0xba, 0x01, 0x00, 0x00, 0x00,        // mov edx,0x1
    0xb8, 0xca, 0x00, 0x00, 0x00,        // mov eax,0xca
    0x0f, 0x05,                          // syscall
    0x83, 0xf8, 0x01,                    // cmp eax,0x1
    0x75, 0xe7,                          // jne retry_wake
    // join:
    0x8b, 0x53, 0x10,                    // mov edx,[rbx+0x10]
    0x85, 0xd2,                          // test edx,edx
    0x74, 0x20,                          // je done
    0xb8, 0x01, 0x00, 0x00, 0x00,        // mov eax,0x1
    0x89, 0x43, 0x3c,                    // mov [rbx+0x3c],eax
    0x48, 0x8d, 0x7b, 0x10,              // lea rdi,[rbx+0x10]
    0xbe, 0x80, 0x00, 0x00, 0x00,        // mov esi,0x80
    0x45, 0x31, 0xd2,                    // xor r10d,r10d
    0xb8, 0xca, 0x00, 0x00, 0x00,        // mov eax,0xca
    0x0f, 0x05,                          // syscall
    0x89, 0x43, 0x38,                    // mov [rbx+0x38],eax
    0xeb, 0xd9,                          // jmp join
    // done:
    0x8b, 0x7b, 0x08,                    // mov edi,[rbx+0x8]
    0xb8, 0x3c, 0x00, 0x00, 0x00,        // mov eax,0x3c
    0x0f, 0x05,                          // syscall
    // child:
    0x48, 0x89, 0x63, 0x18,              // mov [rbx+0x18],rsp
    0xbf, 0x03, 0x10, 0x00, 0x00,        // mov edi,0x1003
    0x48, 0x8d, 0x73, 0x20,              // lea rsi,[rbx+0x20]
    0xb8, 0x9e, 0x00, 0x00, 0x00,        // mov eax,0x9e
    0x0f, 0x05,                          // syscall
    0xb8, 0xba, 0x00, 0x00, 0x00,        // mov eax,0xba
    0x0f, 0x05,                          // syscall
    0x89, 0x43, 0x14,                    // mov [rbx+0x14],eax
    0x8b, 0x43, 0x10,                    // mov eax,[rbx+0x10]
    0x89, 0x43, 0x30,                    // mov [rbx+0x30],eax
    0x48, 0x89, 0xdf,                    // mov rdi,rbx
    0xbe, 0x80, 0x00, 0x00, 0x00,        // mov esi,0x80
    0x31, 0xd2,                          // xor edx,edx
    0x45, 0x31, 0xd2,                    // xor r10d,r10d
    0xb8, 0xca, 0x00, 0x00, 0x00,        // mov eax,0xca
    0x0f, 0x05,                          // syscall
    0x89, 0x43, 0x2c,                    // mov [rbx+0x2c],eax
    0x8b, 0x43, 0x04,                    // mov eax,[rbx+0x4]
    0x89, 0x43, 0x08,                    // mov [rbx+0x8],eax
    // Child may exit only after the parent publishes its marker and parks
    // in FUTEX_WAIT on child_tid in the same guest block.
    // await_join:
    0x8b, 0x43, 0x3c,                    // mov eax,[rbx+0x3c]
    0x83, 0xf8, 0x01,                    // cmp eax,0x1
    0x75, 0xf8,                          // jne await_join
    0xbf, 0x4d, 0x00, 0x00, 0x00,        // mov edi,0x4d
    0xb8, 0x3c, 0x00, 0x00, 0x00,        // mov eax,0x3c
    0x0f, 0x05,                          // syscall
];

// Both guest threads wait on an unchanged word.
#[rustfmt::skip]
const ALL_PARKED: &[u8] = &[
    0xb8, 0x38, 0x00, 0x00, 0x00,        // mov eax,0x38
    0x0f, 0x05,                          // syscall
    0x48, 0x89, 0xdf,                    // mov rdi,rbx
    0xbe, 0x80, 0x00, 0x00, 0x00,        // mov esi,0x80
    0x31, 0xd2,                          // xor edx,edx
    0x45, 0x31, 0xd2,                    // xor r10d,r10d
    0xb8, 0xca, 0x00, 0x00, 0x00,        // mov eax,0xca
    0x0f, 0x05,                          // syscall
];

// Child parks while parent invokes exit_group for the whole guest.
#[rustfmt::skip]
const EXIT_GROUP: &[u8] = &[
    0xb8, 0x38, 0x00, 0x00, 0x00,        // mov eax,0x38
    0x0f, 0x05,                          // syscall
    0x83, 0xf8, 0x00,                    // cmp eax,0x0
    0x75, 0x14,                          // jne parent
    0x48, 0x89, 0xdf,                    // mov rdi,rbx
    0xbe, 0x80, 0x00, 0x00, 0x00,        // mov esi,0x80
    0x31, 0xd2,                          // xor edx,edx
    0x45, 0x31, 0xd2,                    // xor r10d,r10d
    0xb8, 0xca, 0x00, 0x00, 0x00,        // mov eax,0xca
    0x0f, 0x05,                          // syscall
    // parent:
    0xbf, 0x20, 0x00, 0x00, 0x00,        // mov edi,0x20
    0xb8, 0xe7, 0x00, 0x00, 0x00,        // mov eax,0xe7
    0x0f, 0x05,                          // syscall
];

// Child publishes readiness and calls nanosleep in one block, under the process
// mutex. Observing readiness in the parent means the sleep has already begun.
#[rustfmt::skip]
const CHILD_SLEEP: &[u8] = &[
    0xb8, 0x38, 0x00, 0x00, 0x00,        // mov eax,0x38
    0x0f, 0x05,                          // syscall
    0x83, 0xf8, 0x00,                    // cmp eax,0x0
    0x75, 0x29,                          // jne parent
    0xb8, 0x01, 0x00, 0x00, 0x00,        // mov eax,0x1
    0x89, 0x43, 0x38,                    // mov [rbx+0x38],eax
    0x48, 0x8d, 0x7b, 0x40,              // lea rdi,[rbx+0x40]
    0x31, 0xf6,                          // xor esi,esi
    0xb8, 0x23, 0x00, 0x00, 0x00,        // mov eax,0x23
    0x0f, 0x05,                          // syscall
    0xb8, 0x01, 0x00, 0x00, 0x00,        // mov eax,0x1
    0x89, 0x43, 0x3c,                    // mov [rbx+0x3c],eax
    0xbf, 0x5b, 0x00, 0x00, 0x00,        // mov edi,0x5b
    0xb8, 0x3c, 0x00, 0x00, 0x00,        // mov eax,0x3c
    0x0f, 0x05,                          // syscall
    // parent:
    0x8b, 0x43, 0x38,                    // mov eax,[rbx+0x38]
    0x83, 0xf8, 0x00,                    // cmp eax,0x0
    0x74, 0xf8,                          // je parent
];

fn read_word(run: &PreparedRun, offset: u64) -> u32 {
    u32::from_le_bytes(
        run.mem
            .read(SHARED + offset, 4)
            .unwrap()
            .try_into()
            .unwrap(),
    )
}

fn read_qword(run: &PreparedRun, offset: u64) -> u64 {
    u64::from_le_bytes(
        run.mem
            .read(SHARED + offset, 8)
            .unwrap()
            .try_into()
            .unwrap(),
    )
}

#[test]
fn producer_wakes_consumer_and_joins_via_clear_child_tid() {
    let session = Session::load(&pe_with_code(PRODUCER_CONSUMER), &ModuleTable::new()).unwrap();
    check_producer_consumer(session);
}

#[test]
fn static_elf_producer_wakes_consumer_and_joins_via_clear_child_tid() {
    let elf = static_elf::image(STACK - 0x2_0000, PRODUCER_CONSUMER);
    let session = Session::load_static_elf(&elf).unwrap();
    check_producer_consumer(session);
}

fn check_producer_consumer(mut session: Session) {
    // Prepare again after shutdown to catch stale shared state and wait queues.
    for _ in 0..2 {
        let mut run = prepare(&session);
        run.mem.write(SHARED + 56, &u32::MAX.to_le_bytes()).unwrap();
        // SAFETY: RBX stays at SHARED. All guest reads and writes, including
        // the TID words and ARCH_GET_FS output, fit in its first 64 stack bytes.
        let outcome =
            unsafe { session.run_threaded(&mut run.ctx, &mut run.mem, &mut run.state, 10_000) };
        if cfg!(target_arch = "aarch64") {
            assert!(
                matches!(outcome, Ok(RunOutcome::Exited(42))),
                "{outcome:?}; frame={:?}; shared={:?}",
                run.state.gpr,
                (0..14).map(|n| read_word(&run, 4 * n)).collect::<Vec<_>>()
            );
            assert_eq!(read_word(&run, 4), 42);
            assert_eq!(read_word(&run, 8), 42);
            let tid = read_word(&run, 40); // parent's clone return value
            assert!(tid > 0);
            assert_ne!(read_word(&run, 52), tid, "child must have its own TID");
            assert_eq!(read_word(&run, 12), tid); // parent_tid
            assert_eq!(read_word(&run, 20), tid); // child's gettid
            assert_eq!(read_word(&run, 48), tid); // child_tid before exit
            assert_eq!(read_word(&run, 16), 0); // cleared at thread exit
            assert_eq!(read_qword(&run, 24), CHILD_SP);
            assert_eq!(read_qword(&run, 32), CHILD_TLS);
            assert_eq!(read_word(&run, 44), 0, "child woke on producer WAKE");
            assert_eq!(read_word(&run, 56), 0, "parent woke on clear_child_tid");
        } else {
            assert!(
                matches!(
                    outcome,
                    Ok(RunOutcome::ExecUnavailable(ExecError::WrongArch))
                ),
                "{outcome:?}"
            );
        }
    }
}

#[test]
fn teardown_reaps_children_on_process_exit_and_deadlock() {
    for (code, deadlock) in [(ALL_PARKED, true), (EXIT_GROUP, false)] {
        let mut session = Session::load(&pe_with_code(code), &ModuleTable::new()).unwrap();
        let mut run = prepare(&session);
        // SAFETY: Both guests use only the futex at SHARED and the two TID
        // words initialized by prepare, all in the arena's live stack mapping.
        let outcome =
            unsafe { session.run_threaded(&mut run.ctx, &mut run.mem, &mut run.state, 100) };
        if cfg!(target_arch = "aarch64") {
            if deadlock {
                assert!(
                    matches!(outcome, Err(ThreadRunError::FutexDeadlock)),
                    "{outcome:?}"
                );
            } else {
                assert!(matches!(outcome, Ok(RunOutcome::Exited(32))), "{outcome:?}");
            }
            assert!(read_word(&run, 12) > 0, "clone must have completed");
            assert_eq!(read_word(&run, 16), 0, "reaped child left its TID behind");
        } else {
            assert!(
                matches!(
                    outcome,
                    Ok(RunOutcome::ExecUnavailable(ExecError::WrongArch))
                ),
                "{outcome:?}"
            );
        }
    }
}

#[test]
fn group_exit_and_step_budget_interrupt_a_sleeping_child() {
    // Append one of two parent continuations. The shared prefix polls the
    // child's readiness marker, then falls through to this instruction.
    let exit32: &[u8] = &[0xbf, 32, 0, 0, 0, 0xb8, 231, 0, 0, 0, 0x0f, 0x05];
    let spin: &[u8] = &[0xeb, 0xfe]; // jmp to self until shared budget expires
    for (continuation, budget_exit) in [(exit32, false), (spin, true)] {
        let code = [CHILD_SLEEP, continuation].concat();
        let mut session = Session::load(&pe_with_code(&code), &ModuleTable::new()).unwrap();
        let mut run = prepare(&session);
        run.mem.write(SHARED + 64, &30_i64.to_le_bytes()).unwrap();
        let start = std::time::Instant::now();
        // SAFETY: SHARED through SHARED + 79 contains the TID words, markers,
        // and full timespec in the mapped stack. Both continuations only branch or exit.
        let outcome =
            unsafe { session.run_threaded(&mut run.ctx, &mut run.mem, &mut run.state, 128) };
        if cfg!(target_arch = "aarch64") {
            if budget_exit {
                assert!(matches!(outcome, Ok(RunOutcome::StepLimit)), "{outcome:?}");
            } else {
                assert!(matches!(outcome, Ok(RunOutcome::Exited(32))), "{outcome:?}");
            }
            assert!(
                start.elapsed() < std::time::Duration::from_secs(20),
                "teardown waited for the 30-second guest sleep"
            );
            assert_eq!(read_word(&run, 56), 1, "child reached nanosleep");
            assert_eq!(read_word(&run, 60), 0, "child returned past the sleep");
            assert_eq!(read_word(&run, 16), 0, "sleeping child was reaped");
        } else {
            assert!(
                matches!(
                    outcome,
                    Ok(RunOutcome::ExecUnavailable(ExecError::WrongArch))
                ),
                "{outcome:?}"
            );
        }
    }
}

#[test]
fn cancellation_before_run_prevents_clone_and_session_can_be_reused() {
    let mut session = Session::load(&pe_with_code(PRODUCER_CONSUMER), &ModuleTable::new()).unwrap();
    let mut run = prepare(&session);
    let control = ThreadRunControl::new();
    control.cancel();
    // SAFETY: prepare seeds RBX at SHARED in this arena. Every data access
    // in PRODUCER_CONSUMER fits in the next 64 mapped stack bytes.
    let stopped = unsafe {
        session.run_threaded_with_control(
            &mut run.ctx,
            &mut run.mem,
            &mut run.state,
            10_000,
            &control,
        )
    };
    assert!(
        matches!(stopped, Err(ThreadRunError::Cancelled)),
        "{stopped:?}"
    );
    assert_eq!(
        read_word(&run, 12),
        0,
        "cancelled before clone published its TID"
    );
    // SAFETY: The pre-cancelled run executed no guest blocks. The fixture's
    // registers and the arena backing its SHARED range remain valid for this run.
    let restarted =
        unsafe { session.run_threaded(&mut run.ctx, &mut run.mem, &mut run.state, 10_000) };
    if cfg!(target_arch = "aarch64") {
        assert!(
            matches!(restarted, Ok(RunOutcome::Exited(42))),
            "{restarted:?}"
        );
    } else {
        assert!(
            matches!(
                restarted,
                Ok(RunOutcome::ExecUnavailable(ExecError::WrongArch))
            ),
            "{restarted:?}"
        );
    }
}
