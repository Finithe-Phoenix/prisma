use prisma_translator::Translator;
use prisma_orchestrator::tso_classifier::{TsoClassifier, MemoryCategory};
use prisma_ir::{Function, BasicBlock, Stmt, Op, StoreMem, StoreMemTSO, Constant, OpSize};
use prisma_backend::lowerer::Lowerer;

#[test]
fn test_basic_translation() {
    let mut t = Translator::new();
    let prog = [0xB8, 0x01, 0x00, 0x00, 0x00, 0x83, 0xC0, 0x01];
    let block = t.translate_block(0x4000, &prog, 64).expect("translation must succeed");
    assert!(!block.code.is_empty(), "Expected ARM64 code to be generated");
    assert_eq!(block.code.len() % 4, 0, "ARM64 instructions are 4 bytes");
    assert_eq!(block.instruction_count, 2, "Should translate 2 instructions");
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
                    Stmt::new(Some(0), Op::Constant(Constant { value: 0x1000, size: OpSize::I64 })),
                    Stmt::new(Some(1), Op::Constant(Constant { value: 42, size: OpSize::I32 })),
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
    
    assert!(!has_barrier_single, "SingleThreaded memory should not emit barriers");
    assert!(has_barrier_unknown, "Unknown/Shared memory should emit barriers for TSO");
}
