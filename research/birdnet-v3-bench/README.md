# birdnet-v3-bench

Spike for adding BirdNET+ V3.0 to Oversample as a fourth classifier, next to BirdNET
V2.4 (which stays). The spike asked four questions: can tract run V3.0, how fast, how
big is the download, and do the scores match the reference. Nothing here is part of the
app build: this folder has its own `[workspace]`. Done 2026-11-20 and 21.

## Answer

- **tract runs all three V3.0 files.** It needs three small changes to the ONNX graph,
  which [`patch_proto`](src/lib.rs) makes in memory before tract builds the model. With
  them, the official float16 files from Zenodo load unchanged, compute in float32, and
  match onnxruntime to 0.0001 in score.
- **Recommended:** the pruned file, `BirdNET+_V3.0_Global_12K_FP16_pruned.onnx` (72.3 MB,
  11,725 classes), downloaded as published and patched in the worker. It gives the same
  1,280-value embedding as the full model, needs 373 MB of WASM memory against 970 MB, and
  is about 15 percent faster in WASM.
- **Cost:** about 355 to 380 ms per 3 s window in tract WASM in node on a quiet machine
  (estimated from a measured 382 to 408 ms under load), about 8 times faster than real
  time. That is about 5 times BirdNET V2.4 per window and 1.8 times Perch's embedding
  model.
- **Quality:** on 18 test windows the pruned file's scores were up to 0.16 lower than the
  full model's. Its top class matched on 17 of 18 windows, and the set of classes at 0.25
  or more matched on 17 of 18. On digital silence (all zeros) it scores 0.25 for Green-billed
  Coucal, so the app must skip all-zero windows. Quiet noise is not a problem: the model
  normalises the level.
- **Later, if accuracy matters more than effort:** the backbone in tract with all three
  heads in plain Rust on int8 weights (option d) would give the full model's scores from
  about 108 MB of downloads and much less memory. This was estimated, not built.

## Files

All from Zenodo record 24362980, "BirdNET+ V3.0 model files", version 3.0.1, published
2026-11-19, Apache 2.0 plus an acceptable-use statement. Each file's MD5 matched the
record's listing; the BirdNET Live zip's own `SHA256SUMS` matched too. Saved under
`~/.cache/oversample-research/birdnet-v3/zenodo/`, outside this repository.

| File | Size | Used for |
|---|---|---|
| `BirdNET+_V3.0_Global_12K_FP16_pruned.onnx` | 72.3 MB | Option (a) |
| `BirdNET+_V3.0_Global_10K_BNL.zip` (contains `BirdNET+_V3.0_Global_10K_FP16_BNL.onnx`, 68.1 MB, its labels, and a 6.9 MB Geomodel) | 69.6 MB | Option (b) |
| `BirdNET+_V3.0_Global_12K_FP16.onnx` | 173.9 MB | Option (c) |
| `BirdNET+_V3.0_Global_12K_FP32.onnx` | 345.4 MB | The reference |
| `BirdNET+_V3.0_Global_12K_Labels.csv` | 8.0 MB | Labels |
| `BirdNET+_Geomodel_V3.0.5_Global_14K.zip` | 111.2 MB | Label join |

SHA-256 of the files the app might download:

| File | SHA-256 |
|---|---|
| `BirdNET+_V3.0_Global_12K_FP16_pruned.onnx` | `feadeccc84feb021aeb9c51ec5288f6f5bd7065fe3773e2d48a69ea8a26b412a` |
| `BirdNET+_V3.0_Global_12K_FP16.onnx` | `ae9c42ae50e39f4d70fb1e5a90549456f8357a183450b60afb88d3f1061f8928` |
| `BirdNET+_V3.0_Global_10K_FP16_BNL.onnx` (inside the zip) | `fa5611f56f8f84c0f36d6a40aed1e21e3abcf67340779dca084f4cccab8d1231` |
| `BirdNET+_V3.0_Global_12K_Labels.csv` | `88d976477fac81127ac8ac4c215c37749d0a6537944112ea606d321317cb7640` |
| `BirdNET+_Geomodel_V3.0.5_Global_14K_Labels.txt` (inside the zip) | `7997f12f1814741a681d317e3dbdc8eb1f6bee9410c277bd3ae8c393f300571b` |

Test audio, all from `batchi-extras` (`xc-cache` was empty): Gentoo Penguin
(`bat-demo-sounds/sounds/XC540767`, the demo file, 6 windows), Pauraque
(`no-bat-sounds/sounds/XC1083752`, 6 windows), Common Scoter (`XC1086259`, 3) and Alpine
Swift (`XC1140427`, 3), then one window of digital silence and windows of white noise at
-90 and -60 dBFS RMS: 21 windows of 3 s at 32 kHz. Timing used 60 windows of the
Pauraque recording (3 minutes).

## What the files are made of

From [inventory.py](inventory.py):

| | Pruned (a) | BirdNET Live (b) | Full FP16 (c) | Full FP32 |
|---|---|---|---|---|
| Opset, IR | 18, 10 | 18, 10 | 18, 10 | 18, 10 |
| Input | `input` float32 `[batch_size, samples]` | same | same | same |
| Outputs | `predictions` `[batch_size, 11725]`, `embeddings` `[batch_size, 1280]` | `predictions` `[batch_size, 10064]`, `embeddings` | as (a) | as (a) |
| Nodes, op types | 1,078, 29 | 1,078, 29 | 1,144, 33 | 827, 32 |
| Weights | 70.1 MB float16 (311 tensors), 0.8 MB float32 | 65.9 MB float16 | 171.4 MB float16 (317), 1.0 MB float32 | 343.9 MB float32 |
| Casts | 140 to float16, 171 to float32 | same | 140 to float16, 177 to float32 | none |

- Op types in all files: Conv, Mul, Sigmoid, Add, Sub, Reshape, ReduceMean, Div, Concat,
  Gather, Transpose, Max, Shape, MatMul, Pad, Clip, ReduceMax, Squeeze, Unsqueeze,
  Slice, Log, Exp, Range, ReduceMin, Resize, Expand, ConstantOfShape, Gemm, and Cast in
  the float16 files. The full model adds MaxPool, Relu, Softmax and ReduceSum for its two
  attention heads.
- There is no STFT or DFT operator. The spectrogram is MatMuls with stored DFT matrices
  (a two-stage 32 x 64 factorisation of the 2,048-point FFT), a 1,025 x 128 mel MatMul,
  a log, and a linear Resize (`pytorch_half_pixel`) to 224 bands.
- Dynamic dimensions: only batch and sample count. Fixing the input at `[1, 96000]` is
  enough for tract; onnx-simplifier is not needed.
- The float16 files are "mixed precision": weights float16, each convolution wrapped in
  Cast(to=float16) and Cast(to=float32), everything else float32. On x86, onnxruntime
  runs those convolutions in float32, so its scores for the float16 file and for a
  float32 copy were identical.
- **All three files expose the embedding** as the `embeddings` output. It is the GeM-pooled
  1,280-channel feature map, the same tensor in every file: the pruned and full float16
  files gave identical embeddings (largest difference 0.0), and the BirdNET Live file's
  are the same too.
- **The pruned and BirdNET Live files are the same network.** Their scores were identical
  on every shared class (joined on `birdnet_id`, largest difference 0.0000), and their
  weights differ only by the 1,661 rows the BirdNET Live head drops. The pruned head is
  one linear layer on the embedding: `predictions = sigmoid(W x embeddings + b)`, with W
  11,725 x 1,280. Recomputing it in numpy from the stored weights matched to 3e-7.
- **The full model's three heads**, from the graph:
  - GeM: the same linear layer as the pruned file, weight 0.40.
  - Temporal attention: GeM over frequency, then max-pool plus average-pool (width 3)
    over time, a 1,280 to 1,024 linear layer with ReLU, then two 1,024 to 11,725
    projections (attention and class) per time step. Softmax over time, weighted sum,
    weight 0.35.
  - Frequency attention: the same over the 7 frequency positions, weight 0.25.
  - The weighted logits are summed and go through one sigmoid.
- Multiply-adds per 3 s window ([macs.py](macs.py)): spectrogram 0.15 G, backbone 3.64 G,
  heads 0.42 G (full) or 0.015 G (pruned). The total, 4.2 G, matches the published 1.38
  GMAC per second. The feature map before pooling is 1,280 x 7 x 9.
- Weights outside the heads: 20.6 M values (41 MB float16, 83 MB float32). Full heads:
  65.7 M values (131 MB float16, 66 MB int8). Pruned head: 15.0 M values (30 MB
  float16, 15 MB int8).

## tract: obstacles and fixes

tract 0.23.10, the version in the repository's `Cargo.lock` that `birdnet-worker` uses,
loaded with the worker's options (`with_ignore_value_info`, `with_ignore_output_shapes`,
input fact `[1, 96000]`).

| # | What happened | Cause | Fix |
|---|---|---|---|
| 1 | Every file, the float32 one included, loads, then fails on the first window: `Evaluating "node_sigmoid_30" Sigmoid: Sigmoid does not support F64` | The head starts from `ConstantOfShape` with no `value` attribute (`new_zeros` in PyTorch). ONNX says that means float32 zeros; tract 0.23 makes float64 zeros, and the Adds after it turn the scores into float64. | Give the node an explicit float32 zero. onnx-simplifier with a fixed input also fixes it, by folding the node away. |
| 2 | With fix 1 only, the float16 files run, but at 22.8 s per window natively (tract computes the float16 convolutions without SIMD on x86), with scores up to 0.023 away from onnxruntime | The Cast(to=float16) nodes | Change them to Cast(to=float32) |
| 3 | Keeping the weights float16 in the file behind a Cast(to=float32) node (`convert.py --weights fp16`) gives correct scores, but runs at 368 to 407 ms per window natively against 173 to 184 ms | tract keeps the Cast in the plan and converts the weights on every run | Convert the weights to float32 before tract builds the model |

[`patch_proto`](src/lib.rs) does all three in memory: it reads the file with
`tract_onnx::onnx().proto_model_for_read`, turns float16 initializers into float32,
Cast(to=float16) into Cast(to=float32) and gives `ConstantOfShape` a float32 value, then
calls `model_for_proto_model`. So **the app can download the official float16 file (half
the size) and still compute in float32**, with no converted file to host. On the
float32 file it only makes the third change. [convert.py](convert.py) makes the same
changes as a file, for onnxruntime checks and for the backbone cut.

No other problems: Resize, Pad, Range, Expand, Gather and the reductions all load and
optimise at a fixed input size.

### tract against onnxruntime

Same file, same 21 windows ([results/compare.txt](results/compare.txt)):

| Run | Same top-1 | Same classes at 0.25 or more | Largest score difference | Embedding cosine (lowest) |
|---|---|---|---|---|
| tract native, pruned file patched in memory | 21 of 21 | 21 of 21 | 0.0002 | 1.000000 |
| tract wasm32 (wasm-bindgen), pruned file patched in memory | 21 of 21 | 21 of 21 | 0.0002 | 1.000000 |
| tract native, BirdNET Live file patched in memory | 21 of 21 | 21 of 21 | 0.0002 | 1.000000 |
| tract native, full FP32 file (fix 1 only) | 21 of 21 | 21 of 21 | 0.0000 | 1.000000 |
| tract native, full FP16 file patched in memory, against the FP32 reference | 21 of 21 | 21 of 21 | 0.0033 | 0.99964 |

The 0.0033 in the last row is the float16 weights themselves: onnxruntime on the FP16
file gives the same difference from FP32.

## Options compared

Times are per 3 s window, one thread, on the Ryzen 5 3600 used for the earlier BirdNET and
Perch notes. **The machine was busy** (load average 8 to 14 on 12 threads, from other
sessions' builds), so every run was repeated over three sessions, taking turns with
reference models: [results/bench-session1.txt](results/bench-session1.txt),
[2](results/bench-session2.txt), [3](results/bench-session3.txt).

| Option | Download (as the app would fetch it) | Classes | tract native, measured | tract WASM in node, measured | WASM memory | Scores against the full model |
|---|---|---|---|---|---|---|
| (a) **Pruned, official FP16 file, patched in memory** | **72.3 MB** | 11,725 | 128 to 157 ms | 382 to 408 ms | 373 MB | Up to 0.16 lower; top-1 17 of 18 |
| (b) BirdNET Live, official FP16 file, patched in memory | 68.1 MB, but only inside a 69.6 MB zip, so it needs a mirror | 10,064 | 145 ms (one run) | not run; the same network as (a) | about as (a) | Same as (a) on shared classes |
| (c) Full, official FP16 file, patched in memory | 173.9 MB | 11,725 | 166 to 173 ms | 452 to 479 ms | 970 MB | Within 0.0033 (float16 weights) |
| (d) Backbone cut in tract + heads in plain Rust | 42.2 MB backbone (float16, measured) + heads: 15 MB int8 for the pruned head, 66 MB int8 for all three | 11,725 | backbone 134 to 139 ms + heads (estimate: under 10 ms) | backbone 389 to 403 ms + heads (estimate: about 1 ms for the pruned head, about 15 ms for all three) | backbone 222 MB + heads | Pruned head: as (a). All three heads: as (c), less int8 rounding |

Reference models in the same sessions:

| Model | tract native, measured under load | tract WASM in node, measured under load | Earlier, on a quiet machine |
|---|---|---|---|
| Perch 2.0 embedding model (`perch_v2_embed.tract.onnx`, 5 s window) | 132 to 140 ms | 224 to 237 ms | 118 ms native, 206 to 219 ms WASM (`../perch.md`) |
| BirdNET V2.4 with embeddings (`birdnet-v2.4-embeddings-fp32.onnx`, the app's file) | 56 to 107 ms | 167 to 257 ms | 26.5 ms native, 64 to 74 ms WASM, but for the `foldconv` file, not this one |

**Quiet-machine estimate.** Perch's embedding model is the better yardstick: it is the
same file and code as its quiet measurement, and the load slowed it by about 8 percent
in WASM and 15 percent natively. BirdNET V2.4's times moved by 2 to 4 times between
sessions, and its quiet numbers are for a different file, so they are left out of the
estimate. Scaling by Perch's slowdown (estimates, not measurements):

| Option | tract native | tract WASM in node | Browser (Chrome ran Perch at 0.9 times node's time) | Faster than real time, WASM |
|---|---|---|---|---|
| (a) Pruned | about 110 to 120 ms | about 355 to 380 ms | about 320 to 345 ms | about 8 times |
| (c) Full | about 145 to 150 ms | about 420 to 445 ms | about 380 to 400 ms | about 7 times |

For comparison: BirdNET V2.4 in the browser takes 64 to 74 ms per 3 s window, and Perch
0.39 to 0.42 s per 5 s window (both files). Per second of audio, V3.0 (a) costs about
110 to 125 ms, Perch about 80 ms and V2.4 about 23 ms.

Other measurements:

- **Load time** (parse, patch, optimise): (a) 1.0 to 1.5 s native and 1.7 to 1.9 s in
  WASM; (c) 2.3 to 2.7 s native and 2.5 to 2.7 s in WASM. The in-memory patch is part of
  these times.
- **Peak memory, native:** (a) 454 to 460 MB RSS, (c) 850 to 880 MB, backbone cut 266 to
  277 MB.
- **WASM to native ratio:** about 3 for V3.0, against about 1.7 for Perch's embedding
  model. Some op in V3.0 is much slower in WASM than natively; a tract profile would show
  which. Not looked into.
- **onnxruntime 1.32 (Python, one thread, same load):** pruned 92 to 99 ms, full 114 to
  126 ms. The BirdNET team's own figure is 2 minutes of audio in about 2.5 s with 8
  threads.
- WASM build: 14.9 MB, the same tract code as the app's worker.

### Option (d) in detail

- **Pruned head in Rust:** one 11,725 x 1,280 matrix-vector product and a sigmoid on the
  embedding, 15 million multiply-adds. The worker's Perch classifier
  ([perch/classifier.rs](../../birdnet-worker/src/perch/classifier.rs)) does 5.8 billion
  int8 multiply-adds in 208 ms in WASM, so this head would take about 1 ms. Saves 15 MB of
  download against (a) (57 MB in all) at the cost of a second file and int8 rounding
  (Perch's int8 head moved scores by up to 0.028). Small to build.
- **All three heads in Rust:** the tract part would have to output the 1,280 x 7 x 9
  feature map instead of the pooled embedding, and the Rust side would redo the GeM
  pooling, including its clamp-and-rescale steps, the pooling over time and frequency,
  two 1,280 to 1,024 layers with ReLU, four 1,024 to 11,725 projections and two softmaxes.
  0.42 billion multiply-adds, so about 15 ms in WASM with int8. Download 42 MB + 66 MB =
  108 MB, against 174 MB for (c); WASM memory about 222 MB + 66 MB, against 970 MB.
  Medium to build, and every step must be checked against onnxruntime. It is the way to
  get the full model's scores on phones.

## Score quality

Against the full FP32 model in onnxruntime, on 18 windows of the four recordings
([results/compare.txt](results/compare.txt), per-window top 5 in
[results/](results/)):

| Variant | Same top-1 | Same classes at 0.25 or more | Largest score difference | Embedding cosine (lowest) |
|---|---|---|---|---|
| Full FP16 (c) | 18 of 18 | 18 of 18 | 0.0033 | 0.99989 |
| Pruned (a) and BirdNET Live (b) | 17 of 18 | 17 of 18 | 0.157 | 0.99989 |

- The pruned scores are mostly lower on true detections: Gentoo Penguin 0.46 to 0.90
  against 0.56 to 0.95 in the full model, Common Scoter 0.80 and 0.93 against 0.88 and
  0.95. The Pauraque scores were within 0.03 of each other.
- The top-1 difference was a window of the Scoter recording with no clear call, where
  every score was under 0.06. The 0.25 difference was one Alpine Swift window, where the
  pruned file gave Straneck's Tyrannulet 0.31 and the full model 0.21; the full model is
  under 0.25 on it.
- Three of the 18 windows (one Scoter, two Alpine Swift) had nothing at 0.25 or more in
  the full model; the pruned file had two such windows.
- The "top-5 same set" count in compare.txt is low for the pruned file (3 of 18) because
  ranks 2 to 5 are all classes under 0.1; it does not change what the app would show.

Silence and noise, highest class:

| Window | Full FP32 | Full FP16 | Pruned and BirdNET Live |
|---|---|---|---|
| Digital silence (all zeros) | Common Blackbird 0.035 | 0.035 | **Green-billed Coucal 0.253**, Mourning Dove 0.246 |
| White noise, -90 dBFS RMS | Great Kiskadee 0.031 | 0.031 | Great Kiskadee 0.021 |
| White noise, -60 dBFS RMS | Northern Cardinal 0.018 | 0.018 | Great Kiskadee 0.016 |

- The -90 and -60 dBFS windows give nearly the same scores, so the model normalises the
  input level. Only exact zeros give the high pruned scores, which is what the BirdNET
  README warns about. Skipping all-zero windows (or windows whose samples are all within
  one LSB of zero) is enough.
- No variant returned NaN on silence; release 3.0.1 fixed that.

## Labels

From [labels.py](labels.py):

- `BirdNET+_V3.0_Global_12K_Labels.csv`: UTF-8, comma-separated, a header and 11,725 rows
  in output order. Columns: `ix`, `birdnet_id` (`BN00001`...), `scientific_name`,
  `common_name`, `taxon_group`, `inat_id`, `ebird_code`, then `common_name_en` and 35
  more `common_name_<lang>` columns (43 columns). `ix` runs 0 to 11,724 in order;
  `birdnet_id` and `scientific_name` are both unique. No field is quoted, and the first
  eight columns contain no commas.
- Taxon groups: Aves 9,788, Insecta 802, Amphibia 649, Mammalia 419 (including Homo
  sapiens, "Human", at index 5,130), Actinopterygii 35, Reptilia 15, Anthropogenic 14,
  Arachnida 1, Geophony 1 (Fire), Music 1. The 16 non-animal classes are the last 16
  (11,709 to 11,724).
- 272 rows (263 insects, 9 amphibians) have no English name; `common_name` then repeats
  the scientific name. 1,940 rows have no eBird code and 28 no iNaturalist id.
- **Trimmed file for the app:** one line per class, tab-separated: `birdnet_id`,
  scientific name, English name (the scientific name when there is none), taxon group.
  630 KB, 183 KB gzipped, against 8.0 MB (2.7 MB gzipped) for the full CSV.
- **Geomodel V3.0.5** (in `BirdNET+_Geomodel_V3.0.5_Global_14K.zip`): its labels text
  file has 14,057 lines of `birdnet_id`, scientific name, English name. Joined on
  `birdnet_id`, 10,926 of the 11,709 animal classes have a Geomodel output (the README
  says the same). The 16 non-animal classes have none and should never be filtered.
- **The app's Geomodel today** is v3.0.4 (`birdnet-geomodel-v3.0.4-labels.txt`, 14,082
  species, no BirdNET ids). V3.0.5 has the same weights: for a test place and week
  (Adelaide, week 20) the 13,994 species the two share by scientific name scored within
  6e-7. Joined on scientific name, v3.0.4 covers 10,874 of the 11,725 audio classes, 52
  fewer than V3.0.5 by id. So the app can keep its Geomodel file and join by name, or
  swap in the V3.0.5 labels for the id join.
- Both the V3.0.5 FP32 and FP16 Geomodel ONNX files and the BirdNET Live 10K Geomodel load
  and run in tract with the worker's loader (`native geo`).
- **BirdNET Live labels:** 10,064 rows, all in the 12K list and in the same order, with a
  `geo_ix` column for its own 10,048-species Geomodel. It drops 1,661 classes: 804 birds,
  473 insects, 227 amphibians, 106 mammals, all 35 fish, all 15 reptiles and the one
  arachnid, so that every species is one its Geomodel can place.

## Recommendation

Ship option (a): the official pruned FP16 file, downloaded unchanged and patched in
memory by the worker.

- It is the smallest official file that has all 11,725 classes, and the app can verify it
  against the SHA-256 above. No converted file is needed.
- It runs about 15 percent faster than the full model in WASM and needs 373 MB of WASM
  memory against 970 MB, which matters on phones.
- Its embedding is identical to the full model's, so the live view, sound groups and any
  future small classifiers on the embedding lose nothing.
- Its scores are somewhat lower than the full model's (up to 0.16 on these windows). Use
  the BirdNET team's suggested 0.25 as the default minimum confidence, and skip all-zero
  windows.
- Not (b): it is the same network with 1,661 fewer classes, and the file is only
  published inside a zip, so the browser cannot fetch it directly.
- Not (c) for now: 2.4 times the download and 2.6 times the WASM memory for better
  scores. If the score gap matters later, (d) with all three heads gives the full model's
  scores at a lower cost than (c).

**Hosting.** Zenodo's API file URLs
(`https://zenodo.org/api/records/24362980/files/<name>/content`) send
`Access-Control-Allow-Origin: *`, and a Zenodo record version never changes, so the app
could fetch the pruned file directly. Downloads from Zenodo in this session ran at 30 KB/s
to about 3 MB/s, and the API reports rate limits, so a Hugging Face mirror of the
unchanged file (plus the trimmed label file) would be faster and match how the other
models are served. Apache 2.0 allows the mirror with the licence and notice files.
Publishing to Hugging Face is the owner's step. This spike uploaded nothing.

**Licence.** The V3.0 weights and labels are Apache 2.0 (`MODEL_LICENSE.txt` in the
record), with an acceptable-use policy (`ACCEPTABLE_USE.txt`). It covers converted
copies too. It rules out uses that harm wildlife or people (poaching, military
targeting, covert surveillance of people), asks users to report the model version and
precision with results, and says derived apps and redistributed copies should not use
the names "BirdNET", "BirdNET+" or "BirdNET V3.0", or the project's branding, in a way
that implies the BirdNET team endorses them. So the app can say which model it runs
(for example "Model: BirdNET+ V3.0 (pruned, 3.0.1)"), but a mirror's name and the app's
feature name should not look official. Unlike V2.4, there is no non-commercial clause. Cite Lasseck, Eibl, Klinck and Kahl
(2026), BirdNET+ V3.0 model files (Version 3.0.1), Zenodo, doi 10.5281/zenodo.24362980.

## Integration plan

Sizes: small is under a day, medium one to three days, large more.

| Step | Files | Size |
|---|---|---|
| 1. Add `Method::BirdNetV3`: `model_rate` 32,000, `window_samples` 96,000, `embedding_len` 1,280, `min_source_rate` None; new constants `V3_WINDOW_SAMPLES`, `V3_EMBEDDING_LEN`, `V3_CLASSES` (11,725); a `ModelPart::BirdNetV3` in the audio slot. `Method` is matched in about 40 places across `src/`, `birdnet-worker/` and `oversample-ipc/`; the compiler lists them. | `oversample-ipc/src/classifier.rs` and the match sites | Medium |
| 2. Catalog: `FileKind::BirdNetV3` and `FileKind::BirdNetV3Labels`, two `ModelFile`s with URL, size and SHA-256, `ModelChoice::BirdNetV3`, `all_files`, and an `attribution` entry (model "BirdNET+ V3.0, pruned, 3.0.1", Apache 2.0, the citation above, a link to the acceptable-use policy; see its branding rule). | `oversample-ipc/src/classifier/catalog.rs` | Small |
| 3. Engine: a `BirdNetV3` model type that loads through `patch_proto` (copy it from `src/lib.rs` here), keeps the `embeddings` and `predictions` outputs, and runs one `[1, 96000]` window. The outputs are already probabilities, so the top-k must not apply a sigmoid again (`engine::top_scores` does). Skip all-zero windows. | `birdnet-worker/src/engine.rs` (or a new `birdnet-worker/src/v3.rs`), `birdnet-worker/src/session.rs` (the audio-model enum, the method-and-model check near line 495, `live_window`) | Medium |
| 4. One model family at a time: `ensure_family(perch: bool)` restarts the worker when the family changes; make it a three-way family so V2.4, V3.0 and Perch replace one another. | `src/classifier/worker.rs`, `src/classifier/mod.rs` | Small |
| 5. Resampling to 32 kHz: already there for Perch (`Method::model_rate`), so nothing new beyond the 3 s window. | `birdnet-worker/src/resample.rs`, `windows.rs` | Small |
| 6. Embeddings: the per-method `embedding_len()` already sizes the stored embeddings; add 1,280 and check the embedding store and the live view's buffers. | `src/state.rs`, `src/live_embedding.rs` | Small |
| 7. Labels: a parser for the trimmed file (or the first eight columns of the full CSV), setting `is_sound` for the Anthropogenic, Geophony and Music groups and keeping Human as a species; a `birdnet_v3_labels` loader like `perch_labels`. | `src/classifier/types.rs`, `src/classifier/mod.rs` | Small |
| 8. Location list: add the method to `applies_to`, and join V3.0's classes to the Geomodel by scientific name (works with today's v3.0.4 file) or switch to the V3.0.5 labels and join by `birdnet_id`. Never mark the 16 non-animal classes. | `src/classifier/location.rs`, catalog | Small (name join) or medium (V3.0.5 labels, which also need a mirror since they are inside a zip) |
| 9. Species map and live-view assets: a V3.0 map from the head weights (as `research/species-map` did for V2.4 and Perch), and PC and UMAP files for the 1,280-value embedding (`research/live-embedding`). Until they exist, `map_spec` can return None for V3.0. | `src/classifier/species_map.rs`, `research/species-map/`, `research/live-embedding/`, `index.html` copy-file links | Large |
| 10. Live view: a `BIRDNET_V3: LiveModel` (3 s window, 1,280 values, `min_gap_ms` about 500 to allow for 350 ms readings in the browser) and `model_for`. | `src/live_embedding.rs` | Medium (after step 9) |
| 11. UI: the method list and its files, the per-method minimum confidence (new setting, default 0.25), credits, and the places that name a method in text. | `src/components/file_sidebar/call_detection_panel/classifier_section.rs`, `call_detection_panel.rs`, `src/classifier/mod.rs`, `summary.rs`, `strips.rs`, `src/components/sound_groups.rs`, `spectrogram.rs` | Medium |
| 12. Tests: a real-model test like the Perch one (top class on the Gentoo Penguin demo, embedding length, silence skipped). | `birdnet-worker/tests/real_models.rs` | Small |
| 13. Hosting: mirror the pruned file and the trimmed labels on Hugging Face (owner's step), or point the catalog at the Zenodo API URLs. | Hugging Face, catalog | Small |

## Steps

Python 3.12 venv: `pip install onnx onnxruntime numpy soundfile scipy onnxsim`. Keep the
model files, the venv and both Cargo target folders outside this synced folder.

```bash
Z=~/.cache/oversample-research/birdnet-v3/zenodo     # downloads (MD5 checked against the record)
C=~/.cache/oversample-research/birdnet-v3/conv       # converted copies
B=path/to/research/birdnet-v3-bench
X=path/to/batchi-extras
P=path/to/venv/bin/python
for f in BirdNET+_V3.0_Global_12K_Labels.csv BirdNET+_V3.0_Global_12K_FP16_pruned.onnx \
    BirdNET+_V3.0_Global_10K_BNL.zip BirdNET+_V3.0_Global_12K_FP16.onnx \
    BirdNET+_V3.0_Global_12K_FP32.onnx BirdNET+_Geomodel_V3.0.5_Global_14K.zip; do
  curl -sSL -o "$Z/$f" "https://zenodo.org/api/records/24362980/files/$f/content"
done
md5sum $Z/*      # compare with https://zenodo.org/api/records/24362980 ("checksum")

# 1. Windows: 3 s at 32 kHz, plus silence and noise; 3 minutes for timing
$P -I $B/prep.py --max-windows 6 --quiet --list birds.list birds.f32 \
  "$X/bat-demo-sounds/sounds/XC540767 - Gentoo Penguin - Pygoscelis papua.mp3" \
  "$X/no-bat-sounds/sounds/XC1083752 - Pauraque - Nyctidromus albicollis.wav" \
  "$X/no-bat-sounds/sounds/XC1086259 - Common Scoter - Melanitta nigra.wav" \
  "$X/no-bat-sounds/sounds/XC1140427 - Alpine Swift - Tachymarptis melba.wav"
$P -I $B/prep.py --repeat 60 long.f32 "$X/no-bat-sounds/sounds/XC1083752 - Pauraque - Nyctidromus albicollis.wav"
$P -I $B/../perch-bench/prep.py --rate 48000 --win 144000 --repeat 60 long48.f32 "<same Pauraque file>"
$P -I $B/../perch-bench/prep.py --repeat 36 long160.f32 "<same Pauraque file>"

# 2. What the files are made of; multiply-adds
$P -I $B/inventory.py $Z/*.onnx
$P -I $B/convert.py $Z/BirdNET+_V3.0_Global_12K_FP32.onnx $C/full.fp32.onnx
$P -I $B/convert.py $C/full.fp32.onnx $C/full.fp32.sim.onnx --shape
$P -I $B/macs.py $C/full.fp32.sim.onnx

# 3. onnxruntime references (top 5 per window, embeddings and scores saved)
$P -I $B/ref.py $Z/BirdNET+_V3.0_Global_12K_FP32.onnx birds.f32 --save out/birds.fp32 \
  --labels $Z/BirdNET+_V3.0_Global_12K_Labels.csv --list birds.list
#    and the same for the FP16, pruned and BirdNET Live files (--save out/birds.fp16 and so on)

# 4. tract native (the official files, patched in memory)
cargo build --release          # with CARGO_TARGET_DIR outside this folder
$CARGO_TARGET_DIR/release/native $Z/BirdNET+_V3.0_Global_12K_FP16_pruned.onnx birds.f32 out/birds.tract-mem-pruned

# 5. Compare (the last 3 windows are silence and noise)
$P -I $B/compare.py --quiet-last 3 $Z/BirdNET+_V3.0_Global_12K_Labels.csv out/birds.fp32 \
  out/birds.fp16 out/birds.pruned out/birds.bnl:path/to/BirdNET+_V3.0_Global_10K_Labels.csv

# 6. tract wasm32-unknown-unknown + wasm-bindgen in node
CARGO_PROFILE_RELEASE_DEBUG=false CARGO_PROFILE_RELEASE_STRIP=true RUSTFLAGS="-C target-feature=+simd128" \
  wasm-pack build --release --target nodejs --out-dir $PKG --no-typescript
node $B/bench-bindgen.mjs $PKG $Z/BirdNET+_V3.0_Global_12K_FP16_pruned.onnx birds.f32 out/birds.wasm-pruned

# 7. Timing rounds, taking turns with BirdNET V2.4 and Perch's embedding model
PERCH=path/to/perch_v2_embed.tract.onnx bash $B/bench.sh $CARGO_TARGET_DIR/release/native $PKG \
  path/to/birdnet-v2.4-embeddings-fp32.onnx 2 $Z/BirdNET+_V3.0_Global_12K_FP16_pruned.onnx \
  $Z/BirdNET+_V3.0_Global_12K_FP16.onnx $C/backbone.f16.onnx
#    backbone cut: convert.py PRUNED $C/backbone.f16.onnx --weights asis --outputs embeddings

# 8. Labels: trimmed file and the Geomodel joins
$P -I $B/labels.py $Z/BirdNET+_V3.0_Global_12K_Labels.csv out/birdnet-v3-labels.tsv \
  --geo305 <zip>/BirdNET+_Geomodel_V3.0.5_Global_14K_Labels.csv \
  --geo304 path/to/birdnet-geomodel-v3.0.4-labels.txt --bnl path/to/BirdNET+_V3.0_Global_10K_Labels.csv
$CARGO_TARGET_DIR/release/native geo <zip>/BirdNET+_Geomodel_V3.0.5_Global_14K_FP32.onnx -34.9 138.6 20 out/geo.f32
```

## Not done

- No browser run (Chrome, Firefox, WebKitGTK) and no phone. Browser times above are
  estimates from node.
- The timing ran on a busy machine; the quiet-machine numbers are scaled estimates.
- Option (d) was not built. Its head times are estimates from the Perch classifier's
  speed.
- No profile of why V3.0 is 3 times slower in WASM than natively.
- Only 18 recording windows, all birds with clear calls. No frogs, insects, bats or
  soundscapes.
