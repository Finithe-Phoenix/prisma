// Prisma translation facade — the integrated Rust pipeline.
//
// Combines the decoder, optimization passes, ARM64 lowerer, and translation
// cache into one entry point: bytes in, optimized ARM64 machine code out,
// memoized by (guest_addr, content hash). Mirrors the C++ `prisma_translator`
// facade. `translate` handles one guest instruction; `translate_block` chains a
// straight-line run up to the next control transfer, caching each instruction
// independently. Fusing a block into a single optimized region (which needs
// function-global SSA renumbering across instructions) is the documented
// follow-up.

#![deny(unsafe_op_in_unsafe_fn, unused_must_use)]
#![warn(clippy::all, clippy::pedantic, clippy::nursery)]
#![allow(clippy::missing_const_for_fn, clippy::must_use_candidate)]

include!("types.rs");

impl Translator {
    include!("translator_methods.rs");
}

#[cfg(test)]
mod tests;
