//! End-to-end: a guest `call`/`ret` chains across blocks through the run loop.
//!
//! ```text
//!   call func        ; pushes the return address, jumps to func
//!   mov eax, 60      ; <- ret lands here
//!   syscall          ; exit(edi)
//! func:
//!   mov edi, 7
//!   ret              ; pops the return address, jumps back
//! ```
//!
//! The call pushes its return address onto the guest stack and exits to the run
//! loop at `func`; `ret` pops it and exits back to the instruction after the
//! call (RFC 0020 makes the guest stack real, so the stack *is* the return-
//! address stack — no host call frame). Three blocks chain: entry -> func ->
//! after-call, then `exit(edi)` reports 7 — observable proof the call/ret pair
//! round-tripped. Before this, `ret` ended the block with a host return and the
//! run loop halted with `NonSyscallExit`.
//!
//! aarch64-gated; on the x86 dev host the load/translate/prepare path runs and
//! `run` reports execution unavailable.

use prisma_orchestrator::module_table::ModuleTable;
use prisma_session::{RunOutcome, Session};

/// A minimal PE32+ with one `.text` section (image base 0x1_4000_0000, entry RVA
/// 0x1000, virtual size 0x10000) whose file-backed bytes are `code`.
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
    buf[sec + 16..sec + 20].copy_from_slice(&(code.len() as u32).to_le_bytes());
    buf[sec + 20..sec + 24].copy_from_slice(&raw_off.to_le_bytes());
    buf.extend_from_slice(code);
    buf
}

#[rustfmt::skip]
const CALL_RET_PROGRAM: [u8; 18] = [
    0xE8, 0x07, 0x00, 0x00, 0x00, // call func (+7 -> offset 12)
    0xB8, 0x3C, 0x00, 0x00, 0x00, // mov eax, 60   (ret lands here)
    0x0F, 0x05,                   // syscall       (exit(edi))
    // func (offset 12):
    0xBF, 0x07, 0x00, 0x00, 0x00, // mov edi, 7
    0xC3,                         // ret
];

// Compact stack base just above the image so the arena window stays small.
const STACK_BASE: u64 = 0x1_4002_0000;

#[test]
fn guest_call_ret_chains_across_blocks() {
    let mut s = Session::load(&pe_with_code(&CALL_RET_PROGRAM), &ModuleTable::new())
        .expect("load the call/ret program");
    let mut p = s
        .prepare_arena(STACK_BASE, &[b"prog"], &[])
        .expect("prepare against a contiguous arena");

    // entry -> func -> after-call -> syscall is ~4 blocks; budget generously.
    let outcome = s.run(&mut p.ctx, &mut p.mem, &mut p.state, 16);

    if cfg!(target_arch = "aarch64") {
        // func set edi=7 and ret returned to the exit sequence, so exit == 7.
        assert!(
            matches!(outcome, RunOutcome::Exited(7)),
            "outcome={:?} rdi={} rsp={:#x}",
            outcome,
            p.state.gpr[7], // RDI
            p.state.gpr[4], // RSP — back to its initial value if ret balanced
        );
    } else {
        assert!(
            matches!(outcome, RunOutcome::ExecUnavailable(_)),
            "off-target the run reports execution is unavailable: {outcome:?}"
        );
    }
}

// Indirect call through a register (call rax) + ret. Exercises CallReg/JumpReg
// block-exit: the target is a dynamic guest PC in a register, not a constant.
//   movabs rax, func ; call rax ; mov eax,60 ; syscall ; func: mov edi,9 ; ret
// func is at entry+0x13 = 0x1_4000_1013.
#[rustfmt::skip]
const CALL_REG_PROGRAM: [u8; 25] = [
    0x48, 0xB8, 0x13, 0x10, 0x00, 0x40, 0x01, 0x00, 0x00, 0x00, // movabs rax, 0x1_4000_1013
    0xFF, 0xD0,                                                 // call rax
    0xB8, 0x3C, 0x00, 0x00, 0x00,                               // mov eax, 60 (ret lands here)
    0x0F, 0x05,                                                 // syscall
    // func (offset 0x13):
    0xBF, 0x09, 0x00, 0x00, 0x00,                               // mov edi, 9
    0xC3,                                                       // ret
];

#[test]
fn guest_indirect_call_reg_chains() {
    let mut s = Session::load(&pe_with_code(&CALL_REG_PROGRAM), &ModuleTable::new())
        .expect("load the call-reg program");
    let mut p = s
        .prepare_arena(STACK_BASE, &[b"prog"], &[])
        .expect("prepare against a contiguous arena");

    let outcome = s.run(&mut p.ctx, &mut p.mem, &mut p.state, 16);

    if cfg!(target_arch = "aarch64") {
        // call rax jumped to func (edi=9), ret returned to the exit sequence.
        assert!(
            matches!(outcome, RunOutcome::Exited(9)),
            "outcome={:?} rdi={}",
            outcome,
            p.state.gpr[7], // RDI
        );
    } else {
        assert!(
            matches!(outcome, RunOutcome::ExecUnavailable(_)),
            "{outcome:?}"
        );
    }
}
