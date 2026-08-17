#!/usr/bin/env bash
set -euo pipefail

cleanup_prefix() {
  /opt/prisma-wine/bin/wineserver -k >/dev/null 2>&1 || true
  /opt/prisma-wine/bin/wineserver -w >/dev/null 2>&1 || true
  rm -rf -- "$WINEPREFIX"
}

trap cleanup_prefix EXIT

expected_version=30.6.3
for cycle in 1 2 3; do
  system32="$WINEPREFIX/drive_c/windows/system32"
  mkdir -p "$system32"
  cp /opt/prisma-wine/lib/wine/aarch64-windows/xtajit64.dll \
    "$system32/xtajit64.dll"

  set +e
  output="$(timeout 60s /opt/prisma-wine/bin/wine /opt/prisma-fixtures/oh-my-posh.exe version 2>&1)"
  status=$?
  set -e

  printf 'cycle=%s exit=%s\n' "$cycle" "$status"
  printf '%s\n' "$output"
  test "$status" -eq 0
  printf '%s\n' "$output" | grep -Fxq "$expected_version"

  cleanup_prefix
done
