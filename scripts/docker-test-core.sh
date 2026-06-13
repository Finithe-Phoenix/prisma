#!/usr/bin/env bash
# Build and test the Prisma C++ core in a Linux container — natively on
# x86_64, or under QEMU on linux/arm64 to actually EXECUTE the x86->ARM64
# JIT e2e tests on a non-ARM host.
#
# Usage:
#   scripts/docker-test-core.sh            # x86_64 (image: prisma-build-lean)
#   scripts/docker-test-core.sh arm64      # linux/arm64 under QEMU
#
# The arm64 path needs the prisma-arm64-build image:
#   docker build --platform linux/arm64 -t prisma-arm64-build:latest \
#       -f docker/Dockerfile.arm64-build docker/
set -euo pipefail

ARCH="${1:-x86_64}"
REPO="$(cd "$(dirname "$0")/.." && pwd)"

case "$ARCH" in
  x86_64|amd64)
    PLATFORM="linux/amd64"; IMAGE="prisma-build-lean:latest" ;;
  arm64|aarch64)
    PLATFORM="linux/arm64"; IMAGE="prisma-arm64-build:latest" ;;
  *) echo "unknown arch: $ARCH (use x86_64 | arm64)" >&2; exit 2 ;;
esac

echo ">> core build+test on $PLATFORM ($IMAGE)"
docker run --rm --platform "$PLATFORM" \
  -v "$REPO":/work -w /work "$IMAGE" bash -lc '
    set -e
    echo "ARCH=$(uname -m)"
    cmake -S core -B /tmp/cb -G Ninja -DCMAKE_BUILD_TYPE=Debug \
          -DCMAKE_C_COMPILER=clang -DCMAKE_CXX_COMPILER=clang++
    cmake --build /tmp/cb -j"$(nproc)"
    /tmp/cb/prisma_core_tests --reporter compact
  '
