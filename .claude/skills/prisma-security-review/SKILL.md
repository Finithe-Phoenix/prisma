---
name: prisma-security-review
description: Threat-model Prisma changes against the project's actual sensitive surfaces — JIT W^X / MAP_JIT transitions, signal handlers (SIGSEGV/SIGILL/SIGBUS), cache binary deserialization (RFC 0007 format), translation_cache key collisions (FNV-1a), and future P2P signed envelopes (Ed25519, Fase 2.5). Use when touching runtime/, cache/, or anything that crosses the host-to-guest trust boundary.
---

# Prisma security review

Prisma has a small but sharp attack surface. This skill catalogues the surfaces and the failure modes to look for.

## Surface 1 — JIT memory (`core/src/runtime/jit_memory.cpp`, `core/include/prisma/jit_memory.hpp`)

### Threats
- **W^X violation**: writeable + executable simultaneously. Linux: `mmap(PROT_READ|PROT_WRITE|PROT_EXEC)` is illegal under hardened kernels (`vm.mmap_min_addr`, SELinux, grsecurity). macOS: must use `MAP_JIT` + `pthread_jit_write_protect_np` toggle, never both bits at once.
- **Use-after-free of the JIT region** while another thread executes it.
- **Page-size assumption**: hard-coding 4 KiB breaks on Apple Silicon (16 KiB), some ARM64 servers (64 KiB).

### What to check on a diff
- mprotect flags only transition R/W ↔ R/X, never R/W/X.
- On macOS code paths, every write to the buffer is bracketed by `pthread_jit_write_protect_np(false)` / `(true)`.
- Capacity is page-rounded via `sysconf(_SC_PAGESIZE)` or equivalent — not a hard-coded constant.
- `make_executable()` is the only path that flips to R/X. No second call site bypasses it.

## Surface 2 — Signal handlers (`core/src/runtime/signal_handler.cpp`)

### Threats
- **Async-signal-safety**: handler code calling `malloc`, `printf`, `std::*` is undefined behavior. Linux `signal-safety(7)` lists what's allowed.
- **`setjmp`/`longjmp` past C++ destructors** — destructors do not run on longjmp. RAII state leaks. The `ScopedProtected` wrapper exists for this; verify it's used.
- **Re-entrancy**: SIGSEGV inside the handler.

### What to check
- Handler bodies only call sigsafe APIs.
- No `std::cerr`, `std::cout`, no allocation.
- `siginfo_t` is only consulted, never mutated.
- Handler installation uses `sigaction` with `SA_SIGINFO`, not `signal()`.

## Surface 3 — Cache binary deserialization (`core/src/cache/translation_cache.cpp`, RFC 0007)

### Threats
- A malicious or corrupted cache file is **memory that becomes executable**. Deserialization bugs here are RCE-equivalent.
- Integer-overflow in `entry_count * sizeof(entry)`, `code_size` masquerade.
- TOCTOU between header validation and entry reading.

### What to check (RFC 0007 § Error handling)
- `load_from_file` returns `IoError` without mutating cache state on any error path.
- Header validated before any entry bytes read: `magic == 0x4843'4143'4D53'5250` ("PRSMCACH" LE), `version == 1`.
- `entry_count` validated against remaining file bytes (Truncated error).
- `code_size` per entry bounded — does the reader cap it? An attacker setting `code_size = 0xFFFFFFFFFFFFFFFF` should not allocate that.
- `cpu_fingerprint` slot: today it's `0`; once Fase 2.5 fills it, a mismatch must reject the cache. **Do not ship cache sharing without this check** (RFC 0007 § Costs).

### What to test
- `test_translation_cache.cpp`'s "load leaves cache unchanged on error" case must still pass on every PR touching the cache deserializer.
- Fuzz the binary loader with AFL++ (extend `fuzz/decoder/` style harness to `fuzz/cache/`).

## Surface 4 — Translation cache key collisions

The cache is keyed by `(guest_addr, FNV-1a 64 of guest bytes)`. FNV-1a is fast but not cryptographic.

### Threats
- A guest with attacker-controlled code at address X can construct a code blob whose FNV-1a collides with a legitimate translation. The collision serves the attacker's pre-cached translation.
- This matters for shared caches (Fase 2.5+). Today (single-process), an attacker who can write to `(guest_addr, hash)` already has full RCE — so FNV-1a is fine.

### What to check
- Any code path that lets *external* (peer or CDN) cache entries enter the live cache must validate via the SHA-256 envelope (RFC 0007 § Forward-compat path).
- Don't change FNV-1a to a fancy crypto hash for performance theater — the threat is signing, not hashing.

## Surface 5 — P2P signed envelopes (Fase 2.5, RFC to be written)

Not yet implemented. When designing:

- **Ed25519 signatures over the body** (the cache file as RFC 0007 v1 specifies).
- Public keys distributed via a curated set (initial), then Web of Trust later.
- Reject envelopes with unknown signer, expired signature, or fingerprint mismatch.
- Revocation list mechanism — bad signer key must be evictable without an app update.
- Never load body bytes before envelope verification.

## Surface 6 — Syscall translation layer (Fase 2)

Not yet implemented. When it lands:

- **`clone()`, `mmap()`, `futex()`** are the dangerous ones per the plan.
- Path translation: guest paths must be confined to the container prefix. No `..` escape.
- `ioctl()` flag pass-through to host: deny by default, allowlist known-safe codes.
- `ptrace()`: disable entirely or sandbox-confine.

## Process

1. Identify which of the 6 surfaces the diff touches (often: cache + runtime).
2. For each touched surface, walk the threats list. PASS / FAIL / N/A.
3. If the diff touches deserialization or signal handlers, demand a fuzz harness update or new test case.
4. If the diff touches MAP_JIT or mprotect: verify both Linux and macOS code paths separately.
5. Output: numbered list of findings with file:line and concrete remediation.

## Output format

```
Security review — <commit range or files>

Surfaces touched: <list>

Findings:
1. [SEV: high/med/low] <file:line> — <issue> — <remediation>
...

Recommendation: <ship / hold for fix / block on test>
```
