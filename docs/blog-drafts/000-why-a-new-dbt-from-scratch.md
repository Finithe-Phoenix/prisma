---
title: "Why we're writing a new DBT from scratch"
status: draft
author: Danny
date: 2026-04-20
tags: [dbt, prisma, motivation, fex, box64, qemu]
---

# Why we're writing a new DBT from scratch

When you tell someone you're building a dynamic binary translator,
the first response is usually "isn't there one already?". Yes, there
are several, and they're good. FEX, Box64, QEMU's user-mode TCG,
Apple's Rosetta 2, Microsoft's xtajit. Tens of thousands of
engineering-hours each. Many of them already run impressive
workloads, including mainstream Windows games via Wine.

So Prisma needs a real answer. This is mine.

## What we're trying to do

**Prisma is a x86_64 → ARM64 dynamic binary translator built for
Windows-on-Android use cases**, written from scratch over a 48-54
month plan. The end state is: a Pixel 7a or POCO X6 Pro running an
unmodified Win32 binary at speeds that make playing it on the bus
feel reasonable.

We're not the first attempt — Winlator, GameNative, and the various
forks already get a long way. Prisma differs in five technical bets
that I'll get to below. None of them require Prisma to exist on day
one. They all require *some* DBT to exist that I have full control
over.

## The five existing DBTs and what they get right

- **FEX** — production-quality x86 → ARM64. Used by SteamOS, Asahi
  Linux. Template-based code generation, tight inner loops. Linux-
  centric.
- **Box64** — same x86 → ARM64 territory, ARM-Linux focused, ships
  in Termux and Steam Deck builds via the Box86/Box64 stack. Less
  ambitious IR, faster development pace.
- **QEMU TCG** — the granddaddy. Cross-platform, cross-architecture,
  proper IR (TCG ops). Slower than FEX/Box64 because of the IR
  abstraction tax.
- **Rosetta 2** — Apple's macOS x86 → ARM64. Best AOT performance
  in the industry. Closed source. macOS-only.
- **xtajit / xtajit64** — Microsoft's Windows-on-ARM x86 emulator.
  Closed source, Windows-only, unimpressive performance until
  Prism (Windows 11 24H2).

Each of these is the right answer to a slightly different question.
For our specific cell — Windows-on-Android, formal verifiability
where it matters, distributed cache, NPU-assisted hot-path
recognition — the existing answers don't quite fit.

## What we're betting on (the five technical pillars)

These are spelled out in
[`PROYECTO_PLAN_EJECUCION.md`](../../PROYECTO_PLAN_EJECUCION.md);
the short version:

1. **Formally verified IR.** Prisma's IR is specified in Lean 4
   (`ir-spec/`). Each pass ships with a soundness proof. The TSO-
   adaptive pass — which downgrades acquire/release loads to plain
   loads in single-threaded regions — needs the proof to ship safely
   without spending months tracking down a memory-model bug. Nobody
   else in the DBT space has done this. Blog 002 in this series
   walks through the IR design.

2. **TSO-adaptive optimisation.** x86 is total-store-order; ARM64
   is relaxed. The conservative answer (acquire+release on every
   memory op) gives you correct multithreaded behaviour at a 2-3×
   speed cost. The Prisma bet is that we can prove specific regions
   single-threaded and downgrade them. Pillar 2 (Lean) underwrites
   this.

3. **Distributed translation cache.** A cold-start of a translated
   binary costs minutes. A warm cache cuts it to seconds. Today's
   DBTs have local caches; Prisma extends that to a P2P network
   where translations cross machines. RFC 0007 + 0008 specify the
   on-disk format and zstd compression that this rests on.

4. **NPU-assisted hot-path recognition.** Modern Android SoCs ship
   an NPU. Almost nothing uses it on the PC-emulation hot path.
   Prisma's plan: a tiny model that classifies basic blocks for
   "worth aggressive optimisation" vs "translate cheaply", running
   on the NPU concurrently with the JIT. Speculative; the model
   work doesn't start until Fase 2.5.

5. **Hybrid DBT + KVM.** Where the host CPU has the right
   virtualisation hooks, run x86 in KVM-on-ARM where possible and
   fall back to translation only for the parts the hypervisor
   can't handle. Apple Virtualization Framework lays the groundwork.

The papers are downstream of these. Each pillar ends with a
peer-reviewed publication target — first-paper draft starts at the
end of Fase 2.

## "Why not fork Winlator / FEX / X?"

This came up the same week the project started. I rejected it on
**2026-04-19** after a serious review (the rejected route is
documented in
[`compass_artifact_wf-*.md`](../../compass_artifact_wf-b07eb771-9280-4242-b5b8-be65147fa39a_text_markdown.md)
in this repo).

The short reasons:

- **Forking is locked into the upstream's design choices.** FEX is
  template-based; that's a fundamental decision (RFC 0001 in our
  repo argues why we picked SSA + IR instead). Forking and rewriting
  the IR is the same work as starting from scratch.
- **The five pillars don't slot into the existing DBTs.** Adding a
  Lean spec to FEX would mean re-deriving every pass's semantics
  from C++ source — strictly more work than starting from a
  semantics-first IR. The TSO-adaptive proof has nothing to attach
  to in a template-based JIT.
- **Academic credibility.** Reviewers for VEE / MICRO / POPL want
  the artefact to be the system, not "modifications to system X".
  The papers won't get the same reception as a forked add-on.
- **Strategic timeline.** The 48-54 month plan is built around
  having full degrees of freedom on every layer. Constraining
  ourselves to upstream's licensing, release cadence, and code
  conventions burns a year of room we'd rather spend on the actual
  research bets.

The honest cost: a fork could ship something playable in weeks. We
won't. Prisma's first runnable Windows binary is months out, not
weeks. We made that trade deliberately.

## "What if the bets don't pan out?"

Each pillar has an explicit decision point in the plan. If TSO-
adaptive proves intractable, we ship the conservative version and
publish the negative result. If the NPU classifier doesn't beat a
simple heuristic, we drop the model and use the heuristic.
Honesty is in the manifesto:

> 4. **Honest failure mode.** Los decision points del plan son
>    reales. Si un pilar no funciona, publicar resultados negativos
>    honestamente. No esconder ni glosar.

A pillar failing doesn't kill Prisma. The remaining four still
give us a competitive DBT and a stack of papers. Failing five out
of five would; that's the edge case the plan acknowledges.

## What this blog is not

This is not a competitive announcement. FEX, Box64, QEMU, Winlator
— the people behind those have been generous with their work and
their writing. Prisma is downstream of all of them in the sense
that everything we know about how to do this came from reading their
code and their papers. If Prisma succeeds, it's because we stood on
the shoulders of those projects.

It is also not a recruiting pitch. The codebase is private during
Fase 1 and Fase 2. We open up around the time the first paper drops
(Fase 2.5).

## What this blog *is*

A foundational document. When someone in 2027 asks "why does Prisma
exist?", this is the post that answers. The technical posts that
follow (001 through 003 already drafted, more coming) explain *how*
we're doing it. This one explains *why*.

If you're reading this and you're a researcher in the space — DBT,
formal verification of compilers, mobile SoC architecture, NPU
software — drop me a line. The collaboration story for this project
is real, and it's much more interesting than the solo-builder story.

---

*Status: draft. Will publish to `prisma-emu.dev/blog/00-why` as the
intro post when the site goes up.*
