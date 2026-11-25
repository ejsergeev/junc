#!/usr/bin/env bash
# Slint prototype performance: cold start, PSS, CPU with meters at 7 Hz, CPU when hidden (unmapped).
# usage: perf.sh [renderer]   renderer = winit-femtovg (default, GL) | winit-software | winit-skia
# env: N=10 cold starts, SEC=60 per CPU window. Needs DISPLAY (X11); on Wayland, "hidden" = move to another workspace by hand.
set -u
[ -f /tmp/junc-session.env ] && source /tmp/junc-session.env
D=$(cd "$(dirname "$0")" && pwd); BIN=$D/target/release/junc-proto
R=${1:-winit-femtovg}; N=${N:-10}; SEC=${SEC:-60}; HZ=$(getconf CLK_TCK)
cpu_ticks() { awk '{print $14+$15}' /proc/"$1"/stat; }
pss() { awk '/^Pss:/{print $2}' /proc/"$1"/smaps_rollup; }
cpu_pct() { local p=$1 a b; a=$(cpu_ticks "$p"); sleep "$SEC"; b=$(cpu_ticks "$p"); awk -v a="$a" -v b="$b" -v hz="$HZ" -v s="$SEC" 'BEGIN{printf "%.2f", 100*(b-a)/hz/s}'; }
ctx() { awk '/ctxt_switches/{s+=$2} END{print s}' /proc/"$1"/task/*/status; }

echo "# slint prototype perf renderer=$R cpus=$(nproc) $(date -u +%FT%TZ)"
echo "cold start x$N (process start -> first frame presented; software renderer: first event-loop turn):"
for i in $(seq 1 "$N"); do JUNC_LANG=en SLINT_BACKEND=$R "$BIN" --cold-start --no-meters 2>/dev/null | grep -oE '(first_frame|first_loop_turn)_ms=[0-9]+'; done \
  | tee /tmp/cold.txt | sort -t= -k2 -n | awk -F= '{v[NR]=$2; n=$1} END{printf "  %s median=%d min=%d max=%d\n", n, v[int((NR+1)/2)], v[1], v[NR]}'

JUNC_LANG=en SLINT_BACKEND=$R "$BIN" >/dev/null 2>&1 & P=$!
sleep 5
echo "pss_kb_meters_on=$(pss $P)"
c0=$(ctx $P); echo "cpu_pct_meters_7hz_visible=$(cpu_pct $P)"; c1=$(ctx $P)
echo "wakeups_per_s_visible=$(( (c1-c0)/SEC ))"
WID=$(xdotool search --pid $P | tail -1); xdotool windowunmap "$WID"; sleep 1
c0=$(ctx $P); echo "cpu_pct_meters_7hz_hidden(unmapped)=$(cpu_pct $P)"; c1=$(ctx $P)
echo "wakeups_per_s_hidden=$(( (c1-c0)/SEC ))"
echo "pss_kb_end=$(pss $P)"
kill $P
JUNC_LANG=en SLINT_BACKEND=$R "$BIN" --no-meters >/dev/null 2>&1 & P=$!
sleep 5; echo "cpu_pct_idle_no_meters=$(cpu_pct $P) pss_kb_no_meters=$(pss $P)"; kill $P
