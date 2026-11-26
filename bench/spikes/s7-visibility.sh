#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# S7 (architecture §7.6, §12): hidden-window detection on Hyprland. The
# default GUI scenario meters a playing node; the window goes to another
# workspace, to a hidden special workspace (how
# omarchy-hyprland-window-minimize minimizes; not Omarchy's own, so the dock
# never shows the test window), the same with Hyprland's IPC out of reach
# (frame starvation alone), and closed. Each time the meter stream must be
# gone within 1 s and the GUI must take no CPU while hidden.
#
# Needs a running Hyprland session; the window opens on a temporary
# headless monitor, never on the user's screens. Audio: the isolated
# PipeWire of lib.sh only.
# usage: bench/spikes/s7-visibility.sh [JUNC_GUI]
#   (junc-gui built with: cargo build -p junc-gui --features scenarios)
set -euo pipefail
BIN=$(realpath "${1:-${CARGO_TARGET_DIR:-target}/debug/junc-gui}")
REAL_XDG=$XDG_RUNTIME_DIR
[ -n "${HYPRLAND_INSTANCE_SIGNATURE:-}" ] || { echo "s7: needs a Hyprland session" >&2; exit 1; }
source "$(dirname "$0")/lib.sh"
hypr() { XDG_RUNTIME_DIR=$REAL_XDG hyprctl "$@"; }
OUT=JUNC-S7
NODE=junc.virtual_mic_for_zoom
gui=""
cleanup() {
  set +e
  [ -n "$gui" ] && kill "$gui" 2>/dev/null
  [ -n "${src:-}" ] && kill "$src" 2>/dev/null
  hypr output remove "$OUT" >/dev/null 2>&1 || true
  spike_down
}
trap cleanup EXIT
spike_up
pw-cat -p --raw --format s16 --channels 2 --rate 48000 -P "{ node.name=$NODE }" /dev/urandom \
  >/dev/null 2>&1 & src=$!
hypr -j monitors | jq -e --arg o "$OUT" 'any(.[]; .name == $o)' >/dev/null ||
  hypr output create headless "$OUT" >/dev/null
sleep 0.5
WS=$(hypr -j monitors | jq -r --arg o "$OUT" '.[] | select(.name == $o) | .activeWorkspace.id')
echo "s7: headless monitor $OUT shows workspace $WS; renderer: Slint default"

now() { date +%s.%N; }
ms() { awk -v a="$1" -v b="$(now)" 'BEGIN { printf "%d", (b - a) * 1000 }'; }
meter() { [ -n "$(node_id junc-gui.meter)" ]; }
ticks() { awk '{ print $14 + $15 }' "/proc/$gui/stat"; }
# Waits up to $2 s for `meter` to be $1 (up|down); prints ms or "never".
wait_meter() {
  local t0; t0=$(now)
  while true; do
    if { [ "$1" = up ] && meter; } || { [ "$1" = down ] && ! meter; }; then ms "$t0"; return 0; fi
    awk -v a="$t0" -v b="$(now)" -v t="$2" 'BEGIN { exit !((b - a) > t) }' && { echo never; return 1; }
    sleep 0.02
  done
}
# Hyprland 0.56 dispatchers are Lua.
dsp() { hypr dispatch "$1" >/dev/null; }
addr() { hypr -j clients | jq -r --argjson p "$gui" 'first(.[] | select(.pid == $p) | .address) // empty'; }
move() { dsp "hl.dsp.window.move({ workspace = \"$1\", follow = false, window = \"address:$(addr)\" })"; }
launch() {
  local before; before=$(hypr -j activewindow | jq -r '.address // empty')
  dsp "hl.dsp.exec_cmd(\"env $1 PIPEWIRE_RUNTIME_DIR=$XDG_RUNTIME_DIR $BIN --scenario default --meters\", { workspace = \"$WS silent\", no_initial_focus = true })"
  gui=""
  for _ in $(seq 1 100); do gui=$(pgrep -n -x junc-gui || true); [ -n "$gui" ] && break; sleep 0.05; done
  echo "s7: junc-gui $gui started: meters up after $(wait_meter up 10 || true) ms"
  for _ in $(seq 1 40); do [ -n "$(addr)" ] && break; sleep 0.05; done
  # Never leave the user's focus on the test window.
  if [ -n "$before" ] && [ "$(hypr -j activewindow | jq -r '.address // empty')" != "$before" ]; then
    dsp "hl.dsp.focus({ window = \"address:$before\" })"
  fi
  echo "s7: window $(addr) on workspace $(hypr -j clients | jq -r --argjson p "$gui" 'first(.[] | select(.pid == $p) | .workspace.name)')"
}
# hide_show NAME WORKSPACE: hidden there, CPU while hidden, shown again.
hide_show() {
  local t
  move "$2"
  t=$(wait_meter down 1) || true
  sleep 0.5; local a; a=$(ticks); sleep 3; local b; b=$(ticks)
  echo "s7: $1: meters stopped after $t ms; hidden CPU $((b - a)) ticks in 3 s"
  move "$WS"
  echo "s7: $1: shown again, meters back after $(wait_meter up 2 || true) ms"
}

launch ""
sleep 1; a=$(ticks); sleep 5; b=$(ticks)
echo "s7: visible, metering: $((b - a)) ticks in 5 s ($(awk -v t=$((b - a)) 'BEGIN { printf "%.1f", t / 5 }') % of a core)"
hide_show "other workspace" 77
hide_show "minimized" special:junc-s7-minimized
dsp "hl.dsp.window.close({ window = \"address:$(addr)\" })"
echo "s7: closed: meters stopped after $(wait_meter down 1 || true) ms"
sleep 0.5; kill "$gui" 2>/dev/null || true

# Without Hyprland's IPC: frame starvation alone. Hyprland keeps drawing
# hidden windows at misc:render_unfocused_fps (15 by default), so here this
# is expected to find nothing: on Hyprland the IPC is what works.
echo "s7: misc:render_unfocused_fps = $(hypr getoption misc:render_unfocused_fps | awk '/int:/ { print $2 }')"
launch "-u HYPRLAND_INSTANCE_SIGNATURE"
hide_show "other workspace, frames only" 77
hide_show "minimized, frames only" special:junc-s7-minimized
dsp "hl.dsp.window.close({ window = \"address:$(addr)\" })"
sleep 0.5
