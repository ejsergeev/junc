#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# S10: can a JUNC node be renamed (node.description) without recreating it,
#      so apps recording from it keep their device?
set -u; source "$(dirname "$0")/lib.sh"
spike_up
mk_node "node.name=junc.r node.description=Old media.class=Audio/Source/Virtual audio.position=[FL FR]"
wait_node junc.r; id=$(node_id junc.r)
desc() { pw-dump "$id" | jq -r '.[0].info.props["node.description"]'; }
echo "before: $(desc)"
pw-cli set-param "$id" Props '{ params = [ "node.description" "Via Props params" ] }' >/dev/null 2>&1; sleep 0.3
echo "set-param Props params: $(desc)"
pw-cli set-param "$id" Props '{ "node.description": "Via Props key" }' >/dev/null 2>&1; sleep 0.3
echo "set-param Props key:    $(desc)"
pactl list sources 2>/dev/null | grep -A1 'Name: junc.r' | sed -n 2p | sed 's/^\s*/pulse: /'
spike_down

# Part 2: recreate with the same node.name and a new description; are
# recording apps moved back to it?
spike_up
mk_node "node.name=junc.r node.description=Old media.class=Audio/Source/Virtual audio.position=[FL FR]"
wait_node junc.r
pw-record -P "{ node.name=rec.native }" --target junc.r /dev/null & A=$!
parecord -d junc.r --raw /dev/null & B=$!
sleep 1.2
# Which node feeds each recording stream (by its application/node name).
feeds() { pw-dump | jq -r --arg re "$1" '
  (map(select(.type=="PipeWire:Interface:Node")) | map({key: (.id|tostring), value: .info.props}) | from_entries) as $n
  | [.[] | select(.type=="PipeWire:Interface:Link") | .info
     | select(($n[(.["input-node-id"]|tostring)]["node.name"] // "") | test($re))
     | $n[(.["output-node-id"]|tostring)]["node.name"]] | unique | join(",")'; }
targets() { feeds "$1"; }
echo "before recreate: native<-$(targets rec.native) pulse<-$(targets 'parecord|PulseAudio.*')"
pw-cli destroy "$(node_id junc.r)" >/dev/null; sleep 0.5
echo "node gone:       native<-$(targets rec.native) pulse<-$(targets 'parecord|PulseAudio.*')"
mk_node "node.name=junc.r node.description=New media.class=Audio/Source/Virtual audio.position=[FL FR]"
sleep 1.5
echo "recreated:       native<-$(targets rec.native) pulse<-$(targets 'parecord|PulseAudio.*')"
kill $A $B; spike_down
