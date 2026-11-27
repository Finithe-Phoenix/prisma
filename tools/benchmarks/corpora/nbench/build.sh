#!/usr/bin/env bash
set -euo pipefail

out=${1:-$(dirname "$0")}
mkdir -p "$out"
out=$(cd "$out" && pwd)
archive="$out/nbench-byte-2.2.3.tar.gz"
if [[ ! -f "$archive" ]]; then
    curl -fL --retry 3 https://www.math.utah.edu/~mayer/linux/nbench-byte-2.2.3.tar.gz -o "$archive.tmp"
    mv "$archive.tmp" "$archive"
fi
echo "723dd073f80e9969639eb577d2af4b540fc29716b6eafdac488d8f5aed9101ac  $archive" | sha256sum -c
build=$(mktemp -d "$out/nbench-build.XXXXXX")
trap 'rm -rf "$build"' EXIT
tar -xzf "$archive" -C "$build" --strip-components=1
make -C "$build" CC="${CC:-x86_64-linux-gnu-gcc}" CFLAGS="-std=gnu89 -O2 -static -fcommon"
cp "$build/nbench" "$out/nbench.x86_64"
cp "$build/NNET.DAT" "$out/"
cp "$(dirname "$0")/SMOKE.CFG" "$out/"
