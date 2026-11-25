#!/usr/bin/env bash
# Keyboard-only walk: press a key N times, after each press print the AT-SPI focused object.
# usage: keyboard_walk.sh <key> <count> [app-substring]
source /tmp/junc-session.env
KEY=${1:-Tab}; N=${2:-30}; APP=${3:-junc}
D=$(dirname "$0")
WID=$(xdotool search --name '^JUNC$' | head -1)
xdotool windowactivate --sync "$WID" 2>/dev/null || xdotool windowfocus --sync "$WID"
for i in $(seq 1 "$N"); do
  xdotool key --window "$WID" "$KEY"; sleep 0.25
  printf '%2d %-10s ' "$i" "$KEY"; /usr/bin/python3.12 "$D/atspi_dump.py" focus "$APP"
done
