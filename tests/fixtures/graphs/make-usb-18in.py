#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Derives the synthetic 18-input USB card fixtures (F15, layer 1) from the
real Komplete Audio 2 snapshots: same PipeWire/ACP structure, renamed to a
made-up card, with the Pro Audio capture node widened to 18 channels.

usage: python3 tests/fixtures/graphs/make-usb-18in.py   (from the repo root)
"""
import copy
import json

DIR = "tests/fixtures/graphs"
OLD = "Native_Instruments_Komplete_Audio_2_FFFFFFFF-00"
NEW = "JUNC_Test_18in-00"
CHANNELS = 18


def rename(text: str) -> str:
    return (
        text.replace(OLD, NEW)
        .replace("Native_Instruments_Komplete_Audio_2_FFFFFFFF", "JUNC_Test_18in")
        .replace("Native Instruments Komplete Audio 2", "JUNC Test 18in")
        .replace("Komplete Audio 2", "JUNC Test 18in")
        .replace("Native Instruments", "JUNC Test")
    )


def load(name: str):
    with open(f"{DIR}/{name}") as f:
        return json.loads(rename(f.read()))


def save(name: str, objs) -> None:
    with open(f"{DIR}/{name}", "w") as f:
        json.dump(objs, f, indent=2, sort_keys=True)
        f.write("\n")


save("usb-18in-stereo.json", load("komplete-audio-2-stereo.json"))

pro = load("komplete-audio-2-proaudio.json")
node = next(
    o
    for o in pro
    if o["type"].endswith("Node")
    and o["info"]["props"].get("node.name", "").endswith(".pro-input-0")
)
props = node["info"]["props"]
props["audio.channels"] = CHANNELS
props["audio.position"] = "[ " + ", ".join(f"AUX{c}" for c in range(CHANNELS)) + " ]"
node["info"]["n-output-ports"] = CHANNELS
ports = [
    o for o in pro if o["type"].endswith("Port") and o["info"]["props"]["node.id"] == node["id"]
]
template = ports[0]
serial = max(o["info"]["props"].get("object.serial", 0) for o in pro if "info" in o) + 1
for c in range(len(ports), CHANNELS):
    p = copy.deepcopy(template)
    p["id"] = 9000 + c
    pp = p["info"]["props"]
    pp.update(
        {
            "audio.channel": f"AUX{c}",
            "object.id": p["id"],
            "object.serial": serial + c,
            "object.path": pp["object.path"].rsplit("_", 1)[0] + f"_{c}",
            "port.alias": f"JUNC Test 18in:capture_AUX{c}",
            "port.id": c,
            "port.name": f"capture_AUX{c}",
        }
    )
    pro.append(p)
save("usb-18in-proaudio.json", pro)
