# BirdNET on the device: feasibility

Status: research, 2026-11-11. Nothing here is wired into the app.

## Recommendation

Run BirdNET V2.4 on the device, in the web build and in Tauri, with no server. Use
[tract](https://github.com/sonos/tract) (pure Rust) in a Web Worker. The Phase 3 server
in `batchi-backend` does not need to run BirdNET, so BirdNET needs no sign-in and no
server cost.

- **Speed:** tract compiled to WASM with SIMD runs one 3 s window in 64 to 74 ms on one
  thread. It analyses 3 minutes of audio in 4 to 4.6 s on a Ryzen 5 3600, which is about
  40 times faster than real time (3 minutes of audio played at normal speed).
- **The app's target:** the same tract code builds for `wasm32-unknown-unknown` with
  wasm-bindgen, the target the web build uses, and gives the same results there.
- **Accuracy:** tract's output matches the official TFLite model. Top-1 was the same on
  all 19 test windows, and the largest confidence difference was 0.0005.
- **One code path (expected, not yet tested in an app):** the Tauri app runs the same
  WASM frontend, so a worker built for the web build should serve Tauri unchanged. A
  native Tauri command could later use the same Rust code (tract native: 26.5 ms per
  window) or onnxruntime (17.5 ms).
- **Model file:** download the 52 MB ONNX file at runtime and cache it (in the browser's
  Origin Private File System, OPFS, which the app already uses for annotations; in the
  app data folder in Tauri). Don't commit it to this repository or bundle it in the app.
- **Licence:** BirdNET's own sources give three different licences for the V2.4 weights:
  CC BY-NC-SA 4.0, CC BY-NC 4.0 and CC BY-SA 4.0. Until the BirdNET team says which
  applies, follow the strictest, CC BY-NC-SA 4.0. Under it, a free app may download and
  run the weights with attribution, and the BirdNET feature must never sit behind a paid
  tier. See [Licence](#5-licence).

These numbers come from node (V8, the engine in Chrome), one thread, no Web Worker. No
browser, Tauri webview or phone was tested.

Bats need a different model, because BirdNET V2.4 has no bat classes. BattyBirdNET (small bat
classifier layers, called heads, on BirdNET V2.4 embeddings) looks promising. The BattyBirdNET backbone is a 67 MB
BirdNET V2.4 file with an embeddings output, so bat analysis would download that file
in place of the 52 MB one. The backbone loads and runs in tract; the BattyBirdNET bat
heads were only run in Python onnxruntime. See [Bats](#bats).

## What was measured

Prototype: [birdnet-bench/](birdnet-bench/), a standalone crate and scripts outside the
app's workspace. Test audio: four bird recordings from `batchi-extras/no-bat-sounds`
(Common Swift, Common Pauraque, Common Scoter, Alpine Swift), resampled to 48 kHz and cut
into 19 windows of 3 s. Speed runs used 60 windows (3 minutes). Machine: Ryzen 5 3600
(6 cores, 2019), Linux, node 24 (V8, the engine in Chrome and Edge), while other builds
were running, so times are approximate.

Model: `BirdNET_v2.4_fp32_foldconv.onnx` (52 MB) unless noted. "foldconv" is the name
tphakala gives one of their conversions; its graph has only standard ONNX operators and
no DFT operator.

The Memory column is not comparable across rows: native runs give the process's peak
resident memory (RSS), tract in WASM gives the WASM linear memory, and onnxruntime-web
gives the whole node process.

| Runtime | Threads | ms per 3 s window | 3 min of audio | Memory |
|---|---|---|---|---|
| TFLite (LiteRT, Python), official model | 1 | 36 | 2.2 s | 195 MB RSS |
| onnxruntime, Python or Rust `ort` | 1 | 17.5 | 1.1 s | 180 MB RSS |
| onnxruntime, Python | 4 | 10.5 | 0.6 s | 233 MB RSS |
| tract, native | 1 | 26.5 | 1.6 s | 195 MB RSS |
| **tract, wasm32 + SIMD, opt-level 3** | 1 | **64** | **3.9 s** | 160 MB WASM memory |
| tract, wasm32 + SIMD, opt-level "s" | 1 | 113 | 6.4 s | 160 MB WASM memory |
| tract, wasm32, no SIMD | 1 | 175 | 10.6 s | 160 MB WASM memory |
| tract, `wasm32-unknown-unknown` + wasm-bindgen + SIMD, opt-level 3 | 1 | 74 | 4.6 s | 383 MB node RSS |
| onnxruntime-web 1.31.1, WASM backend | 1 | 60 | 3.7 s | 510 MB node RSS |
| onnxruntime-web 1.31.1, WASM backend | 4 | 28 | 1.8 s | 520 MB node RSS |

The plain wasm32 rows use `wasm32-wasip1` with a raw C ABI, so node can load them without
wasm-bindgen. The wasm-bindgen row is the app's own target; it is about 10 ms per window
slower, probably from copying each window and its results across the JS boundary.

Model load (parse and optimise) takes 0.3 s native and 0.5 to 0.6 s in WASM. A batch of 8
windows in tract was slower per window than one window at a time.

Agreement with the official TFLite model on the 19 windows:

| Run | Same top-1 | Largest confidence difference |
|---|---|---|
| tract native, fp32 | 19 of 19 | 0.0004 |
| tract wasm32 (wasip1 and wasm-bindgen builds), fp32 | 19 of 19 | 0.0005 |
| onnxruntime, fp32 | 19 of 19 | 0.0019 |
| onnxruntime, int8 | 19 of 19 | 0.008 |

Top-1 results: Common Pauraque, Common Scoter and Alpine Swift at 0.91 to 1.00. The Common
Swift recording came out as Pallid Swift (0.92), a close relative.

Not measured: WebGPU, TF.js (WebGL), Safari or WebKitGTK, phones, and the location
model. Slower laptops and phones were not tested. At about 40 times real time there is a
wide margin: a device 10 times slower would still analyse audio 4 times faster than real
time.

## 1. Model files

| Model | Format | Size | Source | Licence |
|---|---|---|---|---|
| BirdNET V2.4 acoustic, 6,522 classes | TFLite FP32 | 51.7 MB | Zenodo record 15050749 (`BirdNET_v2.4_tflite.zip`); BirdNET-Analyzer V2.4 zip; copied in `justinchuby/BirdNET-onnx` | Zenodo: CC BY-NC 4.0; Analyzer: CC BY-NC-SA 4.0 |
| BirdNET V2.4 acoustic | TFLite FP16 | 25.9 MB | Zenodo record 15050749 (`BirdNET_v2.4_tflite_fp16.zip`) | Zenodo: CC BY-NC 4.0 |
| BirdNET V2.4 acoustic | TFLite INT8 | 41.1 MB | Zenodo record 15050749 (`BirdNET_v2.4_tflite_int8.zip`) | Zenodo: CC BY-NC 4.0 |
| BirdNET V2.4 acoustic | TF.js layers model (13 shards) | 51 MB | Zenodo record 15050749 (`BirdNET_v2.4_tfjs.zip`); `birdnet-team/real-time-pwa`; `georg95/birdnet-web` | Zenodo: CC BY-NC 4.0; the PWA says CC BY-SA 4.0 |
| BirdNET V2.4 acoustic | Keras, TF SavedModel, Raven | 49 to 125 MB zips | Zenodo record 15050749 | Zenodo: CC BY-NC 4.0 |
| BirdNET V2.4 acoustic | ONNX FP32 `foldconv` | 52.3 MB | `tphakala/BirdNET-v2.4` on Hugging Face | CC BY-NC-SA 4.0 |
| BirdNET V2.4 acoustic | ONNX INT8 head `foldconv` | 32.3 MB | `tphakala/BirdNET-v2.4` | CC BY-NC-SA 4.0 |
| BirdNET V2.4 acoustic | ONNX FP32 | 51.7 / 66.9 MB | `justinchuby/BirdNET-onnx` (`model.onnx`, `birdnet.onnx`) | CC BY-NC-SA 4.0 |
| BirdNET V2.4 acoustic with embeddings output | ONNX FP32 | 66.9 MB | `tphakala/BattyBirdNET-onnx` | CC BY-NC-SA 4.0 |
| BirdNET V2.4 location model (species by place and week) | TFLite | 29.5 MB | Zenodo record 15050749 (`meta-model.tflite` in each TFLite zip; TF.js copy in the TF.js zip) | Zenodo: CC BY-NC 4.0 |
| BirdNET V2.4 location model, "V2" FP16 | TFLite | 7.1 MB | BirdNET-Analyzer; `justinchuby/BirdNET-onnx` | CC BY-NC-SA 4.0 |
| BirdNET V2.4 location model | ONNX | 14.1 MB | `justinchuby/BirdNET-onnx` | CC BY-NC-SA 4.0 |
| BirdNET V2.4 location model | TF.js graph model | 7.1 MB | `birdnet-team/real-time-pwa` | the PWA says CC BY-SA 4.0 |
| BirdNET+ V3.0 developer preview 3.1, 11,560 classes | ONNX / TFLite FP32 | 541 MB | Zenodo record 20703646 | CC BY-SA 4.0 + terms of use |
| BirdNET+ V3.0 developer preview 3.1 | ONNX / TFLite FP16 | 271 MB | Zenodo record 20703646 | CC BY-SA 4.0 + terms of use |
| BirdNET+ V3.0 developer preview 3.1, pruned | ONNX / TFLite FP16 | 71.5 MB | Zenodo record 20703646 | CC BY-SA 4.0 + terms of use |

V2.4 input is 144,000 samples (3 s at 48 kHz, mono, float32). Output is 6,522 logits;
apply a sigmoid for confidence. The model normalises the audio and computes its two mel
spectrograms itself, so the app only resamples and cuts windows.

The official Zenodo record (BirdNET Model V2.4, published 2025-03-19) has FP32, FP16 and
INT8 TFLite files; the `birdnet` Python package 1.4.0 downloads all three from it. On the
19 test windows, with LiteRT on one thread:

| Official TFLite file | ms per window | Same top-1 as FP32 | Largest confidence difference from FP32 |
|---|---|---|---|
| FP16 (25.9 MB) | 50 | 18 of 19 | 0.089 |
| INT8 (41.1 MB) | 51 | 16 of 19 | 0.28 |

Neither is faster than FP32 (36 ms) on this x86 CPU, and both are less accurate. The
ONNX files are community conversions of the FP32 TFLite file.

The `foldconv` ONNX file uses only standard operators (Conv, MatMul, Reduce*, Sigmoid
and similar), which is why tract and onnxruntime-web both load it. The INT8 file uses
`DynamicQuantizeLinear` and `MatMulInteger`: tract runs it at 114 ms per window natively,
four times slower than FP32, so INT8 only saves 20 MB of download.

V3.0 is a developer preview whose terms say it is for research and evaluation, and that
models, labels and code will change. It takes 32 kHz audio of any length. Even the pruned
file is 71.5 MB, and it was not tested here.

## 2. Browser options

| Option | Loads BirdNET? | Speed (1 thread) | Fits the Rust/WASM stack? |
|---|---|---|---|
| tract (`tract-onnx` 0.23) | Yes: fp32, int8 and the embeddings model | 64 to 74 ms per window | Yes. Pure Rust; built and run for `wasm32-unknown-unknown` with wasm-bindgen |
| onnxruntime-web 1.31 | Yes: fp32 and int8 | 60 ms per window | Through JS, or through the `ort` crate's `ort-web` backend (not tested) |
| TF.js 4.x | Yes: the official BirdNET PWA uses it, with the WebGL backend | Not measured | No. JS, and the PWA's spectrogram layer is custom JS and WebGL code |
| Chirpity (Electron desktop app) | Runs V2.4 as TF.js on `tfjs-node` (native TensorFlow), and a BirdNET V3 ONNX file (87 MB) on `onnxruntime-node` | Not measured | No. Desktop only, CC BY-NC-SA 4.0 |
| TFLite in WASM | Not tried | – | No maintained Rust path |

tract is the recommended option:

- It runs entirely in Rust and adds one dependency.
- Single-thread speed equals onnxruntime-web's.
- Its output was closest to TFLite.

Costs of tract:

- **Code size:** the tract-onnx WASM module is 15.7 MB (3.9 MB gzipped) at opt-level 3,
  or 10.1 MB (2.4 MB gzipped) at opt-level "s". Build it as a separate worker module that
  loads only when the user first runs BirdNET, so the main app bundle stays the same size.
- **Optimisation level:** the app's release profile uses `opt-level = "s"`, which makes
  tract 1.8 times slower (113 ms per window). Give the tract crates (or the worker crate)
  `opt-level = 3` with a `[profile.release.package."..."]` override.
- **SIMD:** without SIMD, tract is 2.7 times slower. `.cargo/config.toml` already enables
  `+simd128` for `wasm32-unknown-unknown`.
- **getrandom:** tract-onnx's default features pull in `getrandom` 0.4, which needs its
  `wasm_js` feature on `wasm32-unknown-unknown`. The feature alone was enough; no
  `--cfg getrandom_backend` flag was needed.
- **Model shapes:** some ONNX conversions store a symbolic batch size that clashes with a
  fixed batch. Load with `with_ignore_value_info(true)` and
  `with_ignore_output_shapes(true)`, as `research/birdnet-bench/src/lib.rs` does.

onnxruntime-web is 2.2 times faster with 4 threads, but WASM threads need
`SharedArrayBuffer`, which needs cross-origin isolation (COOP and COEP headers).
GitHub Pages can't send those headers. WebGPU through onnxruntime-web could be much
faster still, but it was not measured. Threads and WebGPU can both be added later
through the `ort` crate (see section 3), without changing the input pipeline.

## 3. Tauri and native options

The Tauri app runs the same Leptos WASM frontend, so the browser worker should work there
unchanged. Native options, for later if speed matters:

- **tract native:** the same Rust code built for the host. 26.5 ms per window, 2.4 times
  faster than WASM. No native library to ship, which matters on Android.
- **`ort` 2.0 (onnxruntime):** 17.5 ms per window on one thread, 10.5 ms on four. It
  needs the onnxruntime shared library for each platform and Android ABI.
- **One API for all builds:** the `ort` crate now has alternative backends: `ort-web`
  (onnxruntime-web, including WebGPU), `ort-tract` and `ort-candle`, all at 0.5.0 in
  October 2026. One `ort` code path could use onnxruntime natively and onnxruntime-web
  in the browser. Not tested.
- **The `birdnet-onnx` crate** (MIT, by tphakala) wraps `ort` for BirdNET V2.4, V3.0 and
  Perch v2, including the location model. Useful as a reference; it targets native
  builds.

## 4. Input pipeline

- The loader (`oversample-core/src/audio/loader.rs`) already gives mono `f32` samples
  and the file's sample rate (`AudioData::samples`, `AudioData::sample_rate`). Large
  files can be read in parts through `AudioSource::read_region`.
- BirdNET needs 48 kHz. Oversample has no band-limited resampler: `pitch_shift.rs` has
  stretch and compress functions, but they don't filter. Add one, for example `rubato`
  (pure Rust, MIT, works in WASM). Most bat files are 192 to 500 kHz, and bird files
  are 44.1 or 48 kHz. BirdNET-Analyzer resamples every file to 48 kHz, and its docs say
  the model can deal with artifacts from lower sample rates.
- Cut the audio into 3 s windows (144,000 samples) and pad the last window with zeros.
  BirdNET-Analyzer's default overlap is 0 s; an overlap of 1 to 2 s finds more calls and
  costs proportionally more time.
- Oversample has no Web Worker today: long jobs use `spawn_local` with `setTimeout`
  yields. 64 ms per window on the main thread would make scrolling and playback stutter,
  so run inference in a dedicated worker. Trunk can build a worker from a second binary
  (`data-type="worker"`). Send windows to the worker and results back as call or species
  annotations (`#auto=...`, as `call_annotations.rs` does for the call detector).
- The location model can filter the 6,522 classes to species expected at the
  recording's place and week (from GUANO metadata or the xeno-canto record). The location
  model is optional and was not tested.

## Bats

BirdNET V2.4 has no bat classes. The only labels with "bat" in them are Bat Falcon and
Bat Hawk. Time-expanding bat audio into the audible range and feeding it to plain BirdNET
gives bird labels, so it is no use for bats on its own. Options:

| Model | What it is | Licence | On-device fit |
|---|---|---|---|
| BattyBirdNET | Regional bat heads (11 regions, 11 to 38 species, about 0.1 MB each) on BirdNET V2.4 embeddings | CC BY-NC-SA 4.0 | Good: a 67 MB BirdNET V2.4 backbone and the same pipeline as BirdNET |
| BSG-BAT v0.21 | CNN on a log-mel spectrogram of real 384 kHz audio, 21 European species, 6-model ensemble | CC BY 4.0 | Possible: 83 MB per model, and the spectrogram must match its Python code exactly |
| BatDetect2 | PyTorch detector and classifier | CC BY-NC 4.0 | Needs an ONNX export first; no ready ONNX file found |

BattyBirdNET resamples the recording to 256 kHz and passes 144,000-sample windows
(0.56 s) to BirdNET as if they were 48 kHz. That is a 5.33x time expansion, which moves
20 to 120 kHz calls down to 3.75 to 22.5 kHz. A quick test with the EU head on 20
recordings from `bat-demo-sounds`: the top-1 species matched the xeno-canto label in 13.
Pipistrelles, noctules, horseshoe bats, Barbastelle and Natterer's were mostly right.
Daubenton's, Pond, Bechstein's and Brown Long-eared were wrong or low. These recordings
may be in BattyBirdNET's training data (xeno-canto and ChiroVox), so treat 13 of 20 as
optimistic. The embeddings model loads in tract and matches onnxruntime (largest logit
difference 0.04).

BatDetect2 on the Phase 3 server is still an option for bats, but BattyBirdNET on the
device should be tried first, because it reuses everything BirdNET needs.

## 5. Licence

- **Three statements for the same V2.4 weights:**

  | Source | Licence given for the V2.4 models |
  |---|---|
  | BirdNET-Analyzer README and `docs/models.rst`; the tphakala and justinchuby ONNX conversions | CC BY-NC-SA 4.0 |
  | Zenodo record 15050749, "BirdNET Model V2.4", the files the `birdnet` package downloads | CC BY-NC 4.0 |
  | `birdnet-team/real-time-pwa` README (the BirdNET team's own PWA, MIT code) | CC BY-SA 4.0 |

  The PWA's model is V2.4: its TF.js model has 6,522 output units, and its shard sizes
  match the Zenodo TF.js zip. Until the BirdNET team says which applies, follow the
  strictest, CC BY-NC-SA 4.0. The contact address is in the V3.0 terms of use on Zenodo.
- **Required credit:** the tphakala ONNX conversions require "Powered by BirdNET
  (https://birdnet.cornell.edu/)" wherever the model is used.
- **V3.0 preview licence:** CC BY-SA 4.0, which allows commercial use, plus terms that
  forbid poaching and any military use. Its terms also say "Provided solely for research
  and evaluation". Revisit V3.0 when it is released, because without the NC clause a paid
  tier could include it.
- **Free app:** non-commercial use is allowed. BirdNET's README says educational and
  research use counts as non-commercial. Keep BirdNET out of any paid tier and out of
  anything sold.
- **GPL-3.0 and ShareAlike:** Oversample's code runs the model; it is not an adaptation
  of the weights, so the GPL code and the CC weights stay separate works. Downloading the
  weights at runtime keeps that separation clear, and it also means that someone who sells
  a GPL build of Oversample doesn't ship NC-licensed weights. If Oversample converts or
  changes the weights itself (another ONNX export, quantisation, pruning), publish the
  changed file under CC BY-NC-SA 4.0 and say what changed.
- **Hosting:** serve the ONNX file from Hugging Face or from Oversample's own archive
  host, together with its licence and a note of the conversion. CC BY-NC-SA allows
  redistribution with attribution.
- **Attribution in the app:** wherever BirdNET results appear, and in the About box:
  - "Powered by BirdNET (https://birdnet.cornell.edu/)".
  - BirdNET was developed by the K. Lisa Yang Center for Conservation Bioacoustics at the
    Cornell Lab of Ornithology and Chemnitz University of Technology.
  - The model file is the ONNX conversion by tphakala, with a link to CC BY-NC-SA 4.0.
  - Cite Kahl et al. (2021), "BirdNET: A deep learning solution for avian diversity
    monitoring", Ecological Informatics 61.
  - For BattyBirdNET, also credit the BattyBirdNET-Analyzer heads by rdz-oss
    (Zinck 2023).

This is a reading of the licences, not legal advice.

## Next steps

- A small in-app test: a worker that loads the ONNX file from OPFS, a "Find bird
  species" action on the current file, and results as annotations.
- Measure in real browsers (Chrome, Firefox, Safari), in Tauri on Android, and with
  WebGPU through onnxruntime-web.
- Try the location model in tract.
- Ask the BirdNET team which licence applies to the V2.4 weights in their PWA.

## Sources

- BirdNET-Analyzer README and `docs/models.rst`: github.com/birdnet-team/BirdNET-Analyzer
- `birdnet` Python package 1.4.0: pypi.org/project/birdnet (`acoustic/models/v2_4/tf.py`)
- BirdNET Model V2.4 on Zenodo: zenodo.org/records/15050749
- Chirpity: github.com/Mattk70/Chirpity-Electron
- BirdNET Live PWA: github.com/birdnet-team/real-time-pwa
- birdnet-web (TF.js port): github.com/georg95/birdnet-web
- ONNX conversions: huggingface.co/tphakala/BirdNET-v2.4, huggingface.co/justinchuby/BirdNET-onnx
- BirdNET+ V3.0 preview: zenodo.org/records/20703646 (`TERMS_OF_USE.txt`)
- BattyBirdNET: github.com/rdz-oss/BattyBirdNET-Analyzer, huggingface.co/tphakala/BattyBirdNET-onnx
- BSG-BAT: huggingface.co/tphakala/BSG-BAT, doi.org/10.5281/zenodo.15495676
- BatDetect2: github.com/macaodha/batdetect2 (`LICENSE.md`)
- `birdnet-onnx` crate: github.com/tphakala/rust-birdnet-onnx
- `ort`, `ort-web`, `ort-tract`: crates.io
