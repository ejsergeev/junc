#!/usr/bin/env bash
# Hyprland variant of proto/perf.sh: hide = move the window to a silent special workspace.
set -u
BIN=$1; R=$2; N=${N:-10}; SEC=${SEC:-30}; HZ=$(getconf CLK_TCK)
cpu_ticks() { awk '{print $14+$15}' /proc/"$1"/stat; }
pss() { awk '/^Pss:/{print $2}' /proc/"$1"/smaps_rollup; }
cpu_pct() { local a b; a=$(cpu_ticks "$1"); sleep "$SEC"; b=$(cpu_ticks "$1"); awk -v a="$a" -v b="$b" -v hz="$HZ" -v s="$SEC" 'BEGIN{printf "%.2f", 100*(b-a)/hz/s}'; }
ctx() { awk '/ctxt_switches/{s+=$2} END{print s}' /proc/"$1"/task/*/status; }
echo "# renderer=$R cpus=$(nproc) $(date -u +%FT%TZ)"
for i in $(seq 1 "$N"); do JUNC_LANG=en SLINT_BACKEND=$R timeout 10 "$BIN" --cold-start --no-meters 2>/dev/null | grep -oE '(first_frame|first_loop_turn)_ms=[0-9]+'; done \
  | sort -t= -k2 -n | awk -F= '{v[NR]=$2; n=$1} END{printf "cold_start %s median=%d min=%d max=%d n=%d\n", n, v[int((NR+1)/2)], v[1], v[NR], NR}'
JUNC_LANG=en SLINT_BACKEND=$R "$BIN" >/dev/null 2>&1 & P=$!
sleep 4
echo "pss_kb_meters_on=$(pss $P)"
c0=$(ctx $P); echo "cpu_pct_meters_visible=$(cpu_pct $P)"; c1=$(ctx $P); echo "wakeups_per_s_visible=$(( (c1-c0)/SEC ))"
hyprctl dispatch movetoworkspacesilent "special:juncbench,pid:$P" >/dev/null; sleep 1
c0=$(ctx $P); echo "cpu_pct_meters_hidden_special_ws=$(cpu_pct $P)"; c1=$(ctx $P); echo "wakeups_per_s_hidden=$(( (c1-c0)/SEC ))"
echo "pss_kb_end=$(pss $P)"; kill $P
JUNC_LANG=en SLINT_BACKEND=$R "$BIN" --no-meters >/dev/null 2>&1 & P=$!
sleep 4; hyprctl dispatch movetoworkspacesilent "special:juncbench,pid:$P" >/dev/null
echo "cpu_pct_idle_no_meters=$(cpu_pct $P) pss_kb_no_meters=$(pss $P)"; kill $P
