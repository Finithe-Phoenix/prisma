# HANDOFF: prisma-passes (Fase 2)

> Para Claude. Cómo migrar los 16 optimization passes a Rust.

## Dependencias

- `prisma-ir` (para tipos IR)
- `proptest` (para property-based tests)

## Archivos C++ a migrar

| C++ | Rust | Líneas | Prioridad |
|-----|------|--------|-----------|
| `pass_manager.cpp` (131) | `pipeline.rs` | 131 | 1 |
| `const_prop.cpp` (223) | `const_prop.rs` | 223 | 2 |
| `algebraic.cpp` (149) | `algebraic.rs` | 149 | 3 |
| `strength_reduce.cpp` (91) | `strength_reduce.rs` | 91 | 4 |
| `const_prop_2.cpp` (inline) | `const_prop.rs` (2nd func) | ~80 | 5 |
| `redundant_load.cpp` (71) | `redundant_load.rs` | 71 | 6 |
| `cse.cpp` (90) | `cse.rs` | 90 | 7 |
| `global_cse.cpp` (122) | `global_cse.rs` | 122 | 8 |
| `copy_prop.cpp` (110) | `copy_prop.rs` | 110 | 9 |
| `dead_store.cpp` (84) | `dead_store.rs` | 84 | 10 |
| `branch_fold.cpp` (132) | `branch_fold.rs` | 132 | 11 |
| `dce.cpp` (260) | `dce.rs` | 260 | 12 |
| `flag_write_elim.cpp` (76) | `flag_write_elim.rs` | 76 | 13 |
| `tail_call.cpp` (44) | `tail_call.rs` | 44 | 14 |
| `x87_stack.cpp` (146) | `x87_stack.rs` | 146 | 15 |
| `licm.cpp` (170) | `licm.rs` | 170 | 16 |
| `peephole.cpp` (293) | `peephole.rs` | 293 | 17 |

## Pipeline trait

```rust
pub trait Pass {
    fn name(&self) -> &'static str;
    fn run(&self, func: Function) -> Function;
}

pub struct PassPipeline {
    passes: Vec<Box<dyn Pass>>,
}

impl PassPipeline {
    pub fn new() -> Self;
    pub fn add(&mut self, pass: Box<dyn Pass>);
    pub fn run(&self, func: Function) -> (Function, PipelineStats);
}

pub fn default_pipeline() -> PassPipeline {
    let mut p = PassPipeline::new();
    p.add(Box::new(const_prop::ConstProp));
    p.add(Box::new(algebraic::Algebraic));
    p.add(Box::new(strength_reduce::StrengthReduce));
    p.add(Box::new(const_prop::ConstProp));      // second pass
    p.add(Box::new(redundant_load::RedundantLoad));
    p.add(Box::new(cse::Cse));
    p.add(Box::new(copy_prop::CopyProp));
    p.add(Box::new(dead_store::DeadStore));
    p.add(Box::new(branch_fold::BranchFold));
    p.add(Box::new(dce::Dce));
    p
}
```

## Property-based tests

Cada pass debe pasar:

1. **Idempotence:** `run(run(program)) == run(program)`
2. **No-growth:** `run(program).stmts.len() <= program.stmts.len()`
3. **No side effects:** programa con solo constantes queda igual

```rust
#[test]
fn const_prop_is_idempotent() {
    let mut rng = Rng::seed_from_u64(0x1234);
    for _ in 0..100 {
        let program = random_program(&mut rng, 50);
        let pass = const_prop::ConstProp;
        let once = pass.run(program.clone());
        let twice = pass.run(once.clone());
        assert_eq!(once, twice, "const_prop failed idempotence");
    }
}
```

## Differential testing

```rust
#[test]
fn differential_default_pipeline() {
    let program = random_program(&mut rng, 100);
    // Run through Rust pipeline
    let rust_out = default_pipeline().run(program.clone()).0;
    // Run through C++ pipeline (via FFI)
    let cpp_out = unsafe { cpp_run_pipeline(&program) };
    // Compare structural equality
    assert_eq!(rust_out, cpp_out);
}
```

## Pass timing hooks

```rust
pub struct PipelineStats {
    pub pass_times: Vec<(String, std::time::Duration)>,
    pub total_time: std::time::Duration,
}
```

## Checklist

- [x] `Pass` trait definido con `name()` y `run()`
- [x] `PassPipeline` con `run()`, `size()`, `pass_names()`, `default_pipeline()`
- [x] **16/16 pases implementados** en Rust con paridad C++. Los 13 del
      default_pipeline + global_cse, loop_invariant_motion (function-level
      pipeline) + tail_call_optimise (standalone). Nuevo módulo `cfg.rs`
      (successors/postorder/dominators CHK/natural_loops) que vive en
      prisma-passes para no tocar el prisma-ir compartido con codex.
- [x] Property tests: idempotencia + no-crecimiento por pass (los 13) +
      end-to-end del default_pipeline (orden + fold + cross-block DCE).
- [ ] Tests differentiales contra C++ (siguiente: FFI comparator pipeline→pipeline,
      requiere superficie C API nueva — territorio capi compartido con codex)
- [x] Pass timing hooks — `PassPipeline::run_with_stats` -> `PipelineStats`
      (per-pass + total Duration), sin romper la firma de `run()`
- [x] `cargo +1.95.0-x86_64-pc-windows-msvc test -p prisma-passes` verde (40 tests)
- [x] `cargo +1.95.0-x86_64-pc-windows-msvc clippy -p prisma-passes --all-targets -- -D warnings` limpio

## SPARK 2026-06-12 (claude): default optimization pipeline completo en Rust

`default_pipeline()` ahora ejecuta los 10 pases en el orden del C++ pass manager:

```
constant_propagate -> algebraic_simplify -> strength_reduce -> constant_propagate
-> redundant_load_eliminate -> common_subexpression_eliminate -> copy_prop
-> dead_store_eliminate -> branch_fold -> dead_code_eliminate
```

Pases portados esta sesión (eran stubs `// TODO`), cada uno espejando exactamente
la semántica del `.cpp` homónimo en `core/src/passes/`:

- `algebraic.rs` — identidades x*0, 0&x, x|-1, x-x/x^x→0, x%1, UMulHi*1; folds
  de dos constantes se dejan a const_prop (contratos disjuntos).
- `strength_reduce.rs` — `Mul x,(1<<k)` → `Shl x,k`, minteando un Constant para
  el shift count con allocador de ref global a la función.
- `redundant_load.rs` — segundo `LoadMem (addr,size)` → copia `Or %v,%v`; store/
  fence flushea; LoadMemTSO intacto.
- `cse.rs` — dedupe de `(op,lhs,rhs,size)` → copia; ops flag/side-effect flushean.
- `dead_store.rs` — store sobrescrito sin observación intermedia se descarta;
  StoreMemTSO nunca se mata.
- `branch_fold.rs` — `CmpFlags const,const` + `CondJumpRel` → `JumpRel`; ccs
  flag-direct (Cc/Nc/Ov/...) no se resuelven (unwrap_or(false) como el C++).
- `dce.rs` — barrido de liveness reverso; `is_pure_for_dce` y
  `collect_operand_refs` espejan el allowlist y el visitor del C++ exactamente
  (incluido WriteFlagsCountZero impuro con operandos src+result).

Pases adicionales para llegar a paridad 13/13 (orden exacto del C++ pass manager):

- `peephole.rs` — motor de reglas a punto fijo (8 iters): xor x,x→0, or/and x,x→x,
  add 0, sub 0, mul 1, mul 0→0, identity Extend→Truncate. `try_const` busca el def
  hacia atrás en el bloque, igual que el C++.
- `x87_stack.rs` — forwarding de ST(i) con alias-chain + KnownStack[8]; X87Load de
  slot conocido → copia `Or src,src`; call/branch/trap limpian conocimiento.
- `flag_write_elim.rs` — descarta CmpFlags/AluFlags/WriteFlagsCountZero sin lector
  (CondJumpRel/Select pinean); Compare nunca se descarta.

Orden del default_pipeline (13): const_prop, algebraic, strength_reduce, peephole,
const_prop, redundant_load, cse, x87_stack, copy_prop, dead_store, branch_fold,
flag_write_elim, dce.

Diferencia estructural: la mayoría de pases operan por basic block (consts/last_load
/last_cmp se reinician por bloque) — conservador y sound. **DCE es la excepción**:
liveness es **global a la función** (barrido reverso sobre todos los bloques en
orden de programa, un único live-set), porque los refs SSA son function-scoped y un
valor definido en un bloque puede usarse en uno posterior.

Revisión 2026-06-12 (codex + gemini sobre el diff completo):
- Codex: 6 pases OK; **1 BLOCKER real** — DCE per-bloque rompía liveness cross-block.
  Corregido (DCE ahora global) + test de regresión `cross_block_use_keeps_def`.
- Gemini: marcó `WriteFlagsCountZero.result` como def — **falso positivo**. La
  lowering C++ (`lowering.cpp:342` `bump(op.src); bump(op.result)` y `1996-1997`
  `reg_of` de ambos) confirma que `result` es un OPERANDO leído, no el def del stmt.
  Mantener ambos refs vivos es correcto.

Pendiente: comparador diferencial FFI pipeline-vs-pipeline + pass timing hooks.

## SPARK 2026-06-12 (claude) cont.: crate prisma-passes COMPLETO (16/16)

Segundo commit (bb5c385): pases function-level + análisis CFG.

- `cfg.rs` — primer análisis de grafo de control para el IR Rust:
  `successors` (lee terminadores Jump/CondJump/CondJumpFlags; los terminadores
  guest-PC como JumpRel/CondJumpRel/Return/JumpReg no aportan sucesores
  intra-función), `postorder` iterativo, `dominators` (Cooper-Harvey-Kennedy
  con números de postorden), `natural_loops` (back-edges donde el header
  domina al tail + reverse-reachability del cuerpo). Vive en prisma-passes,
  NO en prisma-ir, para no colisionar con codex en el archivo IR compartido.
  Si codex/Gemini quieren estas primitivas en prisma-ir (territorio IR-CORE),
  coordinar el move; por ahora son privadas a passes.
- `global_cse.rs` / `licm.rs` — usan cfg::; paridad con los .cpp. Como anota
  el C++, son no-ops prácticos en funciones de un solo bloque (el translator
  actual emite single-block); el plumbing es el entregable.
- `tail_call.rs` — standalone (no en ningún pipeline default, igual que C++).
- `default_function_pipeline()` = global_cse -> licm (espejo del C++).

Todos los stubs `// TODO(Fase 2)` de prisma-passes están cerrados. 75 tests,
clippy --all-targets -D warnings limpio.
