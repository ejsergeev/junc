#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# AT-SPI check (architecture §11.3): run a GUI scenario in a private
# headless session and compare its accessibility tree (roles, names,
# descriptions, states, values) with tests/golden/atspi/<scenario>.txt.
# JUNC_BLESS=1 rewrites the golden.
# usage: scripts/gui/atspi-check.sh SCENARIO…
#   (needs junc-gui built with: cargo build -p junc-gui --features scenarios)
set -euo pipefail
cd "$(dirname "$0")/../.."
BIN=${JUNC_GUI:-${CARGO_TARGET_DIR:-target}/debug/junc-gui}
source scripts/gui/session.sh
fail=0
for s in "$@"; do
  SLINT_BACKEND=winit-software "$BIN" --scenario "$s" >"$session_tmp/gui-$s.log" 2>&1 &
  gui=$!
  got=""
  for _ in $(seq 1 30); do
    sleep 0.5
    if ! kill -0 "$gui" 2>/dev/null; then
      echo "atspi $s: junc-gui exited early:"; cat "$session_tmp/gui-$s.log"; exit 1
    fi
    got=$(python3 scripts/gui/atspi_dump.py tree junc 2>/dev/null || true)
    [[ $got == *"push button"* || $got == *"label"* ]] && break
  done
  kill "$gui" 2>/dev/null || true; wait "$gui" 2>/dev/null || true
  golden=tests/golden/atspi/$s.txt
  if [[ -n ${JUNC_BLESS:-} ]]; then
    mkdir -p tests/golden/atspi; printf '%s\n' "$got" >"$golden"
  fi
  if diff -u "$golden" <(printf '%s\n' "$got") >"$session_tmp/$s.diff"; then
    echo "atspi $s: ok"
  else
    echo "atspi $s: differs from $golden"; cat "$session_tmp/$s.diff"; fail=1
  fi
done
exit $fail
