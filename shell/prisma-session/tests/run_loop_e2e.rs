//! End-to-end: a guest LOOP runs through the DBT by chaining blocks.
//!
//! The guest counts an accumulator up while a counter decrements, looping via a
//! conditional branch, then exits with the accumulator as its status:
//!
//! ```text
//!   mov edi, 0          ; accumulator (exit status)
//!   mov ecx, 5          ; loop counter
//! top:
//!   add edi, 1
//!   sub ecx, 1
//!   cmp ecx, 0          ; set flags from the counter
//!   jnz top             ; CondJumpRel back to top while ecx != 0
//!   mov eax, 60         ; exit
//!   syscall             ; exit(edi)
//! ```
//!
//! The `jnz` is a relative branch with no sibling block in the single-block
//! translation, so it exits via `next_pc`/`EXIT_BRANCH` and `Session::run` chains
//! back to `top`. After 5 iterations `edi == 5`, so the guest exits with status 5
//! — observable proof the loop actually ran on hardware. aarch64-gated; on the
//! x86 dev host the load/translate/prepare path runs and `run` reports execution
//! unavailable before any block executes.
//!
//! The loop condition uses an explicit `cmp` because the Rust decoder does not
//! yet emit the implicit flag write of an arithmetic op (`sub`/`add` set the
//! result but not NZCV) — a separate decoder-correctness gap. `cmp`/`test` do
//! emit flags, so this exercises block chaining independently of that gap.

use prisma_orchestrator::module_table::ModuleTable;
use prisma_session::{RunOutcome, Session};

/// A minimal PE32+ with one `.text` section (base 0x1_4000_0000, entry RVA
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
const LOOP_PROGRAM: [u8; 28] = [
    0xBF, 0x00, 0x00, 0x00, 0x00, // mov edi, 0     (accumulator / status)
    0xB9, 0x05, 0x00, 0x00, 0x00, // mov ecx, 5     (counter)
    // top: (entry + 10)
    0x83, 0xC7, 0x01,             // add edi, 1
    0x83, 0xE9, 0x01,             // sub ecx, 1
    0x83, 0xF9, 0x00,             // cmp ecx, 0     (sets flags)
    0x75, 0xF5,                   // jnz top        (rel8 -11 -> entry+10)
    0xB8, 0x3C, 0x00, 0x00, 0x00, // mov eax, 60    (exit)
    0x0F, 0x05,                   // syscall        (exit(edi))
];

#[test]
fn guest_loop_runs_by_chaining_blocks_across_a_branch() {
    let mut s =
        Session::load(&pe_with_code(&LOOP_PROGRAM), &ModuleTable::new()).expect("load the loop");
    let mut p = s.prepare(0x2_0000_0000, &[b"prog"], &[]).expect("prepare");
    // 5 iterations is ~6 blocks; a generous step budget proves termination.
    let outcome = s.run(&mut p.ctx, &mut p.mem, &mut p.state, 64);

    if cfg!(target_arch = "aarch64") {
        // edi counted 5 loop iterations, so the guest exits with status 5.
        assert!(matches!(outcome, RunOutcome::Exited(5)), "{outcome:?}");
    } else {
        assert!(
            matches!(outcome, RunOutcome::ExecUnavailable(_)),
            "off-target the run reports execution is unavailable: {outcome:?}"
        );
    }
}
