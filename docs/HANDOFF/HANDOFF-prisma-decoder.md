# HANDOFF: prisma-decoder (Fase 3)

> Para CODEX. Cómo migrar el decoder x86_64 → IR a Rust.
> Este es el archivo más grande del core (7,318 líneas).
> **No intentar migrar de una vez** — hacerlo por grupos de opcodes.

## Estrategia

Migrar en 8 sub-fases, cada una correspondiente a una familia de
opcodes. Cada sub-fase produce un módulo Rust independiente y su
conjunto de tests.

```
shell/prisma-decoder/src/
  lib.rs          — decode_one() entry point + error types
  modrm.rs        — ModR/M + SIB byte parsing
  prefixes.rs     — REX, VEX, EVEX, operand/address size override
  ops/
    mod.rs        — dispatch tabla + mod opend re-exports
    alu.rs        — ADD, SUB, MUL, DIV, INC, DEC, NEG, etc.
    moves.rs      — MOV, MOVZX, MOVSX, CMOVcc, XCHG
    shifts.rs     — SHL, SHR, SAR, ROL, ROR, RCL, RCR
    bits.rs       — AND, OR, XOR, NOT, TEST, BSF, BSR, BT*, etc.
    sse.rs        — SSE/SSE2/SSE3/SSE4.x packed+scalar
    avx.rs        — AVX-128/256 VEX-coded
    fma.rs        — FMA3 132/213/231 forms
    bmi.rs        — BMI1/BMI2 (LZCNT, TZCNT, POPCNT, PDEP, PEXT, etc.)
    aes_sha.rs    — AES-NI + SHA-NI + AESKEYGENASSIST
    x87.rs        — x87 FPU instructions
    system.rs     — CPUID, XGETBV, RDTSC, SYSCALL, INT3, UD2, etc.
```

## Archivos C++ a migrar

| C++ | Rust | Líneas | Prioridad |
|-----|------|--------|-----------|
| `x86_decoder.cpp` prefix/REX/VEX parsing | `prefixes.rs` | ~800 | Alta |
| `x86_decoder.cpp` ModR/M + SIB | `modrm.rs` | ~400 | Alta |
| `x86_decoder.cpp` ALU group | `ops/alu.rs` | ~600 | Alta |
| `x86_decoder.cpp` MOV group | `ops/moves.rs` | ~500 | Alta |
| `x86_decoder.cpp` SSE | `ops/sse.rs` | ~2000 | Media |
| `x86_decoder.cpp` AVX-256 | `ops/avx.rs` | ~800 | Media |
| `x86_decoder.cpp` FMA3 | `ops/fma.rs` | ~300 | Media |
| `x86_decoder.cpp` BMI | `ops/bmi.rs` | ~400 | Media |
| `x86_decoder.cpp` AES/SHA | `ops/aes_sha.rs` | ~300 | Baja |
| `x86_decoder.cpp` x87 | `ops/x87.rs` | ~600 | Baja |
| `x86_decoder.cpp` system | `ops/system.rs` | ~200 | Media |

## API

```rust
pub fn decode_one(bytes: &[u8], offset: usize) -> Result<Decoded, DecodeError>;

pub struct Decoded {
    pub stmts: Vec<Stmt>,
    pub bytes_consumed: usize,
}

pub enum DecodeError {
    Truncated,
    UnsupportedOpcode(u8),
    InvalidModRm(usize),
    InvalidSib(usize),
    InvalidLock,
    InvalidVex,
}
```

## Pattern: ModR/M byte parsing

```rust
pub struct ModRm {
    pub mod_: u8,    // bits 7-6
    pub reg: u8,     // bits 5-3
    pub rm: u8,      // bits 2-0
}

pub fn parse_modrm(bytes: &[u8], offset: usize) -> Result<(ModRm, usize), DecodeError> {
    let b = bytes.get(offset).ok_or(DecodeError::Truncated)?;
    Ok((ModRm {
        mod_: (b >> 6) & 0x3,
        reg: (b >> 3) & 0x7,
        rm:  b & 0x7,
    }, offset + 1))
}

/// Returns the effective address as IR statements + the final Ref.
pub fn decode_ea(modrm: &ModRm, rex: RexPrefix, sib: Option<Sib>,
                  disp_size: OpSize, stmts: &mut Vec<Stmt>) -> Option<Ref> {
    match (modrm.mod_, modrm.rm) {
        (0, 4) => decode_sib(sib, rex, 0, stmts),       // [SIB]
        (0, 5) => Some(disp32_as_ref()),                   // [rip+disp32]
        (0, rm) => Some(gpr_ref(rm, rex)),                 // [reg]
        (1, 4) => decode_sib(sib, rex, disp8, stmts),     // [SIB+disp8]
        (1, rm) => Some(disp8_add(gpr_ref(rm, rex))),
        (2, 4) => decode_sib(sib, rex, disp32, stmts),    // [SIB+disp32]
        (2, rm) => Some(disp32_add(gpr_ref(rm, rex))),
        (3, _) => unreachable!("register direct"),
        _ => None,
    }
}
```

## Pattern: opcode dispatch

```rust
pub fn decode_opcode(bytes: &[u8], offset: usize, prefixes: &PrefixSet)
    -> Result<Decoded, DecodeError> {
    let op = bytes.get(offset).ok_or(DecodeError::Truncated)?;
    let mut cursor = offset + 1;
    match op {
        0x00 => decode_alu(AluOp::Add, ModRmMode::Store, bytes, &mut cursor, prefixes),
        0x01 => decode_alu(AluOp::Add, ModRmMode::Store, bytes, &mut cursor, prefixes),
        0x02 => decode_alu(AluOp::Add, ModRmMode::Load, bytes, &mut cursor, prefixes),
        0x03 => decode_alu(AluOp::Add, ModRmMode::Load, bytes, &mut cursor, prefixes),
        // ... ~250 opcodes primarios
        0x0F => decode_two_byte_opcode(bytes, cursor, prefixes),
        _ => Err(DecodeError::UnsupportedOpcode(op)),
    }
}
```

## Tests mínimos por sub-fase

- Cada instrucción decodificada produce el IR esperado
- Bytes truncados → `DecodeError::Truncated`
- Prefijos inválidos → `DecodeError::InvalidVex` etc.
- Fuzzing con proptest: bytes arbitrarios nunca crash
- **Differential:** misma salida de IR que C++ decoder

## Checklist

- [ ] `prefixes.rs` maneja REX, VEX C4/C5, operand/address size
- [ ] `modrm.rs` parsea ModR/M + SIB + displacement
- [ ] ALU ops completas (ADD/SUB/MUL/DIV/AND/OR/XOR/SHL/SHR/etc.)
- [ ] MOV family (MOV/MOVZX/MOVSX/CMOVcc/XCHG/BSWAP)
- [ ] SSE/SSE2 completo (packed + scalar, integer + FP)
- [ ] AVX-128 + AVX-256 VEX-coded
- [ ] FMA3 (132/213/231, PS/PD/SS/SD)
- [ ] BMI1/BMI2 (LZCNT, TZCNT, POPCNT, PDEP, PEXT, BZHI, MULX, etc.)
- [ ] AES/SHA-NI
- [ ] x87 (reduced-F64 bridge)
- [ ] System (CPUID, XGETBV, RDTSC, SYSCALL, INT3, UD2)
- [ ] `cargo fuzz` target para fuzzing continuo
- [ ] Tests differentiales contra C++
