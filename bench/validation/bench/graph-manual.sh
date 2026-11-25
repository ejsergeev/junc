#!/usr/bin/env bash
# Reference JUNC graph built by hand with pw-cli / pw-link / pw-loopback.
#   fake devices : fake.hwin (Audio/Source, mono)  fake.hp + fake.obs (Audio/Sink, stereo)
#   junction bus : junc.bus  null-audio-sink, 4 ch [FL FR RL RR]
#   inputs       : 3 app streams (apps.sh, target.object=junc.bus) + fake.hwin -> bus FL/FR
#   listen       : 2 monitor outputs = pw-loopback bus.monitor -> fake.hp / fake.obs
# usage: graph-manual.sh devices|up|down
set -u
export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/tmp/xdg}
LOGDIR=${LOGDIR:-/tmp/pwlogs}; mkdir -p "$LOGDIR"

node() { pw-cli create-node adapter "{ factory.name=support.null-audio-sink object.linger=true $* }" >/dev/null; }
wait_node() { for _ in $(seq 1 200); do pw-cli ls Node 2>/dev/null | grep -q "node.name = \"$1\"" && return 0; sleep 0.01; done; echo "timeout waiting for $1" >&2; return 1; }

devices() {   # stand-ins for hardware; shared by both graphs
  pw-cli ls Node | grep -q '"fake.hp"' && return
  node node.name=fake.hwin node.description=FakeMic media.class=Audio/Source/Virtual audio.position=[MONO]
  node node.name=fake.hp   node.description=FakeHeadphones media.class=Audio/Sink audio.position=[FL FR]
  node node.name=fake.obs  node.description=FakeOBS media.class=Audio/Sink audio.position=[FL FR]
  wait_node fake.hwin; wait_node fake.hp; wait_node fake.obs
}

up() {
  devices
  node node.name=junc.bus node.description=JUNC-Bus media.class=Audio/Sink audio.position=[FL FR RL RR] node.autoconnect=false
  wait_node junc.bus
  # hardware input -> bus (mono to FL+FR)
  pw-link fake.hwin:capture_MONO junc.bus:playback_FL
  pw-link fake.hwin:capture_MONO junc.bus:playback_FR
  # two monitor outputs
  for out in hp obs; do
    pw-loopback -n "junc.listen.$out" -m '[ FL FR ]' \
      --capture-props="{ target.object=junc.bus stream.capture.sink=true node.passive=true audio.position=[FL FR] }" \
      --playback-props="{ target.object=fake.$out node.dont-reconnect=false }" >"$LOGDIR/loopback-$out.log" 2>&1 &
    echo $! >"$LOGDIR/loopback-$out.pid"
  done
  wait_node "output.junc.listen.hp"; wait_node "output.junc.listen.obs"
}

down() {
  for out in hp obs; do [ -f "$LOGDIR/loopback-$out.pid" ] && kill "$(cat "$LOGDIR/loopback-$out.pid")" 2>/dev/null; done
  for n in junc.bus; do id=$(pw-cli ls Node | awk -v n="\"$n\"" '/^\s*id /{id=$2} $0 ~ "node.name = "n {print id}' | tr -d ,); [ -n "$id" ] && pw-cli destroy "$id" >/dev/null; done
}

"${1:-up}"
