"""Check that BirdNET+ V3.0's maps put a recording's windows beside the
species they sound like.

Runs the pruned model (onnxruntime) on 3 s windows end to end, as Analyse
does, places each window with a class at MIN or more by the app's rule
(src/classifier/species_map.rs: the confidence-weighted mean of the top 3
classes within the radius of the top class), and prints, per window and
layout: the top 3 classes, which of them pulled the dot, the 5 classes
nearest the dot, and the rank of the top class among all classes by cosine
between the window's embedding and the class rows (1 = the row nearest the
embedding). Draws each layout with the windows on it.

  python -I v3_place_check.py MODEL.onnx LABELS.tsv TSNE.bin UMAP.bin OUT_DIR \\
      --radius 0.105 --umap-radius 0.07 [--min 0.25] RECORDING [RECORDING ...]

Writes OUT_DIR/v3-place-tsne.png and OUT_DIR/v3-place-umap.png.

Needs onnxruntime, numpy, scipy, soundfile, scikit-learn, matplotlib.
"""

import os
import sys
from math import gcd

import numpy as np
import onnxruntime as ort
import soundfile as sf
from scipy.signal import resample_poly
from sklearn.neighbors import NearestNeighbors

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from project_v3 import group_of, read_labels, unit  # noqa: E402

RATE = 32000
WIN = 3 * RATE
PLACE_TOP = 3


def read_map(path):
    return np.frombuffer(open(path, "rb").read(), "<i2").reshape(-1, 2).astype(np.float32) / 32767


def place(top, conf, xy, radius):
    x1 = xy[top[0]]
    used, sw, s = [], 0.0, np.zeros(2)
    for c, w in zip(top[:PLACE_TOP], conf[:PLACE_TOP]):
        if np.linalg.norm(xy[c] - x1) > radius:
            continue
        used.append(c)
        s += xy[c] * w
        sw += w
    return s / sw, used


def main():
    args = sys.argv[1:]
    opts = {"--radius": None, "--umap-radius": None, "--min": "0.25"}
    for k in list(opts):
        if k in args:
            i = args.index(k)
            opts[k] = args[i + 1]
            del args[i : i + 2]
    model, labels_tsv, tsne_path, umap_path, out_dir, *recs = args
    radius = {"t-SNE": float(opts["--radius"]), "UMAP": float(opts["--umap-radius"])}
    min_conf = float(opts["--min"])
    labels = read_labels(labels_tsv)
    eng = [r[2] for r in labels]
    groups = [group_of(r[3]) for r in labels]
    maps = {"t-SNE": read_map(tsne_path), "UMAP": read_map(umap_path)}
    near = {k: NearestNeighbors(n_neighbors=5).fit(v) for k, v in maps.items()}

    import onnx
    from onnx import numpy_helper

    inits = {i.name: i for i in onnx.load(model).graph.initializer}
    rows = unit(numpy_helper.to_array(inits["head.weight_stored_fp16"]).astype(np.float32))
    sess = ort.InferenceSession(model, providers=["CPUExecutionProvider"])

    placed = {k: [] for k in maps}
    for path in recs:
        x, sr = sf.read(path, dtype="float32", always_2d=True)
        x = x.mean(1)
        g = gcd(sr, RATE)
        y = resample_poly(x, RATE // g, sr // g).astype(np.float32)
        tag = os.path.basename(path).split(" - ")[1] if " - " in path else os.path.basename(path)
        print(f"\n{os.path.basename(path)} ({sr} Hz, {len(x) / sr:.1f} s)")
        for k in range(len(y) // WIN):
            w = y[k * WIN : (k + 1) * WIN][None]
            if not w.any():
                continue
            pred, emb = sess.run(["predictions", "embeddings"], {"input": w})
            pred, emb = pred[0], emb[0]
            top = np.argsort(-pred)[:5]
            conf = pred[top]
            cos = rows @ (emb / np.linalg.norm(emb))
            cos_rank = int((cos > cos[top[0]]).sum()) + 1
            head = ", ".join(f"{eng[c]} {conf[i]:.2f}" for i, c in enumerate(top[:3]))
            print(f"  {k * 3:>3}-{k * 3 + 3} s: {head}; top class's row is number {cos_rank} by cosine to the embedding")
            if conf[0] < min_conf:
                print(f"    below {min_conf}: not placed")
                continue
            for name, xy in maps.items():
                at, used = place(top, conf, xy, radius[name])
                _, idx = near[name].kneighbors(at[None])
                nb = ", ".join(eng[i] for i in idx[0])
                pulled = [eng[c] for c in used]
                print(f"    {name}: at ({at[0]:+.3f}, {at[1]:+.3f}), pulled by {len(used)} of 3 ({'; '.join(pulled)}); nearest: {nb}")
                placed[name].append((at, tag))

    import matplotlib

    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    colours = {"birds": "#c9ced4", "insects": "#f2c09a", "amphibians": "#a7d8b9", "mammals": "#b4cff2", "other animals": "#dcb2e6", "sounds": "#555555"}
    marks = ["o", "s", "^", "D", "v"]
    for name, xy in maps.items():
        fig, ax = plt.subplots(figsize=(9, 9), dpi=110)
        g = np.array(groups)
        for gname, c in colours.items():
            sel = g == gname
            ax.scatter(xy[sel, 0], xy[sel, 1], s=2, c=c, lw=0, label=gname)
        tags = sorted({t for _, t in placed[name]}, key=[t for _, t in placed[name]].index)
        for i, t in enumerate(tags):
            pts = np.array([p for p, tt in placed[name] if tt == t])
            ax.scatter(pts[:, 0], pts[:, 1], s=40, marker=marks[i % len(marks)], facecolor="none", edgecolor=f"C{i}", lw=1.5, label=f"{t} ({len(pts)} windows)")
        ax.set_aspect("equal")
        ax.set_xlim(-1.05, 1.05)
        ax.set_ylim(-1.05, 1.05)
        ax.set_title(f"BirdNET+ V3.0 {name} layout: test recordings' windows")
        ax.legend(loc="lower left", markerscale=2, fontsize=8)
        fig.tight_layout()
        out = os.path.join(out_dir, f"v3-place-{'tsne' if name == 't-SNE' else 'umap'}.png")
        fig.savefig(out)
        plt.close(fig)
        print(f"wrote {out}")


if __name__ == "__main__":
    main()
