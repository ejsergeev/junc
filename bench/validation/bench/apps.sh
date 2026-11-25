#!/usr/bin/env bash
# Fake "app" streams: pw-cat playing silence forever, named juncbench-app{1..3}.
# MODE=target  -> stream carries target.object=junc.bus (WirePlumber links it; manual graph)
# MODE=daemon  -> node.autoconnect=false; juncd must notice and link it
# usage: apps.sh start|stop|churn <minutes>   (churn: restart one app per minute, round robin)
set -u
export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/tmp/xdg}
LOGDIR=${LOGDIR:-/tmp/pwlogs}; mkdir -p "$LOGDIR"
MODE=${MODE:-target}
HDR=$(cd "$(dirname "$0")" && pwd)/silence-header.wav   # WAV header (~6 h of s16 stereo) + /dev/zero = endless silence

start_one() {
  local i=$1 props="application.name=juncbench-app$i node.name=juncbench-app$i"
  if [ "$MODE" = target ]; then props="$props target.object=junc.bus"; else props="$props node.autoconnect=false"; fi
  (cat "$HDR"; cat /dev/zero) | pw-cat -p --latency 1024 -P "{ $props }" - >"$LOGDIR/app$i.log" 2>&1 &
  echo $! >"$LOGDIR/app$i.pid"   # pid of pw-cat (last in pipeline)
}
stop_one() { [ -f "$LOGDIR/app$1.pid" ] && kill "$(cat "$LOGDIR/app$1.pid")" 2>/dev/null || pkill -f "application.name=juncbench-app$1 " 2>/dev/null; rm -f "$LOGDIR/app$1.pid"; }

case "${1:-start}" in
  start) for i in 1 2 3; do start_one "$i"; done;;
  stop)  for i in 1 2 3; do stop_one "$i"; done;;
  churn) mins=${2:-60}; for m in $(seq 1 "$mins"); do sleep 60; i=$(( (m-1) % 3 + 1 )); stop_one "$i"; sleep 0.5; start_one "$i"; echo "$(date +%s.%N) restarted app$i" ; done;;
esac
