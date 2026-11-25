# SPDX-License-Identifier: GPL-3.0-or-later
"""S4 helper: write a float32 stereo noise WAV, or compare a recording with it.
usage: s4_wav.py gen OUT.wav | s4_wav.py cmp REF.wav REC.wav GAIN"""
import struct, sys
import numpy as np

def write(path, data, rate=48000):
    frames, ch = data.shape
    raw = data.astype("<f4").tobytes()
    fmt = struct.pack("<HHIIHH", 3, ch, rate, rate * ch * 4, ch * 4, 32)
    with open(path, "wb") as f:
        f.write(b"RIFF" + struct.pack("<I", 4 + 8 + len(fmt) + 8 + len(raw)) + b"WAVE")
        f.write(b"fmt " + struct.pack("<I", len(fmt)) + fmt)
        f.write(b"data" + struct.pack("<I", len(raw)) + raw)

def read(path):
    b = open(path, "rb").read(); i = 12; fmt = None
    while i < len(b):
        cid, size = b[i:i+4], struct.unpack("<I", b[i+4:i+8])[0]
        body = b[i+8:i+8+size]
        if cid == b"fmt ": fmt = struct.unpack("<HHIIHH", body[:16])
        if cid == b"data":
            assert fmt and fmt[0] == 3 and fmt[5] == 32, f"need float32, got {fmt}"
            if size in (0, 0xFFFFFFFF): body = b[i+8:]
            return np.frombuffer(body[: len(body) // 4 * 4], "<f4").reshape(-1, fmt[1])
        i += 8 + size + (size & 1)

if sys.argv[1] == "gen":
    rng = np.random.default_rng(7)
    x = rng.uniform(-0.5, 0.5, (48000 * 3, 2)).astype(np.float32)
    x[np.abs(x) < 1e-3] = 1e-3        # no zeros, so the start can be found
    write(sys.argv[2], x)
else:
    ref, rec, g = read(sys.argv[2]), read(sys.argv[3]), np.float32(sys.argv[4])
    want = (ref * g).astype(np.float32)
    nz = np.flatnonzero(np.abs(rec[:, 0]) > 0)
    if not nz.size: print("RESULT no signal recorded"); sys.exit(1)
    s = nz[0]; n = min(len(want), len(rec) - s) - 64
    a, b = rec[s:s+n], want[:n]
    exact = np.array_equal(a, b); maxerr = float(np.max(np.abs(a - b)))
    print(f"RESULT gain={g} offset={s} compared={n} frames bit_exact={exact} max_abs_err={maxerr:.3e}")
