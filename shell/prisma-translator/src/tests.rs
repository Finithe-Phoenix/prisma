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
    fn fused_single_instruction_matches_the_simple_path() {
        // With one instruction the renumber base is 0 (identity shift), so the
        // fused lowering must byte-match the plain single-instruction path.
        let mut a = Translator::new();
        let mut b = Translator::new();
        let fused = a.translate_fused_block(0x2_0000, ADD_RAX_IMM8, 64).unwrap();
        let simple = b.translate(0x2_0000, ADD_RAX_IMM8).unwrap();
        assert_eq!(fused.code, simple.code);
        assert_eq!(fused.instruction_count, 1);
    }

    #[test]
    fn fused_block_is_never_larger_than_separate_translation() {
        // mov rax, rcx ; mov rax, rcx — fusing exposes the redundant first
        // store / repeated load to the optimizer, so the single fused region
        // can only be <= the two independently lowered instructions.
        let mut prog = Vec::new();
        prog.extend_from_slice(MOV_RAX_RCX);
        prog.extend_from_slice(MOV_RAX_RCX);

        let mut t1 = Translator::new();
        let fused = t1.translate_fused_block(0x3_0000, &prog, 64).unwrap();
        let mut t2 = Translator::new();
        let separate = t2.translate_block(0x3_0000, &prog, 64).unwrap();

        assert_eq!(fused.instruction_count, 2);
        assert!(
            fused.code.len() <= separate.code.len(),
            "fused {} bytes should not exceed separate {} bytes",
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
