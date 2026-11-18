"""Data for the live view's component maps, second version (birdnet-pc.bin).

pc_bands.py fitted PCA on the class rows; the live embedding then had to be
mapped onto them by percentile, and the light rarely fell on the heard
species. This version fits PCA on embeddings of real recordings instead, so
a live reading is a plain projection in the same space as the fit.

Species are not recordings, so each species needs a place on each component:

  - on components where the class rows projected onto the axes line up with
    the recording-based places (correlation FALLBACK_MIN_CORR or more, for
    the species that have both; printed): every species by its class row;
  - else the median score of the reference readings whose top class it is
    (score 0.5 or more), when it has at least MIN_READINGS of them;
  - else no place: the species stays dim (rank byte 255).

Writes the same layout as pc_bands.py (see there), with rank 255 meaning no
place. Prints the variance shares, the counts, and the same check as
pc_bands.py: fitted on half the recordings, checked on the other half.

  python -I pc_rec.py MODEL.onnx OUT.bin DIR [DIR ...]

Each DIR holds emb_raw.npz (key "birdnet", [N, 3, 1024]) and labels.npz (key
"rec", [N]) from batchi-noisier2noise/birdset_embed.py.

Needs onnx, numpy, scikit-learn.
"""

import os
import struct
import sys

import numpy as np
import onnx
from onnx import numpy_helper
from sklearn.decomposition import PCA

DENSE = "const_fold_opt__1508"
BIAS = "model/CLASS_DENSE_LAYER/BiasAdd/ReadVariableOp/resource"
CLASSES = 6522
N = 8
QUANTILES = np.linspace(0, 100, 21)
MIN_SCORE = 0.5
MIN_READINGS = 5
FALLBACK_MIN_CORR = 0.5


def load(dirs):
    embs, recs = [], []
    for i, d in enumerate(dirs):
        e = np.load(f"{d}/emb_raw.npz")["birdnet"]
        rec = np.load(f"{d}/labels.npz")["rec"]
        embs.append(e.reshape(-1, 1024))
        recs.append(np.repeat([f"{i}:{r}" for r in rec], e.shape[1]))
    return np.concatenate(embs).astype(np.float32), np.concatenate(recs)


def species_places(proj, top, ok):
    """Median component scores of each species' confident readings; NaN where
    a species has fewer than MIN_READINGS."""
    place = np.full((CLASSES, proj.shape[1]), np.nan, np.float32)
    counts = np.bincount(top[ok], minlength=CLASSES)
    for c in np.flatnonzero(counts >= MIN_READINGS):
        place[c] = np.median(proj[ok & (top == c)], axis=0)
    return place


def ranks(place):
    """Percentile rank 0..254 of each placed species per component, 255 for none."""
    out = np.full(place.shape, 255, np.uint8)
    for k in range(place.shape[1]):
        have = np.flatnonzero(~np.isnan(place[:, k]))
        order = have[np.argsort(place[have, k])]
        out[order, k] = np.round(np.arange(len(order)) / max(len(order) - 1, 1) * 254)
    return out


def fit(e, w, bias, verbose):
    unit = e / np.maximum(np.linalg.norm(e, axis=1, keepdims=True), 1e-9)
    pca = PCA(n_components=N, random_state=1).fit(unit)
    axes = pca.components_.astype(np.float32)
    proj = unit @ axes.T  # uncentred: a constant shift per axis, ranks unchanged
    logit = e @ w + bias
    top = logit.argmax(1)
    ok = 1 / (1 + np.exp(-logit.max(1))) >= MIN_SCORE
    place = species_places(proj, top, ok)
    rows = w.T / np.linalg.norm(w.T, axis=1, keepdims=True)
    row_proj = rows @ axes.T
    have = ~np.isnan(place[:, 0])
    corr = [np.corrcoef(place[have, k], row_proj[have, k])[0, 1] for k in range(N)]
    if verbose:
        print("variance shares (%):", " ".join(f"{v * 100:.1f}" for v in pca.explained_variance_ratio_))
        print(f"species placed from recordings: {have.sum()} of {CLASSES}")
        print("class-row projection vs recording-based place, correlation per PC:",
              " ".join(f"{c:.2f}" for c in corr))
    # Where the class rows line up with the recording-based places, rank all
    # species by their class row alone. Mixing the two sources in one ranking
    # pushed the 45 recording-placed species to the ends of every axis (their
    # medians spread wider than a regression-shrunk fallback), which lit the
    # wrong species and flattered the check.
    for k in range(N):
        if corr[k] >= FALLBACK_MIN_CORR:
            place[:, k] = row_proj[:, k]
    ref = np.percentile(proj, QUANTILES, axis=0).T.astype(np.float32)
    return pca, axes, ref, ranks(place)


def check(axes, ref, rank, e, w, bias):
    unit = e / np.maximum(np.linalg.norm(e, axis=1, keepdims=True), 1e-9)
    proj = unit @ axes.T
    logit = e @ w + bias
    top = logit.argmax(1)
    keep = 1 / (1 + np.exp(-logit.max(1))) >= MIN_SCORE
    rng = np.random.default_rng(1)
    for k in range(N):
        sel = keep & (rank[top, k] != 255)
        p = np.interp(proj[sel, k], ref[k], QUANTILES / 100)
        r = rank[top[sel], k] / 254
        d = np.abs(p - r).mean()
        ds = np.abs(p - rng.permutation(r)).mean()
        c = np.corrcoef(p, r)[0, 1]
        print(f"  PC{k + 1}: {sel.sum()} readings, mean gap {d:.3f}, shuffled {ds:.3f}, correlation {c:.2f}")


def main():
    model_path, out_path, *dirs = sys.argv[1:]
    inits = {i.name: i for i in onnx.load(model_path).graph.initializer}
    w = numpy_helper.to_array(inits[DENSE]).astype(np.float32)
    bias = numpy_helper.to_array(inits[BIAS]).astype(np.float32)
    e, rec = load(dirs)

    pca, axes, ref, rank = fit(e, w, bias, True)
    unlit = (rank == 255).sum(0)
    print("species with no place, per PC:", " ".join(str(u) for u in unlit))
    with open(out_path, "wb") as f:
        f.write(b"OPC1")
        f.write(struct.pack("<4H", N, 1024, CLASSES, len(QUANTILES)))
        f.write(pca.explained_variance_ratio_.astype("<f4").tobytes())
        f.write(axes.astype("<f4").tobytes())
        f.write(ref.astype("<f4").tobytes())
        f.write(rank.T.tobytes())
    print(f"wrote {out_path}, {os.path.getsize(out_path)} bytes, from {len(e)} readings")

    print(f"check, all readings (fit and check on the same data), top class {MIN_SCORE} or more:")
    check(axes, ref, rank, e, w, bias)
    # Held out: fit on half the recordings, check on the rest.
    recs = np.unique(rec)
    half = set(np.random.default_rng(2).permutation(recs)[: len(recs) // 2])
    a = np.array([r in half for r in rec])
    _, axes_a, ref_a, rank_a = fit(e[a], w, bias, False)
    print("check, held out (fit on half the recordings, check on the other half):")
    check(axes_a, ref_a, rank_a, e[~a], w, bias)


if __name__ == "__main__":
    main()
