#!/usr/bin/env bash
# Run Orca with a debug log, drive the prototype by keyboard, extract what Orca would speak.
# Works on X11 (Xvfb). On Hyprland run the same key sequence by hand with `orca --debug-file`.
set -u
source /tmp/junc-session.env
D=$(cd "$(dirname "$0")" && pwd); BIN="$D/../target/release/junc-proto"
OUT=${OUT:-$D}; PY=${PY:-python3.12}
pkill -f "bin/orca" ; pkill -x junc-proto; sleep 0.5
rm -f /tmp/orca-debug.log
$PY /usr/bin/orca --replace --debug-file=/tmp/orca-debug.log >/tmp/orca.out 2>&1 &
sleep 6
SLINT_BACKEND=winit-software JUNC_LANG=${JUNC_LANG:-en} "$BIN" --no-meters >/tmp/proto-orca.log 2>&1 &
sleep 3
WID=$(xdotool search --name '^JUNC$' | head -1); xdotool windowfocus --sync "$WID"; sleep 1
k() { xdotool key --window "$WID" "$@"; sleep 1.2; }
for i in 1 2 3 4; do k Tab; done   # -> Enable Firefox (switch)
k space                            # toggle off
k Tab                              # -> slider
k Right                            # value change
k shift+Tab shift+Tab              # -> Add input…
k Return                           # open popover
k Tab; k Escape                    # trap + close
sleep 2
grep -E "SPEECH OUTPUT|speech.speak|SPEECH:" /tmp/orca-debug.log | sed -E 's/^[^A-Z]*//' | grep -v "^$" > "$OUT/orca-speech-${JUNC_LANG:-en}.txt"
wc -l "$OUT/orca-speech-${JUNC_LANG:-en}.txt"
pkill -x junc-proto; pkill -f "bin/orca"
