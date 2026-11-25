#!/usr/bin/env bash
# JUNC performance baseline harness. Re-runnable in CI or on a real Omarchy box.
#
# usage: run-bench.sh <manual|juncd|juncd-links> [outdir]
#   manual       graph-manual.sh (pw-cli + pw-link + 2x pw-loopback); apps use target.object (WirePlumber links)
#   juncd        juncd daemon, listen outputs = in-process module-loopback
#   juncd-links  juncd daemon, listen outputs = direct bus-monitor -> device links (no extra nodes)
# env (seconds):  STABLE_SEC=600  IDLE_SEC=60  CHURN_SEC=0 (3600 for the 1 h run)  RECOVERY_N=5  LAT_SEC=6
#                 PW_SESSION=manual|systemd (systemd = real machine: uses systemctl --user restart)
# output: <outdir>/<mode>/{summary.txt, pwtop.txt, obs.log, juncd.log, pss.csv, ...}
set -u
MODE=${1:?mode}; OUTROOT=${2:-$(cd "$(dirname "$0")" && pwd)/results}
D=$(cd "$(dirname "$0")" && pwd); OUT="$OUTROOT/$MODE"; mkdir -p "$OUT"; rm -f "$OUT"/*
export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}; [ -d "$XDG_RUNTIME_DIR" ] || export XDG_RUNTIME_DIR=/tmp/xdg
export LOGDIR=$OUT/logs; mkdir -p "$LOGDIR"
STABLE_SEC=${STABLE_SEC:-600}; IDLE_SEC=${IDLE_SEC:-60}; CHURN_SEC=${CHURN_SEC:-0}; RECOVERY_N=${RECOVERY_N:-5}; LAT_SEC=${LAT_SEC:-6}
BIN=$D/juncd/target/release
S="$OUT/summary.txt"; : >"$S"
say() { echo "$*" | tee -a "$S"; }
now_ms() { date +%s%3N; }
pss_kb() { local t=0; for p in "$@"; do v=$(awk '/^Pss:/{print $2}' /proc/"$p"/smaps_rollup 2>/dev/null); t=$((t + ${v:-0})); done; echo $t; }
ctxsw() { local t=0; for p in "$@"; do for f in /proc/"$p"/task/*/status; do v=$(awk '/ctxt_switches/{s+=$2} END{print s}' "$f" 2>/dev/null); t=$((t + ${v:-0})); done; done; echo $t; }
daemon_pids() {
  case $MODE in
    manual) pgrep -f "pw-loopback -n junc.listen" | tr '\n' ' ';;   # the only long-lived helpers of the manual graph
    *) pgrep -x juncd | tr '\n' ' ';;
  esac
}
graph_up() {
  case $MODE in
    manual) "$D/graph-manual.sh" up;;
    juncd) "$BIN/juncd" >/dev/null 2>>"$OUT/juncd.log" & ;;
    juncd-links) "$BIN/juncd" --listen=links >/dev/null 2>>"$OUT/juncd.log" & ;;
  esac
}
apps_mode() { [ "$MODE" = manual ] && echo target || echo daemon; }
wait_obs() { # $1 = event regex, $2 = after epoch ms, $3 = timeout s ; prints event epoch ms
  local end=$(( $(date +%s) + $3 ))
  while [ "$(date +%s)" -le "$end" ]; do
    t=$(awk -v a="$2" -v ev="$1" '$1=="OBS" && $3 ~ ev && $2+0 >= a {print $2; exit}' "$OUT/obs.log")
    [ -n "$t" ] && { echo "$t"; return 0; }; sleep 0.02
  done; echo NA; return 1
}

say "# JUNC bench mode=$MODE host=$(hostname) kernel=$(uname -r) cpus=$(nproc) pipewire=$(pipewire --version | awk '/libpipewire/{print $NF; exit}') date=$(date -u +%FT%TZ)"
# ---------- clean start ----------
pkill -x juncd; pkill -f "pw-loopback -n junc.listen"; MODE=x "$D/apps.sh" stop; pkill -x pwobs
"$D/pw-session.sh" restart; sleep 2
"$D/graph-manual.sh" devices      # no-op when devices exist (CI config / real hardware)
stdbuf -oL "$BIN/pwobs" >"$OUT/obs.log" 2>&1 &
sleep 0.5
t_up=$(now_ms); graph_up
MODE=$(apps_mode) "$D/apps.sh" start
t_c=$(wait_obs '^complete' "$t_up" 20); say "build_graph_ms=$(( ${t_c%.*} - t_up )) (graph_up + 3 apps -> full graph present)"
sleep 2
DP=$(daemon_pids); say "daemon_pids=$DP"

# ---------- latency ----------
"$D/latency.sh" "$LAT_SEC" | tee "$OUT/latency.txt" | sed 's/^/latency: /' | tee -a "$S"

# ---------- idle: PSS + wakeups (graph running, no topology changes) ----------
say "pss_idle_kb=$(pss_kb $DP)"
c0=$(ctxsw $DP); sleep "$IDLE_SEC"; c1=$(ctxsw $DP)
say "wakeups_per_s_idle=$(awk -v a="$c0" -v b="$c1" -v s="$IDLE_SEC" 'BEGIN{printf "%.2f", (b-a)/s}') (all threads, voluntary+involuntary ctx switches)"
for p in $DP; do for t in /proc/$p/task/*; do
  n=$(cat "$t/comm"); v=$(awk '/ctxt_switches/{s+=$2} END{print s}' "$t/status"); echo "$p $n $v"; done; done >"$OUT/threads_ctxsw_end.txt"

# ---------- stability: pw-top for STABLE_SEC ----------
pw-top -b -n "$STABLE_SEC" >"$OUT/pwtop.txt" 2>&1 &
TP=$!
( while kill -0 $TP 2>/dev/null; do echo "$(date +%s),$(pss_kb $DP)"; sleep 10; done ) >"$OUT/pss.csv" &
wait $TP
python3 "$D/pwtop_summary.py" "$OUT/pwtop.txt" | tee "$OUT/pwtop_summary.txt" | sed 's/^/pwtop: /' | tee -a "$S"

# ---------- churn: restart one app per minute ----------
if [ "$CHURN_SEC" -gt 0 ]; then
  say "pss_before_churn_kb=$(pss_kb $DP)"
  MODE=$(apps_mode) "$D/apps.sh" churn $(( CHURN_SEC / 60 )) >"$OUT/churn.log" 2>&1
  say "pss_after_churn_kb=$(pss_kb $DP) churn_minutes=$(( CHURN_SEC / 60 ))"
fi
awk '$3=="app_linked"{sub("relink_ms=","",$5); print $5}' "$OUT/obs.log" | sort -n >"$OUT/relink_ms.txt"
say "relink_ms $(python3 -c 'import sys,statistics as s; v=[float(x) for x in open(sys.argv[1]) if x.strip()]; q=lambda p: v[min(len(v)-1,int(p*len(v)))]; print(f"n={len(v)} min={v[0]:.2f} p50={s.median(v):.2f} p95={q(.95):.2f} max={v[-1]:.2f}" if v else "n=0")' "$OUT/relink_ms.txt")"

# ---------- recovery after PipeWire restart ----------
for i in $(seq 1 "$RECOVERY_N"); do
  t0=$(now_ms); "$D/pw-session.sh" restart
  if [ "$MODE" = manual ]; then "$D/graph-manual.sh" up >/dev/null 2>&1; fi   # manual graph must be rebuilt by hand
  tc=$(wait_obs '^core_complete' "$t0" 15)
  MODE=$(apps_mode) "$D/apps.sh" stop; MODE=$(apps_mode) "$D/apps.sh" start   # apps died with PipeWire
  echo "$i $(awk -v a="$t0" -v b="$tc" 'BEGIN{ if (b=="NA") print "NA"; else printf "%.0f", b-a }')" >>"$OUT/recovery_ms.txt"
  sleep 3
done
say "recovery_ms(core graph back after restart; manual includes re-running graph-manual.sh) $(awk '{printf "%s ", $2}' "$OUT/recovery_ms.txt")"
say "daemon_pids_after=$(daemon_pids) pss_end_kb=$(pss_kb $(daemon_pids))"
pkill -x pwobs; MODE=x "$D/apps.sh" stop; pkill -x juncd; pkill -f "pw-loopback -n junc.listen"
echo "done: $S"
