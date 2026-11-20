# BirdNET on the device: feasibility

Status: research, 2026-11-11; [BirdNET+ V3.0](#birdnet-v30) and [BirdBox Just-Bird](#birdbox-just-bird-bird-sound-boxes) sections 2026-11-21.

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
- **One code path:** the same worker ran unchanged in Chrome, Firefox and the Tauri
  desktop app on Linux. Only the model cache differs: WebKitGTK has no OPFS, so the
  Tauri build needs to cache the model file through Tauri. A native Tauri command could
  later use the same Rust code (tract native: 26.5 ms per window) or onnxruntime
  (17.5 ms).
- **Model file:** download the 52 MB ONNX file at runtime and cache it (in the browser's
  Origin Private File System, OPFS, which the app already uses for annotations; in the
  app data folder in Tauri). Don't commit it to this repository or bundle it in the app.
- **Licence:** BirdNET's own sources give three different licences for the V2.4 weights:
  CC BY-NC-SA 4.0, CC BY-NC 4.0 and CC BY-SA 4.0. Until the BirdNET team says which
  applies, follow the strictest, CC BY-NC-SA 4.0. Under it, a free app may download and
  run the weights with attribution, and the BirdNET feature must never sit behind a paid
  tier. See [Licence](#5-licence).

A second test ran the model in a Web Worker in Chrome 156, Firefox 159 and the Tauri
desktop app on Linux (WebKitGTK 2.52): 66 to 76 ms per window, 38 to 43 times real time,
the same top species as TFLite on all 60 windows, and no dropped frames on the page while
the worker ran. See [Browser and Tauri test](#browser-and-tauri-test). Not tested:
phones, Safari, Windows and macOS webviews, WebGPU.

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

## Browser and Tauri test

Test page: [birdnet-bench/web/](birdnet-bench/web/). The tract code from the node test,
built with wasm-pack for `wasm32-unknown-unknown` (`--target web`, SIMD, opt-level 3,
14.9 MB WASM file), runs in a module Web Worker. The page downloads the 52 MB ONNX file,
stores it in OPFS, and runs 60 windows (3 minutes of the Common Pauraque recording,
repeated). [birdnet-bench/web/run.py](birdnet-bench/web/run.py) starts the page's server,
opens one browser, samples the browser's memory, and stops both when the page posts its
results. Each browser ran three page loads in a row:

- **cold:** OPFS cleared, model downloaded from the local server, run in the worker.
- **warm:** model read from OPFS, run in the worker.
- **main:** model read from OPFS, run on the page's main thread, with a `setTimeout`
  yield between windows (how Oversample runs long jobs today).

Same machine as above (Ryzen 5 3600, 12 threads, Linux, Wayland), 2026-11-11.

| Browser | ms per window (median, worker) | 3 min of audio | Model load, cold | Model load, warm |
|---|---|---|---|---|
| Chrome 156 (Chrome for Testing, headless) | 66 to 69 | 4.2 to 4.4 s | 0.70 s | 0.57 s |
| Firefox 159 (headless) | 73 to 76 | 4.6 s | 0.91 s | 0.58 s |
| Tauri desktop app, WebKitGTK 2.52.6 (a window on screen) | 74 | 4.6 to 4.8 s | 0.98 s | 1.0 s (no OPFS; downloaded again) |

- **Load time:** the warm load is reading the model (33 ms from OPFS in Chrome, 90 to
  100 ms in Firefox), starting the WASM module (40 to 200 ms) and parsing and optimising
  the model in tract (360 to 590 ms). The cold download took 70 to 240 ms only because
  the server was on the same machine; over a real connection, 52 MB takes about 4 s at
  100 Mbit/s and about 21 s at 20 Mbit/s. Writing the model to OPFS took 60 to 70 ms.
- **First window:** 83 to 173 ms, about twice the median, in every browser.
- **Accuracy:** in all three browsers, top-1 matched TFLite on all 60 windows, and the
  confidence of the top species matched to 3 decimal places.
- **Page responsiveness:** a square on the page rotated every animation frame, and the
  page recorded the gaps between frames.

  | Where the model ran | Chrome | Firefox | Tauri (WebKitGTK) |
  |---|---|---|---|
  | Web Worker, during load and run | No frame over 17 ms | One frame of 50 ms during the cold load, otherwise none over 17 ms | No frame over 24 ms |
  | Main thread, yielding between windows | 31 of 129 frames over 50 ms, longest 633 ms | 60 of 78 frames over 50 ms, longest 449 ms | 55 of 61 frames over 50 ms, longest 830 ms |

  On the main thread the page drew at about 13 to 30 frames a second while BirdNET ran.
  The longest gap includes building the model (0.4 to 0.6 s), which can't be split up.
  In a worker the page kept 60 frames a second.
- **Memory:** summed proportional set size (PSS) of the browser's processes, peak during
  the three page loads, minus the peak for the same page held open with no model:

  | Browser | Peak with BirdNET | Peak without | Added by BirdNET |
  |---|---|---|---|
  | Chrome | 720 MB | 435 MB | about 285 MB |
  | Firefox | 983 MB | 678 MB | about 305 MB |
  | Tauri (WebKitGTK) | 730 MB | 407 MB | about 323 MB |

  The WASM memory alone was 160 MB in every browser. The rest is probably the 52 MB model
  held as JS bytes during loading, the worker itself, and the 14.9 MB WASM module.

Findings for the app:

- **WebKitGTK has no OPFS.** In the Tauri app on Linux, `navigator.storage` is
  undefined, so the model can't be cached in OPFS there. Cache it through Tauri instead
  (download in Rust to the app data folder, then read it from the webview). Oversample's
  annotations already use Tauri files when `is_tauri` is set, but `src/xc_client.rs`
  calls OPFS for the XC species list cache without that check.
- **Use a worker, and build the model in it.** Running on the main thread, even with a
  yield after every window, made the page stutter for the whole run, and building the
  model blocked it for about half a second.
- **Hold memory in mind for phones.** About 300 MB extra while BirdNET runs is fine on a
  desktop, but may be too much for a low-memory Android phone. Freeing the model when the
  run ends gives the memory back.

Not tested:

- **Phones:** no Android device was connected (`adb devices` listed none), so there are
  no phone numbers. The Android build uses the system WebView (Chrome-based), so the
  worker code should run there, but its speed and memory are unknown.
- **Safari and the Tauri webviews on macOS (WKWebView) and Windows (WebView2).** WebView2
  is Chrome-based. WKWebView is WebKit, like WebKitGTK; OPFS support there was not
  checked.
- **Headed Chrome and Firefox, and the app's in-app browser pane.** Chrome and Firefox
  ran headless; the Tauri window ran on screen.
- **WebGPU, onnxruntime-web in a browser, and more than one worker thread.**

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
| BirdNET+ V3.0 Global 12K (release 3.0.1), 11,725 classes | ONNX FP32 | 345 MB | Zenodo record 24362980 | Apache 2.0 + acceptable-use policy |
| BirdNET+ V3.0 Global 12K (3.0.1) | ONNX FP16 (mixed precision) | 174 MB | Zenodo record 24362980 | Apache 2.0 + acceptable-use policy |
| BirdNET+ V3.0 Global 12K (3.0.1), pruned to one of three heads | ONNX FP16 | 72 MB | Zenodo record 24362980 | Apache 2.0 + acceptable-use policy |
| BirdNET+ V3.0 Global 10K, the BirdNET Live app's model, 10,064 classes | ONNX FP16, inside a 70 MB zip with a 7 MB Geomodel | 68 MB | Zenodo record 24362980 | Apache 2.0 + acceptable-use policy |

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

BirdNET+ V3.0 was released on 2026-11-13 (3.0.1 on 2026-11-19) and replaces the developer
preview this page first described. It takes 32 kHz audio and was tested in tract; see
[BirdNET+ V3.0](#birdnet-v30).

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

## BirdNET+ V3.0

Tested 2026-11-20 and 21; details, commands and the integration plan are in
[birdnet-v3-bench/README.md](birdnet-v3-bench/README.md). The app is to offer V3.0 next
to V2.4, not in its place: BattyBirdNET's heads need V2.4's 1,024-value embedding, and
V2.4 is the fast option.

- **Release:** "BirdNET+ V3.0 Global 12K", release 3.0.1 (2026-11-19), Zenodo record
  24362980, Apache 2.0 with an acceptable-use policy. 11,725 classes: 9,788 birds, 802
  insects, 649 amphibians, 419 mammals (including Human), 35 fish, 15 reptiles, 1
  arachnid and 16 non-animal sounds. Input is 32 kHz mono of any length (3 s windows of
  96,000 samples are the default); outputs are scores already through a sigmoid
  (`predictions`) and a 1,280-value embedding (`embeddings`). The spectrogram is inside
  the model, built from MatMuls (no STFT operator).
- **tract:** tract 0.23.10 runs every file after three in-memory changes to the graph:
  float16 weights to float32, Cast(to=float16) to Cast(to=float32), and an explicit
  float32 value for one `ConstantOfShape` (tract makes float64 zeros there, and the run
  then fails at the final Sigmoid). With them, the official float16 files download
  unchanged and match onnxruntime to 0.0002.
- **Recommended file:** the pruned one (72 MB, one of the three classifier heads). Its
  embedding is identical to the full model's, it needs 373 MB of WASM memory against
  970 MB for the full file (174 MB), and its scores were up to 0.16 lower than the full
  model's on 18 test windows (top class the same on 17).
- **Speed (estimated for a quiet machine from runs on a busy one):** about 355 to 380 ms
  per 3 s window in tract WASM in node, about 320 to 345 ms in a browser, so about 8
  times faster than real time. That is about 5 times the 64 to 74 ms measured for V2.4's
  `foldconv` file. Measured under load: 382 to 408 ms in WASM, 128 to 157 ms natively,
  2.1 to 2.3 times the app's own V2.4 file in the same sessions.
- **Silence:** the pruned file scores up to 0.25 on digital silence (all zeros), so the
  app must skip all-zero windows. Low noise is fine.
- **The BirdNET Live model** in the same record is the pruned network with 1,661 fewer
  classes, so it is no faster.
- **Location list:** the Geomodel V3.0.5 in the record has the same weights as the
  v3.0.4 file the app already uses (scores within 6e-7). V3.0's classes join to it by
  `birdnet_id` (10,926 of 11,709 animal classes) or to the app's v3.0.4 labels by
  scientific name (10,874).

## BirdBox Just-Bird (bird sound boxes)

Tested 2026-11-21; details, commands, pictures and the integration plan are in
[birdbox-bench/README.md](birdbox-bench/README.md). Just-Bird (BirdBox v2.1.0, BirdNET
Team) is a YOLO11s detector that draws a box in time and frequency around each bird
sound, with no species. The app would pair each box with BirdNET+ V3.0 for a species.

- **The release file is the bare network:** `Just-Bird_fp16.onnx`, 18.9 MB, a 3 x 256 x 256
  spectrogram image in, 1,344 decoded boxes with a `bird` score out. The spectrogram,
  NMS and window fusion are outside it. BirdBox's all-in-one graph (audio in, merged
  boxes out) is a different file, the demo's `docs/models/Just-Bird_fp16.onnx` (21.2 MB),
  with the same network weights.
- **tract:** runs the release file after the in-memory float16-to-float32 change used for
  V3.0, and matches onnxruntime to 0.00004 in score. The int8 file gives wrong output in
  tract 0.23.10; use the float16 one.
- **Same boxes:** the spectrogram image, NMS and window fusion, ported to Rust from
  BirdBox's MIT code, give the same boxes as BirdBox's CLI: 239 of 239 merged boxes on 8
  recordings (11 minutes) matched at IoU 0.75, mean IoU 0.997, confidence within 0.0007.
- **Speed:** 172 to 176 ms per 3 s window in tract WASM in node, a window every 1.5 s, so
  about 8 times faster than real time (measured on a quiet machine); 23 times natively.
  122 MB of WASM memory. Per second of audio about the same cost as V3.0.
- **Fetching:** the GitHub release URL sends no CORS headers. The demo graph at the tag on
  `raw.githubusercontent.com` does, and the worker can cut the network out of it in
  memory (tested: the same boxes), so no mirror is needed. A Hugging Face mirror of the
  release file is the other way.
- **Licence:** weights AGPL-3.0, code MIT. Download the unmodified file at run time and
  credit it; a mirror must carry the AGPL text and point to the source.
- **What the boxes look like:** tight boxes on single calls in soundscapes; a steady song
  becomes one long box (up to 16 s); some faint calls are missed; voices get false boxes
  at low confidence (a minimum of 0.3 removes most). No boxes on a bat recording.

## 5. Licence

- **Three statements for the same V2.4 weights:**

  | Source | Licence given for the V2.4 models |
  |---|---|
  | BirdNET-Analyzer README and `docs/models.rst`; the tphakala and justinchuby ONNX conversions | CC BY-NC-SA 4.0 |
  | Zenodo record 15050749, "BirdNET Model V2.4", the files the `birdnet` package downloads | CC BY-NC 4.0 |
  | `birdnet-team/real-time-pwa` README (the BirdNET team's own PWA, MIT code) | CC BY-SA 4.0 |

  The PWA's model is V2.4: its TF.js model has 6,522 output units, and its shard sizes
  match the Zenodo TF.js zip. Until the BirdNET team says which applies, follow the
  strictest, CC BY-NC-SA 4.0. The contact address (ccb-birdnet@cornell.edu) is in the
  V3.0 acceptable-use policy on Zenodo.
- **Required credit:** the tphakala ONNX conversions require "Powered by BirdNET
  (https://birdnet.cornell.edu/)" wherever the model is used.
- **V3.0 licence:** the released V3.0 weights and labels (3.0.1) are Apache 2.0, with no
  non-commercial clause, so unlike V2.4 they could be part of a paid tier. The
  acceptable-use policy that comes with them rules out uses that harm wildlife or people
  (poaching, military targeting, covert surveillance of people) and asks that derived
  apps and copies not use the BirdNET names or branding in a way that implies the
  BirdNET team endorses them. It replaces the developer preview's CC BY-SA 4.0 terms,
  which limited it to research and evaluation. Cite Lasseck, Eibl, Klinck and Kahl
  (2026), BirdNET+ V3.0 model files (Version 3.0.1), doi 10.5281/zenodo.24362980.
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

- A small in-app test: a worker that loads the ONNX file from OPFS (or from the app
  data folder in Tauri), a "Find bird species" action on the current file, and results
  as annotations.
- Measure on an Android phone (in Chrome and in the Tauri Android app), in Safari, and
  with WebGPU through onnxruntime-web.
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
- BirdNET+ V3.0 (release 3.0.1): zenodo.org/records/24362980 (`MODEL_LICENSE.txt`,
  `ACCEPTABLE_USE.txt`); github.com/birdnet-team/birdnet-V3.0-dev (README, `docs/model.md`)
- BattyBirdNET: github.com/rdz-oss/BattyBirdNET-Analyzer, huggingface.co/tphakala/BattyBirdNET-onnx
- BSG-BAT: huggingface.co/tphakala/BSG-BAT, doi.org/10.5281/zenodo.15495676
- BatDetect2: github.com/macaodha/batdetect2 (`LICENSE.md`)
- `birdnet-onnx` crate: github.com/tphakala/rust-birdnet-onnx
- `ort`, `ort-web`, `ort-tract`: crates.io
