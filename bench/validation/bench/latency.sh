#!/usr/bin/env bash
# Latency added by the junction, measured two ways on two paths:
#   baseline : -> fake.hp:playback_FL ... fake.hp:monitor_FL
#   junction : -> junc.bus:playback_FL -> bus monitor -> listen loopback -> fake.hp ... fake.hp:monitor_FL
# Tools: jack_iodelay (single client, i.e. a graph cycle) and impulse_latency.py (separate TX/RX
# clients, no cycle). Extra quanta = junction - baseline, using the same tool.
# usage: latency.sh [seconds]
set -u
export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/tmp/xdg}
D=$(cd "$(dirname "$0")" && pwd); SECS=${1:-6}
iodelay() { # $1 = out target port
  stdbuf -o0 pw-jack jack_iodelay >/tmp/iodelay.out 2>&1 &   # prints with \r, unbuffer it
  local pid=$!
  for _ in $(seq 1 100); do pw-link -o 2>/dev/null | grep -q "^jack_delay:out" && break; sleep 0.05; done
  pw-link "jack_delay:out" "$1"; pw-link "fake.hp:monitor_FL" "jack_delay:in"
  sleep "$SECS"; kill "$pid"; wait "$pid" 2>/dev/null
  local r; r=$(tr "\r" "\n" </tmp/iodelay.out | grep -E "frames .*roundtrip" | tail -1 | awk '{print $1}')
  echo "${r:-NA}"
}
impulse() { pw-jack python3 "$D/impulse_latency.py" "$1" fake.hp:monitor_FL "$SECS" 2>/dev/null | tail -1; }
Q=$(pw-metadata -n settings 0 clock.quantum 2>/dev/null | sed -nE "s/.*value:'([0-9]+)'.*/\1/p"); Q=${Q:-1024}
echo "quantum=$Q"
ib=$(iodelay fake.hp:playback_FL); ij=$(iodelay junc.bus:playback_FL)
echo "jack_iodelay baseline_frames=$ib junction_frames=$ij"
pb=$(impulse fake.hp:playback_FL); pj=$(impulse junc.bus:playback_FL)
echo "impulse      baseline: $pb"; echo "impulse      junction: $pj"
fb=$(echo "$pb" | sed -nE 's/^frames=([0-9]+).*/\1/p'); fj=$(echo "$pj" | sed -nE 's/^frames=([0-9]+).*/\1/p')
if [ -n "$fb" ] && [ -n "$fj" ]; then awk -v b="$fb" -v j="$fj" -v q="$Q" 'BEGIN{printf "RESULT extra_frames=%d extra_quanta=%.2f\n", j-b, (j-b)/q}'
else echo "RESULT extra_frames=NA"; fi
