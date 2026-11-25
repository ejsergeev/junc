#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# S11: a Cable's target app hears only the Cable. A recording app is linked
#      by WirePlumber to the default source (fake.hwin); JUNC adds its own
#      link from the Cable's source; moving the recording stream to the Cable
#      with `target.object` must drop WirePlumber's link and keep JUNC's;
#      clearing the key must restore the default source.
set -u; source "$(dirname "$0")/lib.sh"
spike_up
mk_node "node.name=junc.s11 media.class=Audio/Source/Virtual audio.position=[FL FR] node.autoconnect=false"
wait_node junc.s11; wpctl set-default "$(node_id fake.hwin)"
pw-cat -r -P "{ node.name=s11.rec application.name=s11rec }" --raw /dev/null & R=$!
trap 'kill $R 2>/dev/null; spike_down' EXIT
wait_node s11.rec; sleep 1
# Sources linked into the recording stream.
sources() { pw-link -l -I | awk '/s11\.rec:input_/{f=1;next} f&&/\|<-/{sub(/.*\|<- /,""); print; next} {f=0}' | sed 's/^ *[0-9]* *//' | sort | tr '\n' ' '; }
echo "1 default routing:            $(sources)"
link "junc.s11:capture_FL" "s11.rec:input_MONO"; sleep 0.5
echo "2 + JUNC link:                $(sources)"
rid=$(node_id s11.rec)
pw-metadata "$rid" target.object junc.s11 >/dev/null; sleep 1.5
echo "3 target.object = junc.s11:   $(sources)"
pw-metadata -d "$rid" target.object >/dev/null; sleep 1.5
echo "4 cleared:                    $(sources)"

