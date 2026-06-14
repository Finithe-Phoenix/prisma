# Lean Cross-Check Template (Para Gemini)

> Template para verificar que los tipos Rust del IR corresponden a la
> spec formal en Lean 4. Ejecutar después de cada cambio al IR.

## Crate: _______________
## Lean file: `ir-spec/PrismaIR/Syntax.lean`
## Rust file: `shell/prisma-*/src/lib.rs`
## Date: _______________

---

### Procedure

```bash
# 1. Build Lean spec
cd ir-spec && lake build

# 2. Run Rust tests
cd shell && cargo test --package prisma-ir

# 3. Check sorry budget
cat ir-spec/.sorry-budget
```

### Cross-Check Table

Para cada `Op` variant en Rust, verificar presencia en Lean:

| Rust variant | Lean equivalent | Match? | Notes |
|---|---|---|---|
| `Add` | `add` | ✅/❌ | |
| `Sub` | `sub` | ✅/❌ | |
| `Mul` | `mul` | ✅/❌ | |
| ... (listar todas las ~80) | | | |

### Missing in Lean (Rust has, Lean doesn't)
1. ___________
2. ___________

### Missing in Rust (Lean has, Rust doesn't)
1. ___________
2. ___________

### Derived traits check

- [ ] Lean `inductive` names match Rust `#[derive]` enum variants
- [ ] Field order matches between Lean structure and Rust struct
- [ ] Lean `Nat` ↔ Rust `u64` correspondence is documented
- [ ] Lean `BitVec n` ↔ Rust `u8`/`u16`/`u32`/`u64` mapping is documented

### Build results

- [ ] `lake build` passes
- [ ] `cargo test` passes
- [ ] No new `sorry` budget exceeded
- [ ] `.sorry-budget` current count: ____ / max: ____

### Verdict

- [ ] **SYNCED** — all types match
- [ ] **MINOR** — doc-level discrepancies, no semantic mismatch
- [ ] **MISMATCH** — semantic differences found (list below)
