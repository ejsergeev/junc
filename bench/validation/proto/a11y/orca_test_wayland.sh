#!/usr/bin/env bash
# Wayland/Hyprland port of proto/a11y/orca_test.sh (wtype + focus guard). Orca speaks aloud.
set -u
P_DIR=$1; OUT=$2; BIN=$P_DIR/target/release/junc-proto; LOG=${LOG:-/tmp/junc-orca-debug.log}
pkill -x orca; pkill -x junc-proto; sleep 0.5; rm -f "$LOG"
/usr/bin/orca --replace --debug-file="$LOG" >/tmp/junc-orca.out 2>&1 & O=$!
sleep 6
SLINT_BACKEND=winit-femtovg JUNC_LANG=en "$BIN" --no-meters >/dev/null 2>&1 & P=$!
sleep 3; hyprctl dispatch focuswindow "pid:$P" >/dev/null; sleep 1
guard() { local a; a=$(hyprctl activewindow -j | jq -r .pid); [ "$a" = "$P" ] || { echo "ABORT: focus left the prototype"; kill $P $O; exit 3; }; }
k() { guard; wtype "$@"; sleep 1.6; }
for i in 1 2 3 4; do k -k Tab; done      # -> Enable Firefox (switch)
k -k space                               # toggle off
k -k Tab                                 # -> slider
k -k Right                               # value change
k -M shift -k Tab -k Tab -m shift        # -> Add input…
k -k Return                              # open popover
k -k Tab; k -k Escape                    # trap + close
sleep 2
grep -E "SPEECH OUTPUT|speech.speak|SPEECH:" "$LOG" | sed -E 's/^[^A-Z]*//' | grep -v "^$" > "$OUT"
wc -l "$OUT"; kill $P $O
