# birdbox-bench

Spike, 2026-11-21: can Oversample run BirdBox "Just-Bird" on the device and get the same
boxes as BirdBox's own tools? Just-Bird is a YOLO11s object detector from the BirdNET team
that draws a box (start, end, low and high frequency, confidence) around each bird sound
on a spectrogram image, with one class, `bird`. Nothing here is part of the app build: the
crate has its own `[workspace]`, and no model file is committed.

Source: <https://github.com/birdnet-team/BirdBox>, tag `v2.1.0` (released 2026-11-17),
docs <https://birdnet-team.github.io/BirdBox/>. Code MIT, model weights AGPL-3.0.

## Answer

- **Go.** tract 0.23.10 runs the released network file (`Just-Bird_fp16.onnx`) after the
  same in-memory change as BirdNET+ V3.0 (float16 weights and casts to float32). Its
  output matches onnxruntime on a float32 copy to 0.00004 in score and 0.000 px in box.
- **Same boxes as the reference.** With the spectrogram image, NMS and window fusion
  written in Rust (in this crate), all 239 merged boxes on 8 recordings (11 minutes)
  match BirdBox's CLI one to one at IoU 0.75, with mean IoU 0.997 and confidence within
  0.0007. Of 839 per-window boxes, 838 match at IoU 0.5; the difference is boxes at the
  0.15 threshold and float16 rounding in the reference, not the Rust code.
- **Speed.** About 176 ms per 3 s window for the network in tract WASM in node (measured on
  a quieter machine, load average 4), plus about 10 ms for the spectrogram, floor and image. Windows start
  every 1.5 s, so a file is analysed at about 8 times real time (120 s in 14.8 s).
  Natively 57 to 59 ms per window, 23 times real time. WASM memory 122 MB.
- **Ship the float16 file.** About 19 to 21 MB. The int8 file gives wrong output in
  tract (and is slower there), and BirdBox's own tests show int8 changes 7.5 percent of
  the boxes even in onnxruntime.
- **Fetching it.** The GitHub release asset URL sends no CORS headers, so a browser
  cannot fetch it. Two ways round that, both work here:
  1. Fetch BirdBox's all-in-one demo graph (`docs/models/Just-Bird_fp16.onnx`, 21.2 MB)
     from `raw.githubusercontent.com` at the tag, which does send
     `Access-Control-Allow-Origin: *`, and cut the network out of it in memory
     ([`cut_demo_graph`](src/network.rs)). Tested natively and in WASM: the same boxes.
     No mirror needed.
  2. Mirror the release file (18.9 MB) on Hugging Face, as the other models are served
     (the owner's step). AGPL-3.0 then applies to the mirror, which must carry the
     licence and a link to the source.
- **What the boxes look like:** see [What the boxes show](#what-the-boxes-show). In short:
  tight boxes on single calls in soundscapes; whole phrases or songs get one long box
  (up to 16 s); some faint calls are missed; voices get false boxes at low confidence.

**Correction to the brief:** the release asset is *not* the all-in-one pipeline. It is
the bare Ultralytics export (image in, decoded boxes out; the DFL box decoding is inside).
The all-in-one graph (audio in, merged boxes out, with Loop, If, NonMaxSuppression and
two local functions) is the demo's file in the repository, `docs/models/Just-Bird_fp16.onnx`,
which has a different SHA-256 and the same network weights (166 of 175 tensors
bit-identical, the rest within 6e-8). So step 4 of the brief (cutting the network out)
was not needed for the release file, and became the way to fetch from a CORS-enabled URL.

## Files

From the v2.1.0 release (SHA-256 checked against its `SHA256SUMS.txt`) and the repository
at tag v2.1.0, saved under `~/.cache/oversample-research/birdbox/`.

| File | Bytes | SHA-256 | What it is |
|---|---|---|---|
| `Just-Bird_fp16.onnx` (release asset) | 18,947,043 | `6d2e614b7d0eb8069c3108033a4c8876018e8d7085e1034d6e49a39f15401a97` | The network |
| `Just-Bird_int8.onnx` (release asset) | 13,759,720 | `de906ee147cdc60ab4f600975c89f2f57b89b564c05afb504d965fe5a0a163ab` | The network, int8 weights (QDQ) |
| `docs/models/Just-Bird_fp16.onnx` (repository) | 21,150,203 | `34b5720cb4fb65d5201d9e9d7cecd3b670d4838552d2a84a6843964407f7e057` | The demo's all-in-one graph |

Test audio, resampled once to 32 kHz mono float32 by [prep.py](prep.py) with
`librosa.resample` (soxr_hq, what BirdBox's CLI uses), so every pipeline read the same
samples:

| Name | Source | Length | What is in it |
|---|---|---|---|
| gentoo | `batchi-extras/bat-demo-sounds/sounds/XC540767 - Gentoo Penguin` (the app's demo, MP3, 48 kHz) | 47 s | Penguin colony |
| pauraque | `no-bat-sounds/sounds/XC1083752 - Pauraque` (192 kHz) | 25.6 s | Repeated calls, other birds |
| canary | `no-bat-sounds/sounds/Sonniss - SBahob_Serinus Canaria Domesticus Singing 013` (192 kHz) | 16 s | Continuous song |
| soundscape | BirdBox's demo sample `docs/audio/BirdNET_soundscape_32kHz.flac` (BirdNET-Analyzer's example) | 120 s | Soundscape |
| hawaii | BirdBox's parity file `tests/test.wav` (Hawai'i, Navine et al. 2022, CC BY 4.0) | 300 s | Quiet soundscape, faint calls |
| rain | `sound-event-sounds/sounds/Rain and wind in London 2026 08 28.wav` (44.1 kHz) | 34.3 s | Rain, wind |
| speech | `sound-event-sounds/sounds/Group discussion 1.ogg` (44.1 kHz) | 29.7 s | Voices |
| bat | `bat-demo-sounds/sounds/XC1008337 - Nathusius's Pipistrelle` (384 kHz) | 7 s | Bat calls (all above 16 kHz except social calls and clicks) |

`xc-cache` was empty. The repository has no example audio with expected output files;
its parity test (`tests/model_format_parity.py`) compares formats against the PyTorch
model, so its matching rule is reused here.

## What the files are made of

From [inventory.py](inventory.py):

| | Release `Just-Bird_fp16.onnx` | Release `Just-Bird_int8.onnx` | Demo graph `docs/models/Just-Bird_fp16.onnx` |
|---|---|---|---|
| Producer, opset | pytorch 2.12.1 via Ultralytics 8.4.84, opset 20, IR 9 | onnx.quantize, opset 20 plus com.microsoft and others | `birdbox.onnx_export`, opset 17 plus domain `birdbox.stream`, IR 8 |
| Input | `images` float32 `[1, 3, 256, 256]` (fixed) | same | `audio` float32 `[num_samples]`, `conf` `[1]`, `song_gap` `[1]` |
| Output | `output0` float32 `[1, 5, 1344]` | same | `detections` float32 `[num_detections, 8]` |
| Nodes | 320: Conv 88, Mul 78, Sigmoid 77, Concat 21, Add 16, Reshape 11, Split 10, MaxPool 3, Transpose 3, MatMul 2, Softmax 2, Resize 2, Slice 2, Sub 2, Cast 2, Div 1 | 861 (adds 327 DequantizeLinear, 216 QuantizeLinear) | 76 in the main graph; 1,172 in the Loop body; nested If and Loop for fusion; 2 local functions |
| Weights | 18.8 MB float16 (180 tensors) | 8.1 MB int8 + 5.3 MB float32 | network 19.9 MB float16 inside the Loop body |
| Metadata | `imgsz [256, 256]`, `names {0: 'bird'}`, `stride 32`, `nms False`, `end2end False` | same | conf 0.15, phrase gap 0.05, NMS IoU 0.7, max 300 boxes per class and window, frontend description |

- **Release file:** float16 weights; a Cast to float16 after the input and a Cast to
  float32 before the output. The two Casts come last in the node list, after the nodes
  that use them, so `onnx.checker` rejects the file as not topologically sorted;
  onnxruntime and tract both accept it.
- **Output:** for each of 1,344 anchors (32 x 32 + 16 x 16 + 8 x 8 grid cells): box centre
  x, centre y, width and height in image pixels (already decoded from the DFL
  distributions inside the graph), and the `bird` score after its sigmoid. No NMS.
- **Demo graph:** where the spectrogram ends and the network starts: in the Loop body,
  the rendered image (a Gather into the colour table, flip, column Gather) is cast to
  float16 (`b_/Cast_25`), the network runs (`b_core.network.*` weights, as initializers
  of the body), and `b_/network/model.23/Concat_9` is cast back to float32 (`b_/Cast_26`).
  After it: NonMaxSuppression, pixel to time and Hz, cutting at the file end. The Loop
  (one iteration per 60 s segment) carries the per-window rows (`raw_all`); after it,
  nested If and Loop nodes do the window fusion. Dynamic: audio length and box count.
  The spectrogram is a strided Conv with a DFT kernel rebuilt at load time from one
  period of cos and sin values, then a mel MatMul, a log, and a TopK-based percentile.
  The two local functions (`stream_spectrogram`, `stream_detect`) are the demo's
  streaming entry points.

## The pipeline (what the Rust code does)

From `docs/how-it-works/spectrogram.md` and `overview.md`, and the code in
`src/inference/utils/pcen_inference.py`, `src/inference/onnx_export.py` and
`src/inference/detect_birds.py` (all MIT, BirdNET-Team). Ported to
[src/frontend.rs](src/frontend.rs) and [src/post.rs](src/post.rs), with attribution in
each file.

1. 32 kHz mono, scaled by 2^31 (it cancels out later). Silence is added at the end so
   the last 3 s window on the 1.5 s grid is complete.
2. Frames on one grid for the whole file: frame m covers samples 375 m - 250 to
   375 m + 1798. The 250 samples before the file start are samples 15,750 to 15,999
   (a leftover of BirdBox's old PCEN padding; the model was trained with it).
3. Per frame: periodic flattop window, 2048-point FFT, power, 256 HTK mel bands from 50 Hz
   to 15 kHz (librosa's filterbank with Slaney area normalisation), dB with power
   floored at 1e-10.
4. Window c (start 1.5 c s) is frames 128 c to 128 c + 251. Its noise floor per band is
   the 20th percentile (numpy's linear interpolation) of that band over the frames in
   the 60 s before the window's end, leaving out frames of digital silence. Nothing after
   a window changes it, so the same code works on a live stream.
5. Pixel: (dB - floor + 3) x 100 / 48, then x 256 / 100 rounded down, clamped to 0 to 255,
   as the index into matplotlib's "inferno" map rounded to 8 bits; band 255 at the top;
   252 frames to 256 columns by nearest frame (pixel j shows frame floor((j + 0.5) x 252 / 256)).
   RGB planes divided by 255. Steps 4 and 5 run in float32 in the same order as BirdBox's
   numpy and matplotlib code.
6. Network (tract).
7. Boxes scoring above 0.15; greedy NMS at IoU above 0.7, at most 300; clamp to the image;
   x to seconds (3 s per 256 px), y to Hz on the HTK mel axis, rounded to whole Hz.
   Boxes that start in the end padding are dropped and the rest cut at the real end.
8. Window fusion (Just-Bird's default): a box counts only in the window that owns its
   centre (the window's middle half); boxes of one window at 2D IoU 0.4 or more are
   averaged weighted by confidence; a box lying 80 percent or more inside a larger one is
   dropped; boxes in one band (frequency IoU 0.8 or more) less than the phrase gap
   (0.05 s) apart are joined, repeated until nothing joins. Gap 0 keeps call-level boxes
   (it still joins boxes that overlap by 0.05 s).

## tract: obstacles and fixes

| # | What happened | Cause | Fix |
|---|---|---|---|
| 1 | None for loading: tract read the release file as it is, unsorted Casts included | | |
| 2 | Float16 weights would run slowly and round differently in tract (seen with V3.0) | Float16 file | [`patch_proto`](src/network.rs): float16 initializers to float32 and Cast(to=float16) to Cast(to=float32), in memory before tract builds the model (180 weights, 1 cast). The official file is downloaded unchanged |
| 3 | The int8 file loads and runs (90.6 ms per image, slower than the float file) but its output is wrong: on the canary file's 10 windows (the only file tried), scores up to 0.72 away from onnxruntime's int8 run, 410 of 466 above-threshold anchors differ | Probably tract 0.23.10's handling of this QDQ graph (uint8 activations, per-channel weights); not looked into | Use the float16 file |
| 4 | The demo graph cut first failed in tract: `graph input images has no type` | The new graph's input had no declared type | Declare it float32 in `cut_demo_graph` |
| 5 | The cut network needs no other change: 175 weights and the Cast converted by the same `patch_proto` | | |

onnxruntime needed one more change for its float32 check copy ([convert.py](convert.py)):
the declared types of values (`value_info`) also had to become float32. tract is loaded
with `with_ignore_value_info`, so the in-memory patch does not need that.

## Results

All numbers in [results/compare.txt](results/compare.txt) ([compare_all.sh](compare_all.sh)).
Boxes are matched one to one by 2D IoU in seconds x Hz, pairs taken by decreasing IoU, as
BirdBox's `tests/model_format_parity.py` does; window boxes only within the same window.

**The reference.** BirdBox's CLI (`detect_birds.py` at v2.1.0, matplotlib PNGs, the
release `Just-Bird_fp16.onnx` in onnxruntime 1.27, default settings) and BirdBox's demo
graph in onnxruntime agree exactly: 839 of 839 window boxes and 239 of 239 merged boxes
matched, mean IoU 0.9993, confidence within 0.0015. That is the reference's own noise.

**Stage by stage**, each Rust stage fed the reference's own input:

| Stage | Result |
|---|---|
| Image: Rust vs the CLI's PNG as Ultralytics reads it | 2 to 4 pixels in 100,000 differ, each by one colour step (largest 0.016); the rest identical. Probably float rounding at a colour boundary (not traced). Effect on the network: at most 0.0007 in score, 0.05 px in box, no anchor crosses 0.15 |
| Network: tract vs onnxruntime, same images, float32 copy | score within 0.00004, boxes within 0.000 px, no anchor crosses 0.15 |
| Network: tract (float32) vs onnxruntime on the float16 file (what the CLI runs) | score within 0.004, boxes within 0.33 px; 4 of about 5,400 above-threshold anchors cross 0.15 (float16 arithmetic in onnxruntime) |
| Network cut from the demo graph vs release file, both in tract | score within 0.00001, boxes identical |
| NMS: Rust on onnxruntime's float16 output vs the CLI's window boxes | 839 of 839 matched, confidence identical |
| Fusion: Rust on the CLI's window boxes vs the CLI's merged boxes | 239 of 239, IoU 1.0000, confidence identical |

**End to end** (Rust image, tract network, Rust NMS and fusion) against the CLI:

| Recording | Window boxes, CLI | Rust | Matched at IoU 0.5 | at 0.75 | Merged boxes, CLI | Rust | Matched at 0.5 | at 0.75 |
|---|---|---|---|---|---|---|---|---|
| gentoo | 76 | 76 | 76 | 76 | 27 | 27 | 27 | 27 |
| pauraque | 40 | 40 | 40 | 40 | 15 | 15 | 15 | 15 |
| canary | 39 | 39 | 39 | 39 | 3 | 3 | 3 | 3 |
| soundscape | 243 | 243 | 243 | 243 | 56 | 56 | 56 | 56 |
| hawaii | 409 | 411 | 408 | 407 | 127 | 127 | 127 | 127 |
| rain | 11 | 11 | 11 | 11 | 2 | 2 | 2 | 2 |
| speech | 21 | 21 | 21 | 21 | 9 | 9 | 9 | 9 |
| bat | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| **All** | **839** | **841** | **838** | **837** | **239** | **239** | **239** | **239** |

- Merged boxes: mean IoU 0.9966, lowest 0.886; confidence difference mean 0.0001,
  largest 0.0007.
- Window boxes: mean IoU 0.9966; confidence difference mean 0.0002, largest 0.047. The 3
  unmatched Rust boxes and 1 unmatched CLI box are in the Hawai'i file (one Rust-only box
  scores 0.15005, just above the threshold). They come from float32 against float16
  arithmetic in the network, not from the Rust image: Rust NMS on onnxruntime's float32
  output for the CLI's own images gives exactly the same 838 matched, 1 missed and 3
  extra, and the Rust pipeline matches that run 841 of 841. None of it reaches the
  merged boxes.
- Call-level boxes (phrase gap 0) against the demo graph at gap 0: 246 of 246 matched at
  IoU 0.75.
- WASM (node) and the demo-graph cut, on the soundscape: 56 of 56 merged boxes, the same
  as native.

## What the boxes show

PNGs in [results/png/](results/png/): a linear spectrogram from 0 to 16 kHz in rows of
15 s (white at each frequency's median level, darker above it), CLI boxes in solid blue,
Rust boxes in dashed orange. In every file the two sets lie on top of each other, so
the boxes look like one purple-grey line. Hawai'i shows its first 120 s.

- **Soundscapes** ([soundscape](results/png/soundscape.png), [hawaii](results/png/hawaii.png)):
  short calls get tight boxes, close to what a person would draw, in time and in
  frequency, including faint ones in the Hawai'i file. A bird singing steadily in one
  band gets one long box: 15.7 s (46.3 to 62 s, 2 to 6 kHz) in the soundscape, because the
  join links its overlapping boxes across windows. Missed: a few faint calls (Hawai'i
  15.8 s, 34 s, 37.8 s, 82.4 s and 88.7 s near 4.5 kHz; soundscape 25 to 26 s at 6 to 10 kHz).
  Clicks and low rumble are not boxed.
- **Gentoo Penguin** ([gentoo](results/png/gentoo.png)): each braying display gets one box
  over its whole length and from 0 to 12 to 15 kHz; one box (2.7 to 4.9 s) starts about
  0.2 s after the sound and ends 0.3 s before it. Single calls with harmonics (11.6,
  12.4, 13.6, 20, 20.6, 21.5 s) get tight boxes. Five low boxes (0.5 to 4 kHz at 12 to
  14 s, 19.4 to 20.5 s, 31.8 to 33 s, 36.2 to 36.9 s, 38 to 39.2 s) sit on faint
  low-frequency sound that is hard to see; their confidences are 0.15, 0.30, 0.22, 0.16
  and 0.21.
- **Pauraque** ([pauraque](results/png/pauraque.png)): every one of the 12 repeated calls
  (1 to 2.5 kHz) has its own tight box; another bird's song (4 to 7 kHz) has three boxes.
  A person would mark the same things. Not boxed: a short tone at 13.5 kHz (8.5 s) and
  one at 14.5 kHz (20.2 s).
- **Canary** ([canary](results/png/canary.png)): continuous song becomes two boxes covering
  0.4 to 5.5 s and 5.7 to 16 s; the boxes stop at 11.7 kHz while the song reaches 16 kHz.
  Correct as "a bird sings here", not useful as call boxes.
- **Speech** ([speech](results/png/speech.png)): 9 false boxes on voices and sibilants,
  confidence 0.15 to 0.55 (median 0.18; 3 at 0.3 or more).
- **Rain** ([rain](results/png/rain.png)): 2 boxes, confidence 0.24 and 0.25 (one on a
  short sound at 2 to 5 kHz, one on low rumble).
- **Bat** ([bat](results/png/bat.png)): no boxes. At 32 kHz only the bat's social calls,
  clicks and the low end of its pulses remain; Just-Bird ignores them. It is not a bat
  detector.

So for the app: with a minimum confidence of 0.3 (applied to the window boxes before
fusion, as BirdBox does; `native decode --conf 0.3`), rain gives no boxes and speech 3
instead of 9, while Pauraque keeps 14 (of 15), the soundscape 53 (of 56), canary 2 (of
3), but the penguins only 13 (of 27) and Hawai'i 50 (of 127). BirdBox's own default is
0.15; offer the setting.

## Speed

[results/timing.txt](results/timing.txt). Ryzen 5 3600, one thread. The WASM figures and
the soundscape native figures were taken on a quieter machine (load average 4 on 12
threads); the per-file native runs under heavy load (about 34) gave the same network
times. The native build used thin LTO (fat LTO was not built: this session had a memory
limit); the WASM build too.

| Step, per 3 s window (a window every 1.5 s) | tract native | tract WASM in node (measured) |
|---|---|---|
| Spectrogram (128 new frames) | 1.4 ms | 3.2 ms |
| Rolling floor (sort per band over up to 60 s) | 0.5 to 4.7 ms (grows to 60 s of history) | 6.6 ms with the image |
| Image | 0.6 ms | (above) |
| Network | 57 to 59 ms | 172 to 176 ms (first window 210 ms) |
| NMS | under 0.01 ms | 0.03 ms |
| Fusion, whole file | 0.1 to 0.2 ms | 2 ms |
| **Whole file** | 120 s in 5.1 s, **23x real time** | 120 s in 14.8 s, **8x real time** |
| Load (parse, patch, optimise) | 360 to 380 ms | 800 to 850 ms |
| Memory | 137 to 146 MB peak RSS | 122 to 126 MB WASM memory |

- WASM to native for the network: 3.0, the same ratio as BirdNET+ V3.0.
- Per second of audio in WASM: about 125 ms, the same as V3.0 (110 to 125 ms) and about
  5 times BirdNET V2.4.
- The rolling floor could be made incremental (keep each band's last 60 s sorted and
  merge 128 frames in and out per window, as BirdBox's demo does) if it ever matters.
- For scale: BirdBox's demo graph in onnxruntime took 40.7 s for the 300 s file on one
  thread (7x real time, under load), tract native 13.0 s.
- A browser was not run. From the V3.0 and Perch notes, Chrome runs tract WASM at about
  0.9 times node's time (estimate).

## Recommendation

- Use the float16 network. Download about 19 MB (release file, via a mirror) or 21 MB
  (demo graph, direct from GitHub at the tag, cut in memory). Compute in float32 in tract.
- Not int8: wrong output in tract 0.23.10, slower than float in tract, 5 MB smaller, and
  BirdBox measured 7.5 percent of boxes changing with int8 even in onnxruntime.
- Fetch: prefer the demo graph at
  `https://raw.githubusercontent.com/birdnet-team/BirdBox/v2.1.0/docs/models/Just-Bird_fp16.onnx`
  (CORS `*`, 206 to ranged requests; checked with [cors.sh](cors.sh)), checked against
  the SHA-256 above (a download from that URL on 2026-11-21 had exactly that hash),
  because a tag can be moved. raw.githubusercontent.com is not a CDN
  and may rate-limit; if that is a problem, mirror the release file on Hugging Face
  (owner's step). jsDelivr refuses the file (403, probably its 20 MB limit for
  GitHub files); the GitHub Pages copy (`birdnet-team.github.io/BirdBox/models/...`) has CORS
  but follows the latest release, not a fixed version.
- Resampling: here it was done once with librosa for every pipeline. The app's resampler
  is different, so files not recorded at 32 kHz will give slightly different boxes than
  BirdBox's CLI (BirdBox's docs say the same of its browser demo). Not measured.

## Licence

- **Weights: AGPL-3.0** (release `NOTICE.txt`: "These YOLO11 model weights were trained
  with Ultralytics and are licensed under the GNU Affero General Public License v3.0").
  The owner accepted this for the trial.
- **Code: MIT** (BirdBox repository). The Rust port of the spectrogram, NMS and fusion is
  from MIT code, attributed in the source files; the mel filterbank follows librosa
  (ISC), as BirdBox's own copy does; the inferno colour map is CC0.
- What the app would do and say:
  - Download the unmodified file at run time, separately from the app, and never bundle
    it. Converting it to float32 or cutting the network out happens in memory on the
    user's device.
  - Credits, from `CITATION.cff`: "Bird boxes: BirdBox Just-Bird model, version 2.1.0,
    by Elias Schlosser, Stefan Kahl and Maximilian Eibl (BirdNET Team). Model weights
    licensed under the GNU AGPL-3.0; trained with Ultralytics YOLO11 (Jocher and Qiu,
    2024). Source: github.com/birdnet-team/BirdBox." with links to the licence text and
    the source.
  - A mirror of the file (converted or not) is a distribution: it must carry the
    AGPL-3.0 text and `NOTICE.txt`, and point to the corresponding source (the
    repository at v2.1.0).
  - Open question for the owner, not settled here: whether the app plus AGPL weights
    counts as one combined work under AGPL. BirdBox's own `CITATION.cff` says that
    because it ships YOLO11-trained weights, "the project as a whole is subject to
    AGPL-3.0", so the authors read the AGPL broadly. Downloading the unmodified file at
    run time keeps the app and the weights as separate as they can be. This is not legal
    advice.

## Integration plan

Sizes: small is under a day, medium one to three days, large more.

**Shape:** Just-Bird gives boxes, not per-window class scores, and no embedding. So its
results are not a `ClassifierRun` (which is windows of top-k scores plus embeddings); they
are a new per-file result next to it, `BoxRun`: the window boxes (kept at a low
threshold such as 0.05, as BirdBox's demo does, so the confidence and phrase-gap
settings re-fuse instantly without running the model again) and the merged boxes
computed from them. Boxes apply to audible files: the model sees 50 Hz to 15 kHz at
32 kHz. For time-expanded or heterodyne files they would land in the wrong place unless
times and frequencies are converted, so skip those at first.

| Step | Files | Size |
|---|---|---|
| 1. Worker engine: move `network.rs` (`patch_proto`, `cut_demo_graph`), `frontend.rs`, `post.rs` and `inferno.rs` into the worker as a `birdbox` module; one entry point "32 kHz audio in, window boxes out", per window so progress and cancel work as for the classifiers | `birdnet-worker/src/birdbox/` (new), `birdnet-worker/src/session.rs` | Medium |
| 2. Catalog and IPC: a `ModelFile` for the demo graph (URL at the tag, size, SHA-256, AGPL attribution) or the mirrored release file; request and reply messages for a box run; resampling to 32 kHz with the existing resampler | `oversample-ipc/src/classifier/catalog.rs`, `oversample-ipc/src/classifier.rs`, `birdnet-worker/src/resample.rs` | Small |
| 3. `BoxRun` in state, keyed per file like the classifier runs; fusion re-run on the UI side when the minimum confidence or phrase gap changes (under 2 ms in WASM for 2 minutes) | `src/classifier/types.rs` (or a new `src/classifier/boxes.rs`), `src/state.rs` | Small |
| 4. Overlay: draw the merged boxes on the spectrogram for audible files, as a new source next to the call detector's boxes (`draw_calls`); confidence in the hover text | `src/canvas/overlays.rs`, `src/components/spectrogram.rs` | Medium |
| 5. Species per box, the BirdNET website demo's rule (BirdBox `docs/models-and-metrics/just-bird-species-from-birdnet.md`, placement `grid15`): run BirdNET+ V3.0 on BirdBox's own windows, one every 1.5 s (a 1.5 s step, which `ClassifierRun::step_secs` and the refine passes already allow), alongside the detection. A box up to 1.5 s long takes the window that owns its centre (the window's middle 1.5 s, the fusion rule); a longer box takes the windows from the one around its first 1.5 s to the one around its last, and each species keeps its highest score over them. The box takes the highest-scoring bird; the demo names it only from 0.35, otherwise "Aves", and lists the top 5 at 0.05 or more. BirdBox measured this as good as a window centred on each box (top-1 0.506 against 0.507 on 11,552 annotated boxes; 0.025 worse for boxes of 1.5 to 3 s). A box that grows or changes with the settings needs no new scoring | `src/classifier/` (new pairing function), overlay hover text | Medium |
| 6. Relation to the per-window strips: the strips stay per window. Boxes can feed them two ways: a "Bird sound" strip row (highest box confidence per window, from `BoxRun`), and an option to run V3.0 only on windows that have a box (Hawai'i: 154 of 199 windows have one; rain: 7 of 22) | `src/classifier/strips.rs`, `src/components/score_strips.rs` | Small to medium |
| 7. Add boxes as annotations, tagged `auto=birdbox-just-bird`, as the call detector does with `call_annotations.rs` (replaceable on a later run) | `src/call_annotations.rs` or a sibling | Small |
| 8. UI: a Bird boxes method in the Call detection panel, minimum confidence (default 0.3 or BirdBox's 0.15; owner's choice) and box size (calls or phrases: phrase gap 0 or 0.05 s), credits | `src/components/file_sidebar/call_detection_panel/` | Medium |
| 9. Live: the rolling floor makes each window final as soon as its audio has arrived, so boxes can appear in the live view 3 s behind; about 186 ms of WASM work per 1.5 s | `src/live_embedding.rs` or a new live source | Medium to large |
| 10. Tests: a real-model test against the CLI's boxes for the Gentoo demo (store them as a fixture), plus the stage tests here (image, NMS, fusion) as unit tests with small fixtures | `birdnet-worker/tests/real_models.rs` | Small |
| 11. Hosting decision: raw GitHub at the tag (no mirror) or a Hugging Face mirror (owner's step) | catalog | Small |

Cost when paired with V3.0 (an estimate built on V3.0's own estimated WASM time):
Just-Bird about 125 ms and V3.0 on a 1.5 s grid about 240 ms per second of audio in WASM,
so about 2.7 times faster than real time together (V3.0 on its own 3 s grid: about 4
times). Running V3.0 only on windows with boxes saves little on
busy recordings and a lot on quiet ones.

## Steps

Python 3.12 venv (BirdBox's `requirements.txt` pins): `torch torchvision` (CPU wheels),
`numpy==2.0.2 pandas==2.3.3 scipy==1.15.2 librosa==0.11.0 soundfile==0.13.1
ultralytics==8.4.84 onnx==1.22.0 opencv-python-headless matplotlib==3.10.9 tqdm
onnxruntime==1.27.0`. Keep the model files, the venv, the BirdBox checkout and both
Cargo target folders outside this synced folder.

```bash
C=~/.cache/oversample-research/birdbox; W=$C/work; B=path/to/research/birdbox-bench
X=path/to/batchi-extras; P=$C/venv/bin/python
# 0. Downloads: release assets (checked against SHA256SUMS.txt) and the source at the tag
gh release download v2.1.0 -R birdnet-team/BirdBox -D $C/release \
  -p Just-Bird_fp16.onnx -p Just-Bird_int8.onnx -p SHA256SUMS.txt -p NOTICE.txt -p LICENSE-AGPL-3.0
gh api repos/birdnet-team/BirdBox/tarball/v2.1.0 > $C/src.tgz   # unpacked to $C/repo
bash $B/cors.sh
# 1. What the files are made of
$P -I $B/inventory.py $C/release/Just-Bird_fp16.onnx $C/release/Just-Bird_int8.onnx $C/repo/docs/models/Just-Bird_fp16.onnx
$P -I $B/same_weights.py $C/release/Just-Bird_fp16.onnx $C/repo/docs/models/Just-Bird_fp16.onnx
$P -I $B/convert.py $C/release/Just-Bird_fp16.onnx $C/conv/Just-Bird_fp32-from-fp16.onnx
$P -I $B/cut_graph.py $C/repo/docs/models/Just-Bird_fp16.onnx $C/conv/Just-Bird_cut-from-demo.onnx
# 2. Audio (32 kHz float32 WAV; hawaii with --max-seconds 300)
$P -I $B/prep.py $W/audio "gentoo=$X/bat-demo-sounds/sounds/XC540767 - Gentoo Penguin - Pygoscelis papua.mp3" ...
# 3. References: the CLI, its images and network outputs, the demo graph (gap 0.05 and 0)
bash $B/run_cli.sh $P $C/repo $C/release/Just-Bird_fp16.onnx $W/audio $W/cli
$P -I $B/cli_tsv.py $W/cli $W/ref bat canary gentoo pauraque rain soundscape speech hawaii
$P -I $B/ref_images.py $C/repo $W/ref $W/audio/*.wav --model ort16=$C/release/Just-Bird_fp16.onnx \
  --model ort32=$C/conv/Just-Bird_fp32-from-fp16.onnx --model ortint8=$C/release/Just-Bird_int8.onnx
$P -I $B/ref_graph.py $C/repo/docs/models/Just-Bird_fp16.onnx $W/ref $W/audio/*.wav
$P -I $B/ref_graph.py $C/repo/docs/models/Just-Bird_fp16.onnx $W/ref $W/audio/*.wav --gap 0 --tag graph0
# 4. Rust, native (one build at a time; CARGO_BUILD_JOBS=4 and thin LTO kept memory down)
CARGO_BUILD_JOBS=4 CARGO_PROFILE_RELEASE_LTO=thin CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
  CARGO_TARGET_DIR=$C/target cargo build --release
N=$C/target/release/native
$N net $C/release/Just-Bird_fp16.onnx $W/ref/NAME.images.f32 $W/ref/NAME.head.tract16.f32      # and the int8 file, the demo graph
$N decode $W/ref/NAME.head.ort16.f32 $W/audio/NAME.wav $W/stage/NAME.dec
$N fuse $W/ref/NAME.cli.raw.tsv $W/audio/NAME.wav $W/stage/NAME.fuse
$N detect $C/release/Just-Bird_fp16.onnx $W/audio/NAME.wav $W/rust/NAME --images
$N fuse $W/rust/NAME.raw.tsv $W/audio/NAME.wav $W/rust/NAME.gap0 --gap 0
# 5. WASM in node (audio as raw float32: NAME.f32)
CARGO_BUILD_JOBS=4 CARGO_PROFILE_RELEASE_LTO=thin CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
  CARGO_PROFILE_RELEASE_DEBUG=false CARGO_PROFILE_RELEASE_STRIP=true RUSTFLAGS="-C target-feature=+simd128" \
  CARGO_TARGET_DIR=$C/target-wasm wasm-pack build --release --target nodejs --out-dir $C/pkg-node --no-typescript
node $B/bench-bindgen.mjs $C/pkg-node $C/release/Just-Bird_fp16.onnx $W/audio/soundscape.f32 $W/rust/soundscape.wasm.boxes.tsv
# 6. Comparisons and pictures
bash $B/compare_all.sh $P $W > $B/results/compare.txt
$P -I $B/plot.py $W/audio "$W/ref/{name}.cli.boxes.tsv" "$W/rust/{name}.boxes.tsv" $B/results/png \
  gentoo pauraque canary soundscape hawaii rain speech bat --max-seconds 120
```

## Not done

- No browser and no phone. Browser times are estimates from node.
- Fat-LTO builds were not made (memory limit in this session); thin LTO may be a few
  percent slower.
- Why tract gets the int8 file wrong was not looked into.
- The app's resampler was not compared with librosa's; all files here were resampled once
  with librosa.
- Only 8 recordings (about 11 minutes). No frogs, insects or long dawn choruses.
- The box-to-species pairing (plan step 5) was read from BirdBox's study, not run here.
