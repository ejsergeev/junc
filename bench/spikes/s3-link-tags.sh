#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# S3: do custom junc.* props on a link created through the link factory (pw-cli create-link) stay
#     visible in the registry after the creating client exits, and does
#     object.linger keep the link alive (and its absence remove it)?
set -u; source "$(dirname "$0")/lib.sh"
spike_up
mk_node "node.name=junc.s3 node.description=JUNC-s3 media.class=Audio/Duplex audio.position=[FL FR] node.autoconnect=false"
wait_node junc.s3
src=$(node_id fake.hwin); dst=$(node_id junc.s3)
port() { pw-dump | jq -r --argjson n "$1" --arg d "$2" --arg c "$3" '.[]|select(.type=="PipeWire:Interface:Port" and .info.props["node.id"]==$n and .info.direction==$d and .info.props["audio.channel"]==$c)|.id' | head -1; }
o=$(port "$src" output MONO); iL=$(port "$dst" input FL); iR=$(port "$dst" input FR)
# pw-cli exits right after create-object: the creating client is gone once it returns.
pw-cli create-link "$src" "$o" "$dst" "$iL" "{ object.linger=true junc.managed=true junc.junction=s3 junc.role=link junc.ref=\"in:fake/1->bus/1\" }" >/dev/null
pw-cli create-link "$src" "$o" "$dst" "$iR" "{ object.linger=false junc.managed=true junc.ref=nolinger }" >/dev/null
sleep 1
pw-dump | jq -r --argjson s "$src" --argjson d "$dst" '.[]|select(.type=="PipeWire:Interface:Link" and .info["output-node-id"]==$s and .info["input-node-id"]==$d)|"link \(.id) state=\(.info.state) props: " + ([.info.props|to_entries[]|select(.key|test("^(junc\\.|object\\.linger)"))|"\(.key)=\(.value)"]|join(" "))'
echo "links fake.hwin -> junc.s3 after client exit: $(pw-dump | jq --argjson s "$src" --argjson d "$dst" '[.[]|select(.type=="PipeWire:Interface:Link" and .info["output-node-id"]==$s and .info["input-node-id"]==$d)]|length')"
spike_down
