#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Run the live PipeWire tests (hardware layer 2) in an isolated PipeWire +
# WirePlumber + pipewire-pulse instance with null devices. Never touches a
# desktop session: it uses its own XDG_RUNTIME_DIR and config dir.
# usage: scripts/ci/live-pipewire.sh [cargo test args…]
set -euo pipefail
cd "$(dirname "$0")/../.."
root=$PWD
export XDG_RUNTIME_DIR=${JUNC_TEST_RUNTIME_DIR:-$(mktemp -d /tmp/junc-live-XXXXXX)}
export XDG_CONFIG_HOME=$root/bench/validation/bench/ci-config
export LOGDIR=$XDG_RUNTIME_DIR/logs PW_SESSION=manual
export PULSE_SERVER=unix:$XDG_RUNTIME_DIR/pulse/native
unset PIPEWIRE_REMOTE
chmod 700 "$XDG_RUNTIME_DIR"
session=$root/bench/validation/bench/pw-session.sh
trap '"$session" stop >/dev/null 2>&1 || true' EXIT
"$session" start
for _ in $(seq 1 100); do [ -S "$XDG_RUNTIME_DIR/pulse/native" ] && break; sleep 0.05; done
sleep 1
JUNC_LIVE_PIPEWIRE=1 cargo test -p junc-pw --test live --locked "$@" -- --nocapture --test-threads=1
