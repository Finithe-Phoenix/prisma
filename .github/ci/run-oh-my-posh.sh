#!/usr/bin/env bash
set -euo pipefail

expected_version=30.6.3
for cycle in 1 2 3; do
  set +e
  output="$(timeout 60s /opt/prisma-wine/bin/wine /opt/prisma-fixtures/oh-my-posh.exe version 2>&1)"
  status=$?
  set -e

  printf 'cycle=%s exit=%s\n' "$cycle" "$status"
  printf '%s\n' "$output"
  test "$status" -eq 0
  printf '%s\n' "$output" | grep -Fxq "$expected_version"

  /opt/prisma-wine/bin/wineserver -k
  /opt/prisma-wine/bin/wineserver -w
  rm -rf -- "$WINEPREFIX"
done

