#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# S4: is an input gain node (null-audio-sink, Audio/Duplex/Internal) plus the
#     Junction node bit-exact at 100 %, and exact x0.5 at 50 %?
# Path: pw-cat -> gain node -> links -> Junction (Audio/Source/Virtual, see S1b)
#       -> pw-record, every link made explicitly as juncd would.
set -u; source "$(dirname "$0")/lib.sh"
PY=${PY:-python3}; W=$(mktemp -d)
"$PY" "$SPIKE_DIR/s4_wav.py" gen "$W/ref.wav"
spike_up
mk_node "node.name=junc.s4 node.description=JUNC-s4 media.class=Audio/Source/Virtual audio.position=[FL FR] node.autoconnect=false"
mk_node "node.name=junc.s4.in.test media.class=Audio/Duplex/Internal audio.position=[FL FR] node.autoconnect=false monitor.channel-volumes=true"
wait_node junc.s4; wait_node junc.s4.in.test
for c in FL FR; do link "junc.s4.in.test:capture_$c" "junc.s4:input_$c"; done
run() { # $1 = gain
  gid=$(node_id junc.s4.in.test)
  pw-cli set-param "$gid" Props "{ channelVolumes = [ $1 $1 ] }" >/dev/null
  pw-record -P "{ node.name=s4.rec node.autoconnect=false }" --format f32 --rate 48000 --channels 2 "$W/rec-$1.wav" & R=$!
  pw-cat -p -P "{ node.name=s4.play node.autoconnect=false }" "$W/ref.wav" & C=$!
  # Link the recorder first; the player starts once its ports are linked.
  for c in FL FR; do link "junc.s4:capture_$c" "s4.rec:input_$c"; done
  for c in FL FR; do link "s4.play:output_$c" "junc.s4.in.test:playback_$c"; done
  wait $C
  sleep 0.5; kill -INT $R; wait $R 2>/dev/null
  "$PY" "$SPIKE_DIR/s4_wav.py" cmp "$W/ref.wav" "$W/rec-$1.wav" "$1"
}
run 1.0; run 0.5
spike_down; rm -rf "$W"
