#!/bin/bash
set -e

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
cd "$DIR"

if [ ! -d "coremark-src" ]; then
    git clone https://github.com/eembc/coremark.git coremark-src
fi

cd coremark-src
make PORT_DIR=linux64 CC=x86_64-linux-gnu-gcc XCFLAGS="-O2 -static" link

cp coremark.exe ../coremark.x86_64
cd ..
echo "Built coremark.x86_64"
