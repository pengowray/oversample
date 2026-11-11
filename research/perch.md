# Perch 2.0 on the device: evaluation

Status: research, 2026-11-11. Nothing here is wired into the app. Companion to
[birdnet-on-device.md](birdnet-on-device.md), which answers the same questions for
BirdNET V2.4. Prototype: [perch-bench/](perch-bench/).

## Recommendation

**Add Perch 2.0 as a second model, not as a replacement for BirdNET.** Ship it as the
45 MB embedding model and use it for similar-call search, clustering and non-bird
species. Keep BirdNET V2.4 as the bird and bat classifier for now.

- **Licence:** Apache 2.0 for the weights, stated by Google on the Kaggle model page.
  This is much more permissive than BirdNET's CC BY-NC-SA 4.0. It allows a paid tier,
  changed files (quantised, cut, sliced) and bundling with the app. Apache 2.0 is
  compatible with GPL-3.0.
- **It runs in tract, with one fix.** tract 0.23 needs a small rewrite of the ONNX file
  (see [Running it in tract](#3-running-it-in-tract)). After that, tract's output
  matches Google's own TensorFlow SavedModel: the same top species on 22 of 22 windows,
  logits within 0.00013, embeddings with cosine similarity 0.9999999.
- **Size is the problem with the full model.** The ONNX file is 409 MB. 364 MB of that
  is the classifier for 14,795 classes; the embedding part is 45 MB. In the app's
  WASM target, the full model needs 1.2 GB of WASM memory and runs at 7 times real
  time. The embedding part alone needs 140 MB and runs at 23 to 24 times real time.
- **Slower than BirdNET.** In the same process, under the same load, the Perch
  embedding model analysed 3 minutes of audio in 7.5 to 7.9 s; BirdNET V2.4 took 4.1 to
  4.8 s. Perch is about 1.8 times slower per second of audio.
- **Bats: no better than BirdNET.** Google removed bats from Perch's training data, and
  only 2 of its 14,795 classes are bats (Eptesicus serotinus and E. nilssonii). On 184
  time-expanded bat recordings of 92 species, Perch and BirdNET embeddings found a
  recording of the same species as the nearest neighbour equally often (0.53 to 0.57).
  Both beat a plain spectrum baseline (0.33). See [Bats](#4-bats).
- **Species map: yes.** Perch's classifier stores 4 prototype vectors per class. Their
  weighted mean gives one vector per class, and those vectors group by genus as well
  as BirdNET's classifier weights do (0.59 against 0.60, on the 6,262 species both
  models know). A UMAP map of all 14,742 live classes is a 916 kB JSON file. See
  [Species map](#5-species-map).

Why not replace BirdNET:

- BirdNET is 1.8 times faster and its full classifier is 52 MB, against Perch's 409 MB.
- BattyBirdNET's bat heads only work on BirdNET embeddings.
- Perch's scores come from a softmax-trained head. They are uncalibrated, and species
  in the same window compete with each other. Google recommends tuning a threshold per
  species with your own data.
- Perch has no location model. BirdNET's location model can limit the species list to
  the place and week of the recording. (A community version of Perch, tphakala's
  regional slices, uses BirdNET's location model offline to pick each region's
  species.)

Why add Perch:

- **Licence.** It is the only one of the two that could go in a paid tier, and the app
  can host, cut and quantise the file without ShareAlike or NonCommercial terms.
- **Embeddings.** Google trained them to be linearly separable, for agile modelling
  (a few labelled examples, then a linear classifier) and similarity search. Perch 2.0
  is state of the art on the BirdSet and BEANS benchmarks. That suits the similar-call
  search and clustering planned for the Call detection panel. A custom bat head on
  Perch embeddings would be shippable in any tier, unlike BattyBirdNET (CC BY-NC-SA).
- **Taxa.** Perch knows frogs, crickets, grasshoppers, some mammals (including
  whales) and 198 general sound events (vehicles, water, music). BirdNET V2.4 is
  mostly birds.

How to ship it, if added:

1. The 45 MB embedding model (40 MB gzipped), cached the same way as BirdNET's model.
   It gives the 1,536-value embedding for search and clustering.
2. The classifier as a separate data file, run in plain Rust after the embedding
   model: one matrix multiply against the prototypes of the species that are wanted.
   For a region of about 900 species that is about 20 MB of prototypes in float32.
   The full 14,795-class head (364 MB float32, 182 MB float16) is too big to download.
3. The species map JSON, built offline by [species_map.py](perch-bench/species_map.py).

Not tested: real browsers and the Tauri webview (only node), a Web Worker, phones,
onnxruntime-web, the INT8 file, a float16 head, Perch against BirdNET on bird
identification accuracy, and a classifier trained on Perch embeddings for bats. See
[Not tested](#not-tested).

## 1. Model files

Perch 2.0 is Google DeepMind's bioacoustics model, released in August 2025 (paper:
van Merriënboer et al., "Perch 2.0: The Bittern Lesson for Bioacoustics", arXiv
2508.04665). No newer version was found. The Kaggle variation `perch_v2` is at version
2 (version 2 only fixes a directory problem).

| Model | Format | Size | Source | Licence |
|---|---|---|---|---|
| Perch 2.0, `perch_v2` (needs a GPU) | TF SavedModel | 407 MB | Kaggle `google/bird-vocalization-classifier`, also mirrored at `cgeorgiaw/Perch` on Hugging Face | Apache 2.0 |
| Perch 2.0, `perch_v2_cpu` | TF SavedModel | 392 MB | Kaggle, same page | Apache 2.0 |
| Perch 2.0 | TFLite FP32 | 407 MB | `justinchuby/Perch-onnx` (conversion) | Apache 2.0 |
| Perch 2.0 | ONNX FP32, with a `DFT` operator | 409 MB | `justinchuby/Perch-onnx`; same file in `tphakala/Perch-v2-Models` | Apache 2.0 |
| Perch 2.0 | ONNX FP32, DFT replaced by MatMul | 413 MB | `justinchuby/Perch-onnx` | Apache 2.0 |
| Perch 2.0 | ONNX, INT8 MatMuls | 131 MB | `tphakala/Perch-v2-Models` | Apache 2.0 |
| Perch 2.0, regional slices (43 regions, 272 to 3,535 classes each) | ONNX FP32 / INT8 | 56 to 136 MB / 42 to 62 MB | `tphakala/Perch-v2-Models` | Apache 2.0 |
| Perch 2.0, embedding part only (cut here) | ONNX FP32 | 45 MB | made by [split.py](perch-bench/split.py) | Apache 2.0 |

- **Input:** 160,000 samples, which is 5 s of mono audio at 32 kHz. The model computes
  its own log-mel spectrogram (500 frames by 128 mel bands, 60 Hz to 16 kHz), so the
  app only resamples and cuts windows. BirdNET takes 3 s at 48 kHz.
- **Outputs:** a 1,536-value embedding (the mean over space), a spatial embedding of
  16 x 4 x 1,536 values, the spectrogram, and 14,795 logits.
- **Network:** an EfficientNet-B3 with about 12 million parameters, then a classifier
  with 91 million parameters. In the ONNX file the classifier is one
  1,536 x 59,180 matrix: 4 unit-length prototypes per class (see section 5).
- **Classes:** 14,795. The paper says 14,597 are species and 198 are FSD50K sound
  events. 9,706 classes have an eBird code (birds). The rest are frogs and other
  amphibians, insects, mammals and sound events. Training audio came from xeno-canto,
  iNaturalist, the Animal Sound Archive (Tierstimmenarchiv) and FSD50K.
- **Bats:** the paper says bats were removed from the training data because their
  calls "cannot be represented" with the chosen spectrogram settings. Two bat species
  remain as classes: Eptesicus serotinus and Eptesicus nilssonii.
- **Dead classes:** 53 classes have all four prototype weights at zero, so their
  logit is a constant. Examples: Agriotes lineatus, Bos bison, Ceyx gentianus.

## 2. Licence

- **Weights:** the Kaggle model card says "Copyright 2025 Google, LLC. Licensed under
  the Apache License, Version 2.0", and the `perch_v2` variation lists its licence as
  Apache 2.0. The labels files ship with the model.
- **Code:** `google-research/perch` and `perch-hoplite` are Apache 2.0.
- **Commercial use:** Apache 2.0 allows it. When asked in a GitHub issue about using the
  weights commercially in a mobile app, the Perch team replied that they can't give
  legal advice and pointed to the Apache 2.0 text.
- **What the app must do:** keep the licence text and copyright notice with the model
  file, and say if the file was changed (for example "embedding part cut out, Conv
  layers folded for tract"). There is no ShareAlike or NonCommercial term.
- **Compared with BirdNET:** BirdNET V2.4's weights are CC BY-NC-SA 4.0 at best. Perch
  could go in a paid tier and could be bundled with the app; BirdNET can't.
- **Credit:** cite van Merriënboer et al. (2025), arXiv 2508.04665, and link the Kaggle
  page. Credit justinchuby for the ONNX conversion if that file is used.

This is a reading of the licences, not legal advice.

## 3. Running it in tract

### The tract fix

tract 0.23 can't load justinchuby's ONNX file with a batch size of 1:

```
in output_facts invocation for node_Conv_8: Conv
Bias should be scalar or a vector with one value per output channel.
Output channels is 40, bias is 40,1,F32
```

With batch 2 the same file loads and runs. With a batch of 1, tract's optimiser drops
the batch axis from each convolution, then folds the batch-norm scale and shift that
follow it into the convolution's bias with the wrong shape. This looks like a tract
bug; it was not reported upstream.

[fix_for_tract.py](perch-bench/fix_for_tract.py) does that fold in the ONNX file
before tract sees it: each Conv followed by Transpose, Mul and Add gets new weights and
bias, and the Mul and Add are removed (78 convolutions, 156 nodes). The result is the
same function. tract then loads it with batch 1. tract's `DFT` operator works, so the
no-DFT file isn't needed.

### Speed

Machine: Ryzen 5 3600 (6 cores, 2019), Linux, node 24 (V8, the engine in Chrome),
one thread. Other sessions were building code on the same machine, so the
wasm-bindgen rows ran in one node process that took turns between Perch and BirdNET,
3 rounds each, on the same 3 minutes of audio (Common Pauraque, repeated). BirdNET's
times in that process (68 to 73 ms per window) match the earlier quiet-machine
measurement (64 to 74 ms), so the comparison is fair.

| Runtime | Model | ms per window | 3 min of audio | Real time | Memory |
|---|---|---|---|---|---|
| tract, `wasm32-unknown-unknown` + wasm-bindgen + SIMD, opt-level 3 | Perch, full (14,795 classes) | 686 to 703 per 5 s | 24.8 to 25.8 s | 7x | 1,231 MB WASM memory (1.8 GB node RSS) |
| same | Perch, embedding only | 206 to 219 per 5 s | 7.5 to 7.9 s | 23 to 24x | 140 MB WASM memory |
| same | Perch, embedding only, no DFT | 228 per 5 s | 8.3 s | 22x | 152 MB WASM memory |
| same | Perch, central Europe slice (873 classes) | 260 to 262 per 5 s | 9.4 s | 19x | 217 MB WASM memory |
| same | BirdNET V2.4 (for comparison) | 68 to 73 per 3 s | 4.1 to 4.8 s | 38 to 44x | 160 MB WASM memory |
| tract, native | Perch, full | 281 per 5 s | 10.3 s | 18x | 1,223 MB RSS |
| tract, native | Perch, embedding only | 118 per 5 s | 4.3 s | 42x | 186 MB RSS |
| onnxruntime 1.31, Python, under load | Perch, full | 194 per 5 s | about 7 s | 26x | |
| onnxruntime 1.31, Python, under load | Perch, embedding only | 122 per 5 s | about 4.4 s | 41x | |
| TensorFlow 2.20, Google's `perch_v2_cpu` SavedModel | Perch, full | 1,348 per 5 s | | 4x | |
| LiteRT (TFLite), justinchuby's file | Perch, full | 2,071 per 5 s | | 2x | 2.1 GB RSS |

- The memory column isn't comparable across runtimes; see the BirdNET page for why.
  The WASM memory figures come from the wasm32-wasip1 build of the same code, which
  reports its linear memory directly.
- The tract WASM module is 14.9 MB (3.7 MB gzipped), the same as BirdNET's, because it
  is the same tract code.
- Loading (parse and optimise) takes 1.7 to 2.0 s in WASM for the embedding model and
  2.4 s for the full model, against 0.5 to 0.6 s for BirdNET.
- The classifier is a large part of the time: 14,795 classes x 4 prototypes x 64
  positions x 1,536 values is 5.8 billion multiply-adds per window. A region of about
  900 classes adds 20 to 25 percent to the embedding time.
- The plain wasm32-wasip1 build ran while the machine was busier, so its times (310 to
  410 ms for the embedding model, 1 s for the full model) are left out of the table.

### Accuracy

Test audio: 6 bird recordings from `batchi-extras/no-bat-sounds` (Common Swift, Common
Pauraque, Common Scoter, Alpine Swift, Cave Swiftlet, Asian Palm Swift), resampled to
32 kHz and cut into 22 windows of 5 s. Reference: Google's `perch_v2_cpu` SavedModel
in TensorFlow 2.20 ([ref_tf.py](perch-bench/ref_tf.py)).

| Run | Same top-1 | Largest logit difference | Lowest embedding cosine |
|---|---|---|---|
| tract native, full | 22 of 22 | 0.00013 | 0.9999999 |
| tract wasm-bindgen, full | 22 of 22 | 0.00011 | 0.9999999 |
| tract wasm32-wasip1, full | 22 of 22 | 0.00011 | 0.9999999 |
| tract native and wasm, embedding only (with or without DFT) | | | 0.9999999 |
| tract native and wasm, central Europe slice (its 873 classes) | 22 of 22 | 0.00094 | 0.9999999 |
| embedding model + classifier in numpy ([ref.py](perch-bench/ref.py)) | 22 of 22 | 0.00013 | 0.9999999 |
| onnxruntime, full | 22 of 22 | 0.00013 | 0.9999999 |
| LiteRT, justinchuby's TFLite file | 22 of 22 | 0.00133 | 0.9999999 |

Top-1 results: Common Swift (one window as Pallid Swift), Common Pauraque, Common
Scoter, Alpine Swift and Asian Palm Swift were found, with softmax probabilities of
0.3 to 0.9. The 3 Cave Swiftlet windows came out as other species (Green Kingfisher,
Great Spotted Woodpecker, California Quail) with probabilities under 0.1.

## 4. Bats

Method ([bats.py](perch-bench/bats.py)):

- **Recordings:** every bat recording in `batchi-extras/bat-demo-sounds` that is WAV,
  recorded at 192 kHz or more, of a species with at least 2 such files: 184 recordings,
  92 species, 51 genera. Every species has exactly 2 recordings. None of the xeno-canto
  notes mention time expansion. For 59 species both recordings are by the same
  recordist, and for 33 species by two different recordists.
- **Time expansion:** each recording is resampled to a higher rate and then read as the
  model's own rate. Perch was run at 8x (256 kHz read as 32 kHz: 0.625 s of real time
  per window, calls up to 128 kHz in range) and 10x (320 kHz: 0.5 s, up to 160 kHz).
  BirdNET was run at 5.33x (256 kHz read as 48 kHz, as BattyBirdNET does: 0.56 s, up to
  80 kHz in BirdNET's 15 kHz range) and 10x (480 kHz: 0.3 s, up to 150 kHz).
- **Windows:** most windows of a bat recording hold no call, so each recording's
  embedding is the mean of its 3 windows with the most energy between 15 and 120 kHz.
- **Scores:** cosine distance after subtracting the mean embedding. For each recording,
  is the nearest other recording the same species (chance: 0.005)? The same genus? And
  the silhouette score: how tight each species is compared with its nearest other
  species, from -1 to 1.
- **Baseline:** the mean log power spectrum of the same windows, in 1 kHz bands from 10
  to 128 kHz.

| Embedding | Nearest is same species | Same genus | Species, same recordist | Species, two recordists | Silhouette, species | Silhouette, genus |
|---|---|---|---|---|---|---|
| Perch, 8x | 0.533 | 0.630 | 0.729 | 0.182 | +0.207 | +0.057 |
| Perch, 10x | 0.565 | 0.658 | 0.788 | 0.167 | +0.217 | +0.062 |
| BirdNET, 5.33x (BattyBirdNET's setting) | 0.549 | 0.652 | 0.771 | 0.152 | +0.157 | +0.000 |
| BirdNET, 10x | 0.543 | 0.647 | 0.771 | 0.136 | +0.189 | +0.038 |
| Mean log spectrum (baseline) | 0.332 | 0.413 | 0.508 | 0.015 | -0.121 | -0.393 |

Paired counts, for the species column: Perch at 10x got 15 recordings right that
BirdNET at 5.33x got wrong, and BirdNET got 12 right that Perch got wrong. A McNemar
test on 15 against 12 gives p of about 0.7, so the difference could be chance. Against
the spectrum baseline, Perch at 10x got 51 right that the baseline missed and missed 8
that the baseline got.

What this shows:

- **No clear winner.** Perch at 10x finds the same species slightly more often than
  BirdNET, and its species clusters are a little tighter (silhouette 0.22 against 0.16
  to 0.19). The differences are a few recordings out of 184.
- **Recordist matters more than the model.** When both recordings of a species are by
  the same person, both models find the match 73 to 79 percent of the time. When they
  are by different people, only 14 to 18 percent. Much of what both models match is
  the recorder, microphone and site, not the bat. A similarity search across the
  user's own recordings will work better than one across other people's.
- **Both models beat the plain spectrum,** so both embeddings carry more than the call's
  frequency band.
- **10x is a little better than 8x for Perch.** Calls above 128 kHz (some horseshoe
  bats) are cut off at 8x.

## 5. Species map

Yes, the map can be built from Perch's classifier, the same way as the BirdNET map,
with one change: Perch has no single weight vector per class.

- **What the classifier stores:** for each class, 4 unit-length prototype vectors in
  the 1,536-value spatial embedding space, a non-negative weight for each prototype,
  and a bias. A window's logit for a class is the bias plus the weighted sum, over the
  4 prototypes, of the prototype's best cosine match anywhere in the window's 16 x 4
  spatial grid. [split.py](perch-bench/split.py) extracts the prototypes; ref.py
  rebuilds the logits from them in numpy and matches the model to 0.00013.
- **One point per class:** a class's 4 prototypes point in nearly unrelated directions
  (median cosine 0.06 between prototypes of the same class). Three ways to get one
  vector per class were compared. The score is how often a species' nearest other
  class is in the same genus, counting only species whose genus has at least one
  other species in the model:

  | Class vector | All Perch species (12,829 counted; random 0.001) | Species BirdNET also has (5,477 counted; random 0.002) |
  |---|---|---|
  | Weighted mean of the 4 prototypes | 0.439 | 0.586 |
  | Highest-weight prototype only | 0.248 | 0.337 |
  | Closest pair of prototypes between two classes | 0.312 | 0.415 |
  | BirdNET V2.4 classifier weights, for comparison | 0.597 (all 5,713 counted) | 0.601 |

  On the species both models know, the weighted mean groups relatives as well as
  BirdNET's weights do. Over all of Perch's species the score is lower (0.44); the
  extra species are mostly non-birds, and this check didn't look into why.
- **The map:** UMAP (cosine) on the weighted means, without the 53 dead classes, gives
  a JSON file of 14,742 points, 916 kB (index, name, x, y, coordinates rounded to 4
  decimals). Birds, other species and sound events form separate regions; the two bat
  classes sit at the edge of the non-bird species region.

  ![Perch species map](perch-bench/perch_v2_species_map.png)

  On the 2D map, the nearest class shares the genus for 0.108 of species, against 0.215
  for a BirdNET map made the same way. Squeezing 14,742 classes into 2D loses more
  detail than squeezing 6,522 classes. Zooming will still show genus groups, but
  neighbours on the map are not always relatives.
- **Placing a detection:** use the window's logits (its top classes), not the nearest
  prototype to the pooled 1,536-value embedding. The nearest prototype to the pooled
  embedding matched the top class in only 6 of 22 bird windows, because the classifier
  compares prototypes with each spatial position, not with the average. A detection
  can sit at its top class's point, or at a logit-weighted average of its top few.
- **A bat map** for Perch would have only the 2 Eptesicus classes, so bats would need
  their own map, built from a bat classifier trained on Perch or BirdNET embeddings.

## Not tested

- **Browsers and Tauri.** Perch ran in node only, not in Chrome, Firefox, a Web Worker
  or the Tauri webview. The BirdNET tests showed node's numbers carry over to Chrome
  and to the worker, so the same is likely here.
- **Phones.** No Android device was connected.
- **onnxruntime-web,** WebGPU and more than one thread.
- **INT8 and float16.** The tphakala INT8 file (131 MB) uses MatMulInteger, which was 4
  times slower than float32 in tract for BirdNET. A float16 head was not tried.
- **Bird identification quality.** Perch against BirdNET on the same bird recordings
  was not compared; the 22 test windows only checked that tract matches TensorFlow.
- **A trained bat classifier.** The bat test only used nearest neighbours on raw
  embeddings. A linear classifier on each model's embeddings, trained on more
  recordings per species, is the fairer test of which embedding suits bats.
- **Other time-expansion factors** than 8x and 10x for Perch.
- **The tract bug** was worked around, not reported upstream.

## Sources

- Perch 2.0 on Kaggle, model card and licence:
  kaggle.com/models/google/bird-vocalization-classifier (variations `perch_v2` and
  `perch_v2_cpu`)
- Paper: arxiv.org/abs/2508.04665 ("Perch 2.0: The Bittern Lesson for Bioacoustics")
- Code: github.com/google-research/perch, github.com/google-research/perch-hoplite
- Licence question: github.com/google-research/perch/issues/799
- ONNX and TFLite conversions: huggingface.co/justinchuby/Perch-onnx
- INT8 and regional slices: huggingface.co/tphakala/Perch-v2-Models
- SavedModel mirror and labels: huggingface.co/cgeorgiaw/Perch
- tract: github.com/sonos/tract (`tract-onnx` 0.23.10)
