"""Reference network inputs and outputs, made the way BirdBox's CLI makes them.

For each 32 kHz WAV: the repository's own feature code
(`src/inference/utils/pcen_inference.compute_pcen_for_inference`) cuts the log-floor
clips, its own renderer (`spectrogram_render.render_spectrogram_image`, matplotlib)
writes each clip's PNG, and the PNG is read the way Ultralytics reads it (cv2, BGR to
RGB, divided by 255). That is the exact tensor the CLI gives the network. Then
onnxruntime runs the given network files on those tensors.

  python -I ref_images.py BIRDBOX_REPO OUT_DIR AUDIO.wav [AUDIO.wav ...] --model NAME=MODEL.onnx [...]

Writes OUT_DIR/<name>.images.f32 (clips x 3 x 256 x 256), <name>.times.txt (clip start
times) and <name>.head.<model name>.f32 (clips x 5 x 1344) for each model.
"""

import argparse
import os
import sys
import tempfile

import cv2
import numpy as np
import onnxruntime as ort
import soundfile as sf

ap = argparse.ArgumentParser()
ap.add_argument("repo")
ap.add_argument("out")
ap.add_argument("audio", nargs="+")
ap.add_argument("--model", action="append", default=[])
a = ap.parse_args()

sys.path.insert(0, os.path.join(a.repo, "src"))
sys.path.insert(0, os.path.join(a.repo, "src", "inference"))
from inference.utils import pcen_inference  # noqa: E402
from inference.utils.spectrogram_render import render_spectrogram_image  # noqa: E402

opts = ort.SessionOptions()
opts.intra_op_num_threads = 1
sessions = {}
for item in a.model:
    name, path = item.split("=", 1)
    sessions[name] = ort.InferenceSession(path, opts, providers=["CPUExecutionProvider"])

os.makedirs(a.out, exist_ok=True)
for path in a.audio:
    name = os.path.splitext(os.path.basename(path))[0]
    audio, sr = sf.read(path, dtype="float32")
    clips, _ = pcen_inference.compute_pcen_for_inference(audio, sr, verbose=False)
    images = np.zeros((len(clips), 3, 256, 256), np.float32)
    with tempfile.TemporaryDirectory() as tmp:
        for i, clip in enumerate(clips):
            png = os.path.join(tmp, f"{i}.png")
            render_spectrogram_image(clip["pcen"], png, sr=32000, hop_length=375,
                                     colormap="inferno", vmin=0.0, vmax=100.0)
            bgr = cv2.imread(png)
            assert bgr.shape == (256, 256, 3), bgr.shape
            images[i] = bgr[:, :, ::-1].transpose(2, 0, 1).astype(np.float32) / 255.0
    images.tofile(os.path.join(a.out, f"{name}.images.f32"))
    with open(os.path.join(a.out, f"{name}.times.txt"), "w") as f:
        f.write("\n".join(f"{c['start_time']:.3f}" for c in clips) + "\n")
    for mname, s in sessions.items():
        heads = np.concatenate([s.run(None, {"images": images[i:i + 1]})[0] for i in range(len(clips))])
        heads.astype(np.float32).tofile(os.path.join(a.out, f"{name}.head.{mname}.f32"))
    print(f"{name}: {len(clips)} clips")
