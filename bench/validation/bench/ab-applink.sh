#!/usr/bin/env bash
# A/B: do app-client ERRs depend on WHO links the app streams?
#   A: apps node.autoconnect=false, linked by juncd (juncd-links mode)
#   B: apps target.object=junc.bus, linked by WirePlumber; juncd still builds bus/outputs (JUNCD_APP_PREFIX=none)
# Both run at the same time on two separate PipeWire instances (separate XDG_RUNTIME_DIR), so host noise hits both.
# usage: ab-applink.sh [seconds]
set -u
D=$(cd "$(dirname "$0")" && pwd); SEC=${1:-600}; OUT=$D/results/ab-applink; mkdir -p "$OUT"; rm -f "$OUT"/*
run() { # $1 = A|B
  local X=$1; export XDG_RUNTIME_DIR=/tmp/xdg-ab-$X LOGDIR=$OUT/logs-$X; mkdir -p "$XDG_RUNTIME_DIR" "$LOGDIR"; chmod 700 "$XDG_RUNTIME_DIR"
  "$D/pw-session.sh" restart; sleep 2
  if [ "$X" = A ]; then "$D/juncd/target/release/juncd" --listen=links >/dev/null 2>"$OUT/juncd-$X.log" &
                       sleep 1; MODE=daemon "$D/apps.sh" start
  else JUNCD_APP_PREFIX=none "$D/juncd/target/release/juncd" --listen=links >/dev/null 2>"$OUT/juncd-$X.log" &
                       sleep 1; MODE=target "$D/apps.sh" start; fi
  sleep 3; pw-link -l >"$OUT/links-$X.txt"
  pw-top -b -n "$SEC" >"$OUT/pwtop-$X.txt" 2>&1
  python3 "$D/pwtop_summary.py" "$OUT/pwtop-$X.txt" >"$OUT/summary-$X.txt"
  MODE=x "$D/apps.sh" stop
}
run A & run B & wait
pkill -f "juncd --listen=links"
for X in A B; do echo "== $X"; grep -E "juncbench|ERR_TOTAL|junc.bus" "$OUT/summary-$X.txt"; done | tee "$OUT/result.txt"
