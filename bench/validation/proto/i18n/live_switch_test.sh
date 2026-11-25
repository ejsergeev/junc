#!/usr/bin/env bash
# Live language switch: start with ~/.config/omarchy/language=en, rewrite it, screenshot, check AT-SPI names.
# Uses a throwaway HOME so the real Omarchy config is never touched.
set -u
source /tmp/junc-session.env
D=$(cd "$(dirname "$0")/.." && pwd); OUT=${OUT:-$D/../screenshots/live-switch}; mkdir -p "$OUT"
export HOME=$(mktemp -d); mkdir -p "$HOME/.config/omarchy"; echo en >"$HOME/.config/omarchy/language"
pkill -x junc-proto; sleep 0.3
SLINT_BACKEND=winit-software "$D/target/release/junc-proto" --no-meters >/tmp/live.log 2>&1 &
PID=$!; sleep 2.5
names() { /usr/bin/python3.12 "$D/a11y/atspi_dump.py" tree junc | grep -E "push button|label name='(Inputs|Bus|Listen|المدخلات|קלטים|Входы)" | head -3 | sed "s/^ *//"; }
echo "--- en"; names; import -window root "$OUT/0-en.png"
for l in ar ru he en; do
  t0=$(date +%s%N); printf '%s\n' "$l" >"$HOME/.config/omarchy/language.tmp" && mv "$HOME/.config/omarchy/language.tmp" "$HOME/.config/omarchy/language"
  sleep 0.6; echo "--- $l (written $(( ($(date +%s%N)-t0)/1000000 )) ms ago)"; names; import -window root "$OUT/$l.png"
done
echo "--- process still alive (no restart): $(kill -0 $PID && echo yes || echo no) pid=$PID"; grep language= /tmp/live.log
kill $PID; rm -rf "$HOME"
