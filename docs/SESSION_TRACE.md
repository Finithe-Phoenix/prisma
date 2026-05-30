# SESSION_TRACE.md — Sesión Claude → handoff

Trazabilidad de la sesión de trabajo en la rama
`claude/hopeful-taussig-051239`.

- **Base**: `8884efd` (`docs(claude.md): refresh — F2-IR-048 AVX-128 VEX dispatch landed`).
- **HEAD actual**: `e59b944`.
- **Total de commits**: 41 (todos los SHAs verificables vía `git log 8884efd..HEAD`).
- **Cobertura de tests al cierre**: 797 casos / 4836 assertions verde
  (excluyendo `signal_handler*` que es flaky pre-existente en macOS).
- **Working tree**: limpio. Listo para PR.

## Bloques temáticos en orden cronológico

### Bloque A — F2-IR-005 AVX-256 base (2026-05-05 → 06)

Pair-of-Vec128 representation. `ymm_hi[16]` añadido a `CpuStateFrame`,
nuevos IR ops `LoadVecRegHi` / `StoreVecRegHi`, allowlist por opcode
para VEX.L=1.

| SHA       | Mensaje                                                                  |
|-----------|---------------------------------------------------------------------------|
| `a2fabde` | feat(ir,backend): F2-IR-005 — YMM state, LoadVecRegHi/StoreVecRegHi infrastructure |
| `bd4e6cc` | feat(decoder): F2-IR-005 — VADDPS/SUBPS/MULPS/DIVPS/MIN/MAX/SQRTPS+PD ymm |
| `6cf73f9` | feat(decoder): F2-IR-005 — VANDPS/ORPS/XORPS/PD ymm + VPADD/SUB/PAND/POR/PXOR ymm |
| `c5d4a71` | feat(decoder): F2-IR-005 — VPCMPEQ/GT B/W/D + VUNPCKL/H ymm              |
| `99d2056` | feat(decoder): F2-IR-005 — VSHUFPS/PD + VHADDPS/PD + VCMPxxPS/PD ymm     |
| `3f94098` | docs(claude.md,backlog): F2-IR-005 — AVX-256 first batch landed          |

### Bloque B — F2-BK-006 NEON regalloc (2026-05-06)

| SHA       | Mensaje                                                                          |
|-----------|-----------------------------------------------------------------------------------|
| `05044f8` | feat(backend): F2-BK-006 — widen FP scratch pool V0..V7 → V0..V23                 |
| `0597402` | feat(backend): F2-BK-006 — FP last-use expiry + close two compute_liveness gaps   |
| `0ab0c22` | docs(backlog): F2-BK-006 NEON regalloc closed (pool 24 + FP expiry)               |

Bug latente arreglado: `FpBinOp` faltaba en `compute_liveness`.

### Bloque C — F2-IR-006 FMA (2026-05-06)

96 instrucciones FMA totales (packed PS/PD xmm+ymm + scalar SS/SD +
MADDSUB/MSUBADD packed). Dos IR ops: `VecFpFma` (packed) y
`VecFpScalarFma` (con `scalar_upper` Ref).

| SHA       | Mensaje                                                                       |
|-----------|--------------------------------------------------------------------------------|
| `02c900f` | feat(ir,decoder,backend): F2-IR-006 — VFMADD132/213/231 PS+PD xmm              |
| `d98bdbb` | feat(decoder): F2-IR-006 — VFMSUB / VFNMADD / VFNMSUB families                |
| `cdf9411` | docs(claude.md,backlog): F2-IR-006 — FMA packed xmm landed                    |
| `50caa95` | feat(decoder): F2-IR-006 — VFMADD/SUB/NMADD/NMSUB ymm (256-bit FMA)           |
| `60dac07` | fix(decoder): F2-IR-006 — restrict FMA3 to packed forms (even low-nibble)     |
| `afccedd` | feat(ir,decoder,backend): F2-IR-006 — scalar FMA (VFMADDxxxSS / SD families)  |
| `f340201` | feat(decoder,backend): F2-IR-006 — VFMADDSUB / VFMSUBADD packed (xmm + ymm)   |

Bug latente arreglado: `VecConstant` lowering cargaba solo low 64
bits — ahora carga 128 completos (`vec_const_128` emitter primitive).

### Bloque D — Lane-crossing AVX-256 (2026-05-06)

| SHA       | Mensaje                                                                              |
|-----------|---------------------------------------------------------------------------------------|
| `e358bcd` | feat(decoder): F2-IR-005 follow-up — VBROADCASTSS / VBROADCASTSD / VBROADCASTF128    |
| `99ed88c` | feat(decoder): F2-IR-005 follow-up — VINSERTF128 / VEXTRACTF128 / VPERM2F128         |

Todos sin nuevos IR ops (sintetizados via `VecShuffle32x4` /
`VecUnpack` / `Load+StoreVecReg{Hi}` / `VecConstant`).

### Bloque E — F2-PS-002 + F2-BK-007 (2026-05-06)

| SHA       | Mensaje                                                                       |
|-----------|--------------------------------------------------------------------------------|
| `cac89f7` | feat(passes): F2-PS-002 — DCE drops dead WriteFlags + closes operand-collect gaps |
| `6fb63e0` | docs(backlog): record F2-PS-002 commit SHA cac89f7                            |
| `905f122` | docs(backlog): close umbrella items F2-IR-002..004 + F2-BK-001..004           |
| `2bac887` | docs(claude.md): refresh — full FMA family + lane-crossing AVX-256 landed     |
| `8317648` | feat(ir,decoder,backend): F2-BK-007 — MUL/DIV proper rdx:rax / rax:rdx lowering |
| `de1a836` | feat(passes): F2-BK-007 follow-up — algebraic identities for MUL/DIV/MOD      |
| `85db4c6` | docs(claude.md): refresh — FMADDSUB + F2-BK-007 MUL/DIV landed                |

Nuevos `BinOpKind`: UMulHi, SMulHi, UDiv, SDiv, UMod, SMod. Const-prop
folds con `__int128` para constantes en tiempo de compilación.

### Bloque F — Cobertura ymm extendida (2026-05-06 → 07)

Cobertura amplia de ymm para handlers existentes. Sin nuevos IR ops.

| SHA       | Mensaje                                                                            |
|-----------|-------------------------------------------------------------------------------------|
| `b2b7ea9` | feat(decoder): F2-IR-005 — extend AVX-256 allowlist to integer SIMD full set       |
| `c46554a` | feat(decoder): F2-IR-005 — SSSE3 + SSE4.1 ymm via 0F 38 escape                     |
| `bdef659` | feat(decoder): F2-IR-005 — VPSHUFB / VPABS B/W/D ymm                              |
| `5745961` | feat(decoder): F2-IR-005 — VPSHUFD ymm                                            |
| `0ddc2dd` | feat(decoder): F2-IR-005 — VPSHUFLW / VPSHUFHW ymm                                |
| `3402428` | feat(decoder): F2-IR-005 — VMOVDQA/U / VMOVAPS/UPS / VMOVAPD/UPD ymm              |
| `9a5fbd6` | docs(claude.md): refresh — broad AVX-256 ymm coverage extension                   |
| `52ca5c3` | feat(decoder): F2-IR-005 — VMOVDDUP / VMOVSLDUP / VMOVSHDUP ymm                    |
| `2542ef7` | feat(decoder): F2-IR-005 — VLDDQU / VMOVNT{DQ,PS,PD} ymm                          |
| `82e6845` | feat(decoder): F2-IR-005 — VPMOVMSKB ymm (32-bit MSB extract)                      |
| `cb9256b` | feat(decoder): F2-IR-005 — VMOVMSKPS / VMOVMSKPD ymm                              |
| `8d9f114` | feat(decoder): F2-IR-005 — VROUNDPS / VROUNDPD ymm                                |
| `ab39cc1` | docs(claude.md): refresh — VMOVDUP / VLDDQU / VPMOVMSKB / VMOVMSKPS-PD / VROUNDPS-PD ymm |
| `a5b20fb` | feat(decoder): F2-IR-005 — VPMOVZX/SX BW/BD/BQ/WD/WQ/DQ ymm                       |
| `b99e2ec` | docs(claude.md): refresh — VPMOVZX/SX ymm lane-crossing landed                    |
| `e59b944` | docs(backlog): record real SHA for F2-BK-007                                      |

## Backlog cerrado en la sesión

12 items: F2-IR-002, F2-IR-003, F2-IR-004, F2-IR-005, F2-IR-006,
F2-BK-001, F2-BK-002, F2-BK-003, F2-BK-004, F2-BK-006, F2-BK-007,
F2-PS-002.

## Conteo final

| Métrica                        | Inicio (8884efd) | Fin (e59b944)  |
|--------------------------------|------------------|----------------|
| Catch2 test cases              | 763              | **797**        |
| Catch2 assertions              | 4692             | **4836**       |
| Líneas en `x86_decoder.cpp`    | 4507             | ~5500+         |
| Opcodes ymm activos            | ~0 (gate global) | ~120+          |
| Opcodes FMA totales            | 0                | **96**         |

Sin regresiones funcionales (el único fallo recurrente,
`signal_handler*`, es flaky pre-existente en el entorno macOS).
