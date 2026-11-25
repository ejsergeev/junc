#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# S5: Exclusive via the `target.object` metadata key with WirePlumber 0.5.
#     An app plays to the default sink (fake.hp); JUNC adds its own link to the
#     Input's gain node; moving the stream to the gain node must drop the
#     WirePlumber link to fake.hp and keep JUNC's; clearing the key must
#     restore fake.hp.
set -u; source "$(dirname "$0")/lib.sh"
spike_up
mk_node "node.name=junc.s5.in.app media.class=Audio/Duplex/Internal audio.position=[FL FR] node.autoconnect=false"
wait_node junc.s5.in.app; wpctl set-default "$(node_id fake.hp)"
pw-cat -p -P "{ node.name=s5.app application.name=s5app }" --raw /dev/zero & A=$!
wait_node s5.app; sleep 1
targets() { pw-link -l | awk '/^s5\.app:output_FL/{f=1;next} f&&/^\s+\|->/{sub(/.*\|-> /,""); sub(/:.*/,""); print; next} {f=0}' | sort -u | tr '\n' ' '; }
echo "1 default routing:          $(targets)"
for c in FL FR; do link "s5.app:output_$c" "junc.s5.in.app:playback_$c"; done; sleep 0.5
echo "2 + JUNC link (non-excl.):  $(targets)"
sid=$(node_id s5.app)
pw-metadata "$sid" target.object junc.s5.in.app >/dev/null; sleep 1.5
echo "3 exclusive (metadata set): $(targets)"
pw-metadata -d "$sid" target.object >/dev/null; sleep 1.5
echo "4 exclusive off (cleared):  $(targets)"
kill $A; spike_down
