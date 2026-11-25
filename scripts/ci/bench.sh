#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Daemon performance gate (A17, architecture §11.4): run junc-bench against
# the release build on an isolated PipeWire, compare with the stored
# baseline for this architecture, and prove the gate catches a regression.
# Results land in target/bench-results.json (uploaded by CI).
set -euo pipefail
cd "$(dirname "$0")/../.."
bin=target/release
results=target/bench-results.json
baseline=bench/baseline/$(uname -m).json
scripts/ci/with-pipewire.sh "$bin/junc-bench" run --out "$results" >/dev/null
gate() { "$bin/junc-bench" compare "$results" --baseline "$baseline" --thresholds bench/thresholds.toml "$@"; }
gate
echo "== the gate must fail on an injected regression"
if gate --inject relink_p95_ms=20 --inject pss_kb=2; then
  echo "the gate passed an injected regression"; exit 1
fi
echo "== gate ok"
