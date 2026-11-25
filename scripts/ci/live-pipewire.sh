#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Run the live PipeWire tests (hardware layer 2) against an isolated
# instance (scripts/ci/with-pipewire.sh).
# usage: scripts/ci/live-pipewire.sh [cargo test args…]
set -euo pipefail
cd "$(dirname "$0")/../.."
exec scripts/ci/with-pipewire.sh env JUNC_LIVE_PIPEWIRE=1 \
  cargo test -p junc-pw -p juncd -p junc-cli --test live --test dbus --test cli --locked "$@" \
  -- --nocapture --test-threads=1
