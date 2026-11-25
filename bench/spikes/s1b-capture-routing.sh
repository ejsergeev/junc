#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# S1b: when an app records from a Junction, does WirePlumber link it?
#      Tried for each candidate class via PulseAudio (parecord -d), native
#      (pw-record --target) and as the configured default source.
set -u; source "$(dirname "$0")/lib.sh"
spike_up
for cls in Audio/Duplex Audio/Source/Virtual; do
  n=junc.s1b.$(tr '/' '_' <<<"$cls" | tr 'A-Z' 'a-z')
  mk_node "node.name=$n node.description=$n media.class=$cls audio.position=[FL FR] node.autoconnect=false"
done
sleep 1
linked_from() { pw-link -l | awk -v c="$1" '$1 ~ "^"c":" {getline; print}' | grep -o '[^ >]*:[^ ]*$' | cut -d: -f1 | sort -u | tr '\n' ' '; }
for cls in Audio/Duplex Audio/Source/Virtual; do
  n=junc.s1b.$(tr '/' '_' <<<"$cls" | tr 'A-Z' 'a-z')
  parecord -d "$n" --raw /dev/null & A=$!
  pw-record -P "{ node.name=rec.native }" --target "$n" /dev/null & B=$!
  sleep 1.2
  pa=$(pw-link -l | grep -B1 -E '\|-> (parecord|PulseAudio|.*Record)' | grep -o "^$n" | head -1)
  nat=$(pw-link -l | grep -A1 "^$n:" | grep -c 'rec.native')
  kill $A $B 2>/dev/null; wait $A $B 2>/dev/null
  wpctl set-default "$(node_id "$n")" 2>/dev/null
  pw-record -P "{ node.name=rec.default }" /dev/null & C=$!; sleep 1.2
  def=$(pw-link -l | grep -A1 "^$n:" | grep -c 'rec.default')
  kill $C; wait $C 2>/dev/null
  printf '%-22s pulse(-d)=%s native(--target)=%s default-source=%s\n' "$cls" "${pa:+linked}" "$([ "$nat" -gt 0 ] && echo linked || echo -)" "$([ "$def" -gt 0 ] && echo linked || echo -)"
done
spike_down
