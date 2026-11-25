#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# B13 acceptance: the component gallery in Tokyo Night and Flexoki Light,
# its AT-SPI tree, and the focus ring after keyboard navigation. Runs in a
# private headless session (Xvfb + session bus + AT-SPI bus), never on the
# user's desktop.
# usage: scripts/gui/gallery-check.sh [OUT_DIR]   (needs a built junc-gui)
set -euo pipefail
cd "$(dirname "$0")/../.."
OUT=${1:-target/gallery}; mkdir -p "$OUT"
BIN=${JUNC_GUI:-${CARGO_TARGET_DIR:-target}/debug/junc-gui}
D=:${JUNC_XVFB_DISPLAY:-97}
tmp=$(mktemp -d "${XDG_CACHE_HOME:-$HOME/.cache}/junc-gallery.XXXXXX")
pids=()
cleanup() { kill "${pids[@]}" 2>/dev/null || true; rm -rf "$tmp"; }
trap cleanup EXIT

Xvfb "$D" -screen 0 1800x1300x24 -nolisten tcp >"$tmp/xvfb.log" 2>&1 & pids+=($!)
eval "$(dbus-daemon --session --fork --print-address=1 --print-pid=2 2>"$tmp/dbus.pid" | sed 's/^/DBUS_SESSION_BUS_ADDRESS=/')"
pids+=("$(cat "$tmp/dbus.pid")")
export DBUS_SESSION_BUS_ADDRESS DISPLAY=$D XDG_RUNTIME_DIR=$tmp
unset WAYLAND_DISPLAY
/usr/lib/at-spi-bus-launcher --launch-immediately >"$tmp/atspi.log" 2>&1 & pids+=($!)
sleep 1
# Started by hand: in a private session its D-Bus activation goes through
# systemd, which has no such unit here.
/usr/lib/at-spi2-registryd >"$tmp/registryd.log" 2>&1 & pids+=($!)
sleep 0.5
busctl --user set-property org.a11y.Bus /org/a11y/bus org.a11y.Status IsEnabled b true 2>/dev/null ||
  dbus-send --session --dest=org.a11y.Bus --print-reply /org/a11y/bus \
    org.freedesktop.DBus.Properties.Set string:org.a11y.Status string:IsEnabled variant:boolean:true >/dev/null

run() { # theme
  SLINT_BACKEND=winit-software "$BIN" --scenario gallery --theme "$1" >"$tmp/gui-$1.log" 2>&1 &
  gui=$!
  sleep 3
}
run dark
import -window root "$OUT/gallery-tokyo-night.png"
python3 scripts/gui/atspi_dump.py tree junc >"$OUT/gallery-atspi.txt"
# Five Tabs land on the first switch (Delete, Show Listen, Add input, the
# icon button; the disabled button is skipped).
WID=$(xdotool search --name '^JUNC components$' | head -1)
xdotool windowfocus --sync "$WID"
for _ in 1 2 3 4 5; do xdotool key --window "$WID" Tab; sleep 0.2; done
sleep 0.5
python3 scripts/gui/atspi_dump.py focus junc >"$OUT/gallery-focus.txt"
import -window root "$OUT/gallery-focus.png"
kill "$gui"; wait "$gui" 2>/dev/null || true
run light
import -window root "$OUT/gallery-flexoki-light.png"
kill "$gui"; wait "$gui" 2>/dev/null || true
echo "wrote $OUT: $(ls "$OUT" | tr '\n' ' ')"
