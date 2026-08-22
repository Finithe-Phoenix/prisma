#![cfg(target_arch = "aarch64")]

use prisma_runtime::executor::{execute_block, gpr, CpuStateFrame, ExecError};
use prisma_translator::Translator;
use prisma_xtajit64::{
    dispatch_context, provider_snapshot, Arm64EcContext, BlockExecutor, DispatchLimits,
    DispatchStop, GuestMemory, ProcessInit, ProcessTerm, ThreadInit, STATUS_SUCCESS,
};

const GUEST_PC: u64 = 0x1_4008_99e0;
const GUEST_ARENA_BASE: u64 = 0x1_40d0_0000;
const GUEST_ARENA_BYTES: usize = 0x10_0000;
const GUEST_GLOBAL: u64 = 0x1_40db_8ac0;
const INITIAL_RSP: u64 = 0x1_40dc_0000;
const GUARD_BYTES: usize = 64;
const CANARY: u8 = 0xa5;

const BOOTSTRAP: [u8; 66] = [
    0x48, 0x89, 0xf8, // mov rax,rdi
    0x48, 0x89, 0xf3, // mov rbx,rsi
    0x48, 0x83, 0xec, 0x28, // sub rsp,0x28
    0x48, 0x83, 0xe4, 0xf0, // and rsp,-0x10
    0x48, 0x89, 0x44, 0x24, 0x18, // mov [rsp+0x18],rax
    0x48, 0x89, 0x5c, 0x24, 0x20, // mov [rsp+0x20],rbx
    0x48, 0xc7, 0xc5, 0x00, 0x00, 0x00, 0x00, // mov rbp,0
    0x48, 0x8d, 0x3d, 0xba, 0xf0, 0xd2, 0x00, // lea rdi,[rip+0xd2f0ba]
    0x48, 0x8d, 0x9c, 0x24, 0x00, 0x00, 0xff, 0xff, // lea rbx,[rsp-0x10000]
    0x48, 0x89, 0x5f, 0x10, // mov [rdi+0x10],rbx
    0x48, 0x89, 0x5f, 0x18, // mov [rdi+0x18],rbx
    0x48, 0x89, 0x1f, // mov [rdi],rbx
    0x48, 0x89, 0x67, 0x08, // mov [rdi+0x8],rsp
    0xb8, 0x00, 0x00, 0x00, 0x00, // mov eax,0
];

struct FixtureMemory;

impl GuestMemory for FixtureMemory {
    fn read_code(&self, rip: u64, max_len: usize) -> Result<Vec<u8>, String> {
        let offset = usize::try_from(rip.checked_sub(GUEST_PC).ok_or("RIP below fixture")?)
            .map_err(|_| "RIP outside host usize")?;
        let bytes = BOOTSTRAP.get(offset..).ok_or("RIP beyond fixture")?;
        Ok(bytes[..bytes.len().min(max_len)].to_vec())
    }
}

struct RebasedExecutor {
    mem_base: u64,
}

impl BlockExecutor for RebasedExecutor {
    fn execute(
        &self,
        _guest_rip: u64,
        code: Vec<u8>,
        frame: &mut CpuStateFrame,
    ) -> Result<(), ExecError> {
        frame.mem_base = self.mem_base;
        execute_block(&code, frame)
    }
}

fn write_u64(arena: &mut [u8], guest_address: u64, value: u64) {
    let offset = usize::try_from(guest_address - GUEST_ARENA_BASE).unwrap();
    arena[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn execute_dispatch_instruction(
    translator: &mut Translator,
    guest_pc: &mut u64,
    instruction: &[u8],
    frame: &mut CpuStateFrame,
) {
    let (translation, ended_at_terminator) = translator
        .translate_dispatch_instruction(*guest_pc, instruction)
        .expect("translate exact dispatch instruction");
    assert_eq!(translation.guest_bytes, instruction.len());
    assert!(!ended_at_terminator);
    execute_block(&translation.code, frame).expect("execute exact dispatch instruction");
    *guest_pc = guest_pc.wrapping_add(instruction.len() as u64);
}

#[test]
fn go_page_bits_partial_word_mask_survives_exact_dispatch_boundaries() {
    let mut bitmap = [0_u64; 8];
    let guest_bitmap = 0x1_0000_u64;
    let host_bitmap = bitmap.as_mut_ptr().addr() as u64;
    let mut frame = CpuStateFrame::default();
    frame.mem_base = host_bitmap.wrapping_sub(guest_bitmap);
    frame.gpr[gpr::RAX] = guest_bitmap;
    frame.gpr[gpr::RSI] = 2;
    frame.gpr[gpr::RDI] = 172;

    let mut translator = Translator::for_dispatch();
    let mut guest_pc = 0x1_4004_49cf_u64;

    execute_dispatch_instruction(
        &mut translator,
        &mut guest_pc,
        &[0x83, 0xe7, 0x3f],
        &mut frame,
    ); // and edi,0x3f
    assert_eq!(frame.gpr[gpr::RDI], 44);

    execute_dispatch_instruction(
        &mut translator,
        &mut guest_pc,
        &[0x48, 0x8d, 0x4f, 0x01],
        &mut frame,
    ); // lea rcx,[rdi+1]
    assert_eq!(frame.gpr[gpr::RCX], 45);

    execute_dispatch_instruction(
        &mut translator,
        &mut guest_pc,
        &[0xba, 0x01, 0x00, 0x00, 0x00],
        &mut frame,
    ); // mov edx,1
    assert_eq!(frame.gpr[gpr::RDX], 1);

    execute_dispatch_instruction(
        &mut translator,
        &mut guest_pc,
        &[0x48, 0xd3, 0xe2],
        &mut frame,
    ); // shl rdx,cl
    assert_eq!(frame.gpr[gpr::RDX], 1_u64 << 45);

    execute_dispatch_instruction(
        &mut translator,
        &mut guest_pc,
        &[0x48, 0x83, 0xf9, 0x40],
        &mut frame,
    ); // cmp rcx,64
    assert_eq!(frame.cf, 1);

    execute_dispatch_instruction(
        &mut translator,
        &mut guest_pc,
        &[0x48, 0x19, 0xdb],
        &mut frame,
    ); // sbb rbx,rbx
    assert_eq!(frame.gpr[gpr::RBX], u64::MAX);

    execute_dispatch_instruction(
        &mut translator,
        &mut guest_pc,
        &[0x48, 0x21, 0xda],
        &mut frame,
    ); // and rdx,rbx
    assert_eq!(frame.gpr[gpr::RDX], 1_u64 << 45);

    execute_dispatch_instruction(
        &mut translator,
        &mut guest_pc,
        &[0x48, 0xff, 0xca],
        &mut frame,
    ); // dec rdx
    let expected_mask = (1_u64 << 45) - 1;
    assert_eq!(frame.gpr[gpr::RDX], expected_mask);

    execute_dispatch_instruction(
        &mut translator,
        &mut guest_pc,
        &[0x48, 0x09, 0x14, 0xf0],
        &mut frame,
    ); // or [rax+rsi*8],rdx
    assert_eq!(bitmap[2], expected_mask);
    assert!(bitmap[..2].iter().all(|word| *word == 0));
    assert!(bitmap[3..].iter().all(|word| *word == 0));
}

#[test]
fn exact_oh_my_posh_bootstrap_writes_only_its_guest_stack_and_global() {
    ProcessTerm(std::ptr::null_mut(), 0, STATUS_SUCCESS);
    ProcessTerm(std::ptr::null_mut(), 1, STATUS_SUCCESS);
    assert_eq!(ProcessInit(), STATUS_SUCCESS);
    assert_eq!(ThreadInit(), STATUS_SUCCESS);

    let mut guarded = vec![CANARY; GUARD_BYTES + GUEST_ARENA_BYTES + GUARD_BYTES];
    let arena = &mut guarded[GUARD_BYTES..GUARD_BYTES + GUEST_ARENA_BYTES];
    let host_base = arena.as_ptr().addr() as u64;
    let executor = RebasedExecutor {
        mem_base: host_base.wrapping_sub(GUEST_ARENA_BASE),
    };
    let initial_rdi = 0x0123_4567_89ab_cdef;
    let initial_rsi = 0xfedc_ba98_7654_3210;
    let mut context = Arm64EcContext {
        x25_rsi: initial_rsi,
        x26_rdi: initial_rdi,
        sp_rsp: INITIAL_RSP,
        pc_rip: GUEST_PC,
        ..Arm64EcContext::default()
    };

    let report = dispatch_context(
        &mut context,
        &FixtureMemory,
        &executor,
        DispatchLimits {
            max_blocks: 14,
            max_fetch_bytes: 16,
            max_instructions_per_block: 1,
        },
    )
    .expect("execute exact Oh My Posh bootstrap prefix");

    let expected_rsp = INITIAL_RSP.wrapping_sub(0x28) & !0xf;
    let expected_stack_limit = expected_rsp.wrapping_sub(0x1_0000);
    assert_eq!(report.stop, DispatchStop::BlockLimit);
    assert_eq!(report.blocks, 14);
    assert_eq!(report.instructions, 14);
    assert_eq!(report.rip, GUEST_PC + BOOTSTRAP.len() as u64);
    assert_eq!(context.pc_rip, report.rip);
    assert_eq!(context.x8_rax, 0);
    assert_eq!(context.x27_rbx, expected_stack_limit);
    assert_eq!(context.sp_rsp, expected_rsp);
    assert_eq!(context.fp_rbp, 0);
    assert_eq!(context.x25_rsi, initial_rsi);
    assert_eq!(context.x26_rdi, GUEST_GLOBAL);

    let mut expected_arena = vec![CANARY; GUEST_ARENA_BYTES];
    write_u64(&mut expected_arena, expected_rsp + 0x18, initial_rdi);
    write_u64(&mut expected_arena, expected_rsp + 0x20, initial_rsi);
    write_u64(&mut expected_arena, GUEST_GLOBAL, expected_stack_limit);
    write_u64(&mut expected_arena, GUEST_GLOBAL + 0x08, expected_rsp);
    write_u64(
        &mut expected_arena,
        GUEST_GLOBAL + 0x10,
        expected_stack_limit,
    );
    write_u64(
        &mut expected_arena,
        GUEST_GLOBAL + 0x18,
        expected_stack_limit,
    );
    assert_eq!(arena, expected_arena);
    assert!(guarded[..GUARD_BYTES].iter().all(|byte| *byte == CANARY));
    assert!(guarded[GUARD_BYTES + GUEST_ARENA_BYTES..]
        .iter()
        .all(|byte| *byte == CANARY));

    ProcessTerm(std::ptr::null_mut(), 0, STATUS_SUCCESS);
    ProcessTerm(std::ptr::null_mut(), 1, STATUS_SUCCESS);
    let snapshot = provider_snapshot();
    assert_eq!(snapshot.active_threads, 0);
    assert_eq!(snapshot.tracked_mappings, 0);
    assert_eq!(snapshot.active_dispatches, 0);
    assert_eq!(snapshot.live_runtimes, 0);
    assert_eq!(snapshot.live_dispatch_stacks, 0);
}
