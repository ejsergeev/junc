#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""A stand-in for Hyprland's IPC (architecture §7.6, hidden-window gate).

Serves `j/clients` and `j/monitors` on .socket.sock and events on
.socket2.sock under $XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE, for
one window. Commands on stdin, one per line:

    pid N   the window belongs to process N (no window before that)
    ws N    the window moves to workspace N (the monitor shows workspace 1)
    quit

usage: fake_hyprland.py
"""

import json
import os
import socket
import sys
import threading

base = os.path.join(
    os.environ["XDG_RUNTIME_DIR"], "hypr", os.environ["HYPRLAND_INSTANCE_SIGNATURE"]
)
os.makedirs(base, exist_ok=True)
state = {"ws": 1, "pid": None}
listeners = []
lock = threading.Lock()


def listen(name):
    path = os.path.join(base, name)
    if os.path.exists(path):
        os.unlink(path)
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.bind(path)
    s.listen(8)
    return s


def answer(cmd):
    if cmd == "j/clients":
        if state["pid"] is None:
            return []
        return [
            {
                "pid": state["pid"],
                "mapped": True,
                "hidden": False,
                "workspace": {"id": state["ws"], "name": str(state["ws"])},
                "fullscreen": 0,
            }
        ]
    if cmd == "j/monitors":
        return [{"activeWorkspace": {"id": 1}, "specialWorkspace": {"id": 0}}]
    return {}


def commands(server):
    while True:
        conn, _ = server.accept()
        with conn:
            cmd = conn.recv(4096).decode()
            with lock:
                body = json.dumps(answer(cmd))
            conn.sendall(body.encode())


def events(server):
    while True:
        conn, _ = server.accept()
        with lock:
            listeners.append(conn)


def emit(line):
    with lock:
        for c in list(listeners):
            try:
                c.sendall((line + "\n").encode())
            except OSError:
                listeners.remove(c)


threading.Thread(target=commands, args=(listen(".socket.sock"),), daemon=True).start()
threading.Thread(target=events, args=(listen(".socket2.sock"),), daemon=True).start()
print("ready", flush=True)
for line in sys.stdin:
    words = line.split()
    if not words:
        continue
    if words[0] == "quit":
        break
    if words[0] == "pid":
        with lock:
            state["pid"] = int(words[1])
        emit("openwindow>>abc,1,junc,JUNC")
    if words[0] == "ws":
        with lock:
            state["ws"] = int(words[1])
        emit(f"movewindowv2>>abc,{words[1]},{words[1]}")
        emit(f"movewindow>>abc,{words[1]}")
    print("ok", flush=True)
