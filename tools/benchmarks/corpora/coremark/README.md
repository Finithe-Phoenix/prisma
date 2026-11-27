# CoreMark

The harness builds EEMBC CoreMark from commit
`1f483d5b8316753a742cbf5590caf5bd0a4e4777` using the Linux port. The build
script verifies the archive SHA-256 before extraction. See the upstream
[CoreMark repository](https://github.com/eembc/coremark).

Run `bash build.sh /tmp/prisma-coremark` on Linux x86_64 for the standalone
binary. The harness usage and run-validation rules are in the
[benchmark guide](../../README.md).
