#!/usr/bin/env bash
set -euo pipefail

out=${1:-$(dirname "$0")}
mkdir -p "$out"
out=$(cd "$out" && pwd)
rev=1f483d5b8316753a742cbf5590caf5bd0a4e4777
archive="$out/coremark-$rev.tar.gz"
if [[ ! -f "$archive" ]]; then
    curl -fL --retry 3 "https://codeload.github.com/eembc/coremark/tar.gz/$rev" -o "$archive.tmp"
    mv "$archive.tmp" "$archive"
fi
echo "4067e7f260218df13f2875d8f821a8ff32a84153c6e0759d7c89f96c0a8bf987  $archive" | sha256sum -c
build=$(mktemp -d "$out/coremark-build.XXXXXX")
trap 'rm -rf "$build"' EXIT
tar -xzf "$archive" -C "$build" --strip-components=1
make -C "$build" PORT_DIR=linux CC="${CC:-x86_64-linux-gnu-gcc}" XCFLAGS="-O2 -static" link
cp "$build/coremark.exe" "$out/coremark.x86_64"
