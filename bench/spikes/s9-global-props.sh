#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# S9 (backend): which properties arrive with registry globals, and which need
# binding the proxy (full info)? Checks app keys on stream nodes, junc.* tags
# on nodes and links, and channel keys on ports.
set -u; source "$(dirname "$0")/lib.sh"
spike_up
mk_node "node.name=junc.t node.description=JUNC-t media.class=Audio/Source/Virtual audio.position=[FL FR] junc.managed=true junc.junction=t"
wait_node junc.t
pw-cat -p -P "{ application.name=Firefox application.process.binary=firefox media.role=Music node.name=Firefox }" --raw /dev/zero &
CAT=$!
wait_node Firefox
link "fake.hwin:capture_MONO" "junc.t:input_FL"   # plain link to get ids
src=$(node_id fake.hwin); dst=$(node_id junc.t)
o=$(pw-dump | jq -r --argjson n "$src" '.[]|select(.type=="PipeWire:Interface:Port" and .info.props["node.id"]==$n and .info.direction=="output")|.id' | head -1)
i=$(pw-dump | jq -r --argjson n "$dst" '.[]|select(.type=="PipeWire:Interface:Port" and .info.props["node.id"]==$n and .info.direction=="input")|.id' | tail -1)
pw-cli create-link "$src" "$o" "$dst" "$i" "{ object.linger=true junc.managed=true junc.ref=x }" >/dev/null
sleep 0.5
show() { pw-cli ls "$1" | awk -v pat="$2" 'BEGIN{RS="\tid "} $0 ~ pat {print "id " $0}' | head -30; }
echo "== Node global: junc.t";   show Node 'node.name = "junc.t"'
echo "== Node global: Firefox";  show Node 'node.name = "Firefox"'
echo "== Port global (junc.t input)"; show Port 'port.name = "input_FR"'
echo "== Link globals"; pw-cli ls Link | grep -E 'id |junc|linger' 
kill "$CAT"; spike_down
