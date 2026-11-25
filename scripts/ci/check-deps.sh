#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Enforce the dependency rules of docs/architecture.md §2 on the full
# (transitive) dependency tree of each crate. Exit 1 on any violation.
set -euo pipefail
cd "$(dirname "$0")/../.."

# crate -> space-separated forbidden crates (matched by exact name or prefix*)
declare -A FORBID=(
  [junc-core]="pipewire libspa zbus slint i-slint-* winit"
  [junc-ipc]="pipewire libspa slint i-slint-* winit"
  [junc-pw]="zbus slint i-slint-* winit"
  [junc-l10n]="pipewire libspa slint i-slint-* winit"
  [juncd]="slint i-slint-* winit femtovg skia-* ksni"
  [junc-cli]="pipewire libspa slint i-slint-* winit"
)

fail=0
for crate in "${!FORBID[@]}"; do
  deps=$(cargo tree --locked -p "$crate" -e normal,build --prefix none --format '{p}' \
         | awk '{print $1}' | sort -u)
  for pat in ${FORBID[$crate]}; do
    hits=$(grep -E "^${pat//\*/.*}$" <<<"$deps" || true)
    if [[ -n $hits ]]; then
      echo "FORBIDDEN: $crate depends on $(tr '\n' ' ' <<<"$hits")(rule: $pat)"
      fail=1
    fi
  done
done
(( fail == 0 )) && echo "dependency rules: ok (${#FORBID[@]} crates checked)"
exit $fail
