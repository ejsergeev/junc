#!/usr/bin/env bash
# Keyboard-only interaction checks against the prototype (X11/Xvfb + AT-SPI).
# Prints the AT-SPI focused object after each step; saves focus screenshots to $OUT.
set -u
source /tmp/junc-session.env
D=$(cd "$(dirname "$0")" && pwd); BIN="$D/../target/release/junc-proto"
OUT=${OUT:-$D/../../screenshots/a11y}; mkdir -p "$OUT"
LOG=/tmp/proto-int.log
focus() { /usr/bin/python3.12 "$D/atspi_dump.py" focus junc; }
start() { pkill -x junc-proto; sleep 0.5; SLINT_BACKEND=winit-software JUNC_LANG=en "$BIN" --no-meters >"$LOG" 2>&1 & sleep 2.5;
          WID=$(xdotool search --name '^JUNC$' | head -1); xdotool windowfocus --sync "$WID"; }
k() { xdotool key --window "$WID" "$@"; sleep 0.3; }
step() { local label=$1; shift; k "$@"; printf '%-34s -> %s\n' "$label" "$(focus)"; }
shot() { import -window root "$OUT/$1.png"; }

echo "== A: Shift+Tab (reverse order) from fresh start"
start; for i in 1 2 3; do step "Shift+Tab #$i" shift+Tab; done

echo "== B: Switch toggling with Space / Return"
start; for i in 1 2 3 4; do k Tab; done; printf '%-34s -> %s\n' "Tab x4" "$(focus)"; shot focus-switch
step "space"  space
step "space again" space
step "Return" Return

echo "== C: Slider keys"
step "Tab (to slider)" Tab; shot focus-slider
step "Right" Right
step "Right x5" Right Right Right Right Right
step "Left" Left
step "Page_Up" Prior
step "Home" Home
step "End" End

echo "== D: Popover: open, focus, trap, Esc"
start; for i in 1 2 3; do k Tab; done; printf '%-34s -> %s\n' "Tab x3" "$(focus)"
step "Return (open picker)" Return; shot popover-open
/usr/bin/python3.12 "$D/atspi_dump.py" tree junc | sed -n '/Add input\b\|window\|dialog\|text\b/p' | head -20
xdotool type --window "$WID" --delay 60 "mic"; sleep 0.4; printf '%-34s -> %s\n' "type 'mic'" "$(focus)"
for i in 1 2 3 4 5 6; do step "Tab inside popover #$i" Tab; done
step "Escape" Escape; shot after-escape
step "Tab after Esc" Tab
echo "popup windows still open:"; /usr/bin/python3.12 "$D/atspi_dump.py" tree junc | grep -c "Search streams" || true
pkill -x junc-proto
