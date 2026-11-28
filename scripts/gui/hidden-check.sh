#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Hidden-window gate (architecture §7.6; PRD "CPU when hidden 0 %"): the
# default scenario meters a playing node; a stand-in Hyprland moves the
# window to another workspace; within 1 s the meter stream and its links
# must be gone, then the GUI process must take no CPU time at all for 3 s;
# moved back, the meters must return within 1 s.
# usage: scripts/ci/with-pipewire.sh scripts/gui/hidden-check.sh
#   (needs junc-gui built with: cargo build -p junc-gui --features scenarios)
set -euo pipefail
cd "$(dirname "$0")/../.."
BIN=${JUNC_GUI:-${CARGO_TARGET_DIR:-target}/debug/junc-gui}
# session.sh moves XDG_RUNTIME_DIR; PipeWire stays where it is.
export PIPEWIRE_RUNTIME_DIR=${PIPEWIRE_RUNTIME_DIR:-$XDG_RUNTIME_DIR}
[[ -S $PIPEWIRE_RUNTIME_DIR/pipewire-0 ]] || { echo "hidden: run under scripts/ci/with-pipewire.sh"; exit 1; }
source scripts/gui/session.sh
export HYPRLAND_INSTANCE_SIGNATURE=junc-hidden-check
# The node.name juncd gives the scenario's Junction source.
node=junc.virtual_mic_for_zoom

coproc HYPR { exec python3 -u scripts/gui/fake_hyprland.py 2>"$session_tmp/hypr.log"; }
session_pids+=("$HYPR_PID")
read -r _ <&"${HYPR[0]}"
hypr() { echo "$*" >&"${HYPR[1]}"; read -r _ <&"${HYPR[0]}"; }

pw-cat -p --raw --format s16 --channels 2 --rate 48000 -P "{ node.name=$node }" /dev/urandom \
  >/dev/null 2>&1 & session_pids+=($!)
SLINT_BACKEND=winit-software "$BIN" --scenario default --meters >"$session_tmp/gui.log" 2>&1 &
gui=$!
session_pids+=("$gui")
hypr pid "$gui"

# Links into the meter stream (0 when it is gone).
meter_links() {
  pw-dump 2>/dev/null | python3 -c '
import json, sys
objs = json.load(sys.stdin)
ids = {o["id"] for o in objs if o.get("info", {}).get("props", {}).get("node.name") == "junc-gui.meter"}
print(sum(1 for o in objs if o["type"].endswith("Link") and o["info"].get("input-node-id") in ids) + 10 * len(ids))'
}
# User + system clock ticks of the whole GUI process.
ticks() { awk '{print $14 + $15}' "/proc/$gui/stat"; }
# Waits up to $2 seconds for `meter_links` to satisfy the test in $1;
# prints the time it took in ms.
wait_links() {
  local start=$EPOCHREALTIME
  while true; do
    local n; n=$(meter_links)
    if (( n $1 )); then awk -v a="$start" -v b="$EPOCHREALTIME" 'BEGIN{printf "%d", (b-a)*1000}'; return 0; fi
    if awk -v a="$start" -v b="$EPOCHREALTIME" -v t="$2" 'BEGIN{exit !((b-a) > t)}'; then
      echo "never (links: $n)"; return 1
    fi
    kill -0 "$gui" 2>/dev/null || { echo "junc-gui exited:"; cat "$session_tmp/gui.log"; return 1; }
    sleep 0.05
  done
}

fail() { echo "hidden: $*"; cat "$session_tmp/gui.log"; exit 1; }
t=$(wait_links ">= 12" 10) || fail "the meter never started: $t"
echo "hidden: meters running after start (${t} ms)"
a=$(ticks); sleep 3; b=$(ticks)
echo "hidden: visible, metering: $((b - a)) ticks in 3 s"

hypr ws 2
t=$(wait_links "== 0" 1) || fail "meters still running 1 s after the window left: $t"
echo "hidden: meter stream and links gone ${t} ms after the window left"
sleep 0.5
# Each thread's ticks, time on CPU (ns, schedstat) and context switches
# before and after, to name the one that woke. Ticks alone can hide it:
# the process's count adds up the threads' exact times, theirs are each
# rounded down.
threads() {
  for task in /proc/$gui/task/*; do
    local ns sw
    ns=$(cut -d' ' -f1 "$task/schedstat" 2>/dev/null)
    sw=$(awk '/ctxt_switches/ {n += $2} END {print n}' "$task/status" 2>/dev/null)
    echo "${task##*/} $(tr " " _ <"$task/comm" 2>/dev/null) $(awk '{print $14 + $15}' "$task/stat" 2>/dev/null) ${ns:-0} ${sw:-0}"
  done
}
before=$(threads)
a=$(ticks); sleep 3; b=$(ticks)
after=$(threads)
(( b == a )) || {
  echo "  per thread in those 3 s (tid name: ticks, CPU time, context switches):"
  while read -r tid name t ns sw; do
    read -r ot ons osw < <(awk -v id="$tid" '$1 == id {print $3, $4, $5}' <<<"$before") || true
    if [[ -z ${ot:-} ]]; then
      echo "  $tid $name: new, $t ticks, $((ns / 1000)) us, $sw switches  <- started in those 3 s"
    else
      echo "  $tid $name: $ot -> $t ticks, +$(((ns - ons) / 1000)) us, +$((sw - osw)) switches$( ((ns != ons)) && echo '  <- ran')"
    fi
    ot=
  done <<<"$after"
  # A thread that ended meanwhile leaves its time to the process alone.
  while read -r tid name t _; do
    grep -q "^$tid " <<<"$after" || echo "  $tid $name: $t ticks -> exited  <- ended in those 3 s"
  done <<<"$before"
  fail "the hidden window took $((b - a)) ticks of CPU in 3 s"
}
echo "hidden: no CPU in 3 s while hidden"

hypr ws 1
t=$(wait_links ">= 12" 1) || fail "meters not back 1 s after the window returned: $t"
echo "hidden: meters back ${t} ms after the window returned"
echo "hidden: ok"
