#!/bin/bash
set -e

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
cd "$DIR"

VERSION="2.2.3"
ARCHIVE="nbench-byte-${VERSION}.tar.gz"

if [ ! -f "$ARCHIVE" ]; then
    wget -q "http://www.math.utah.edu/~mayer/linux/$ARCHIVE"
fi

if [ ! -d "nbench-byte-${VERSION}" ]; then
    tar -xf "$ARCHIVE"
fi

cd "nbench-byte-${VERSION}"
# Modify Makefile to use cross compiler and static flag
sed -i 's/^CC = gcc/CC = x86_64-linux-gnu-gcc/' Makefile
sed -i 's/^CFLAGS = -s -static -Wall -O3/CFLAGS = -s -static -Wall -O2/' Makefile
make

cp nbench ../nbench.x86_64
cd ..
echo "Built nbench.x86_64"
