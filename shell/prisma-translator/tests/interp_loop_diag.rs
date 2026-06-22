//! Differential diagnosis of the deferred arithmetic-loop bug, on x86.
//!
//! Runs the exact loop program through decode -> renumber -> optimize (the same
//! `optimize_fused_block` the backend lowers) and then the reference interpreter,
//! chaining blocks the way `Session::run` does. If the interpreter reaches the
//! correct result (`edi == 5`), the IR is correct and the ARM64 failure is in
//! lowering/codegen; if it is also wrong (`edi == 1`), the defect is in the
//! decode/optimize pipeline. Either way this is a permanent IR-level regression
//! test that needs no ARM64.

use prisma_ir::Gpr;
use prisma_translator::interp::{interpret_block, BlockOutcome, GuestRegs};
use prisma_translator::Translator;

const ENTRY_VA: u64 = 0x1_4000_1000;

#[rustfmt::skip]
const LOOP_PROGRAM: [u8; 28] = [
    0xBF, 0x00, 0x00, 0x00, 0x00, // mov edi, 0
    0xB9, 0x05, 0x00, 0x00, 0x00, // mov ecx, 5
    // top: (entry + 10)
    0x83, 0xC7, 0x01,             // add edi, 1
    0x83, 0xE9, 0x01,             // sub ecx, 1
    0x83, 0xF9, 0x00,             // cmp ecx, 0
    0x75, 0xF5,                   // jnz top  (-11 -> entry+10)
    0xB8, 0x3C, 0x00, 0x00, 0x00, // mov eax, 60
    0x0F, 0x05,                   // syscall
];

// Decode -> optimize -> interpret each block, chaining like Session::run, until
// the exit syscall. Returns the final register state.
fn run_to_exit(prog: &[u8]) -> GuestRegs {
    let mut t = Translator::new();
    let mut regs = GuestRegs::default();
    let mut pc = ENTRY_VA;
    for _ in 0..64 {
        let off = usize::try_from(pc - ENTRY_VA).expect("pc within the program");
        let opt = t
            .optimize_fused_block(pc, &prog[off..], 16)
            .expect("optimize the block");
        match interpret_block(&opt.func.blocks[0].stmts, &mut regs) {
            BlockOutcome::Branch(target) => pc = target,
            BlockOutcome::Syscall => break,
            BlockOutcome::Fallthrough => pc += opt.guest_bytes as u64,
            other => panic!("unexpected block outcome {other:?} at pc={pc:#x}"),
        }
    }
    regs
}

#[test]
fn loop_program_interprets_to_five_iterations() {
    let regs = run_to_exit(&LOOP_PROGRAM);
    // The loop adds 1 to edi for each of the 5 decrements of ecx.
    assert_eq!(
        regs.gpr[Gpr::Rdi as usize],
        5,
        "edi should count 5 iterations (ecx ended at {})",
        regs.gpr[Gpr::Rcx as usize]
    );
}

// A bare `sub reg, reg; jnz` loop with NO explicit `cmp` — exercises the reg-reg
// ALU flag write the decoder now emits. Without it, `sub` would leave NZCV
// untouched and the loop would mis-iterate.
#[rustfmt::skip]
const SUB_LOOP_PROGRAM: [u8; 28] = [
    0xBF, 0x00, 0x00, 0x00, 0x00, // mov edi, 0
    0xB9, 0x05, 0x00, 0x00, 0x00, // mov ecx, 5
    0xBA, 0x01, 0x00, 0x00, 0x00, // mov edx, 1
    // top: (entry + 15)
    0x01, 0xD7,                   // add edi, edx   (reg-reg)
    0x29, 0xD1,                   // sub ecx, edx   (reg-reg, sets flags)
    0x75, 0xFA,                   // jnz top  (-6 -> entry+15)
    0xB8, 0x3C, 0x00, 0x00, 0x00, // mov eax, 60
    0x0F, 0x05,                   // syscall
];

#[test]
fn bare_sub_jnz_loop_iterates_via_reg_alu_flags() {
    let regs = run_to_exit(&SUB_LOOP_PROGRAM);
    assert_eq!(
        regs.gpr[Gpr::Rdi as usize],
        5,
        "bare sub;jnz loop must run 5 times (ecx ended at {})",
        regs.gpr[Gpr::Rcx as usize]
    );
}
