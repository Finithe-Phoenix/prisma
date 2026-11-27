# nbench

The harness builds Uwe F. Mayer's BYTEmark 2.2.3 release and verifies its
archive SHA-256. The CI workload is the numeric-sort test with a one-second
minimum sample interval. nbench calibrates and repeats the workload internally.

Run `bash build.sh /tmp/prisma-nbench` on Linux x86_64, then run
`./nbench.x86_64 -cSMOKE.CFG` from that output directory. Invoking the binary
without arguments runs the full upstream suite and uses the bundled NNET.DAT.
See the [benchmark guide](../../README.md).
