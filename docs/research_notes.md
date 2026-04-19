# Research Notes — Prisma DBT

> Notas de investigación personales de Danny mientras construye Prisma. Este documento es el entregable central de Fase 0 y crece continuamente después.

**Formato**: por cada fuente leída, una sección con (1) resumen en 3-5 bullets, (2) lecciones aplicables a Prisma, (3) preguntas abiertas que generó. **No copiar y pegar** — síntesis en tus palabras. Si no puedes resumir algo, no lo entendiste.

**Target Fase 0:** 40-60 páginas. Publicar como blog post al fin de Fase 0.

---

## Reading list estructurada

Marcar cada item con su estado: ⬜ pendiente | 🟡 en progreso | ✅ completado. Al completar, añadir enlace a la sección de notas correspondiente abajo.

### Tier 1: Código fuente de producción (obligatorio, profundo)

- ⬜ **FEX-Emu** — `FEXCore/Source/Interface/Core/` (IR, passes, threading).  
  *Repo:* https://github.com/FEX-Emu/FEX  
  *Target:* entender IR SSA, multipass optimization, cómo maneja TSO. ~20h.
- ⬜ **Box64** — `src/dynarec/arm64/`.  
  *Repo:* https://github.com/ptitSeb/box64  
  *Target:* dynarec de 4 pases, estrategia de wrappers nativos para libc/Mesa. ~15h.
- ⬜ **Wine** — `loader/` + `dlls/ntdll/`.  
  *Repo:* https://gitlab.winehq.org/wine/wine  
  *Target:* cómo carga PE, cómo implementa subset de Win32 API. ~10h.
- ⬜ **DXVK** — arquitectura general, cómo traduce D3D11 state a Vulkan.  
  *Repo:* https://github.com/doitsujin/dxvk  
  *Target:* inspiración para Pilar 6 (graphics translation avanzada). ~8h.

### Tier 2: Papers académicos fundacionales (obligatorios)

- ⬜ **Transmeta Crusoe Code Morphing Software**  
  *Relevancia:* el DBT más ambicioso que ha existido. Modelo de referencia.  
  *Búsqueda:* "Transmeta Crusoe Code Morphing Software" (IEEE Micro 2000, Klaiber).
- ⬜ **IA-32 Execution Layer** (Intel 2003, para Itanium)  
  *Relevancia:* el fracaso más instructivo. Por qué no funcionó técnicamente.
- ⬜ **TOSTING: Investigating Total Store Ordering on ARM** (Springer 2023)  
  *Relevancia:* 8.94% cost de TSO por hardware. Base cuantitativa para el Pilar 3.
- ⬜ **Valgrind VEX IR**  
  *Fuente:* http://www.valgrind.org/docs/manual/ — design document.  
  *Relevancia:* referencia de IR minimalista. Inspiración para `ir-spec/`.
- ⬜ **DynamoRIO architecture papers** (Bruening, MIT PhD thesis 2004)  
  *Relevancia:* referencia clásica de DBT infrastructure.
- ⬜ **Pin: Building Customized Program Analysis Tools** (Luk et al, PLDI 2005)  
  *Relevancia:* dispatch indirecto, side-exit handling.

### Tier 3: Memory models y concurrency (crítico para Pilar 3)

- ⬜ **Sarkar, Sewell et al — "The Semantics of x86-CC Multiprocessor Machine Code"**  
  *Relevancia:* especificación formal del memory model x86. Base para TSO adaptativo.
- ⬜ **Batty et al — "Mathematizing C++ Concurrency" (POPL 2011)**  
  *Relevancia:* cómo especificar formalmente memory models. Inspiración metodológica.
- ⬜ **ARM Architecture Reference Manual, sección B2 (Memory Ordering)**  
  *Relevancia:* entender LDAR/STLR/DMB en detalle.

### Tier 4: Formal methods para compilers/DBT (para Pilar 2)

- ⬜ **CompCert: A Formally Verified C Compiler** (Leroy, CACM 2009)  
  *Relevancia:* el precedente más exitoso de verificación formal en compilers.
- ⬜ **CakeML: A Verified Implementation of ML** (Kumar et al, POPL 2014)  
  *Relevancia:* verified backend, referencia directa para Prisma IR.
- ⬜ **Lean 4 tutorial + mathlib intro**  
  *Fuente:* https://leanprover-community.github.io/  
  *Target:* calibrar tu curva de aprendizaje. Critical para decision point de semana 20.

### Tier 5: Rosetta 2 + Apple silicon (para el techo teórico)

- ⬜ **Rosetta 2 internals** — charla Koh M. Nakagawa, BSides 2021.  
  *Búsqueda:* "Rosetta 2 internals Nakagawa BSides".
- ⬜ **Apple silicon TSO bit** — documentación y análisis independiente.  
  *Relevancia:* entender exactamente qué es lo que no podemos replicar en Android.

### Tier 6: NPU / ML acceleration en móvil (para Pilar 1)

- ⬜ **NNAPI architecture** — Android docs.  
  *Fuente:* https://developer.android.com/ndk/guides/neuralnetworks.
- ⬜ **ONNX Runtime Mobile + NNAPI delegate**  
  *Fuente:* https://onnxruntime.ai/docs/execution-providers/NNAPI-ExecutionProvider.html.
- ⬜ **MediaTek NeuroPilot SDK docs**  
  *Relevancia:* Dimensity 8300-Ultra NPU.
- ⬜ **Qualcomm Hexagon SDK docs**  
  *Relevancia:* Snapdragon NPUs.
- ⬜ **Papers sobre ML-guided compilation** (buscar: "ML for compiler optimization survey").

### Tier 7: Graphics translation (para Pilar 6)

- ⬜ **DXVK architecture blog posts de doitsujin**.
- ⬜ **VKD3D-Proton source tour** — cómo traducen DX12 a Vulkan.
- ⬜ **Mesa Turnip driver architecture**.
- ⬜ **Vortek CPU-assisted driver** (Winlator Cmod fork) — entender limitaciones.

### Tier 8: Virtualización Android (para Pilar 5)

- ⬜ **Android Virtualization Framework (AVF) docs**  
  *Fuente:* https://source.android.com/docs/core/virtualization.
- ⬜ **Danny Lin Windows 11 on Pixel 6 writeup** (abril 2022).
- ⬜ **pKVM design paper** (Google/LSE).

### Tier 9: Ecosistema (conocer al competidor)

- ⬜ **Winlator Cmod source** — entender por qué es el fork más exitoso.
- ⬜ **GameNative (Pluvia fork)** — integración Steam/Epic/GOG.
- ⬜ **Microsoft Prism (lo que es público)** — XTA Cache, ARM64EC, WowBox64.
- ⬜ **ExaGear post-mortem** — por qué Huawei solo compró engine, no marca.

---

## Notas de lectura

Plantilla para cada fuente:

```
## [Título de la fuente]

**Fecha lectura:** YYYY-MM-DD  
**Tiempo invertido:** Xh  
**Link/Ref:** ...

### Resumen (3-5 bullets, en tus palabras)
- ...

### Lecciones aplicables a Prisma
- ...

### Preguntas abiertas
- ...

### Fragmentos de código que vale referenciar más tarde
(si aplica, con path exacto y número de línea del source leído)
```

---

## Decisiones técnicas tomadas durante Fase 0

Documentar aquí cada decisión arquitectónica con:
- **Decisión:**
- **Fecha:**
- **Alternativas consideradas:**
- **Razón:**
- **Reversible?:** (y cómo, si aplica)

_(vacío — empezará a llenarse desde semana 3)_

---

## Preguntas abiertas globales

Lista viva de preguntas que emergen de la investigación y aún no tienen respuesta clara:

- ¿Es viable expresar en Lean 4 una spec completa del IR con todas las instrucciones x86 relevantes, o es demasiado grande para un solo-dev?
- ¿Cuál es el overhead real de NNAPI inference desde C++? ¿Es sub-microsegundo o ya no sirve para hot path prediction?
- ¿Hay alguna forma de detectar en runtime si el SoC tiene FEAT_LSE2 + LRCPC2 + FlagM y explotar las instrucciones acquire/release cheap?
- ¿El protocolo P2P del Pilar 4 debe ser custom o podemos piggyback en libp2p existente?

_(actualizar conforme aparezcan nuevas preguntas)_
