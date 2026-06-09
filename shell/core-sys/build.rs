// Link against the C++ core's prisma_core_c shared library (RFC 0014).
//
// PRISMA_CORE_LIB_DIR must point at the directory holding
// libprisma_core_c.so (normally core/build). When the variable is
// absent we still emit nothing and let `cargo check`/`clippy` succeed
// — only the final link of binaries/tests needs the library. Test
// runners additionally need LD_LIBRARY_PATH (or equivalent) to include
// the same directory.

fn main() {
    println!("cargo:rerun-if-env-changed=PRISMA_CORE_LIB_DIR");
    if let Ok(dir) = std::env::var("PRISMA_CORE_LIB_DIR") {
        println!("cargo:rustc-link-search=native={dir}");
        println!("cargo:rustc-link-lib=dylib=prisma_core_c");
    } else {
        println!(
            "cargo:warning=PRISMA_CORE_LIB_DIR is not set; prisma-core-sys \
             consumers will fail to link. Build the core first \
             (cmake --build core/build --target prisma_core_c) and export \
             PRISMA_CORE_LIB_DIR=$PWD/core/build"
        );
    }
}
