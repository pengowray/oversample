"""UMAP layout of BirdNET V2.4's 6,522 classes, for the live view's component
maps, and a check of which layout and basis light compact regions.

Lays out the class rows (the final dense layer, each row scaled to length 1,
cosine metric; no recordings) with a few UMAP settings, times each, and
scores every layout against every component basis:

  compactness = for each class, the share of its K nearest neighbours on the
  2-D layout that fall in the same band as the class on that component, then
  the mean over classes. 0.5 is chance for halves, 1.0 is perfectly compact.
  For outer thirds, only classes in the outer thirds count, and chance is
  1/3 (a neighbour in the same third).

Writes the chosen layout like research/species-map/birdnet-map.bin: 6,522 x 2
int16, little endian, x then y per class, scaled to -32767..32767.

  python -I species_umap.py MODEL.onnx TSNE_MAP.bin PC_ROWS.bin PC_REC.bin OUT.bin

Needs onnx, numpy, scikit-learn, umap-learn.
"""

import sys
import time

import numpy as np
import onnx
from onnx import numpy_helper
from sklearn.neighbors import NearestNeighbors

DENSE = "const_fold_opt__1508"
CLASSES = 6522
K = 10
SEED = 1
SETTINGS = [(15, 0.1), (50, 0.1), (50, 0.5)]


def class_rows(model_path):
    for init in onnx.load(model_path).graph.initializer:
        if init.name == DENSE:
            w = numpy_helper.to_array(init).astype(np.float32).T
            return w / np.linalg.norm(w, axis=1, keepdims=True)
    raise SystemExit("dense layer not found")


def read_map(path):
    xy = np.frombuffer(open(path, "rb").read(), "<i2").reshape(-1, 2).astype(np.float32)
    return xy / 32767


def read_ranks(path, rows):
    """Percentile of each class on each component. The class-row file
    (pc_bands.py) ranks 0..255; the recordings file (pc_rec.py) ranks 0..254
    with 255 for no place (NaN here)."""
    b = open(path, "rb").read()
    n, d, c, q = np.frombuffer(b[4:12], "<u2")
    off = 12 + 4 * (n + n * d + n * q)
    r = np.frombuffer(b[off:], np.uint8).reshape(n, c).astype(np.float32)
    p = r / (255 if rows else 254)
    if not rows:
        p[r == 255] = np.nan
    return p


def compactness(xy, p, bands, outer):
    nn = NearestNeighbors(n_neighbors=K + 1).fit(xy)
    _, idx = nn.kneighbors(xy)
    idx = idx[:, 1:]
    out = []
    for k in range(p.shape[0]):
        band = np.minimum(np.floor(p[k] * bands), bands - 1)
        have = ~np.isnan(band)
        sel = have.copy()
        if outer:
            sel &= band != 1
        nb = band[idx]
        same = (nb == band[:, None]).mean(1)
        out.append(float(same[sel].mean()) if sel.any() else float("nan"))
    return out


def main():
    model, tsne_path, rows_path, rec_path, out_path = sys.argv[1:]
    rows = class_rows(model)
    import umap

    layouts = {"t-SNE": read_map(tsne_path)}
    for nn, md in SETTINGS:
        t = time.time()
        xy = umap.UMAP(n_neighbors=nn, min_dist=md, metric="cosine", random_state=SEED).fit_transform(rows)
        print(f"UMAP n_neighbors={nn} min_dist={md}: {time.time() - t:.1f} s")
        # Centre on the median and scale so 99.5% of classes are inside -1..1;
        # the few outlying classes are clamped to the edge, so they do not
        # squeeze the rest of the map into the middle.
        xy = xy - np.median(xy, axis=0)
        xy = np.clip(xy / np.percentile(np.abs(xy), 99.5), -1, 1)
        layouts[f"UMAP {nn}/{md}"] = xy.astype(np.float32)

    bases = {"class rows": read_ranks(rows_path, True), "recordings": read_ranks(rec_path, False)}
    best, best_score = None, -1
    for bname, p in bases.items():
        for mode, bands, outer in [("halves", 2, False), ("outer thirds", 3, True)]:
            print(f"\n{bname}, {mode}: share of {K} nearest neighbours in the same band, PC1..PC{p.shape[0]}")
            for lname, xy in layouts.items():
                c = compactness(xy, p, bands, outer)
                mean = np.nanmean(c)
                print(f"  {lname:16s} mean {mean:.2f}:  " + " ".join(f"{v:.2f}" for v in c))
                if lname != "t-SNE" and mode == "halves" and mean > best_score:
                    best, best_score = lname, mean
    print(f"\nchosen: {best}")
    xy = layouts[best]
    # Centre the layout and scale it so its longer side runs from -1 to 1:
    # the map then fills the view, as the t-SNE layout does.
    lo, hi = xy.min(0), xy.max(0)
    xy = (xy - (lo + hi) / 2) / ((hi - lo).max() / 2)
    np.round(xy * 32767).astype("<i2").tofile(out_path)
    print(f"wrote {out_path}")


if __name__ == "__main__":
    main()
