---
title: "JIT memory on Apple silicon — what actually works"
status: draft
author: Danny
date: 2026-04-20
tags: [jit, macos, apple-silicon, mmap, pthread_jit_write_protect_np]
---

# JIT memory on Apple silicon — what actually works

A JIT compiler needs to do one obviously reasonable thing: allocate a
page of memory, write instructions into it, and jump there. On most
systems this is a `mmap(..., PROT_READ|PROT_WRITE|PROT_EXEC)`. On
Apple silicon, that call returns `EACCES`, and the weeks between
"it's just memory permissions" and "finally running code" are a
trip I'd like you to skip.

Here's what we learned building Prisma's runtime, which JIT-translates
x86 bytes into ARM64 on macOS (plus Android later, but today's topic
is Apple's gnarlier restrictions).

## The constraint: W^X, hard-enforced

Apple silicon enforces **W^X** — write XOR execute, never both on the
same page simultaneously. This isn't a mitigation you can opt out of;
hardened runtime propagates the bit through `exec()` and the kernel
denies `mmap` calls that ask for `PROT_WRITE | PROT_EXEC`.

The workaround is a special flag, `MAP_JIT`. With it:

```c
void* region = mmap(nullptr, size,
                    PROT_READ | PROT_WRITE | PROT_EXEC,
                    MAP_PRIVATE | MAP_ANONYMOUS | MAP_JIT,
                    -1, 0);
```

the kernel allocates a region where you can toggle per-thread
between "writable" and "executable" via
`pthread_jit_write_protect_np(bool)`. Writable when you're populating
the code, executable when you're about to run it. The toggle is
cheap — it flips a TLS bit the kernel consults on fault — but
forgetting to toggle is what kills your day one.

Prisma's `JitBuffer::commit(span<byte>)` looks like:

```cpp
void JitBuffer::commit(std::span<const std::uint8_t> code) {
    pthread_jit_write_protect_np(/*enable=*/false);  // writable
    std::memcpy(base_, code.data(), code.size());
    sys_icache_invalidate(base_, code.size());
    pthread_jit_write_protect_np(/*enable=*/true);   // executable
}
```

Four lines. The rest of this post is every way those four lines went
wrong the first time.

## Mistake 1: forgetting `sys_icache_invalidate`

The instruction cache is not coherent with the data cache on ARM64.
Write bytes into a page via stores (data cache), try to execute them
(instruction cache) — and the CPU will cheerfully execute whatever
was at those addresses before. On x86, cache coherency is a hardware
invariant; on ARM64, invalidation is your problem.

`sys_icache_invalidate(base, size)` on macOS emits the required
`dc cvau / dsb ish / ic ivau / dsb ish / isb` sequence. On Linux
`__builtin___clear_cache(begin, end)` does the same.

What this looks like when you forget: the translated code runs
*once* correctly (first use is a cold load), then subsequent runs of
a modified region execute stale bytes. The failure mode is
spectacular and baffling until you understand the cache model.

## Mistake 2: the hardened-runtime entitlement

macOS ships hardened runtime as the default for apps, and hardened
runtime disables JIT by default. You need the entitlement
`com.apple.security.cs.allow-jit` in your app's signing
`Entitlements.plist`. Without it, `mmap` with `MAP_JIT` returns
`EACCES` silently — and by silently I mean "the errno is set but
nothing in the logs tells you it's the entitlement".

During development, `codesign --entitlements entitlements.plist -s -
./binary` after every build gets old fast. CMake can do this for
you post-link (look at Prisma's `core/CMakeLists.txt`). Ship mode
requires a real signing certificate; dev mode needs just the ad-hoc
identity `-`.

## Mistake 3: thread-local protection state

`pthread_jit_write_protect_np` is **thread-local**. Toggle it on
thread A, execute from a different thread B, and thread B's view of
the page is still "writable" — in which case the CPU raises a fault
trying to execute it.

For Prisma this shaped the design: the translator thread and the
dispatcher thread are the **same** thread. A worker translates a
block, flips the page executable, and calls into it. Wanting a
separate thread for translation (to hide latency) means duplicating
the page-protection toggle on every thread boundary. We deferred
that to Fase 2 (F1-RT-009 in the backlog) rather than fight it now.

## Mistake 4: signal recovery

Even with the entitlement, the toggle, and the invalidation, you
will eventually jump into a buggy translation and the CPU will
SIGSEGV. If your runtime does not catch that signal and longjmp back
to the translator, your emulator crashes. Every time.

Prisma installs a SIGSEGV handler that:

1. Checks whether the faulting PC is inside a JIT buffer. If not, it
   re-raises and lets the default handler run.
2. If it is, longjmps back to the `setjmp` the translator saved
   before calling into the code.
3. Marks the offending translation cache entry as stale so a
   retranslation runs on the retry.

The test file `core/tests/test_signal_handler.cpp` exercises both
SIGSEGV and SIGILL recovery. It's also the flakiest test in the
suite on macOS — the signal-handler path interacts badly with lldb
attach, with Address Sanitizer's own signal handler, and with the
kernel's occasional reluctance to let you catch a fault on a
JIT-protected page. We run it under the regular build and skip it
under sanitizers.

## Mistake 5: assuming Linux and macOS agree

macOS: `MAP_JIT` + per-thread toggle + `sys_icache_invalidate`.

Linux: plain `mmap(PROT_READ|PROT_EXEC)` on the final write,
`mprotect` to flip between PROT_WRITE and PROT_EXEC if you need it
(or `memfd_create` + dual-mapped read-only and read-write views),
and `__builtin___clear_cache`. No hardened-runtime entitlement, no
per-thread bit, and `mmap` happily accepts `PROT_WRITE | PROT_EXEC`
if you ask (though it shouldn't be your first choice).

Prisma's `jit_memory.cpp` has `#if defined(__APPLE__)` branches. It's
~150 lines. We keep it small; platform-specific code wants to be
boring.

## What the runtime looks like

```cpp
class JitBuffer {
public:
    JitBuffer(std::size_t capacity);    // mmap MAP_JIT on macOS
    ~JitBuffer();                        // munmap

    // Copy translated bytes in and make the region executable.
    // Flushes the icache; toggles the per-thread bit on macOS.
    void commit(std::span<const std::uint8_t> code);

    // Function pointer into the region. Valid after commit.
    template <typename Fn>
    Fn* as() const;

private:
    void*       base_;
    std::size_t capacity_;
};
```

40 lines of implementation. Nothing sophisticated. The only reason
to write this in 2026 is that nothing off-the-shelf gets all the
cross-platform details right for Prisma's shape. (libffi handles
calling into generated code but won't allocate JIT memory for you.
jitasm is x86-only. vixl assumes you've handled the memory.)

## Takeaways

If you're writing a JIT for Apple silicon:

1. Get the `com.apple.security.cs.allow-jit` entitlement on your
   binary **before** you write any code.
2. Always pair writes to the region with an icache invalidation.
3. Keep translation and execution on the same thread until you have
   a compelling reason not to.
4. Install a signal handler early. Test it.
5. Write the per-platform shim once, behind a clean interface, and
   don't touch it again unless the OS forces you to.

Everything else about the JIT is interesting — this part is just
annoying. Budget a week.

---

*Status: draft. Will publish to `prisma-emu.dev/blog/03-jit-memory`.*
