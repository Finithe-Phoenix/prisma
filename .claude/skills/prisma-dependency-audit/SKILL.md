---
name: prisma-dependency-audit
description: Audit a proposed new dependency (or an audit pass over existing deps) against Prisma's RFC requirement, the no-copy-from-FEX/Box64/QEMU rule, and the license compatibility for the eventual MIT core + commercial Android app split (Fase 6 open-sourcing). Use when CMakeLists/Cargo.toml/build.gradle/lakefile.toml gains a new entry, or periodically as a sweep.
---

# Prisma dependency audit

## Why this exists

Prisma's strategy at Fase 6 splits the codebase: **core + IR spec + NPU models + graphics research are released MIT**; the **Android app and cloud services remain commercial freemium**. Any dependency added today bakes a license decision into one of those two outputs. Audit upfront, don't unwind in 2030.

Also: CLAUDE.md is explicit — *"No copiar código de FEX/Box64/QEMU al core. Inspiración sí, copia no (licencias + originalidad técnica)."*

## Currently-vendored / pulled deps

| Dep | Where | License | Used in | Status |
|---|---|---|---|---|
| vixl | core (FetchContent, BuildVixl.cmake) | BSD-3-Clause | emitter | OK for MIT |
| zstd | core (FetchContent) | BSD-3-Clause + GPLv2 dual | cache compression | OK for MIT (use BSD branch) |
| Catch2 | core tests (FetchContent) | BSL-1.0 | tests only | OK for MIT |
| mathlib | ir-spec (Lake) | Apache 2.0 | Lean proofs | OK for MIT |
| Wine | (future, Fase 3) | LGPL-2.1+ | Windows compat | **Dynamic link only**; do not statically link into core |
| DXVK | (future, Fase 4) | zlib | D3D→Vulkan | OK |
| VKD3D-Proton | (future, Fase 4) | LGPL-2.1+ | D3D12→Vulkan | Dynamic link only |
| Mesa Turnip | (future, Fase 4) | MIT | Adreno driver | OK |
| QEMU | NEVER vendored in core | GPLv2 | reference only | If used at all, isolate as separate process |

## Audit checklist for a new dep

### 1. RFC exists
`docs/rfc/000X-<dep>-dependency.md` must exist before the FetchContent / Cargo.toml entry lands. See RFC 0008 (zstd) for the template. The RFC documents:
- Why this dep specifically (alternatives considered)
- License + version pinning rationale
- Static vs dynamic linking
- Whether it's on the hot path (decoder/runtime) or peripheral (tests/tools)
- Removal path if it goes unmaintained

### 2. License compatibility
Allowed in `core/` (will be MIT at Fase 6):
- **Permissive**: MIT, BSD-2/3, Apache 2.0, ISC, zlib, BSL-1.0, Unlicense.

Allowed only in `android/`, `server/`, `tools/`:
- **Weak copyleft (dynamic link)**: LGPL-2.1+, MPL-2.0 — only when dynamically linked.

**Disallowed in core**:
- GPL-2.0, GPL-3.0, AGPL: would force the entire core to GPL upon release.
- SSPL, Commons Clause, BUSL: not OSI-approved.
- Public-domain dedications without a fallback license: legally ambiguous in some jurisdictions (use Unlicense or MIT instead).

If a dep is dual-licensed (e.g. zstd: BSD + GPLv2): pick the BSD branch explicitly in the RFC.

### 3. No-copy rule (CLAUDE.md)
Audit any new C++ file in `core/` against:

```bash
# Heuristic: identifiers + comments that suggest the file was lifted
grep -rE '(FEXCore|box64|QEMU|tcg_|tlb_table)' core/
```

If a file looks like a near-translation of FEX/Box64/QEMU: send it back. Inspiration noted in `docs/research_notes.md` is the documented path.

### 4. Version pinning
- C++: FetchContent_Declare must pin a `GIT_TAG` (commit SHA preferred, named tag acceptable). No `main` / `master`.
- Rust: `Cargo.toml` uses exact semver `=X.Y.Z`, not `^X.Y.Z`, for security-sensitive deps; `Cargo.lock` checked in.
- Kotlin/Gradle: Gradle `dependencies { implementation("group:name:X.Y.Z") }`, lock file checked in.
- Python: `uv` or `pip-tools` with hashed requirements.
- Lean: lakefile.toml pins a `rev = "<sha>"` for git deps.

### 5. Removal path
The RFC must answer: "What if this dep goes unmaintained or has a critical CVE?" Concrete options:
- Drop the dep + replace with hand-rolled (e.g. vixl is replaceable in theory; zstd is replaceable with a slower codec).
- Vendor the source and maintain it ourselves.
- Reduce the feature it provides.

Without a removal path, deps become permanent technical debt.

### 6. Surface area minimization
If a 5MLOC library is pulled in for one helper function: copy the helper (with attribution + license header), don't pull the library. Examples:
- A 50-line FNV-1a helper does not require a hashing library.
- A 100-line SHA-256 is fine to hand-roll for the `cpu_fingerprint` slot (RFC 0007).

## Periodic sweep procedure

Quarterly (or before each phase boundary):

1. List all FetchContent_Declare, Cargo deps, Gradle deps, Lake deps.
2. For each: confirm version is current vs upstream, no unfixed CVE, license still permissive.
3. Confirm RFC exists and matches actual usage.
4. Output a table: dep | current | latest | CVE | RFC | action.

## Output format

```
Dependency audit — <date or trigger>

New deps:
1. <dep> @ <version>
   License: <SPDX>
   RFC: <path or "MISSING">
   Allowed in: core / android / server / tools / all
   Findings: <list>

Existing deps drift:
1. <dep>: <current> → <latest available> (<CVE if any>)

Action items:
1. ...
```
