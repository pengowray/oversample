"""One PNG per recording: a linear-frequency spectrogram (0 to 16 kHz, grey, louder is
darker) in rows of 15 s, with the reference boxes (BirdBox's CLI, solid blue) and the
Rust boxes (dashed orange) on top.

  python -I plot.py AUDIO_DIR REF.tsv RUST.tsv OUT_DIR NAME [NAME ...] [--max-seconds 120]

REF.tsv and RUST.tsv are patterns with {name} (merged layout: t0, t1, f0, f1, avg_conf,
...).
"""

import argparse
import os

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402
import numpy as np  # noqa: E402
import soundfile as sf  # noqa: E402
from matplotlib.lines import Line2D  # noqa: E402
from matplotlib.patches import Rectangle  # noqa: E402
from scipy.signal import stft  # noqa: E402

REF, OURS = "#2a78d6", "#eb6834"
ROW_S = 15.0

ap = argparse.ArgumentParser()
ap.add_argument("audio")
ap.add_argument("ref")
ap.add_argument("ours")
ap.add_argument("out")
ap.add_argument("names", nargs="+")
ap.add_argument("--max-seconds", type=float, default=120.0)
a = ap.parse_args()
os.makedirs(a.out, exist_ok=True)


def boxes(path):
    try:
        v = np.loadtxt(path, ndmin=2)
    except (OSError, ValueError):
        return np.zeros((0, 7))
    return v if v.size else np.zeros((0, 7))


for name in a.names:
    x, sr = sf.read(os.path.join(a.audio, name + ".wav"), dtype="float32")
    dur = min(len(x) / sr, a.max_seconds)
    x = x[: int(dur * sr)]
    f, t, z = stft(x, sr, nperseg=1024, noverlap=1024 - 256)
    db = 20 * np.log10(np.abs(z) + 1e-9)
    vmax = np.percentile(db, 99.7)
    ref, ours = boxes(a.ref.format(name=name)), boxes(a.ours.format(name=name))
    rows = int(np.ceil(dur / ROW_S))
    fig, axes = plt.subplots(rows, 1, figsize=(14, 2.3 * rows + 0.6), squeeze=False)
    for r, ax in enumerate(axes[:, 0]):
        t0, t1 = r * ROW_S, (r + 1) * ROW_S
        sel = (t >= t0 - 0.1) & (t <= t1 + 0.1)
        ax.pcolormesh(t[sel], f / 1000, db[:, sel], cmap="gray_r", vmin=vmax - 70, vmax=vmax,
                      shading="nearest", rasterized=True)
        for bx, colour, style, width in ((ref, REF, "-", 1.6), (ours, OURS, (0, (3, 2)), 1.4)):
            for b in bx:
                if b[1] < t0 or b[0] > t1:
                    continue
                ax.add_patch(Rectangle((b[0], b[2] / 1000), b[1] - b[0], (b[3] - b[2]) / 1000,
                                       fill=False, edgecolor=colour, linestyle=style, linewidth=width))
        ax.set_xlim(t0, t1)
        ax.set_ylim(0, 16)
        ax.set_ylabel("kHz")
        ax.tick_params(labelsize=8)
    axes[-1, 0].set_xlabel("seconds")
    axes[0, 0].set_title(
        f"{name}: {len(ref)} reference boxes, {len(ours)} Rust boxes"
        + (f" (first {dur:.0f} s)" if len(x) / sr < sf.info(os.path.join(a.audio, name + '.wav')).duration - 0.01 else ""),
        loc="left", fontsize=10)
    axes[0, 0].legend(
        handles=[Line2D([], [], color=REF, lw=1.6, label="BirdBox CLI (reference)"),
                 Line2D([], [], color=OURS, lw=1.4, ls=(0, (3, 2)), label="Rust + tract")],
        loc="upper right", fontsize=8, framealpha=0.9)
    fig.tight_layout()
    fig.savefig(os.path.join(a.out, f"{name}.png"), dpi=80)
    plt.close(fig)
    print(name)
