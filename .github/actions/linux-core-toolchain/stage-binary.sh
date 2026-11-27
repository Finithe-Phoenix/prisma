#!/usr/bin/env bash
# Stage one build product alongside the failure logs without uploading a debug tree.
set -euo pipefail
binary=$1
if [[ ! -f "$binary" ]]; then
  exit 0
fi

staging=$(mktemp -d)
trap 'rm -rf "$staging"' EXIT
objcopy --strip-debug "$binary" "$staging/program"
output="$(basename "$binary").gz"
gzip -1 -c "$staging/program" > "$staging/$output"
if (( $(stat -c %s "$staging/$output") <= 64 * 1024 * 1024 )); then
  mv "$staging/$output" "$output"
else
  echo "Diagnostic binary exceeds the 64 MiB compressed limit, retaining logs."
fi
