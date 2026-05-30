# Prisma Dhrystone-Style Corpus

`dhry_prisma.c` is a small self-authored C workload for the first benchmark
harness slice. It is not a verbatim copy of the historic Dhrystone source.

The goal is to provide the same early benchmark shape: a deterministic,
branch-heavy, pointer-heavy integer program that can be compiled locally and
run under native, QEMU, Box64, FEX, and eventually Prisma.

The program accepts one optional positional argument:

```sh
./dhry_prisma 50000
```

It prints a stable summary line containing the iteration count, checksum, and
accumulator so runners can capture output tails without parsing an external
format.
