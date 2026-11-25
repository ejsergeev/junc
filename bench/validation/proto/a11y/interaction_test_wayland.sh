#!/usr/bin/env bash
# Wayland/Hyprland port of proto/a11y/interaction_test.sh: wtype for keys, grim for window shots.
# Aborts as soon as the focused window is not the prototype, so no key reaches another app.
set -u
P_DIR=$1; OUT=$2; mkdir -p "$OUT"
BIN=$P_DIR/target/release/junc-proto; DUMP=$P_DIR/a11y/atspi_dump.py
focus() { /usr/bin/python3 -I "$DUMP" focus junc 2>/dev/null; }
guard() { local a; a=$(hyprctl activewindow -j | jq -r .pid); [ "$a" = "$P" ] || { echo "ABORT: focus left the prototype (active pid $a)"; kill $P 2>/dev/null; exit 3; }; }
start() { pkill -x junc-proto; sleep 0.5; SLINT_BACKEND=winit-femtovg JUNC_LANG=en "$BIN" --no-meters >/dev/null 2>&1 & P=$!; sleep 2.5
          hyprctl dispatch focuswindow "pid:$P" >/dev/null; sleep 0.5; guard; }
k() { guard; wtype "$@"; sleep 0.35; }
step() { local label=$1; shift; k "$@"; printf '%-34s -> %s\n' "$label" "$(focus)"; }
shot() { local g; g=$(hyprctl clients -j | jq -r --argjson p "$P" '.[]|select(.pid==$p)|"\(.at[0]),\(.at[1]) \(.size[0])x\(.size[1])"' | head -1); grim -g "$g" "$OUT/$1.png"; }
echo "== A: Shift+Tab (reverse order) from fresh start"
start; for i in 1 2 3; do step "Shift+Tab #$i" -M shift -k Tab -m shift; done
echo "== B: Switch toggling with Space / Return"
start; for i in 1 2 3 4; do k -k Tab; done; printf '%-34s -> %s\n' "Tab x4" "$(focus)"; shot focus-switch
step "space" -k space; step "space again" -k space; step "Return" -k Return
echo "== C: Slider keys"
step "Tab (to slider)" -k Tab; shot focus-slider
step "Right" -k Right; step "Right x5" -k Right -k Right -k Right -k Right -k Right; step "Left" -k Left
step "Page_Up" -k Prior; step "Home" -k Home; step "End" -k End
echo "== D: Popover: open, focus, trap, Esc"
start; for i in 1 2 3; do k -k Tab; done; printf '%-34s -> %s\n' "Tab x3" "$(focus)"
step "Return (open picker)" -k Return; shot popover-open
k mic; printf '%-34s -> %s\n' "type 'mic'" "$(focus)"
for i in 1 2 3 4 5 6; do step "Tab inside popover #$i" -k Tab; done
step "Escape" -k Escape; shot after-escape
step "Tab after Esc" -k Tab
kill $P; echo "== done"
