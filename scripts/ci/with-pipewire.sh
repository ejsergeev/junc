#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Run a command against an isolated PipeWire + WirePlumber + pipewire-pulse
# instance with the validation null devices (hardware layer 2). Never
# touches a desktop session: own XDG_RUNTIME_DIR and config dir.
# usage: scripts/ci/with-pipewire.sh COMMAND [ARGS…]
set -euo pipefail
cd "$(dirname "$0")/../.."
root=$PWD
export XDG_RUNTIME_DIR=${JUNC_TEST_RUNTIME_DIR:-$(mktemp -d /tmp/junc-live-XXXXXX)}
export XDG_CONFIG_HOME=$root/bench/validation/bench/ci-config
export LOGDIR=$XDG_RUNTIME_DIR/logs PW_SESSION=manual
# juncd keeps state and the last good config here; never the real home.
export XDG_STATE_HOME=$XDG_RUNTIME_DIR/state
export PULSE_SERVER=unix:$XDG_RUNTIME_DIR/pulse/native
# Tests and the bench restart the session through this script.
export JUNC_PW_SESSION=$root/bench/validation/bench/pw-session.sh
unset PIPEWIRE_REMOTE
chmod 700 "$XDG_RUNTIME_DIR"
trap '"$JUNC_PW_SESSION" stop >/dev/null 2>&1 || true' EXIT
"$JUNC_PW_SESSION" start
for _ in $(seq 1 100); do [ -S "$XDG_RUNTIME_DIR/pulse/native" ] && break; sleep 0.05; done
sleep 1
"$@"
