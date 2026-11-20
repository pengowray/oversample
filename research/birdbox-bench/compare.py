"""Compare two runs stage by stage. Paths are patterns with {name} for the recording.

  python -I compare.py images A.images.f32 B.images.f32 NAME [NAME ...]
      Pixel values (windows x 3 x 256 x 256): pixels where any channel differs, the
      largest difference, and windows with any difference.
  python -I compare.py heads A.head.f32 B.head.f32 NAME [...] [--conf 0.15]
      Network outputs (windows x 5 x 1344): largest score difference, largest box
      difference (pixels) on anchors that score above conf in either, and anchors that
      are above conf in one and not in the other.
  python -I compare.py boxes raw|merged A.tsv B.tsv NAME [...] [--per-file]
      Boxes matched one to one by 2D IoU (time x Hz), pairs taken by decreasing IoU, the
      rule of BirdBox's tests/model_format_parity.py; window boxes only match within
      the same window. A is the reference. Reported at IoU 0.5 and 0.75, with mean IoU
      and confidence differences of the 0.5 pairs (merged boxes: avg_conf).
"""

import argparse

import numpy as np

ap = argparse.ArgumentParser()
ap.add_argument("what")
ap.add_argument("args", nargs="+")
ap.add_argument("--conf", type=float, default=0.15)
ap.add_argument("--per-file", action="store_true")
a = ap.parse_args()


def load_tsv(path):
    try:
        v = np.loadtxt(path, ndmin=2)
    except (OSError, ValueError):
        return np.zeros((0, 7))
    return v if v.size else np.zeros((0, 7))


def iou(x, y):
    t = min(x[1], y[1]) - max(x[0], y[0])
    f = min(x[3], y[3]) - max(x[2], y[2])
    if t <= 0 or f <= 0:
        return 0.0
    inter = t * f
    union = (x[1] - x[0]) * (x[3] - x[2]) + (y[1] - y[0]) * (y[3] - y[2]) - inter
    return inter / union if union > 0 else 0.0


def match(ref, cand, thr, window):
    """ref and cand rows: (t0, t1, f0, f1, conf, window_start). Returns pairs (iou, i, j)."""
    pairs = []
    for i, r in enumerate(ref):
        for j, c in enumerate(cand):
            if window and abs(r[5] - c[5]) > 1e-6:
                continue
            v = iou(r, c)
            if v >= thr:
                pairs.append((v, i, j))
    used_r, used_c, out = set(), set(), []
    for v, i, j in sorted(pairs, reverse=True):
        if i in used_r or j in used_c:
            continue
        used_r.add(i)
        used_c.add(j)
        out.append((v, i, j))
    return out


if a.what == "images":
    pa, pb, *names = a.args
    print("| Recording | Windows | Pixels that differ | Largest difference | Windows with a difference |")
    print("|---|---|---|---|---|")
    for n in names:
        x = np.fromfile(pa.format(name=n), np.float32).reshape(-1, 3, 256, 256)
        y = np.fromfile(pb.format(name=n), np.float32).reshape(-1, 3, 256, 256)
        assert x.shape == y.shape, (n, x.shape, y.shape)
        d = np.abs(x - y)
        px = (d.max(axis=1) > 0)
        print(f"| {n} | {len(x)} | {int(px.sum())} of {px.size} ({px.sum() / px.size:.2e}) | {d.max():.4f} | {int(px.reshape(len(x), -1).any(axis=1).sum())} |")

elif a.what == "heads":
    pa, pb, *names = a.args
    print("| Recording | Windows | Largest score difference | Largest box difference (px), anchors above conf | Anchors above conf in only one |")
    print("|---|---|---|---|---|")
    for n in names:
        x = np.fromfile(pa.format(name=n), np.float32).reshape(-1, 5, 1344)
        y = np.fromfile(pb.format(name=n), np.float32).reshape(-1, 5, 1344)
        sd = np.abs(x[:, 4] - y[:, 4]).max()
        live = (x[:, 4] > a.conf) | (y[:, 4] > a.conf)
        bd = np.abs(x[:, :4] - y[:, :4]).max(axis=1)[live].max() if live.any() else 0.0
        flip = int(((x[:, 4] > a.conf) != (y[:, 4] > a.conf)).sum())
        print(f"| {n} | {len(x)} | {sd:.5f} | {bd:.3f} | {flip} of {int(live.sum())} |")

elif a.what == "boxes":
    kind, pa, pb, *names = a.args
    window = kind == "raw"
    tot = dict(ref=0, cand=0, m50=0, m75=0, ious=[], dconf=[])
    rows = []
    for n in names:
        ra, rb = load_tsv(pa.format(name=n)), load_tsv(pb.format(name=n))

        def rows_of(v):
            if window:   # clip_start, t0, t1, f0, f1, conf
                return [(r[1], r[2], r[3], r[4], r[5], r[0]) for r in v]
            return [(r[0], r[1], r[2], r[3], r[4], 0.0) for r in v]   # t0, t1, f0, f1, avg_conf

        ref, cand = rows_of(ra), rows_of(rb)
        m50 = match(ref, cand, 0.5, window)
        m75 = match(ref, cand, 0.75, window)
        tot["ref"] += len(ref)
        tot["cand"] += len(cand)
        tot["m50"] += len(m50)
        tot["m75"] += len(m75)
        tot["ious"] += [v for v, _, _ in m50]
        tot["dconf"] += [abs(ref[i][4] - cand[j][4]) for _, i, j in m50]
        rows.append((n, len(ref), len(cand), len(m50), len(m75)))
    if a.per_file:
        print("| Recording | Reference | This | Matched at 0.5 | Matched at 0.75 |")
        print("|---|---|---|---|---|")
        for r in rows:
            print("| " + " | ".join(str(v) for v in r) + " |")
    ious, dc = np.array(tot["ious"] or [0.0]), np.array(tot["dconf"] or [0.0])
    print(f"reference {tot['ref']}, this {tot['cand']}; matched at IoU 0.5: {tot['m50']} "
          f"(missed {tot['ref'] - tot['m50']}, extra {tot['cand'] - tot['m50']}); at 0.75: {tot['m75']}; "
          f"mean IoU {ious.mean():.4f}, lowest {ious.min():.4f}; confidence difference mean {dc.mean():.5f}, largest {dc.max():.5f}")
