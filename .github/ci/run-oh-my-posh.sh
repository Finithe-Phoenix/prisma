#!/usr/bin/env bash
set -euo pipefail

readonly expected_wineprefix=/tmp/prisma-prefix
test "${WINEPREFIX:?WINEPREFIX must be set}" = "$expected_wineprefix"

cleanup_prefix() {
  /opt/prisma-wine/bin/wineserver -k >/dev/null 2>&1 || true
  /opt/prisma-wine/bin/wineserver -w >/dev/null 2>&1 || true
  rm -rf -- "$expected_wineprefix"
}

runtime_dir="$(mktemp -d /tmp/prisma-phase1.XXXXXX)"
cleanup_runtime() {
  cleanup_prefix
  rm -rf -- "$runtime_dir"
}

trap cleanup_runtime EXIT

expected_version=30.6.3
expected_stdout="$runtime_dir/expected.stdout"
printf '%s\n' "$expected_version" >"$expected_stdout"
for cycle in 1 2 3; do
  system32="$WINEPREFIX/drive_c/windows/system32"
  mkdir -p "$system32"
  cp /opt/prisma-wine/lib/wine/aarch64-windows/xtajit64.dll \
    "$system32/xtajit64.dll"

  timeout --kill-after=10s 60s \
    /opt/prisma-wine/bin/wine wineboot.exe --init
  cmp /opt/prisma-wine/lib/wine/aarch64-windows/xtajit64.dll \
    "$system32/xtajit64.dll"

  stdout_file="$runtime_dir/cycle-$cycle.stdout"
  stderr_file="$runtime_dir/cycle-$cycle.stderr"

  set +e
  timeout --kill-after=10s 60s \
    /opt/prisma-wine/bin/wine \
    /opt/prisma-fixtures/oh-my-posh.exe version \
    >"$stdout_file" 2>"$stderr_file"
  status=$?
  set -e

  printf 'cycle=%s exit=%s\n' "$cycle" "$status"
  printf 'cycle=%s stdout-begin\n' "$cycle"
  cat "$stdout_file"
  printf 'cycle=%s stdout-end\n' "$cycle"
  printf 'cycle=%s stderr-begin\n' "$cycle"
  cat "$stderr_file"
  printf 'cycle=%s stderr-end\n' "$cycle"
  if [[ "$status" -ne 0 ]]; then
    slow_alias_trace="$(
      awk '
        /prisma-trace: morestack-rip=0x0*1400221a5/ { remaining = 13 }
        remaining > 0 {
          sub(/\r$/, "")
          sub(/^.*prisma-trace: /, "")
          printf "%s%s", separator, $0
          separator = "; "
          remaining--
        }
      ' "$stderr_file"
    )"
    if [[ -n "$slow_alias_trace" ]]; then
      printf '::error title=PRISMA slow heap alias::%s\n' "$slow_alias_trace"
    else
      printf '%s\n' \
        '::error title=PRISMA slow heap alias::event 0x1400221a5 missing'
    fi
  fi
  test "$status" -eq 0
  cmp "$expected_stdout" "$stdout_file"
  ! grep -Fq 'prisma-error:' "$stderr_file"

  cleanup_prefix
done
