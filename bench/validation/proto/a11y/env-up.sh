#!/usr/bin/env bash
# Start a headless desktop-ish session: Xvfb :99 + session D-Bus + AT-SPI bus.
# Writes env to /tmp/junc-session.env; source it in later shells.
set -e
export DISPLAY=:99 LANG=en_US.UTF-8 XDG_RUNTIME_DIR=/tmp/xdg; mkdir -p $XDG_RUNTIME_DIR; chmod 700 $XDG_RUNTIME_DIR
pgrep -x Xvfb >/dev/null || (Xvfb :99 -screen 0 1280x800x24 -nolisten tcp >/tmp/xvfb.log 2>&1 &)
sleep 1
eval "$(dbus-launch --sh-syntax)"
/usr/libexec/at-spi-bus-launcher --launch-immediately >/tmp/atspi.log 2>&1 &
sleep 1
# Tell toolkits (AccessKit checks org.a11y.Status) that AT is enabled
busctl --user set-property org.a11y.Bus /org/a11y/bus org.a11y.Status IsEnabled b true 2>/dev/null || \
  dbus-send --session --dest=org.a11y.Bus --print-reply /org/a11y/bus org.freedesktop.DBus.Properties.Set string:org.a11y.Status string:IsEnabled variant:boolean:true >/dev/null
cat > /tmp/junc-session.env <<E
export DISPLAY=$DISPLAY LANG=$LANG XDG_RUNTIME_DIR=$XDG_RUNTIME_DIR
export DBUS_SESSION_BUS_ADDRESS='$DBUS_SESSION_BUS_ADDRESS'
E
echo "session up: $(cat /tmp/junc-session.env | tr '\n' ' ')"
