#!/usr/bin/env python3
"""Impulse round-trip latency through a PipeWire path, via the JACK API (run under pw-jack).

Complements jack_iodelay: jack_iodelay's MLS detector reported "Signal below threshold" through
pw-loopback (it does pass audio; see report), so we also time single-sample impulses.
usage: pw-jack python3 impulse_latency.py <out-target-port> <in-source-port> [seconds]
prints: frames=<median> min=<..> max=<..> n=<count> quantum=<blocksize>
"""
import sys, time, threading, subprocess
import jack, numpy as np

out_target, in_source = sys.argv[1], sys.argv[2]
secs = float(sys.argv[3]) if len(sys.argv) > 3 else 6
PERIOD = 24000  # one burst every 0.5 s at 48 kHz (bursts start at multiples of PERIOD; PERIOD % quantum must leave room for 32 samples)
# Two separate JACK clients (sender / receiver) so the measured path is not a graph cycle:
# a single client that both feeds the bus and reads the far end forms a loop that PipeWire
# schedules differently through pw-loopback (bursts never arrived), see report.
client = jack.Client("junc_impulse_tx")
rxc = jack.Client("junc_impulse_rx")
outp = client.outports.register("out")
inp = rxc.inports.register("in")
state = {"frame": 0, "sent": [], "recv": [], "armed": False}
lock = threading.Lock()


@client.set_process_callback
def process(n):
    f0 = client.last_frame_time  # shared graph clock
    o = outp.get_array(); o[:] = 0
    k = (-f0) % PERIOD
    if k < n:  # 32-sample burst (a single sample gets smeared below threshold by resamplers)
        o[k:k + 32] = 0.5
        state["sent"].append(f0 + k)


@rxc.set_process_callback
def process_rx(n):
    f0 = rxc.last_frame_time
    i = inp.get_array()
    idx = np.flatnonzero(np.abs(i) > 0.05)
    if idx.size:
        state["recv"].append(f0 + int(idx[0]))


with client, rxc:
    subprocess.run(["pw-link", "junc_impulse_tx:out", out_target], check=False)
    subprocess.run(["pw-link", in_source, "junc_impulse_rx:in"], check=False)
    time.sleep(secs)
    bs = client.blocksize

sent, recv = state["sent"], state["recv"]
# keep only the first received frame of each burst (one per impulse)
firsts = []
for r in recv:
    if not firsts or r - firsts[-1] > PERIOD // 2:
        firsts.append(r)
d = []
for r in firsts:
    prev = [s for s in sent if s <= r]
    if prev:
        d.append(r - prev[-1])
if d:
    print(f"frames={int(np.median(d))} min={min(d)} max={max(d)} n={len(d)} quantum={bs}")
else:
    print(f"frames=NA n=0 quantum={bs} (no impulse received)")
