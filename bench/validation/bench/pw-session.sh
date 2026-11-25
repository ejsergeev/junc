#!/usr/bin/env bash
# Start/stop/restart a user PipeWire session without systemd (CI containers).
# On a real Omarchy box use `systemctl --user restart pipewire wireplumber` instead;
# bench scripts call this only when PW_SESSION=manual (default if no systemd user bus).
# usage: pw-session.sh start|stop|restart
set -u
export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/tmp/xdg}; mkdir -p "$XDG_RUNTIME_DIR"; chmod 700 "$XDG_RUNTIME_DIR"
# CI: use the repo config dir only when no systemd (no logind) is present
[ "${PW_SESSION:-manual}" = manual ] && export XDG_CONFIG_HOME=${XDG_CONFIG_HOME:-$(cd "$(dirname "$0")" && pwd)/ci-config}
LOGDIR=${LOGDIR:-/tmp/pwlogs}; mkdir -p "$LOGDIR"
PIDF=$XDG_RUNTIME_DIR/pw-session.pids   # per-instance, so parallel sessions (ab-applink.sh) don't kill each other
start() {
  pipewire >"$LOGDIR/pipewire.log" 2>&1 & echo $! >"$PIDF"
  for _ in $(seq 1 100); do [ -S "$XDG_RUNTIME_DIR/pipewire-0" ] && break; sleep 0.02; done
  wireplumber >"$LOGDIR/wireplumber.log" 2>&1 & echo $! >>"$PIDF"
  pipewire-pulse >"$LOGDIR/pipewire-pulse.log" 2>&1 & echo $! >>"$PIDF"
}
stop() {
  if [ -f "$PIDF" ]; then kill $(tac "$PIDF") 2>/dev/null; rm -f "$PIDF"
  elif [ "$XDG_RUNTIME_DIR" != "/run/user/$(id -u)" ]; then
    # isolated instance without a pid file: kill only processes bound to this runtime dir
    for p in $(pgrep -x 'pipewire|wireplumber|pipewire-pulse'); do
      tr '\0' '\n' </proc/$p/environ 2>/dev/null | grep -qx "XDG_RUNTIME_DIR=$XDG_RUNTIME_DIR" && kill "$p"
    done
  else pkill -x pipewire-pulse; pkill -x wireplumber; pkill -x pipewire; fi
  sleep 0.3; rm -f "$XDG_RUNTIME_DIR"/pipewire-0*
}
case "${1:-start}" in
  start) start;;
  stop) stop;;
  restart)
    if [ "${PW_SESSION:-manual}" = systemd ]; then systemctl --user restart pipewire wireplumber
    else stop; start; fi;;
esac
