"""Recheck a Perch run against a reference on labelled recordings, by group.

  python recheck.py MODELS_DIR windows.list REF_PREFIX OTHER_PREFIX [OTHER_PREFIX ...]

windows.list comes from `prep.py --list` (each window's source file). Each recording's
group (birds, frogs, grasshoppers, land mammals) and species come from the xeno-canto
`.xc.json` next to it. Prints, per group:

1. How often each other run's top 1 and top 5 match the reference's (same set, same order).
2. How often the reference names the recording's species: the species' best rank over
   the recording's windows, counted as top 1 and top 5. Recordings whose species is not
   among Perch's classes count separately.
3. How often a sound-event class (the 198 FSD50K labels, such as Cricket or Insect) is the
   top 1, or in the top 5.
"""

import collections
import json
import os
import sys

import numpy as np

models, listing, ref, others = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4:]
labels = [l.strip() for l in open(os.path.join(models, "labels.csv"), encoding="utf-8")][1:]
C = len(labels)
event = np.array([" " not in l for l in labels])
index = {l: i for i, l in enumerate(labels)}

files = [l.rstrip("\n") for l in open(listing, encoding="utf-8")]
meta = {}
for f in set(files):
    d = json.load(open(f[: -len(".wav")] + ".xc.json", encoding="utf-8"))
    meta[f] = (d.get("group") or d["grp"], f"{d['gen']} {d['sp']}")
groups = np.array([meta[f][0] for f in files])
load = lambda p: np.fromfile(p + ".label.f32", dtype="<f4").reshape(len(files), C)
rl = load(ref)
top5 = lambda l: np.argsort(-l, axis=1)[:, :5]
r5 = top5(rl)

print("1. Same answers as the reference (windows)")
for p in others:
    l = load(p)
    o5 = top5(l)
    print(f"  {os.path.basename(p)}: largest logit difference {np.abs(l - rl).max():.4f}")
    for g in sorted(set(groups)) + ["all"]:
        m = groups == g if g != "all" else np.ones(len(files), bool)
        t1 = np.sum(o5[m, 0] == r5[m, 0])
        s5 = sum(set(a) == set(b) for a, b in zip(o5[m], r5[m]))
        o = sum((a == b).all() for a, b in zip(o5[m], r5[m]))
        print(f"    {g:13s} {m.sum():3d} windows: top 1 same {t1}, top 5 same set {s5}, same order {o}")

print("\n2. Reference names the recording's species (best rank over its windows)")
by = collections.defaultdict(lambda: [0, 0, 0, 0])
for f in sorted(set(files)):
    g, sp = meta[f]
    if sp == "Mystery mystery":
        continue
    c = by[g]
    c[0] += 1
    if sp not in index:
        c[3] += 1
        continue
    rows = rl[[i for i, x in enumerate(files) if x == f]]
    rank = min(int(np.sum(r > r[index[sp]])) for r in rows)
    c[1] += rank == 0
    c[2] += rank < 5
for g, (n, t1, t5, missing) in sorted(by.items()):
    print(f"  {g:13s} {n:2d} recordings: species not a Perch class {missing}; top 1 {t1}, top 5 {t5}")

print("\n3. Sound-event classes in the reference (windows)")
for g in sorted(set(groups)):
    m = groups == g
    t1 = event[r5[m, 0]].sum()
    t5 = event[r5[m]].any(1).sum()
    common = collections.Counter(labels[i] for i in r5[m].ravel() if event[i]).most_common(4)
    print(f"  {g:13s} top 1 {t1}/{m.sum()}, in top 5 {t5}/{m.sum()}; most often {common}")
