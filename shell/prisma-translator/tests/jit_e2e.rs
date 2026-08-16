use prisma_backend::lowerer::Lowerer;
use prisma_ir::{BasicBlock, Constant, Function, Op, OpSize, Stmt, StoreMem, StoreMemTSO};
use prisma_orchestrator::tso_classifier::{MemoryCategory, TsoClassifier};
use prisma_translator::Translator;

#[test]
fn test_basic_translation() {
    let mut t = Translator::new();
    let prog = [0xB8, 0x01, 0x00, 0x00, 0x00, 0x83, 0xC0, 0x01];
    let block = t
        .translate_block(0x4000, &prog, 64)
        .expect("translation must succeed");
    assert!(
        !block.code.is_empty(),
        "Expected ARM64 code to be generated"
    );
    assert_eq!(block.code.len() % 4, 0, "ARM64 instructions are 4 bytes");
    assert_eq!(
        block.instruction_count, 2,
        "Should translate 2 instructions"
    );
}

#[test]
fn test_go_asmcgocall_stack_restore_return_translation() {
    let mut translator = Translator::new();
    let bytes = [
        0x48, 0x8b, 0x0d, 0xc6, 0x40, 0xd7, 0x00, // mov rcx,[rip+0xd740c6]
        0x65, 0x48, 0x8b, 0x09, // mov rcx,gs:[rcx]
        0x48, 0x8b, 0x7c, 0x24, 0x08, // mov rdi,[rsp+8]
        0x48, 0x8b, 0x77, 0x08, // mov rsi,[rdi+8]
        0x48, 0x2b, 0x34, 0x24, // sub rsi,[rsp]
        0x48, 0x89, 0x39, // mov [rcx],rdi
        0x48, 0x89, 0xf4, // mov rsp,rsi
        0x89, 0x44, 0x24, 0x20, // mov [rsp+0x20],eax
        0x5d, // pop rbp
        0xc3, // ret
    ];

    let optimized = translator
        .optimize_fused_block(0x1_4008_b583, &bytes, 64)
        .expect("Go asmcgocall epilogue must optimize");
    let block = translator
        .translate_fused_block(0x1_4008_b583, &bytes, 64)
        .expect("Go asmcgocall epilogue must lower");

    assert!(matches!(
        optimized.func.blocks[0].stmts.last().map(|stmt| &stmt.op),
        Some(Op::Return(_))
    ));
    assert_eq!(block.instruction_count, 10);
    assert_eq!(block.guest_bytes, bytes.len());
    assert!(block.ended_at_terminator);
}

#[test]
fn test_go_asmcgocall_selects_m_g0_through_exact_displacements() {
    let mut translator = Translator::new();
    let bytes = [
        0x4c, 0x8b, 0x47, 0x30, // mov r8,[rdi+0x30]
        0x49, 0x8b, 0x70, 0x48, // mov rsi,[r8+0x48]
        0x48, 0x39, 0xf7, // cmp rdi,rsi
        0x74, 0x62, // je +0x62
    ];
    let optimized = translator
        .optimize_fused_block(0x1_4008_b538, &bytes, 16)
        .expect("Go g0 selection must optimize");
    let block = translator
        .translate_fused_block(0x1_4008_b538, &bytes, 16)
        .expect("Go g0 selection must lower");

    assert!(optimized.func.blocks[0]
        .stmts
        .iter()
        .any(|stmt| { matches!(&stmt.op, Op::Constant(value) if value.value == 0x30) }));
    assert!(optimized.func.blocks[0]
        .stmts
        .iter()
        .any(|stmt| { matches!(&stmt.op, Op::Constant(value) if value.value == 0x48) }));
    assert_eq!(block.instruction_count, 4);
    assert_eq!(block.guest_bytes, bytes.len());
    assert!(block.ended_at_terminator);
}

#[test]
fn test_tso_lowering() {
    let classifier = TsoClassifier::new();
    let cat_single = classifier.classify(0x7FFF_0000, 4);
    assert_eq!(cat_single, MemoryCategory::SingleThreaded);
    let cat_unknown = classifier.classify(0x1000_0000, 4);
    assert_eq!(cat_unknown, MemoryCategory::Unknown);

    let build_ir = |cat: MemoryCategory| -> Function {
        let store_op = match cat {
            MemoryCategory::SingleThreaded => Op::StoreMem(StoreMem {
                addr: 0,
                value: 1,
                size: OpSize::I32,
            }),
            _ => Op::StoreMemTSO(StoreMemTSO {
                addr: 0,
                value: 1,
                size: OpSize::I32,
            }),
        };
        Function {
            entry: 0,
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![
                    Stmt::new(
                        Some(0),
                        Op::Constant(Constant {
                            value: 0x1000,
                            size: OpSize::I64,
                        }),
                    ),
                    Stmt::new(
                        Some(1),
                        Op::Constant(Constant {
                            value: 42,
                            size: OpSize::I32,
                        }),
                    ),
                    Stmt::new(None, store_op),
                ],
            }],
        }
    };

    let ir_single = build_ir(cat_single);
    let code_single = Lowerer::new().lower_function(&ir_single).unwrap();
    let ir_unknown = build_ir(cat_unknown);
    let code_unknown = Lowerer::new().lower_function(&ir_unknown).unwrap();

    let dmb_ish = 0xD5033BBF_u32;
    let has_barrier_single = code_single.contains(&dmb_ish);
    let has_barrier_unknown = code_unknown.contains(&dmb_ish);

    assert!(
        !has_barrier_single,
        "SingleThreaded memory should not emit barriers"
    );
    assert!(
        has_barrier_unknown,
        "Unknown/Shared memory should emit barriers for TSO"
    );
}
