# Prisma — Estado del proyecto (verificado en fuente)

> Foto del estado real con evidencia de `archivo:línea`. Generado verificando
> el código fuente directamente (no de memoria ni de los docs históricos). Para
> el plan hacia adelante ver [ROADMAP.md](ROADMAP.md).

Última actualización: 2026-06-19 America/Mexico_City.
Método: sondeo del core C++ + verificación paralela de 8 familias de
extensiones con agentes independientes (cada `DECODED+LOWERED+TESTED` abajo
tiene evidencia de fuente).

---

## TL;DR — "estás aquí"

**Un JIT x86→ARM64 funcional para un subconjunto amplio del ISA (incluyendo
SSE..SSE4.1, AVX-128, AVX-256, FMA3, BMI2, AES-NI, SHA-NI, CRC32, x87),
ejecutando en ARM64 real, técnicamente a mitad de Fase 2 — con una reescritura
paralela a Rust en marcha.** Lo que falta para "correr una `.exe` de Windows"
es el **entorno de SO invitado** (PE loader, Win32/NT, Wine): Fase 2.5→3.

---

## 1. Subsistemas del core C++ (`core/src/`, `core/include/prisma/`)

| Subsistema | Estado | Notas |
|------------|--------|-------|
| **Decoder** (`decoder/x86_decoder.cpp`, ~7.8k líneas) | Funcional, amplio | x86-64 → IR; puro/sin efectos secundarios |
| **IR SSA** (`include/prisma/ir.hpp`) | Estable | ~30+ familias de ops; spec Lean 4 autoritativa |
| **Passes** (`passes/`, 16 archivos) | Funcional | Pipeline de 10+ pases (const-prop, DCE, CSE, GCSE, LICM, peephole, x87-stack, etc.) |
| **Emitter** (`backend/emitter.cpp`, ~2.3k) | Funcional | ARM64 vía vixl MacroAssembler |
| **Lowering** (`backend/lowering.cpp`, ~2.5k) | Funcional | IR → ARM64; regalloc, ABI, mapping guest↔host |
| **Cache** (`cache/`) | Funcional | FNV-1a + SHA-256 + compresión; SMC-safe |
| **Runtime** (`runtime/`, 8 archivos) | Funcional | Dispatcher + chaining + RAS; JIT W^X (MAP_JIT/mprotect); signal handlers; SMC guard; syscalls Linux x86-64 |
| **Translator** (`translator/translator.cpp`) | Funcional | Fachada decode→IR→passes→lower→emit; CFG; CPUID leaf model |

**Tests:** ~1,200 `TEST_CASE` Catch2 en ~37 archivos (`core/tests/`).

---

## 2. Cobertura de instrucciones x86 (verificada)

### ✅ Implementado (DECODED + LOWERED + TESTED)

- **Base ALU / shifts / mul-div / bit-manip / strings / control flow** — amplio.
- **POPCNT / LZCNT / TZCNT** — `x86_decoder.cpp:7256+`, `lowering.cpp:1947+`
  (CLZ/RBIT/CNT directos).
- **SSE / SSE2 / SSE3 / SSSE3 / SSE4.1** — aritmética packed/scalar, compares,
  shifts, shuffles, blends, conversiones, dot-product, etc.
- **AVX-128** (VEX) — familia entera y FP.
- **AVX-256** (VEX.L=1, ymm) — `x86_decoder.cpp:2900-3104`. Representación
  pair-of-Vec128 (`ymm_hi[]`, `LoadVecRegHi`/`StoreVecRegHi`, RFC 0011).
  **Incluye lane-crossing**: VPERMQ (F2-IR-051), VPERMD (F2-IR-052),
  VINSERTF128/VEXTRACTF128, VBROADCAST*; VPTEST ymm (F2-IR-049); VPGATHER ymm
  (F2-IR-059).
- **FMA3** — packed + scalar, 132/213/231 × {PS,PD,SS,SD} × {xmm,ymm} +
  MADDSUB/MSUBADD. `x86_decoder.cpp:5341-5659`, `lowering.cpp:2338-2403` (FMLA/
  FMLS). e2e en `test_e2e_dispatcher.cpp`.
- **BMI2** — SHLX/SARX/SHRX/RORX, MULX, BZHI, PDEP/PEXT.
- **AES-NI** — AESENC/ENCLAST/DEC/DECLAST/IMC + AESKEYGENASSIST.
  `x86_decoder.cpp:4810+`, emitter AESE/AESD/AESIMC/TBL (F2-IR-055/058).
- **SHA-NI** — las 7 variantes (SHA1RNDS4/NEXTE/MSG1/MSG2, SHA256RNDS2/MSG1/
  MSG2) → ARMv8 crypto. KATs FIPS 180-4. `x86_decoder.cpp:4345-4610`,
  `lowering.cpp:1759-1796` (F2-IR-060). *Primer DBT que baja SHA256RNDS2 al
  choreography de 4 rondas SHA256H/H2.*
- **CRC32** (SSE4.2 `F2 0F 38 F0/F1`) — ARM64 CRC32C{B/H/W/X}.
  `x86_decoder.cpp:4396+`, `emitter.cpp:1344` (F2-IR-057).
- **MOVBE** (`0F 38 F0/F1`) — vía `Bswap`/REV. `x86_decoder.cpp:4453+` (F2-IR-056).
- **x87 FPU** — FLD/FST/FSTP/FADD/FMUL/FSUB/FDIV/FXCH/FILD/FISTP sobre un puente
  reduced-F64 con stack de 8 (`X87Load/Store/Push/Pop`). `x86_decoder.cpp:2650-2766`,
  `lowering.cpp:1848-1925`, pase F2-PS-001 `passes/x87_stack.cpp`. Precisión
  reducida documentada (RFC 0013).
- **Atomics**: CMPXCHG/8B/16B, XADD, LOCK. **RDTSC, SYSCALL, CPUID, XGETBV,
  VZEROUPPER/ALL.**

### ❌ Deliberadamente ausente (documentado en el modelo CPUID)

- **SSE4.2 string** (PCMPESTRI/PCMPISTRI/PCMPESTRM/PCMPISTRM) —
  `translator.cpp:506` (la capacidad SSE4.2 se anuncia off salvo PCMPGTQ/CRC32).
- **BMI1** (ANDN/BEXTR/BLSI/BLSMSK/BLSR) — `translator.cpp:520`.
- **AVX-512 / EVEX** (masking, broadcast, rounding) — fuera de scope.
- **MMX, F16C, PCLMULQDQ, RDRAND, XSAVE** — no decodificados.

> **Nota de reconciliación docs↔código:** un primer sondeo conservador
> reportó ~80-100 instrucciones y marcó x87/FMA3/AVX-256/SHA como ausentes.
> La verificación paralela dirigida **lo corrige**: x87, FMA3, AVX-256, SHA-NI,
> AES, CRC32 y MOVBE **están todos implementados con evidencia de fuente**. Los
> docs históricos (CLAUDE.md/WORK_QUEUE) eran exactos; el primer sondeo leyó
> extractos y subestimó.

---

## 3. Ejecución JIT en ARM64 (probada)

- **JitBuffer** (`runtime/jit_memory.cpp`): mmap (POSIX) / VirtualAlloc
  (Windows); W^X vía MAP_JIT + `pthread_jit_write_protect_np` en Apple Silicon,
  mprotect en Linux; invalidación de I-cache.
- **Dispatcher** (`runtime/dispatcher.cpp:18`): castea el código traducido a
  puntero a función y lo invoca con el `CpuStateFrame`; chaining + patching.
- **Tests e2e** (`test_e2e.cpp`, `test_jit_execution.cpp`, `test_e2e_dispatcher.cpp`):
  traducen x86 → ARM64, escriben buffer ejecutable, lo llaman como función C++,
  verifican el resultado. Gated a hosts ARM64 (`constexpr is_arm64`) + corre en
  el runner `core-build-arm64` / `ffi-link-arm64` de CI.

**El pipeline de bytes → código corriendo está completo para el ISA del MVP.**

---

## 4. Migración a Rust (`shell/`, RFC 0015)

| Crate | LoC src | Estado |
|-------|---------|--------|
| `prisma-decoder` | ~11.8k | Subset del C++ (one/two-byte, grupos, Jcc, mem forms) |
| `prisma-backend` | ~5.6k | Lowering de ~42 ops a ARM64 |
| `prisma-passes` | ~5.0k | 13 pases + 2 function-pass; completo |
| `prisma-runtime` | ~2.6k | jit_memory (W^X), dispatcher de contrato, smc_guard, syscall boundary, **`executor` (ejecuta bloques traducidos en ARM64)** |
| `prisma-ir` | ~1.4k | Tipos IR + `Op::map_refs` (visitor SSA de 97 variantes) |
| `prisma-cache` | ~1.0k | Cache real (zstd + sha256), fix de DoS de deserialización |
| `prisma-translator` | ~0.7k | **Fachada integrada**: decode→optimizar→lower→cache + `translate_block` + `translate_fused_block` (renumeración SSA) + stats/límites/SMC |
| `orchestrator` | ~0.85k | PE loader (mapeo), container (esqueleto) |

- **Fuzzing de robustez** proptest en las 5 superficies (decoder/cache/passes/
  backend/translator).
- **Ejecución JIT en ARM64 ALCANZADA** (PR #43, sha `cc7ada7`): `prisma-runtime::executor`
  envuelve el cuerpo de bloque traducido con prólogo/epílogo AAPCS64, lo instala
  W^X y lo llama como `extern "C" fn(*mut CpuStateFrame)`. Un programa de varias
  instrucciones traducido por `prisma-translator` (`mov rax,rcx; add rax,0x10`)
  ejecuta en el runner `ffi-link-arm64` de CI y verifica `rax = rcx + 16`. La
  validación conductual corre en ARM64 real (gateada a `aarch64`, espejo del
  `constexpr is_arm64` del core C++).
- Gate cross-language real: `ffi-link` (ubuntu) corre `clippy --workspace` +
  `test --workspace` contra el DLL C++ (C-ABI, RFC 0014).

---

## 5. Lo que NO existe todavía (la brecha hacia Windows)

- **Sin entorno de SO invitado**: sin loader PE real (solo mapeo de memoria),
  sin resolución de imports/DLLs, sin Win32/NT, sin Wine.
- **Syscalls solo Linux x86-64** (POSIX). En Windows es stub `-ENOSYS`.
- **Sin threads de guest** (dispatcher single-threaded), sin TLS sintético más
  allá del segment-base, sin sincronización inter-hilo modelada.
- **Modelo de flags NZCV a medio migrar** (Pilar 2): el pilar `WriteFlags`/
  `ReadFlag`/`CondJumpFlags` **ya existe** y baja a ADDS/SUBS/ANDS + CSET/CSEL
  leyendo NZCV real (`ir.hpp:332-354`, `lowering.cpp:1248-1280`); en paralelo
  sobrevive el legacy `Compare`→CSET-booleano (`lowering.cpp:624-639`). Falta
  retirar el path booleano + síntesis PF/AF. ADC/SBB/RCL/RCR con carry real ya
  aterrizaron (subsistema de CF persistente).
- **Sin entrega de excepciones/señales al guest** (int3/invalid-opcode/SIGSEGV→
  handler del guest). Los signal handlers existen para faults del host.
- **Sin TSO adaptativo** (Pilar 3): todo emite variantes TSO por ahora.
- **Territorios vacíos**: `android/` (andamiaje), `server/` (solo README,
  Fase 2.5). `tools/benchmarks/` **sí tiene harness real** (paquete Python
  `prisma_bench` + corpus Dhrystone + CI `benchmarks-smoke`); lo que falta son
  más corpus (CoreMark/nbench/SPEC) y los runners por baseline.

---

## 6. Posición en el roadmap

| Fase | Ventana (plan) | Estado |
|------|----------------|--------|
| Fase 0-1 (Fundación + decoder + IR Lean) | abr-nov 2026 | ✅ esencialmente |
| **Fase 2 (ISA completo + Linux user-mode)** | dic 2026-may 2027 | **🟡 en curso** (ISA amplio; falta syscall layer, coreutils, benchmark) |
| Fase 2.5 (6 pilares de investigación) | jun-nov 2027 | ⬜ no empezada |
| **Fase 3 (Wine + Windows real)** ← *el hito* | dic 2027-jul 2028 | ⬜ no empezada |
| Fase 4-6 (Juegos, beta, v1.0) | 2028-2030 | ⬜ |

Próximos pasos concretos: ver [ROADMAP.md](ROADMAP.md) §2 (Track inmediato).
