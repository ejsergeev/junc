#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# GUI performance (B26, architecture §11.4): `junc-bench gui` runs the
# release junc-gui (timings on through JUNC_GUI_PERF) against juncd with ten Junctions on
# the isolated PipeWire and writes the results to
# target/gui-bench-results.json.
#
#   scripts/gui/perf.sh             a private Xvfb session (CI): tracked,
#                                   not gated (no GPU, no Hyprland)
#   scripts/gui/perf.sh --hyprland  the owner's laptop: the window on a
#                                   temporary headless monitor, never on
#                                   the user's screens; gated against
#                                   bench/gui-thresholds.toml (the PRD)
#
# Needs: cargo build --release -p juncd -p junc-bench -p junc-gui
set -euo pipefail
cd "$(dirname "$0")/../.."
target=${CARGO_TARGET_DIR:-target}
bin=$target/release
out=$target/gui-bench-results.json
[[ -x $bin/junc-gui && -x $bin/juncd && -x $bin/junc-bench ]] ||
  { echo "perf: build the release binaries first (see the header)"; exit 1; }

if [[ ${1:-} == --hyprland ]]; then
  [[ -n ${HYPRLAND_INSTANCE_SIGNATURE:-} ]] || { echo "perf: needs a Hyprland session"; exit 1; }
  # hyprctl needs the user's runtime directory; with-pipewire.sh moves ours.
  export JUNC_BENCH_HYPR_RUNTIME=$XDG_RUNTIME_DIR
  mon=JUNC-B26
  hyprctl output create headless "$mon" >/dev/null
  trap 'hyprctl output remove "$mon" >/dev/null 2>&1 || true' EXIT
  sleep 0.5
  ws=$(hyprctl -j monitors | jq -r --arg o "$mon" '.[] | select(.name == $o) | .activeWorkspace.id')
  [[ -n $ws ]] || { echo "perf: the headless monitor has no workspace"; exit 1; }
  echo "perf: window on the headless monitor $mon (workspace $ws)"
  # shellcheck disable=SC2016
  timeout -k 10 300 scripts/ci/with-pipewire.sh bash -c \
    'PIPEWIRE_RUNTIME_DIR=$XDG_RUNTIME_DIR exec "$0" gui --out "$1" --hyprland "$2"' \
    "$bin/junc-bench" "$out" "$ws" >/dev/null
  # No baseline: the PRD targets are the gate.
  "$bin/junc-bench" compare "$out" --baseline "$target/no-gui-baseline.json" \
    --thresholds bench/gui-thresholds.toml
else
  # shellcheck disable=SC2016
  timeout -k 10 300 scripts/ci/with-pipewire.sh bash -c \
    'export PIPEWIRE_RUNTIME_DIR=$XDG_RUNTIME_DIR; source scripts/gui/session.sh; "$0" gui --out "$1"' \
    "$bin/junc-bench" "$out"
fi
