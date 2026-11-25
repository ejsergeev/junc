# SPDX-License-Identifier: GPL-3.0-or-later
# Shared helpers for the alpha spikes: an isolated PipeWire + WirePlumber +
# pipewire-pulse instance with the CI null devices and no hardware monitors.
SPIKE_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
BENCH=$SPIKE_DIR/../validation/bench
export XDG_RUNTIME_DIR=${SPIKE_RUNTIME_DIR:-/tmp/junc-spike-xdg}
export XDG_CONFIG_HOME=$BENCH/ci-config
export LOGDIR=${LOGDIR:-/tmp/junc-spike-logs}
export PW_SESSION=manual
export PULSE_SERVER=unix:$XDG_RUNTIME_DIR/pulse/native
unset PIPEWIRE_REMOTE
mkdir -p "$XDG_RUNTIME_DIR" "$LOGDIR"; chmod 700 "$XDG_RUNTIME_DIR"
spike_up() { "$BENCH/pw-session.sh" start; for _ in $(seq 1 100); do [ -S "$XDG_RUNTIME_DIR/pulse/native" ] && break; sleep 0.05; done; sleep 1; }
spike_down() { "$BENCH/pw-session.sh" stop; }
node_id() { pw-dump | jq -r --arg n "$1" '.[]|select(.type=="PipeWire:Interface:Node" and .info.props["node.name"]==$n)|.id' | head -1; }
wait_node() { for _ in $(seq 1 100); do [ -n "$(node_id "$1")" ] && return 0; sleep 0.05; done; echo "timeout: $1" >&2; return 1; }
mk_node() { pw-cli create-node adapter "{ factory.name=support.null-audio-sink object.linger=true $* }" >/dev/null; }
# pw-link with retries: a node's ports appear some time after the node itself.
link() { for _ in $(seq 1 200); do pw-link "$1" "$2" 2>/dev/null && return 0; sleep 0.02; done; echo "link failed: $1 -> $2" >&2; return 1; }
