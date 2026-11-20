"""Reference 2: BirdBox's all-in-one demo graph (docs/models/Just-Bird_fp16.onnx in the
repository at v2.1.0: audio in, merged boxes out, the same network weights as the
release file) in onnxruntime. The graph's per-window rows (`raw_all`, the Loop's
output before window fusion) are added as a second graph output.

  python -I ref_graph.py GRAPH.onnx OUT_DIR AUDIO.wav [...] [--conf 0.15] [--gap 0.05] [--tag graph]

Writes OUT_DIR/<name>.<tag>.boxes.tsv (time_start, time_end, freq_low, freq_high,
avg_conf, max_conf, merged) and <name>.<tag>.raw.tsv (clip_start, time_start, time_end,
freq_low, freq_high, conf).
"""

import argparse
import os
import time

import numpy as np
import onnx
import onnxruntime as ort
import soundfile as sf

ap = argparse.ArgumentParser()
ap.add_argument("graph")
ap.add_argument("out")
ap.add_argument("audio", nargs="+")
ap.add_argument("--conf", type=float, default=0.15)
ap.add_argument("--gap", type=float, default=0.05)
ap.add_argument("--tag", default="graph", help="file name part: <name>.<tag>.boxes.tsv")
a = ap.parse_args()

m = onnx.load(a.graph)
raw_out = onnx.helper.make_tensor_value_info("raw_all", onnx.TensorProto.FLOAT, None)
m.graph.output.append(raw_out)
opts = ort.SessionOptions()
opts.intra_op_num_threads = 1
s = ort.InferenceSession(m.SerializeToString(), opts, providers=["CPUExecutionProvider"])
os.makedirs(a.out, exist_ok=True)
for path in a.audio:
    name = os.path.splitext(os.path.basename(path))[0]
    audio, sr = sf.read(path, dtype="float32")
    assert sr == 32000
    t = time.perf_counter()
    det, raw = s.run(["detections", "raw_all"], {
        "audio": audio,
        "conf": np.array([a.conf], np.float32),
        "song_gap": np.array([a.gap], np.float32),
    })
    ms = (time.perf_counter() - t) * 1000
    with open(os.path.join(a.out, f"{name}.{a.tag}.boxes.tsv"), "w") as f:
        for r in det:
            f.write("\t".join(f"{v:.6f}" for v in r[:7]) + "\n")
    with open(os.path.join(a.out, f"{name}.{a.tag}.raw.tsv"), "w") as f:
        for r in raw:
            f.write("\t".join(f"{v:.6f}" for v in (r[7], r[1], r[2], r[3], r[4], r[5])) + "\n")
    print(f"{name}: {len(raw)} window boxes, {len(det)} merged boxes, {ms:.0f} ms for {len(audio) / 32000:.1f} s")
