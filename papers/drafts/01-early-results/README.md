# `01-early-results/`

First Prisma paper — provisional title:
**"Prisma: A Provably-Sound, NPU-Assisted DBT for x86_64 on ARM64"**.

## Status

DRAFT skeleton (F1-AC-001 / F1-AC-002). Outline, abstract,
section scaffolding, and a stub bibliography. Numbers, figures,
and the related-work synthesis land as the implementation matures.

## Build

```sh
cd papers/drafts/01-early-results
pdflatex main && bibtex main && pdflatex main && pdflatex main
```

## Outline

1. Introduction & contributions
2. IR design (RFC 0001, RFC 0009 cross-refs)
3. Lowering and backend
4. Memory model (TSO → ARM relaxed; F1-IR-026 + RFC 0004 cross-refs)
5. Proofs (Lean 4; sorries gated by CI)
6. Implementation (test surface, language stack)
7. Early measurements *(placeholder — F1-AC-003 / F1-AC-004 fill this)*
8. Related work *(placeholder)*
9. Conclusion

## Target venue

Either ASPLOS or POPL, depending on which year the IR / passes
mature first. The proof story leans POPL; the systems / measurement
story leans ASPLOS. Decision deferred until F1-AC-004 numbers
exist.
