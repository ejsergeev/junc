#!/usr/bin/env python3
"""Summarise `pw-top -b` output: quantum, W/Q, B/Q, ERR deltas per node (first vs last sample)."""
import sys, re, statistics as st

rows = {}  # name -> list of (quant, wq, bq, err)
hdr = re.compile(r"^S\s+ID\s+QUANT")
for line in open(sys.argv[1], errors="replace"):
    if hdr.match(line) or not line[:1] in "RS":
        continue
    parts = line.split()
    if len(parts) < 9 or parts[0] != "R":
        continue
    try:
        quant = int(parts[2]); err = int(parts[8])
        wq = float(parts[6]) if parts[6] != "---" else None
        bq = float(parts[7]) if parts[7] != "---" else None
    except ValueError:
        continue
    name = " ".join(parts[9:]).lstrip("+ ").split()[-1] if len(parts) > 9 else parts[1]
    rows.setdefault(name, []).append((quant, wq, bq, err))

if not rows:
    print("no samples"); sys.exit()
samples = max(len(v) for v in rows.values())
drivers = {n: v for n, v in rows.items() if any(q for q, *_ in v)}
quants = sorted({q for v in drivers.values() for q, *_ in v if q})
print(f"samples={samples} quanta_seen={quants}")
tot = 0
for n, v in sorted(rows.items()):
    d = v[-1][3] - v[0][3]; tot += d
    bq = [x[2] for x in v if x[2] is not None]
    print(f"node={n} err_delta={d} err_end={v[-1][3]} bq_max={max(bq) if bq else 'NA'}")
print(f"ERR_TOTAL_DELTA={tot}")
