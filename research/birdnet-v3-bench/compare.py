"""Compare saved BirdNET+ V3.0 runs (from ref.py or the tract `native` binary) with a
reference run.

  python -I compare.py LABELS.csv REF_PREFIX [--ref-labels CSV] OTHER_PREFIX[:LABELS.csv] ...

Each prefix names <prefix>.score.f32 and <prefix>.emb.f32. A run with a different
class list (the BirdNET Live file) gives its labels file after a colon; its classes
are matched to the reference's on birdnet_id, and only the shared classes are
compared. Windows can be split into groups with --quiet-last N: the last N windows
(silence and noise) are reported apart from the recordings.
"""

import argparse
import csv

import numpy as np

EMB = 1280

ap = argparse.ArgumentParser()
ap.add_argument("labels", help="labels CSV of the reference run")
ap.add_argument("ref")
ap.add_argument("others", nargs="+")
ap.add_argument("--quiet-last", type=int, default=0)
a = ap.parse_args()


def ids(path):
    with open(path, encoding="utf-8") as f:
        return [r["birdnet_id"] for r in csv.DictReader(f)]


def load(prefix, n_classes):
    s = np.fromfile(prefix + ".score.f32", "<f4")
    e = np.fromfile(prefix + ".emb.f32", "<f4").reshape(-1, EMB)
    return s.reshape(len(e), -1), e


ref_ids = ids(a.labels)
rs, re_ = load(a.ref, len(ref_ids))
n = len(rs)
q = a.quiet_last
groups = [("recordings", slice(0, n - q))] + ([("silence+noise", slice(n - q, n))] if q else [])

print(f"reference: {a.ref} ({n} windows, {rs.shape[1]} classes)")
print(
    "run | windows | same top-1 | top-5 same set | same classes at 0.25 or more | "
    "max abs score diff | max abs diff, top-1 class | embedding cosine (min)"
)
for spec in a.others:
    prefix, _, lab = spec.partition(":")
    os_, oe = load(prefix, 0)
    if lab:
        oid = ids(lab)
        pos = {b: i for i, b in enumerate(ref_ids)}
        cols = [pos[b] for b in oid]
        rsel = rs[:, cols]
    else:
        rsel = rs
    for name, g in groups:
        r, o = rsel[g], os_[g]
        top_r, top_o = r.argmax(1), o.argmax(1)
        same1 = int((top_r == top_o).sum())
        t5 = sum(
            set(np.argsort(-r[i])[:5]) == set(np.argsort(-o[i])[:5]) for i in range(len(r))
        )
        over = sum(
            set(np.flatnonzero(r[i] >= 0.25)) == set(np.flatnonzero(o[i] >= 0.25)) for i in range(len(r))
        )
        d = np.abs(r - o).max()
        dtop = np.abs(r[np.arange(len(r)), top_r] - o[np.arange(len(r)), top_r]).max()
        er, eo = re_[g], oe[g]
        cos = (er * eo).sum(1) / (np.linalg.norm(er, axis=1) * np.linalg.norm(eo, axis=1) + 1e-12)
        print(
            f"{prefix} ({name}) | {len(r)} | {same1}/{len(r)} | {t5}/{len(r)} | {over}/{len(r)} | {d:.4f} | {dtop:.4f} | {cos.min():.6f}"
        )
