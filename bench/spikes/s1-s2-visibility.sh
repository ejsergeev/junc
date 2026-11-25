#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# S1: which media.class makes a server-side null-audio-sink visible as both a
#     sink and a source to PulseAudio, JACK and native clients?
# S2: which class/props hide an internal gain node from those device lists?
set -u; source "$(dirname "$0")/lib.sh"
spike_up
declare -A CASES=(
  [s1.duplex]="media.class=Audio/Duplex"
  [s1.sink]="media.class=Audio/Sink"
  [s1.source_virtual]="media.class=Audio/Source/Virtual"
  [s2.sink_internal]="media.class=Audio/Sink/Internal"
  [s2.sink_hidden]="media.class=Audio/Sink node.hidden=true"
  [s2.sink_virtual]="media.class=Audio/Sink node.virtual=true"
  [s2.duplex_internal]="media.class=Audio/Duplex/Internal"
)
for k in $(printf '%s\n' "${!CASES[@]}" | sort); do
  mk_node "node.name=junc.$k node.description=JUNC-$k audio.position=[FL FR] monitor.channel-volumes=true ${CASES[$k]}"
done
for k in "${!CASES[@]}"; do wait_node "junc.$k"; done; sleep 1.5
sinks=$(pactl list short sinks | awk '{print $2}'); sources=$(pactl list short sources | awk '{print $2}')
jack=$(pw-jack jack_lsp 2>/dev/null); wp=$(wpctl status)
printf '%-22s %-6s %-14s %-6s %-6s %-9s %-9s\n' case pa-sink pa-source "(mon)" jack wp-sink wp-source
for k in $(printf '%s\n' "${!CASES[@]}" | sort); do
  n=junc.$k
  ps=$(grep -qx "$n" <<<"$sinks" && echo yes || echo -)
  pr=$(grep -qx "$n" <<<"$sources" && echo yes || echo -)
  pm=$(grep -qx "$n.monitor" <<<"$sources" && echo mon || echo -)
  jk=$(grep -q "^JUNC-$k:" <<<"$jack" && echo yes || echo -)
  ws=$(sed -n '/Sinks:/,/Sources:/p' <<<"$wp" | grep -q "JUNC-$k" && echo yes || echo -)
  wr=$(sed -n '/Sources:/,/Filters:/p' <<<"$wp" | grep -q "JUNC-$k" && echo yes || echo -)
  printf '%-22s %-6s %-14s %-6s %-6s %-9s %-9s   [%s]\n' "$k" "$ps" "$pr" "$pm" "$jk" "$ws" "$wr" "${CASES[$k]}"
done
echo "--- ports per node (in/out/monitor):"
for k in $(printf '%s\n' "${!CASES[@]}" | sort); do
  id=$(node_id "junc.$k")
  pw-dump | jq -r --argjson id "$id" --arg k "$k" '[.[]|select(.type=="PipeWire:Interface:Port" and (.info.props["node.id"]|tonumber? // .)==$id)|.info.props] as $p | "\($k): in=\([$p[]|select(.["port.direction"]=="in")]|length) out=\([$p[]|select(.["port.direction"]=="out" and (.["port.monitor"]//"false")!="true")]|length) monitor=\([$p[]|select(.["port.monitor"]=="true")]|length)"'
done
spike_down
