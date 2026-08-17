# Prisma Wine ARM64EC patch set

This directory versions Prisma's changes on top of the pinned Wine 11.14
submodule. The submodule itself must remain clean; build tooling applies the
patches from `patches/` to a reproducible source archive.

## Initial x64 thread entry

Wine's ARM64EC `RtlUserThreadStart` previously dispatched the native ARM64
`BaseThreadInitThunk`. That thunk then called the x64 application entrypoint
with a direct `blr`, bypassing `__os_arm64x_dispatch_icall`. The Prisma provider
therefore initialized successfully but never received the x64 transition, and
Wine failed on a null execute access.

`0001-prisma-no-preload-reserve.patch` now sends the application entrypoint
through Wine's hybrid indirect-call dispatcher and terminates the thread through
`RtlExitUserThread` after the x64 routine returns. The patch also preserves the
bounded preloader-reservation override needed by QEMU user mode.

The Wine Dockerfile disassembles the installed `RtlUserThreadStart` and rejects
the old thunk signature. The local artifact manifest also binds the Wine source,
Dockerfile and patch hashes, so a stale runtime cannot pass as a cache hit after
the patch changes. Run the lightweight source checks with:

```powershell
py -3.12 -B -m unittest third_party/wine-prisma/test_patch.py -v
```

These checks prove patch applicability and the compiled-artifact gate, not full
compatibility. `F3-WN-019` remains open until official Oh My Posh 30.6.3 prints
the exact version with exit code zero across three clean Wine lifecycles.
