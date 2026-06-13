#!/usr/bin/env bash
# Build and test the Prisma C++ core in a Linux container.
#
# Usage:
#   scripts/docker-test-core.sh              # native x86_64 (prisma-build-lean)
#   scripts/docker-test-core.sh arm64-cross  # cross-compile aarch64, run under
#                                            #   qemu-user (FAST: native compile,
#                                            #   only execution emulated)
#   scripts/docker-test-core.sh arm64        # full linux/arm64 under QEMU
#                                            #   (SLOW: emulated compile too)
#
# arm64-cross is the recommended way to EXECUTE the real x86->ARM64 JIT e2e
# tests on a non-ARM host: it cross-compiles with clang (native speed) and runs
# the resulting aarch64 binary under qemu-aarch64-static. The full `arm64` mode
# emulates the whole compile (~hours) and is kept only as a fallback.
set -euo pipefail

ARCH="${1:-x86_64}"
REPO="$(cd "$(dirname "$0")/.." && pwd)"

run_native() {
  echo ">> core build+test on linux/amd64 (prisma-build-lean)"
  docker run --rm --platform linux/amd64 -v "$REPO":/work -w /work \
    prisma-build-lean:latest bash -lc '
      set -e
      echo "ARCH=$(uname -m)"
      cmake -S core -B /tmp/cb -G Ninja -DCMAKE_BUILD_TYPE=Debug \
            -DCMAKE_C_COMPILER=clang -DCMAKE_CXX_COMPILER=clang++
      cmake --build /tmp/cb -j"$(nproc)"
      /tmp/cb/prisma_core_tests --reporter compact
    '
}

run_arm64_cross() {
  echo ">> core cross-compile aarch64 + run under qemu-aarch64-static"
  docker run --rm -v "$REPO":/work -w /work prisma-build-lean:latest bash -lc '
    set -e
    export QEMU_LD_PREFIX=/usr/aarch64-linux-gnu
    apt-get update -qq >/tmp/apt.log 2>&1
    apt-get install -y -qq g++-aarch64-linux-gnu qemu-user-static >>/tmp/apt.log 2>&1
    cat > /tmp/tc.cmake <<EOF
set(CMAKE_SYSTEM_NAME Linux)
set(CMAKE_SYSTEM_PROCESSOR aarch64)
set(CMAKE_C_COMPILER clang)
set(CMAKE_CXX_COMPILER clang++)
set(CMAKE_C_COMPILER_TARGET aarch64-linux-gnu)
set(CMAKE_CXX_COMPILER_TARGET aarch64-linux-gnu)
set(CMAKE_CXX_FLAGS_INIT "--gcc-toolchain=/usr -isystem /usr/aarch64-linux-gnu/include")
set(CMAKE_C_FLAGS_INIT "--gcc-toolchain=/usr")
set(CMAKE_EXE_LINKER_FLAGS_INIT "-fuse-ld=/usr/bin/aarch64-linux-gnu-ld")
set(CMAKE_FIND_ROOT_PATH /usr/aarch64-linux-gnu)
set(CMAKE_CROSSCOMPILING_EMULATOR /usr/bin/qemu-aarch64-static)
EOF
    cmake -S core -B /tmp/cbx -G Ninja -DCMAKE_BUILD_TYPE=Debug \
          -DCMAKE_TOOLCHAIN_FILE=/tmp/tc.cmake -DCMAKE_CXX_SCAN_FOR_MODULES=OFF
    cmake --build /tmp/cbx -j"$(nproc)"
    echo "ARCH=$(uname -m) running aarch64 binary under qemu:"
    LD_LIBRARY_PATH=/tmp/cbx qemu-aarch64-static /tmp/cbx/prisma_core_tests --reporter compact
  '
}

run_arm64_emulated() {
  echo ">> core build+test on linux/arm64 under QEMU (slow; needs prisma-arm64-build)"
  docker run --rm --platform linux/arm64 -v "$REPO":/work -w /work \
    prisma-arm64-build:latest bash -lc '
      set -e
      echo "ARCH=$(uname -m)"
      cmake -S core -B /tmp/cb -G Ninja -DCMAKE_BUILD_TYPE=Debug \
            -DCMAKE_C_COMPILER=clang -DCMAKE_CXX_COMPILER=clang++ \
            -DCMAKE_CXX_SCAN_FOR_MODULES=OFF
      cmake --build /tmp/cb -j"$(nproc)"
      /tmp/cb/prisma_core_tests --reporter compact
    '
}

case "$ARCH" in
  x86_64|amd64)   run_native ;;
  arm64-cross)    run_arm64_cross ;;
  arm64|aarch64)  run_arm64_emulated ;;
  *) echo "unknown arch: $ARCH (use x86_64 | arm64-cross | arm64)" >&2; exit 2 ;;
esac
