# HANDOFF: prisma-backend (Fase 5)

> Para CODEX. Cómo construir el ARM64 assembler custom + IR lowering en Rust.
> Esta es la fase más compleja porque reemplaza vixl (Google C++ assembler library).

## Estrategia

No portar vixl a Rust. En su lugar, construir un **assembler minimal
custom** que solo emita las ~300 instrucciones ARM64 que Prisma usa.
Si el assembler custom toma más de 12 semanas, mantener el backend
C++ como fallback y congelar.

## Arquitectura

```
shell/prisma-backend/src/
  lib.rs               — Arm64Assembler + Lowerer re-exports
  assembler.rs         — Core assembler: buffer management + instruction encoding
  instructions.rs      — Instruction encoding functions (one per instruction family)
  registers.rs         — Register types (Wn, Xn, Vn, Bn, Hn, Sn, Dn, Qn)
  lowerer.rs           — IR op → ARM64 instruction selection
  abi.rs               — Calling convention helpers (callee-save regs, stack layout)
  fp.rs                — FP/SIMD instruction encoding helpers
  branch.rs            — Branch fixup + label management
```

## Instruction encoding approach

Cada instrucción ARM64 es un u32 (32 bits). La estrategia más simple
es construir el u32 con shifts + ORs:

```rust
pub fn add_x(sp: u8, imm12: u16) -> u32 {
    // ADD Xn, Xn, #imm12
    // Encoding: 1 0 0 1 0 0 0 1 0 0 | imm12 | Rn | Rd
    // sf=1, op=0, S=0, opcode=01011, sh=0
    debug_assert!(imm12 < 4096);
    (0x9100_0000u32)
        | (imm12 as u32) << 10
        | (sp as u32) << 5       // Rn
        | (sp as u32)            // Rd
}

pub fn ret() -> u32 {
    0xD65F_03C0  // RET
}

pub fn b(offset: i32) -> u32 {
    // B label — unconditional branch, PC-relative ±128 MiB
    debug_assert!(offset % 4 == 0, "branch offset must be 4-byte aligned");
    let imm26 = (offset >> 2) as u32 & 0x03FF_FFFF;
    0x1400_0000 | imm26
}
```

## Register types

```rust
#[repr(u8)]
pub enum Register {
    X0, X1, X2, X3, X4, X5, X6, X7,
    X8, X9, X10, X11, X12, X13, X14, X15,
    X16, X17, X18, X19, X20, X21, X22, X23,
    X24, X25, X26, X27, X28, X29, X30, Xzr,
}

pub enum VecRegister {
    V0, V1, V2, V3, V4, V5, V6, V7,
    V8, V9, V10, V11, V12, V13, V14, V15,
    V16, V17, V18, V19, V20, V21, V22, V23,
    V24, V25, V26, V27, V28, V29, V30, V31,
}
```

## Assembly buffer

```rust
pub struct Arm64Assembler {
    buffer: Vec<u32>,
    cursor: usize,
    labels: Vec<Option<usize>>,
    label_fixes: Vec<LabelFix>,  // patches to apply at finalize
}

impl Arm64Assembler {
    pub fn new() -> Self;
    pub fn emit(&mut self, instruction: u32);
    pub fn finalize(&mut self) -> &[u32];
    pub fn bytes(&self) -> &[u8];  // view as bytes for JIT execution
    pub fn cursor_offset(&self) -> usize;
}
```

## IR lowering (from prisma-ir → ARM64)

```rust
pub struct Lowerer<'a> {
    asm: &'a mut Arm64Assembler,
    reg_map: HashMap<Ref, Register>,  // SSA ref → host register
    scratch_pool: Vec<Register>,
}

impl Lowerer {
    pub fn lower_block(&mut self, stmts: &[Stmt]) -> Result<(), LowerError>;
    pub fn lower_stmt(&mut self, stmt: &Stmt) -> Result<(), LowerError>;
}

pub enum LowerError {
    OutOfScratchRegs,
    UnsupportedOp(String),
    Internal(String),
}
```

## Instruction coverage

### ALU (must-have)
`ADD, ADDS, SUB, SUBS, MUL, UMULH, SMULH, SDIV, UDIV, AND, ORR, EOR, BIC, ORN, EON, LSL, LSR, ASR, ROR, CLZ, CLS, RBIT, REV, REV16, REV32, NEG, CMP, CMN, TST, CSEL, CSINC, CSINV, CSNEG`

### SIMD/FP (must-have)
`ADD, SUB, MUL, FADD, FSUB, FMUL, FDIV, FMIN, FMAX, FABS, FNEG, FSQRT, FCVT, SCVTF, FCVTZS, FRINTN, FRINTM, FRINTP, FRINTZ, CMEQ, CMGT, CMGE, CMHI, CMHS, TBL, TBX, EXT, DUP, MOV, INS, SMOV, UMOV, SSHLL, USHLL, SXTL, UXTL, AND, ORR, EOR, BIC, BS, BSL, BIT, BIF`

### Advanced SIMD (nice-to-have)
`SHL, SSHR, USHR, SSRA, USRA, SRSHR, URSHR, SMAX, UMAX, SMIN, UMIN, SADDLP, UADDLP, SADALP, UADALP, SADDLV, UADDLV, ADDV, SMOV, UMOV, ZIP1, ZIP2, UZP1, UZP2, TRN1, TRN2`

### Memory (must-have)
`LDR, STR, LDP, STP, LDRB, STRB, LDRH, STRH, LDRSW, LDUR, STUR, LDURB, STURB, LDXP, STXP, LDAR, STLR, LDAXR, STLXR, CAS, CASAL, LDADDAL`

### Branches (must-have)
`B, BL, BR, BLR, RET, B.EQ, B.NE, B.LT, B.GE, B.LE, B.GT, B.HI, B.LS, B.MI, B.PL, B.VS, B.VC, CBZ, CBNZ, TBZ, TBNZ`

### Crypto (if FEAT_AES/SHA)
`AESE, AESD, AESMC, AESIMC, SHA256H, SHA256H2, SHA256SU0, SHA256SU1, SHA1C, SHA1M, SHA1P, SHA1SU0, SHA1SU1`

### Barriers (must-have)
`DMB, DSB, ISB`

## Tests

- Cada instrucción: assembly + check bytes
- Cada patrón de lowering: IR conocido → secuencia ARM64 esperada
- Differential: mismo lowering que C++ backend produce mismas instrucciones
- `proptest`: IR aleatorio → lowering no crash
- Ejecución real en ARM64: programa JITeado produce resultado correcto

## Checklist

- [~] `Arm64Assembler` con buffer management + `emit()` + `finalize()`
  - 2026-06-12: Rust backend now has cursor tracking plus exact encoders for
    `NOP`, `RET`, `B`, `B.cond`, `BR/BLR X`, `CBZ/CBNZ X`, branch labels/fixups,
    `MOV Xd,Xn`, `MOVZ/MOVK`, `ADD/SUB Xd,Xn,#imm12`,
    `ADD/SUB Xd,Xn,Xm`,
    `AND/ORR/EOR Xd,Xn,Xm`, `LSLV/LSRV/ASRV/RORV Xd,Xn,Xm`,
    `MUL/UMULH/SMULH/UDIV/SDIV/MSUB Xd,Xn,Xm,Xa`, unsigned-offset
    `LDR/STR` for
    `I8`/`I16`/`I32`/`I64`, `CMP Xn,Xm`, `CSET Xd,cond`, and SP-indexed
    64-bit pair `STP/LDP`.
    `finish_bytes()` emits little-endian instruction bytes. ABI helpers emit
    callee-saved block prologue/epilogue and patchable-tail offsets.
    `Lowerer::lower_function()` handles the first real IR slice: wide
    constants, `LoadReg`/`StoreReg` over `CpuStateFrame::gpr[]`,
    `Add/Sub` with register RHS or 12-bit RHS immediates, logical `And/Or/Xor`,
    variable `Shl/Shr/Sar/Ror`, `Mul/UMulHi/SMulHi/UDiv/SDiv/UMod/SMod`, `Compare`,
    `CmpFlags` including C++-matching sub-64-bit operand alignment for
    `I8`/`I16`/`I32`, sized
    `LoadMem/StoreMem`, direct `Jump`, boolean `CondJump`, `CondJumpFlags`,
    `JumpReg`, and `Return`. Broad op lowering still pending.
- [ ] ALU instructions completas (ADD..ROR, CMP, CSEL, etc.)
- [ ] Memory instructions (LDR/STR/LDP/STP todas las variantes)
- [ ] Branches (B/BL/BR/BLR/RET + condiciones)
- [ ] SIMD/FP (ADD/SUB/MUL/FADD/FSUB/FMUL + load/store)
- [ ] Crypto (AESE/AESD/AESMC/AESIMC/SHA256*)
- [ ] Barriers (DMB/DSB/ISB)
- [ ] `Lowerer` mapea todos los IR ops a secuencias ARM64
- [~] Label fixup para branches forward
  - 2026-06-12: `Arm64Assembler` has opaque labels, forward/backward `B`
    patching on `finish()`/`finish_bytes()`, and an unresolved-label panic
    test. `Lowerer::lower_function()` now lowers direct IR `Jump` across basic
    blocks with exact forward/backward branch tests. `B.cond`, `CBZ/CBNZ X`
    encoders, boolean `CondJump`, and `CondJumpFlags` lowering are covered
    with exact tests. Real condition-code lowering is covered through
    `Compare` -> `CMP/CSET` and `CmpFlags` -> `B.cond`. `BR/BLR X` encoders
    and `JumpReg` lowering are also covered by exact tests.
- [ ] Register allocator (aleatorio o greedy simple)
- [ ] `cargo test --package prisma-backend` verde
- [~] Tests differentiales contra C++ emitter + lowerer (smoke scaffold activo)
- [ ] Benchmark: generated code quality vs C++ backend

## QA note 2026-06-12

Backend Rust slice review after `B.cond` + `CmpFlags`/`CondJumpFlags`:

- Unit tests pass with `cargo +1.95.0-x86_64-pc-windows-msvc test -p prisma-backend`:
  50 passed.
- Full `cargo test -p prisma-backend` currently runs the same unit tests, then
  fails in doctests because the active `stable-x86_64-pc-windows-msvc`
  toolchain has no runnable `rustdoc.exe`.
- `cargo clippy -p prisma-backend --all-targets -- -D warnings` is blocked by
  the local toolchain install: both `stable` and `1.96.0` contain
  `cargo-clippy.exe`, but not `clippy-driver.exe`.
  The repository Windows gate uses `+1.95.0-x86_64-pc-windows-msvc`; that
  toolchain successfully runs fmt, tests, and clippy for `prisma-backend`.
- Current Rust coverage includes exact encoders/lowering for `B.cond`,
  `CondJumpFlags`, `JumpReg`, `LoadReg`/`StoreReg` over `CpuStateFrame::gpr[]`,
  `Add/Sub` register-register lowering, sized memory ops, logical `And/Or/Xor`, variable
  `Shl/Shr/Sar/Ror`, scalar `Mul/UMulHi/SMulHi/UDiv/SDiv/UMod/SMod`, and
  `CmpFlags` sub-64-bit alignment before `B.cond`.
- Broad differential coverage now has a first bridge: `C++` smoke fixtures can
  dump emitted ARM64 bytes via `prisma_translator_translate_with_code`
  (`PRISMA_CAPI_VERSION = 3`) and compare against Rust smoke bytes in
  `shell/core/tests/smoke_differential.rs`. `Compare` now aligns non-I64
  operands before `cmp_x` (same path as `CmpFlags`), with explicit unit coverage
  for `I8`, `I16`, and `I32`.

Recommended next smallest implementation task: start the differential harness
with a tiny corpus of C++-known IR snippets: `CmpFlags+CondJumpFlags` for
I8/I16/I32/I64 and scalar `BinOp And/Or/Xor/Shl/Shr/Sar/Ror/Mul/UMulHi/SMulHi/UDiv/SDiv`
I64 plus `UMod/SMod` I64. Use the draft in
`docs/REVIEW_TEMPLATES/rust-backend-differential-plan.md` as the starting
contract before adding broader scalar ops.
