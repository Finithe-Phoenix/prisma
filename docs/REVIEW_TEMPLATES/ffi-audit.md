# FFI Audit Template

> Template para auditar el bridge C entre Rust y C++. Usar cada vez
> que se añada/modifique una función FFI.

## Crate: _______________
## C header: `core/include/prisma/_____.h`
## Rust FFI: `shell/prisma-*/src/ffi.rs`
## Date: _______________

---

### Function inventory

| Function | C header | Rust FFI | Caller (C++) | Notes |
|---|---|---|---|---|
| `prisma_*` | ✅/❌ | ✅/❌ | ✅/❌ | |
| ... | | | | |

### ABI compliance

- [ ] All `extern "C"` functions are `#[no_mangle]`
- [ ] All enums crossing FFI use `#[repr(C)]` or `#[repr(u8/32)]`
- [ ] All structs crossing FFI use `#[repr(C)]`
- [ ] No `bool` in FFI boundary (use `u8` or `c_int`)
- [ ] No `usize` in FFI boundary (use `u64` or `size_t`)
- [ ] No `String` or `Vec` in FFI boundary (use raw pointers + lengths)

### Ownership and lifetimes

For each function, document:

```
prisma_cache_create
  - params: (capacity_bytes: u64, out: *mut *mut CacheCtx)
  - ownership: caller allocates out ptr, callee allocates CacheCtx
  - freeing: prisma_cache_destroy
  - unsafe: writes to *out, assumes valid pointer
  - SAFETY: out must be non-null and point to valid memory
```

### Error handling

- [ ] All FFI functions return `prisma_status` enum
- [ ] `prisma_status` variants:
  - `PRISMA_OK = 0`
  - `PRISMA_ERR_GENERIC = 1`
  - `PRISMA_ERR_INVALID_ARG = 2`
  - `PRISMA_ERR_OUT_OF_MEMORY = 3`
  - `PRISMA_ERR_UNSUPPORTED = 4`
- [ ] Rust panics are caught with `catch_unwind` and converted to error
- [ ] Error messages propagated via out parameter or thread-local

### Thread safety

- [ ] C API can be called from multiple threads
- [ ] Internal state uses `Mutex` or `RwLock`
- [ ] Documented: "thread-safe" or "not thread-safe" per function

### Verdict

- [ ] **CLEAN** — no FFI issues
- [ ] **MINOR** — cosmetic issues only
- [️] **ISSUES FOUND** — must fix before next phase
