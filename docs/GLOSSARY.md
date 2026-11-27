# Prisma Glossary

Project-specific terms, abbreviations, and jargon used throughout the
Prisma codebase and documentation.

---

## Architecture

| Term | Definition |
|------|-----------|
| **DBT** | Dynamic Binary Translator — the core technology. Translates x86/x64 machine code to ARM64 at runtime. |
| **Guest** | The x86_64 program being translated and executed. All guest state (registers, memory, flags) is emulated. |
| **Host** | The ARM64 machine (or macOS/Linux system) on which Prisma runs. Host resources include real CPU registers, memory, and syscalls. |
| **JIT** | Just-In-Time compilation. Prisma translates guest basic blocks into ARM64 machine code on demand, then caches and executes them. |
| **Block** | A basic block of guest x86_64 instructions ending at a branch, call, return, or syscall. The unit of translation. |
| **Dispatcher** | The run-loop (`Dispatcher::run`) that chains translated blocks: translate → execute → get next PC → repeat. |
| **Translation Cache** | In-memory (and optionally on-disk) store of previously translated blocks, keyed by `(guest_addr, content_hash)`. Avoids re-translation on cache hit. |

## IR (Intermediate Representation)

| Term | Definition |
|------|-----------|
| **IR** | Intermediate Representation — Prisma's SSA-form IR that sits between decoding and lowering. Defined in `ir.hpp`. |
| **SSA** | Static Single Assignment — each IR `Ref` is defined exactly once. Enables clean optimization passes. |
| **Ref** | A `uint32_t` handle to an SSA value in the IR. Compressed FEX-style for cache efficiency. |
| **Stmt** | A single IR statement (operation + operands + result ref). |
| **OpSize** | The bit-width of an IR operand (`Op8`, `Op16`, `Op32`, `Op64`, `Op128`, `Op256`). |
| **BinOp** | Binary operation node (Add, Sub, And, Or, Xor, Shl, Shr, Sar, Rol, Ror, Mul, etc.). |
| **WriteFlags** | IR op that computes x86 flags (CF, ZF, SF, OF, PF, AF) from an arithmetic result. |
| **ReadFlag** | IR op that extracts a single flag bit from a `WriteFlags` result. |
| **Extend** | Zero-extend or sign-extend an IR value to a wider size. |
| **Truncate** | Narrow an IR value to a smaller size. |
| **Select** | Conditional select (`cond ? val_true : val_false`), used for CMOV lowering. |
| **InlineAsm** | Escape hatch: raw bytes passed through for instructions we cannot yet translate. |
| **GuestPc** | Pseudo-op recording the guest program counter for a translated region. |
| **Function** | Container for multiple `BasicBlock`s with an entry block; enables CFG-level analysis. |

## Decoder

| Term | Definition |
|------|-----------|
| **ModR/M** | The x86 ModR/M byte that encodes register and addressing mode operands. |
| **SIB** | Scale-Index-Base byte — used with ModR/M rm=100 for complex addressing modes. |
| **REX** | 64-bit extension prefix byte (0x40..0x4F). REX.W = 64-bit operand; REX.R/X/B extend register fields. |
| **VEX** | 2-byte (C5) or 3-byte (C4) prefix for AVX instructions. Encodes a non-destructive third operand (vvvv). |
| **Mandatory prefix** | A 66/F2/F3 byte that changes an opcode's meaning (e.g., 66 0F 6F = MOVDQA vs plain 0F 6F = MMX MOVQ). |

## Backend / Lowering

| Term | Definition |
|------|-----------|
| **Lowering** | Converting IR ops into ARM64 machine instructions (via vixl). |
| **Emitter** | The ARM64 code emitter (`prisma_emitter`) that wraps vixl to generate instruction bytes. |
| **vixl** | Google's ARM64 assembler/disassembler library, vendored under `core/third_party/vixl/`. |
| **NEON** | ARM64 SIMD extension (128-bit vectors). Used to lower SSE/AVX operations. |
| **Pinned register** | A host ARM64 register dedicated to a specific purpose (e.g., x19 = CpuStateFrame pointer, x14 = guest RSP). |
| **Scratch register** | A host register temporarily allocated by the linear-scan allocator for intermediate values. |
| **Spill/Reload** | When scratch registers are exhausted, values are spilled to stack memory and reloaded later. |
| **MAP_JIT** | macOS-specific flag for `mmap` that permits W^X JIT code emission on Apple silicon. |

## Passes (Optimization)

| Term | Definition |
|------|-----------|
| **Pass** | A transformation over the IR statement list. Registered in `PassManager`. |
| **PassManager** | Orchestrates the sequence of optimization passes (`default_pipeline`). |
| **const_prop** | Constant propagation — evaluates operations with known-constant inputs at compile time. |
| **DCE** | Dead Code Elimination — removes IR statements whose results are never used. |
| **CSE** | Common Subexpression Elimination — reuses previously computed identical expressions. |
| **LICM** | Loop-Invariant Code Motion — hoists loop-invariant computations out of loops. |
| **Peephole** | Pattern-matching pass that replaces short IR sequences with more efficient equivalents. |
| **Flag-write elimination** | Removes `WriteFlags` ops when no subsequent `ReadFlag` consumes them. |

## Runtime

| Term | Definition |
|------|-----------|
| **CpuStateFrame** | The in-memory representation of all guest CPU state: 16 GPRs, flags, XMM/YMM registers, x87 stack, fs/gs bases. |
| **kHaltSentinel** | Special PC value (0) that signals the dispatcher to stop execution. |
| **SMC** | Self-Modifying Code — when guest code writes to a page containing translated blocks, those translations must be invalidated. |
| **ScopedProtected** | RAII guard used around JIT block execution for signal handler recovery. |
| **RAS** | Return Address Stack — a small predictor stack in the dispatcher that accelerates guest CALL/RET sequences. |
| **Direct threading** | Optimization where a translated block jumps directly to the next block's code, bypassing the dispatcher. |

## Cache

| Term | Definition |
|------|-----------|
| **FNV-1a** | Hash function used for content-based cache keys (detecting self-modifying code). |
| **LRU** | Least Recently Used — eviction policy for the translation cache. |
| **Byte-budget** | Maximum total bytes of translated code; triggers eviction when exceeded. |
| **Trust envelope** | SHA-256 signature over cached translations for cross-device sharing (Fase 2.5+). |

## Lean 4 / Formal Verification

| Term | Definition |
|------|-----------|
| **Lean 4** | Proof assistant and programming language used for the formal IR specification (`ir-spec/`). |
| **Lake** | Lean 4's build system (analogous to Cargo for Rust). |
| **sorry** | Lean keyword that admits an unproven proposition. Budget-controlled in CI. |
| **mathlib** | Lean's mathematics library. Currently deferred; `bv_decide` from `Std` suffices. |

## Syscall Layer

| Term | Definition |
|------|-----------|
| **Syscall dispatch** | Translation of guest x86_64 syscall numbers to host POSIX calls or emulated behaviour. |
| **CLONE_VM** | Linux clone flag indicating the child shares the parent's address space (threads). |
| **futex** | Fast Userspace Mutex — Linux synchronization primitive. Prisma maps to C++20 `std::atomic_wait`/`notify`. |
| **rt_sigaction** | Syscall to register a guest signal handler. Prisma stores these in a host-side table. |
| **rt_sigreturn** | Syscall to restore guest CPU state after a signal handler returns. |
| **strace logger** | `PRISMA_STRACE=1` environment variable enables per-syscall logging. |

## Project Structure

| Term | Definition |
|------|-----------|
| **core/** | C++20 DBT engine (decoder, IR, passes, emitter, lowering, cache, runtime). |
| **ir-spec/** | Lean 4 formal specification of the IR. |
| **shell/** | Rust workspace for the orchestrator/loader (migration in progress). |
| **android/** | Future Kotlin + Jetpack Compose Android app. |
| **server/** | Future Python P2P backend for distributed translation cache. |
| **papers/** | LaTeX drafts for academic publications. |
| **fuzz/** | AFL++ fuzzing harnesses. |
| **tools/** | Benchmark drivers, differential testing, coverage scripts. |

## Abbreviations

| Abbr | Meaning |
|------|---------|
| **AVX** | Advanced Vector Extensions (256-bit SIMD) |
| **FMA** | Fused Multiply-Add |
| **SSE** | Streaming SIMD Extensions (128-bit) |
| **TSO** | Total Store Order — x86 memory model, stricter than ARM64's relaxed model |
| **W^X** | Write XOR Execute — security policy where memory is either writable or executable, never both simultaneously |
| **DXVK** | DirectX-to-Vulkan translation layer |
| **VKD3D** | Direct3D 12 to Vulkan translation layer |
| **AVF** | Android Virtualization Framework |
| **BTCpu** | Wine's Binary Translation CPU interface (`wow64cpu.dll`) |
| **NPU** | Neural Processing Unit — dedicated AI accelerator on mobile SoCs |
