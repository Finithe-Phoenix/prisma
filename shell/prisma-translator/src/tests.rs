use super::*;

// mov rax, rcx  (REX.W 89 /r)
const MOV_RAX_RCX: &[u8] = &[0x48, 0x89, 0xC8];
// add rax, 0x10 (REX.W 83 /0 ib)
const ADD_RAX_IMM8: &[u8] = &[0x48, 0x83, 0xC0, 0x10];

#[test]
fn decode_block_successors_follows_a_jmp_without_lowering() {
    // EB 0E = JMP +0x0E -> targets guest_addr + 2 + 0x0E. Decoding alone
    // finds the target even though the lowerer cannot yet lower JumpRel.
    let succ = decode_block_successors(0x4_0000, &[0xEB, 0x0E], 64);
    assert_eq!(succ, vec![0x4_0000 + 2 + 0x0E]);
    // A straight-line run with no terminator falls through to the next PC.
    let mut prog = Vec::new();
    prog.extend_from_slice(MOV_RAX_RCX);
    prog.extend_from_slice(ADD_RAX_IMM8);
    let fall = decode_block_successors(0x4_0000, &prog, 64);
    assert_eq!(fall, vec![0x4_0000 + prog.len() as u64]);
}

#[test]
fn static_successors_extracts_relative_targets() {
    use prisma_ir::{CallRel, JumpRel, Op, Return};
    assert_eq!(
        static_successors(&Op::JumpRel(JumpRel {
            target_guest_pc: 0x1234
        })),
        vec![0x1234]
    );
    // A call's successors are the callee and the return site.
    assert_eq!(
        static_successors(&Op::CallRel(CallRel {
            target_guest_pc: 0x2000,
            return_guest_pc: 0x1005,
        })),
        vec![0x2000, 0x1005]
    );
    // A return is a dynamic transfer: no static successor.
    assert!(static_successors(&Op::Return(Return)).is_empty());
}

#[test]
fn straight_line_block_successor_is_the_fall_through_pc() {
    // Two non-terminator instructions, no control transfer: the single
    // successor is the PC just past the block.
    let mut prog = Vec::new();
    prog.extend_from_slice(MOV_RAX_RCX);
    prog.extend_from_slice(ADD_RAX_IMM8);
    let mut t = Translator::new();
    let block = t.translate_block(0x4_0000, &prog, 64).expect("translate");
    assert!(!block.ended_at_terminator);
    assert_eq!(block.successors, vec![0x4_0000 + prog.len() as u64]);
}

#[test]
fn translate_emits_code_and_reports_guest_size() {
    let mut t = Translator::new();
    let out = t.translate(0x1000, MOV_RAX_RCX).unwrap();
    assert!(!out.from_cache);
    assert_eq!(out.guest_bytes, 3);
    // mov rax, rcx lowers to a load + store; the optimizer must not delete
    // the architectural StoreReg, so the code is non-empty.
    assert!(!out.code.is_empty());
    assert_eq!(out.code.len() % 4, 0, "ARM64 instructions are 4 bytes");
}

#[test]
fn second_translation_is_a_cache_hit_with_identical_code() {
    let mut t = Translator::new();
    let first = t.translate(0x2000, ADD_RAX_IMM8).unwrap();
    assert!(!first.from_cache);
    assert_eq!(t.cached_count(), 1);

    let second = t.translate(0x2000, ADD_RAX_IMM8).unwrap();
    assert!(second.from_cache);
    assert_eq!(second.code, first.code);
    assert_eq!(t.cached_count(), 1);
}

#[test]
fn distinct_addresses_cache_separately() {
    let mut t = Translator::new();
    let _ = t.translate(0x3000, MOV_RAX_RCX).unwrap();
    let _ = t.translate(0x4000, MOV_RAX_RCX).unwrap();
    assert_eq!(t.cached_count(), 2);
}

#[test]
fn running_the_pipeline_is_deterministic() {
    let mut a = Translator::new();
    let mut b = Translator::new();
    assert_eq!(
        a.translate(0x5000, ADD_RAX_IMM8).unwrap().code,
        b.translate(0x5000, ADD_RAX_IMM8).unwrap().code
    );
}

#[test]
fn undecodable_bytes_report_a_decode_error() {
    let mut t = Translator::new();
    // Empty input cannot be decoded.
    assert!(matches!(
        t.translate(0x6000, &[]),
        Err(TranslateError::Decode(_))
    ));
}

#[test]
fn translate_block_stops_at_terminator() {
    // mov rax, rcx ; add rax, 0x10 ; ret
    let mut prog = Vec::new();
    prog.extend_from_slice(MOV_RAX_RCX);
    prog.extend_from_slice(ADD_RAX_IMM8);
    prog.push(0xC3); // ret

    let mut t = Translator::new();
    let block = t.translate_block(0x7000, &prog, 64).unwrap();
    assert!(block.ended_at_terminator);
    assert_eq!(block.instruction_count, 3);
    assert_eq!(block.guest_bytes, prog.len());
    // Each guest instruction cached independently.
    assert_eq!(t.cached_count(), 3);
    // Concatenated code is the sum of the per-instruction translations.
    assert!(!block.code.is_empty());
    assert_eq!(block.code.len() % 4, 0);
}

#[test]
fn translate_block_honours_instruction_cap() {
    // Three movs, no terminator; cap at 2 instructions.
    let mut prog = Vec::new();
    for _ in 0..3 {
        prog.extend_from_slice(MOV_RAX_RCX);
    }
    let mut t = Translator::new();
    let block = t.translate_block(0x8000, &prog, 2).unwrap();
    assert!(!block.ended_at_terminator);
    assert_eq!(block.instruction_count, 2);
    assert_eq!(block.guest_bytes, MOV_RAX_RCX.len() * 2);
}

#[test]
fn single_instruction_block_moves_the_exact_translation() {
    let bytes = [0x48, 0x83, 0xec, 0x28]; // sub rsp, 0x28
    let guest_addr = 0x1_4008_99e6;
    let mut single = Translator::new();
    let expected = single.translate(guest_addr, &bytes).unwrap();

    let mut blocked = Translator::new();
    let block = blocked.translate_block(guest_addr, &bytes, 1).unwrap();

    assert_eq!(block.code, expected.code);
    assert_eq!(block.instruction_count, 1);
    assert_eq!(block.guest_bytes, bytes.len());
    assert!(!block.ended_at_terminator);
    assert_eq!(block.successors, vec![guest_addr + bytes.len() as u64]);
    assert_eq!(blocked.cached_count(), 1);
}

#[test]
fn dispatch_instruction_reports_termination_without_cfg_allocation() {
    let mut translator = Translator::for_dispatch();
    assert_eq!(translator.pipeline.size(), 0);
    let (arithmetic, arithmetic_terminates) = translator
        .translate_dispatch_instruction(0x1_4008_99e6, &[0x48, 0x83, 0xec, 0x28])
        .unwrap();
    let optimized_arithmetic = Translator::new()
        .translate(0x1_4008_99e6, &[0x48, 0x83, 0xec, 0x28])
        .unwrap();
    assert_eq!(arithmetic.guest_bytes, 4);
    assert!(!arithmetic.code.is_empty());
    assert_eq!(arithmetic.code, optimized_arithmetic.code);
    assert!(!arithmetic_terminates);

    let (jump, jump_terminates) = translator
        .translate_dispatch_instruction(0x1_4008_ccc0, &[0xe9, 0xfb, 0xcc, 0xff, 0xff])
        .unwrap();
    let optimized_jump = Translator::new()
        .translate(0x1_4008_ccc0, &[0xe9, 0xfb, 0xcc, 0xff, 0xff])
        .unwrap();
    assert_eq!(jump.guest_bytes, 5);
    assert_eq!(jump.code, optimized_jump.code);
    assert!(jump_terminates);
    assert_eq!(translator.cached_count(), 0);
    assert_eq!(translator.stats().total(), 0);
}

#[test]
fn encode_words_materializes_exact_little_endian_buffer() {
    let encoded = encode_words(&[0x1122_3344, 0xaabb_ccdd]);

    assert_eq!(encoded, [0x44, 0x33, 0x22, 0x11, 0xdd, 0xcc, 0xbb, 0xaa]);
}

#[test]
fn translate_block_runs_to_end_without_terminator() {
    let mut prog = Vec::new();
    prog.extend_from_slice(MOV_RAX_RCX);
    prog.extend_from_slice(ADD_RAX_IMM8);

    let mut t = Translator::new();
    let block = t.translate_block(0x9000, &prog, 64).unwrap();
    assert!(!block.ended_at_terminator);
    assert_eq!(block.instruction_count, 2);
    assert_eq!(block.guest_bytes, prog.len());
}

#[test]
fn translate_block_first_instruction_error_propagates() {
    let mut t = Translator::new();
    // A lone REX.W prefix has no opcode byte: a truncated first instruction.
    assert!(matches!(
        t.translate_block(0xA000, &[0x48], 64),
        Err(TranslateError::Decode(_))
    ));
}

#[test]
fn translate_block_on_empty_input_is_an_empty_block() {
    let mut t = Translator::new();
    let block = t.translate_block(0xB000, &[], 64).unwrap();
    assert_eq!(block.instruction_count, 0);
    assert_eq!(block.guest_bytes, 0);
    assert!(block.code.is_empty());
    assert!(!block.ended_at_terminator);
}

#[test]
fn stats_track_hits_and_misses() {
    let mut t = Translator::new();
    assert_eq!(t.stats(), TranslatorStats::default());

    t.translate(0xC000, MOV_RAX_RCX).unwrap(); // miss
    t.translate(0xC000, MOV_RAX_RCX).unwrap(); // hit
    t.translate(0xC008, ADD_RAX_IMM8).unwrap(); // miss

    let s = t.stats();
    assert_eq!(s.cache_hits, 1);
    assert_eq!(s.cache_misses, 2);
    assert_eq!(s.total(), 3);

    t.reset_stats();
    assert_eq!(t.stats(), TranslatorStats::default());
}

#[test]
fn set_cache_limits_evicts_to_the_entry_budget() {
    let mut t = Translator::new();
    t.set_cache_limits(1, 0); // at most one entry
    t.translate(0xD000, MOV_RAX_RCX).unwrap();
    t.translate(0xD008, MOV_RAX_RCX).unwrap();
    t.translate(0xD010, MOV_RAX_RCX).unwrap();
    assert_eq!(t.cached_count(), 1, "LRU should hold the budget at 1 entry");
}

#[test]
fn invalidate_evicts_one_address_and_forces_a_re_translation() {
    let mut t = Translator::new();
    t.translate(0xE000, MOV_RAX_RCX).unwrap();
    t.translate(0xE008, ADD_RAX_IMM8).unwrap();
    assert_eq!(t.cached_count(), 2);

    t.invalidate(0xE000);
    assert_eq!(t.cached_count(), 1, "only the rewritten address is dropped");

    // Re-translating the invalidated address is a fresh miss, not a hit.
    let again = t.translate(0xE000, MOV_RAX_RCX).unwrap();
    assert!(!again.from_cache);
}

#[test]
fn clear_cache_drops_every_translation() {
    let mut t = Translator::new();
    t.translate(0xF000, MOV_RAX_RCX).unwrap();
    t.translate(0xF008, ADD_RAX_IMM8).unwrap();
    assert_eq!(t.cached_count(), 2);
    t.clear_cache();
    assert_eq!(t.cached_count(), 0);
}

#[test]
fn fused_block_stops_at_terminator() {
    // mov rax, rcx ; add rax, 0x10 ; ret
    let mut prog = Vec::new();
    prog.extend_from_slice(MOV_RAX_RCX);
    prog.extend_from_slice(ADD_RAX_IMM8);
    prog.push(0xC3);

    let mut t = Translator::new();
    let block = t.translate_fused_block(0x1_0000, &prog, 64).unwrap();
    assert!(block.ended_at_terminator);
    assert_eq!(block.instruction_count, 3);
    assert_eq!(block.guest_bytes, prog.len());
    assert!(!block.code.is_empty());
    assert_eq!(block.code.len() % 4, 0);
}

#[test]
fn translates_oh_my_posh_pshufd_broadcast_to_arm64() {
    let mut translator = Translator::new();
    let out = translator
        .translate(0x1_4001_6d32, b"\x66\x0F\x70\xC0\x00")
        .expect("translate PSHUFD xmm0, xmm0, 0");
    assert_eq!(out.guest_bytes, 5);
    assert!(!out.code.is_empty());
    assert_eq!(out.code.len() % 4, 0);
}

#[test]
fn rep_string_successors_include_bounded_reentry_and_completion() {
    let pc = 0x1_4001_5c3e;
    assert_eq!(
        decode_block_successors(pc, b"\xF3\x48\xA5", 64),
        vec![pc, pc + 3]
    );
}

#[test]
fn fused_windows_entry_stops_at_first_relative_call_after_prefixed_nop() {
    let bytes = [
        0x66, 0x90, // two-byte NOP
        0x56, // push rsi
        0x57, // push rdi
        0x53, // push rbx
        0x48, 0x83, 0xec, 0x20, // sub rsp, 0x20
        0xb9, 0x01, 0x00, 0x00, 0x00, // mov ecx, 1
        0xe8, 0xfd, 0x01, 0x00, 0x00, // call rel32
        0xe8, 0x08, 0x02, 0x00, 0x00, // unreachable next call
    ];
    let mut translator = Translator::new();
    let block = translator
        .translate_fused_block(0x1_4000_13a0, &bytes, 64)
        .expect("translate Windows entry block");
    assert!(block.ended_at_terminator);
    assert_eq!(block.instruction_count, 7);
    assert_eq!(block.guest_bytes, 19);
}

#[test]
fn fused_single_instruction_preserves_the_simple_body_after_its_pc_marker() {
    // With one instruction the renumber base is 0 (identity shift). The fused
    // lowering therefore adds only its exact-PC marker before the plain body.
    let mut a = Translator::new();
    let mut b = Translator::new();
    let fused = a.translate_fused_block(0x2_0000, ADD_RAX_IMM8, 64).unwrap();
    let simple = b.translate(0x2_0000, ADD_RAX_IMM8).unwrap();
    assert!(fused.code.ends_with(&simple.code));
    assert!(fused.code.len() > simple.code.len());
    assert_eq!(fused.instruction_count, 1);
}

#[test]
fn fused_block_pc_marker_overhead_is_bounded() {
    // mov rax, rcx ; mov rax, rcx — fusing exposes the redundant first
    // store / repeated load to the optimizer, so the single fused region
    // remains bounded by the two independently lowered instructions plus one
    // worst-case five-word GuestPc materialization per fused instruction.
    let mut prog = Vec::new();
    prog.extend_from_slice(MOV_RAX_RCX);
    prog.extend_from_slice(MOV_RAX_RCX);

    let mut t1 = Translator::new();
    let fused = t1.translate_fused_block(0x3_0000, &prog, 64).unwrap();
    let mut t2 = Translator::new();
    let separate = t2.translate_block(0x3_0000, &prog, 64).unwrap();

    assert_eq!(fused.instruction_count, 2);
    assert!(
        fused.code.len() <= separate.code.len() + fused.instruction_count * 20,
        "fused {} bytes exceeds bounded marker overhead over separate {} bytes",
        fused.code.len(),
        separate.code.len()
    );
}

#[test]
fn fused_block_is_deterministic() {
    let mut prog = Vec::new();
    prog.extend_from_slice(MOV_RAX_RCX);
    prog.extend_from_slice(ADD_RAX_IMM8);
    let mut a = Translator::new();
    let mut b = Translator::new();
    assert_eq!(
        a.translate_fused_block(0x4_0000, &prog, 64).unwrap(),
        b.translate_fused_block(0x4_0000, &prog, 64).unwrap()
    );
}

#[test]
fn fused_block_stops_before_a_truncated_trailing_instruction() {
    // Proptest regression ccba251a: the final decoded instruction reports
    // consuming beyond this eight-byte buffer. The fused path must keep its
    // published byte count within the caller-provided slice.
    let bytes = [132u8, 6, 24, 101, 0, 15, 120, 53];
    let mut t = Translator::new();
    let block = t
        .optimize_fused_block(0, &bytes, 3)
        .expect("optimize the bounded prefix");
    assert!(block.guest_bytes <= bytes.len());
    assert!(block.instruction_count <= 3);
}

#[test]
fn fused_block_marks_each_exact_guest_instruction_boundary() {
    use prisma_ir::{Gpr, Op};

    let mut translator = Translator::new();
    let guest_pc = 0x1_4000_1000;
    let block = translator
        .optimize_fused_block(
            guest_pc,
            &[
                0x48, 0x89, 0xd8, // mov rax, rbx
                0x48, 0x83, 0xc0, 0x01, // add rax, 1
            ],
            2,
        )
        .expect("optimize fused block");
    let markers: Vec<_> = block.func.blocks[0]
        .stmts
        .iter()
        .filter_map(|stmt| match &stmt.op {
            Op::GuestPc(marker) => Some(marker.pc),
            _ => None,
        })
        .collect();

    assert_eq!(markers, vec![guest_pc, guest_pc + 3]);
    assert_eq!(block.instruction_count, 2);

    let stmts = &block.func.blocks[0].stmts;
    let second_marker = stmts
        .iter()
        .position(|stmt| matches!(&stmt.op, Op::GuestPc(marker) if marker.pc == guest_pc + 3))
        .expect("second instruction marker");
    assert!(
        stmts[..second_marker]
            .iter()
            .any(|stmt| matches!(&stmt.op, Op::StoreReg(store) if store.reg == Gpr::Rax)),
        "first instruction flushes RAX before the next exception boundary"
    );
    assert!(
        stmts[second_marker + 1..]
            .iter()
            .any(|stmt| matches!(&stmt.op, Op::LoadReg(load) if load.reg == Gpr::Rax)),
        "second instruction reloads RAX after the exception boundary"
    );
}

#[test]
fn truncated_trailing_instruction_does_not_slice_out_of_bounds() {
    // Regression: the proptest minimal failing input — a decode that
    // reports consuming past the buffer. Before the bounds check this
    // panicked with "range end index out of range"; now it must stop
    // cleanly (terminate the block) or return a typed error, never panic.
    let bytes = [106u8, 0, 144, 15, 255, 45];
    let mut t = Translator::new();
    // PUSH/NOP decode fine first, so the block ends gracefully rather than
    // erroring; the contract under test is simply "does not panic".
    let _ = t.translate_block(0, &bytes, 3);
}

#[test]
fn a_lone_truncated_instruction_errors_instead_of_panicking() {
    // A single instruction whose operand runs off the end: with no prior
    // instruction to fall back on, it surfaces as a typed Truncated error.
    // 0x0F 0xFF ... is enough to make the decoder want more than is present.
    let bytes = [0x0Fu8, 0xFF];
    let mut t = Translator::new();
    let result = t.translate_block(0, &bytes, 4);
    // Either a typed error or a clean (possibly empty) block — never a panic.
    if let Err(e) = result {
        assert!(matches!(
            e,
            TranslateError::Truncated { .. } | TranslateError::Decode(_)
        ));
    }
}

#[test]
fn fused_oh_my_posh_rip_relative_store_keeps_absolute_guest_address() {
    use prisma_ir::{Constant, Op};

    // Exact sequence at Oh My Posh v30.6.3 0x140020aa9. The store starts at
    // +0x10 and must address 0x140dff430, never its raw disp32 0x00dde970.
    let bytes = [
        0x48, 0x89, 0xc1, // mov rcx, rax
        0x48, 0xd3, 0xe6, // shl rsi, cl
        0x48, 0x83, 0xf8, 0x40, // cmp rax, 0x40
        0x48, 0x19, 0xd2, // sbb rdx, rdx
        0x48, 0x21, 0xd6, // and rsi, rdx
        0x48, 0x89, 0x35, 0x70, 0xe9, 0xdd, 0x00, // mov [rip+disp32], rsi
        0x48, 0x29, 0x05, 0x71, 0xe9, 0xdd, 0x00, // sub [rip+disp32], rax
    ];
    let mut translator = Translator::new();
    let block = translator
        .optimize_fused_block(0x1_4002_0AA9, &bytes, 64)
        .expect("optimize exact Oh My Posh block");
    let constants: Vec<u64> = block
        .func
        .blocks
        .iter()
        .flat_map(|block| block.stmts.iter())
        .filter_map(|stmt| match stmt.op {
            Op::Constant(Constant { value, .. }) => Some(value),
            _ => None,
        })
        .collect();
    assert!(
        constants.contains(&0x1_40DF_F430),
        "constants={constants:#x?}"
    );
    assert!(
        constants.contains(&0x1_40DF_F438),
        "constants={constants:#x?}"
    );
    assert!(
        !constants.contains(&0x00DD_E970),
        "constants={constants:#x?}"
    );
    assert!(
        !constants.contains(&0x00DD_E971),
        "constants={constants:#x?}"
    );
}

#[test]
fn fused_oh_my_posh_test_after_rip_load_has_one_memory_read() {
    use prisma_ir::{Constant, Op};

    // Exact Oh My Posh v30.6.3 block at 0x14005dfcc. TEST ECX,ECX is a
    // register-only operation; the only guest-memory access in this block is
    // the RIP-relative MOV from 0x140dff2e4.
    let bytes = [
        0x8b, 0x0d, 0x12, 0x13, 0xda, 0x00, // mov ecx,[rip+0xda1312]
        0x85, 0xc9, // test ecx,ecx
        0x0f, 0x87, 0x77, 0x01, 0x00, 0x00, // ja 0x14005e151
    ];
    let mut translator = Translator::new();
    let block = translator
        .optimize_fused_block(0x1_4005_DFCC, &bytes, 3)
        .expect("optimize exact Oh My Posh load/test/branch block");
    let stmts: Vec<_> = block
        .func
        .blocks
        .iter()
        .flat_map(|block| block.stmts.iter())
        .collect();
    println!("optimized Oh My Posh block: {stmts:#?}");
    let loads = stmts
        .iter()
        .filter(|stmt| matches!(stmt.op, Op::LoadMem(_)))
        .count();
    let constants: Vec<u64> = stmts
        .iter()
        .filter_map(|stmt| match stmt.op {
            Op::Constant(Constant { value, .. }) => Some(value),
            _ => None,
        })
        .collect();

    assert_eq!(loads, 1, "stmts={stmts:#?}");
    assert!(
        constants.contains(&0x1_40DF_F2E4),
        "constants={constants:#x?}"
    );
    assert!(
        !constants.contains(&0x00DA_1312),
        "constants={constants:#x?}"
    );

    let lowered = Translator::new()
        .translate_fused_block(0x1_4005_DFCC, &bytes, 3)
        .expect("lower exact Oh My Posh load/test/branch block");
    let words: Vec<u32> = lowered
        .code
        .as_chunks::<4>()
        .0
        .iter()
        .map(|word| u32::from_le_bytes(*word))
        .collect();
    println!("lowered Oh My Posh words: {words:08x?}");
}

#[test]
fn fused_oh_my_posh_direct_call_keeps_target_and_return_pc() {
    use prisma_ir::Op;

    // Exact Oh My Posh v30.6.3 sequence at 0x14005df46. The fused block must
    // stop at CALL and publish the callee plus the address immediately after it.
    let bytes = [
        0x55, // push rbp
        0x48, 0x89, 0xe5, // mov rbp,rsp
        0x48, 0x83, 0xec, 0x08, // sub rsp,8
        0x89, 0x44, 0x24, 0x18, // mov [rsp+0x18],eax
        0x90, 0x90, // nop; nop
        0x48, 0x8d, 0x05, 0x05, 0xd9, 0xd5, 0x00, // lea rax,[0x140dbb860]
        0x0f, 0x1f, 0x44, 0x00, 0x00, // nop [rax+rax]
        0xe8, 0x3b, 0x21, 0xfc, 0xff, // call 0x1400200a0
    ];
    let mut translator = Translator::new();
    let block = translator
        .optimize_fused_block(0x1_4005_DF46, &bytes, 32)
        .expect("optimize exact Oh My Posh direct-call block");
    let calls: Vec<_> = block
        .func
        .blocks
        .iter()
        .flat_map(|block| block.stmts.iter())
        .filter_map(|stmt| match &stmt.op {
            Op::CallRel(call) => Some(call),
            _ => None,
        })
        .collect();

    assert!(block.ended_at_terminator);
    assert_eq!(block.guest_bytes, bytes.len());
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].target_guest_pc, 0x1_4002_00A0);
    assert_eq!(calls[0].return_guest_pc, 0x1_4005_DF65);

    let lowered = Translator::new()
        .translate_fused_block(0x1_4005_DF46, &bytes, 32)
        .expect("lower exact Oh My Posh direct-call block");
    let words: Vec<u32> = lowered
        .code
        .as_chunks::<4>()
        .0
        .iter()
        .map(|word| u32::from_le_bytes(*word))
        .collect();
    println!("Oh My Posh direct-call ARM64: {words:08x?}");
}

#[test]
fn fused_oh_my_posh_procresize_uses_signed_greater_equal_exit() {
    use prisma_ir::{CondCode, Op};

    // Exact Oh My Posh v30.6.3 block at 0x14005d3e5:
    // mov edx,[rsp+0x44]; cmp eax,edx; jge 0x14005d405.
    // During the one-P bootstrap, eax (nprocs) equals edx (old), so the
    // signed-greater-or-equal branch must skip the P-destruction loop.
    let bytes = [
        0x8b, 0x54, 0x24, 0x44, // mov edx,[rsp+0x44]
        0x39, 0xd0, // cmp eax,edx
        0x7d, 0x18, // jge 0x14005d405
    ];
    let mut translator = Translator::new();
    let optimized = translator
        .optimize_fused_block(0x1_4005_D3E5, &bytes, 3)
        .expect("optimize exact procresize loop guard");
    let branch = optimized
        .func
        .blocks
        .iter()
        .flat_map(|block| block.stmts.iter())
        .find_map(|stmt| match &stmt.op {
            Op::CondJumpRel(branch) => Some(branch),
            _ => None,
        })
        .expect("procresize guard must remain a conditional branch");
    assert_eq!(branch.cc, CondCode::Sge);
    assert_eq!(branch.target_guest_pc, 0x1_4005_D405);
    assert_eq!(branch.fallthrough_guest_pc, 0x1_4005_D3ED);

    let lowered = Translator::new()
        .translate_fused_block(0x1_4005_D3E5, &bytes, 3)
        .expect("lower exact procresize loop guard");
    let words: Vec<u32> = lowered
        .code
        .as_chunks::<4>()
        .0
        .iter()
        .map(|word| u32::from_le_bytes(*word))
        .collect();
    assert!(
        words
            .iter()
            .any(|word| word & 0xffe0_0c00 == 0x9a80_0000 && (word >> 12) & 0xf == 0xa),
        "lowered guard must contain CSEL.GE: {words:08x?}",
    );
}
