/*
 * Prisma Dhrystone-style integer benchmark corpus.
 *
 * This is intentionally self-authored instead of a verbatim copy of the
 * historic Dhrystone source. It keeps the same role in the benchmark suite:
 * a small, branch-heavy, pointer-heavy integer workload that every backend
 * can compile and execute without external assets.
 */

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct Record {
  int32_t discriminator;
  int32_t enum_value;
  int32_t int_value;
  char string_value[32];
  struct Record* next;
} Record;

static uint64_t mix_u64(uint64_t x) {
  x ^= x >> 33;
  x *= 0xff51afd7ed558ccdULL;
  x ^= x >> 33;
  x *= 0xc4ceb9fe1a85ec53ULL;
  x ^= x >> 33;
  return x;
}

static int proc_1(Record* rec, Record* other, int iter) {
  rec->discriminator = (rec->discriminator + iter + other->enum_value) & 7;
  rec->int_value = (rec->int_value * 3 + other->int_value + iter) & 0x7fffffff;
  rec->enum_value ^= (rec->int_value >> (iter & 7)) & 15;
  rec->next = other;
  return rec->int_value ^ rec->enum_value ^ rec->discriminator;
}

static int proc_2(int value, const char* text) {
  int acc = value;
  for (size_t i = 0; text[i] != '\0'; ++i) {
    acc += (int)((unsigned char)text[i] * (i + 3));
    acc ^= (acc << 5) | ((int)((uint32_t)acc) >> 27);
  }
  return acc;
}

static int func_1(int lhs, int rhs) {
  if ((lhs & 3) == (rhs & 3)) {
    return lhs + rhs + 17;
  }
  if ((lhs & 1) != 0) {
    return lhs - rhs + 23;
  }
  return rhs - lhs + 31;
}

int main(int argc, char** argv) {
  int iterations = 50000;
  if (argc > 1) {
    iterations = atoi(argv[1]);
    if (iterations <= 0) {
      fprintf(stderr, "iterations must be positive\n");
      return 2;
    }
  }

  Record a = {1, 2, 3, "PRISMA-BENCH-A", NULL};
  Record b = {2, 3, 5, "PRISMA-BENCH-B", &a};
  a.next = &b;

  uint64_t checksum = 0x123456789abcdef0ULL;
  int acc = 0;

  for (int i = 0; i < iterations; ++i) {
    Record* first = (i & 1) ? &a : &b;
    Record* second = first->next;
    acc += proc_1(first, second, i);
    acc ^= proc_2(first->int_value + i, second->string_value);
    acc += func_1(first->enum_value, second->discriminator);
    checksum ^= mix_u64((uint64_t)(uint32_t)acc + (uint64_t)i);
    if ((i & 15) == 0) {
      char tmp[32];
      snprintf(tmp, sizeof(tmp), "ITER-%08x", (unsigned)i);
      memcpy(first->string_value, tmp, sizeof(tmp));
      first->string_value[sizeof(first->string_value) - 1] = '\0';
    }
  }

  printf("prisma_dhrystone iterations=%d checksum=%016llx acc=%d\n",
         iterations,
         (unsigned long long)checksum,
         acc);
  return 0;
}
