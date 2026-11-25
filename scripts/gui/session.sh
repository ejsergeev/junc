# SPDX-License-Identifier: GPL-3.0-or-later
# Sourced by the GUI checks: a private headless session (Xvfb, its own
# session bus, the AT-SPI bus and registry), never the user's desktop.
# Sets DISPLAY, DBUS_SESSION_BUS_ADDRESS, XDG_RUNTIME_DIR and `session_tmp`;
# everything it starts stops on exit.
mkdir -p "${XDG_CACHE_HOME:-$HOME/.cache}"
session_tmp=$(mktemp -d "${XDG_CACHE_HOME:-$HOME/.cache}/junc-gui-session.XXXXXX")
session_pids=()
session_stop() { kill "${session_pids[@]}" 2>/dev/null || true; rm -rf "$session_tmp"; }
trap session_stop EXIT
D=:${JUNC_XVFB_DISPLAY:-97}
Xvfb "$D" -screen 0 1800x1300x24 -nolisten tcp >"$session_tmp/xvfb.log" 2>&1 & session_pids+=($!)
DBUS_SESSION_BUS_ADDRESS=$(dbus-daemon --session --fork --print-address=1 --print-pid=2 2>"$session_tmp/dbus.pid")
session_pids+=("$(cat "$session_tmp/dbus.pid")")
export DBUS_SESSION_BUS_ADDRESS DISPLAY=$D XDG_RUNTIME_DIR=$session_tmp
unset WAYLAND_DISPLAY
/usr/lib/at-spi-bus-launcher --launch-immediately >"$session_tmp/atspi.log" 2>&1 & session_pids+=($!)
sleep 1
# Started by hand: in a private session its D-Bus activation goes through
# systemd, which has no such unit here.
/usr/lib/at-spi2-registryd >"$session_tmp/registryd.log" 2>&1 & session_pids+=($!)
sleep 0.5
busctl --user set-property org.a11y.Bus /org/a11y/bus org.a11y.Status IsEnabled b true 2>/dev/null ||
  dbus-send --session --dest=org.a11y.Bus --print-reply /org/a11y/bus \
    org.freedesktop.DBus.Properties.Set string:org.a11y.Status string:IsEnabled variant:boolean:true >/dev/null
