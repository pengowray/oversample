"""BirdNET+ V3.0 embeddings of BirdSet's HSN and POW test windows, the
recordings the V2.4 and Perch component files take their reference
quantiles from (batchi-noisier2noise/birdset_embed.py made those).

For each 5 s window listed in LABELS.npz (keys "rec" and "start", in
seconds), the 3 s starting OFFSET seconds in (default 1, the middle 3 s) is
run through the model with onnxruntime. The recordings are 32 kHz, the
model's rate, so nothing is resampled. Windows of digital silence are not
run (the pruned model gives birds there); they are marked in "silent".

  python -I birdset_embed_v3.py MODEL.onnx DATA_DIR LABELS.npz OUT.npz [--offset 1]

As run on 2026-11-21 (MODEL is the official pruned file the app downloads,
BirdNET+_V3.0_Global_12K_FP16_pruned.onnx):

  for s in HSN:HSN_hp POW:POW; do
    python -I birdset_embed_v3.py $MODEL ~/datasets/birdset/${s%%:*} \\
        ~/datasets/birdset/out/${s##*:}/labels.npz \\
        ~/.cache/oversample-research/birdnet-v3/birdset/${s%%:*}.npz
  done

Writes OUT.npz:
  v3        [N, 1280] float32, the embedding (zeros for a silent window)
  top       [N, 5] int16, the 5 best classes
  topconf   [N, 5] float32, their confidences (after the model's sigmoid)
  silent    [N] bool

Needs numpy, soundfile, onnxruntime.
"""

import glob
import os
import sys
import time

import numpy as np
import onnxruntime as ort
import soundfile as sf

RATE = 32000
WIN = 3 * RATE
BATCH = 16


def main():
    args = sys.argv[1:]
    offset = 1
    if "--offset" in args:
        i = args.index("--offset")
        offset = int(args[i + 1])
        del args[i : i + 2]
    model, data_dir, labels, out = args
    L = np.load(labels)
    rec_of, start_of = L["rec"], L["start"]
    files = {os.path.basename(p).rsplit(".", 1)[0]: p for p in glob.glob(f"{data_dir}/data/**/*.ogg", recursive=True)}
    o = ort.SessionOptions()
    o.intra_op_num_threads = int(os.environ.get("THREADS", "6"))
    o.log_severity_level = 3
    sess = ort.InferenceSession(model, o, providers=["CPUExecutionProvider"])
    n = len(rec_of)
    E = np.zeros((n, 1280), np.float32)
    T = np.zeros((n, 5), np.int16)
    C = np.zeros((n, 5), np.float32)
    S = np.zeros(n, bool)
    t0 = time.time()
    done = 0
    for k, rec in enumerate(sorted(set(rec_of))):
        x, sr = sf.read(files[rec], dtype="float32", always_2d=True)
        assert sr == RATE, (rec, sr)
        x = x.mean(1)
        idx = np.where(rec_of == rec)[0]
        wins = []
        for s in start_of[idx]:
            a = (int(s) + offset) * RATE
            w = x[a : a + WIN]
            wins.append(np.pad(w, (0, WIN - len(w))))
        wins = np.stack(wins).astype(np.float32)
        silent = ~wins.any(1)
        S[idx] = silent
        run = idx[~silent]
        rw = wins[~silent]
        for i in range(0, len(rw), BATCH):
            pred, emb = sess.run(["predictions", "embeddings"], {"input": rw[i : i + BATCH]})
            j = run[i : i + BATCH]
            E[j] = emb
            top = np.argsort(-pred, axis=1)[:, :5]
            T[j] = top
            C[j] = np.take_along_axis(pred, top, 1)
        done += len(idx)
        el = time.time() - t0
        print(f"{k + 1} recordings, {done}/{n} windows, {el:.0f} s, {el / done * 1000:.0f} ms per window", flush=True)
    os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
    np.savez(out, v3=E, top=T, topconf=C, silent=S)
    print(f"wrote {out}: {n} windows, {S.sum()} silent")


if __name__ == "__main__":
    main()
