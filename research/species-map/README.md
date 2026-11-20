# Species map

A 2-D map of BirdNET V2.4's 6,522 classes, made from the model's final dense
layer (one 1,024-weight row per class) with t-SNE on cosine distance.
`project.py` makes it; its docstring has the command.

Check on 2026-11-11 (`check.txt`):

| Neighbours that share the genus | Share |
|---|---|
| 5 nearest on the map | 0.332 |
| 5 nearest in the 1,024-d weights | 0.317 |
| Random pairs | 0.0016 |

So the map keeps the model's own grouping: a class's map neighbours share its
genus about 200 times more often than random pairs. UMAP (0.19) and PCA
(0.013) kept less.

## Perch 2.0

`project_perch.py` makes `perch-map.bin`, a map of Perch 2.0's 14,795 classes,
and `perch-check.txt` compares the projections. Its docstring has the command.

**Which weights.** The released Perch 2.0 model has only one classifier: a
prototype head with 4 unit-length prototypes per class, a weight per prototype
and a bias (tensors `P`, `W` and `b` in `perch_v2_head.npz`, cut from
`perch_v2.onnx` by `research/perch-bench/split.py`). The paper's linear head
and its genus, family and order outputs are not in the release. So each
class's vector is the `W`-weighted mean of its 4 prototypes, L2-normalised,
compared by cosine. `research/perch.md` section 5 found this groups relatives
better than a single prototype or the closest pair of prototypes. The 53
classes whose prototype weights are all 0 never score above their bias; they
are left off the map.

**Check on 2026-11-16.** Share of each class's 5 nearest neighbours in the same
genus (14,542 species), the same bird order (9,698 birds, orders from the eBird
taxonomy) and the same group (bird, other species or sound event):

| Method | Genus | Bird order | Group |
|---|---|---|---|
| Random pairs | 0.0007 | 0.246 | 0.539 |
| Full 1,536-d vectors | 0.248 | 0.676 | 0.858 |
| PCA to 50-d | 0.188 | 0.655 | 0.849 |
| PCA to 2-d | 0.003 | 0.314 | 0.698 |
| **t-SNE, perplexity 30, full vectors** | **0.235** | **0.670** | **0.850** |
| t-SNE, perplexity 30, after PCA to 50-d | 0.182 | 0.646 | 0.845 |
| t-SNE, perplexity 50, after PCA to 50-d | 0.176 | 0.641 | 0.848 |
| t-SNE, perplexity 100, after PCA to 50-d | 0.166 | 0.637 | 0.847 |
| UMAP cosine, 15 neighbours, full vectors | 0.087 | 0.540 | 0.820 |
| UMAP cosine, 15 neighbours, after PCA to 50-d | 0.074 | 0.565 | 0.820 |
| UMAP cosine, 50 neighbours, full vectors | 0.057 | 0.525 | 0.818 |
| UMAP cosine, 50 neighbours, after PCA to 50-d | 0.056 | 0.550 | 0.821 |

The map uses t-SNE (cosine, perplexity 30, `init="pca"`) on the full vectors.
It keeps nearly all of the grouping the 1,536-d vectors have, on all three
levels. PCA to 50-d first loses a quarter of the genus share, a higher
perplexity loses more, and UMAP keeps the least. Birds, other species and
sound events sit in separate regions (`perch-map.png`).

Perch's genus share (0.25) is lower than BirdNET's (0.32) in the full vectors
too, so this is the model, not the map.

**Placement radius.** Classes sit closer together on the Perch map: the
median distance from a class to its 3rd nearest class is 0.0089 map units,
against 0.0149 on the BirdNET map. The placement radius is scaled the same
way, from 0.15 to 0.09 (`PERCH_RADIUS` in `src/classifier/species_map.rs`).

**Licence.** Perch 2.0 is Apache 2.0, so the map is too.

## BirdNET+ V3.0

`project_v3.py` makes `birdnet-v3-map.bin`, a map of all 11,725 of BirdNET+
V3.0's classes, `v3-check.txt` (the projections compared) and `v3-map.png`
(the map coloured by taxon group). `v3_place_check.py` places recordings'
windows on it. Commands below.

**Which weights.** The app runs the pruned file
(`BirdNET+_V3.0_Global_12K_FP16_pruned.onnx`). Of the full model's three
heads (GeM pooling, temporal attention, frequency attention) it keeps only
the GeM head, and that head is one linear layer on the 1,280-value
`embeddings` output: a Gemm with `head.weight_stored_fp16` (11,725 x 1,280,
float16, `transB`) and `head.bias`, then

    predictions = sigmoid(5 * (embeddings @ W.T + b) / 5)

(the 5 and the divisor `scalar_tensor_default` = 5 are left over from the
full model's sum of heads). The script recomputes this from the stored
weights and matches onnxruntime to 1.2e-7. So a class's vector is its row
of W, as for BirdNET V2.4, scaled to length 1 and compared by cosine. Every
row has a length between 9.2 and 39.4, so no class is left out. The
attention heads are not in the file the app runs, so they play no part in
what the app shows.

**Check on 2026-11-21** (`v3-check.txt`). Share of each class's 5 nearest
neighbours in the same genus (11,709 species) and the same taxon group
(birds, insects, amphibians, mammals, other animals, sounds):

| Method | Genus | Group |
|---|---|---|
| Random pairs | 0.0012 | 0.706 |
| Full 1,280-d vectors | 0.307 | 0.958 |
| PCA to 50-d | 0.193 | 0.951 |
| PCA to 2-d | 0.003 | 0.728 |
| **t-SNE, perplexity 30, full vectors** | **0.339** | **0.967** |
| UMAP (umap-js), 15 neighbours, after PCA to 50-d | 0.075 | 0.950 |

The map uses the same t-SNE settings as the V2.4 and Perch maps. It keeps
all of the genus grouping the 1,280-d rows have (more, like V2.4's), and
insects, amphibians, mammals and the 16 sound classes each sit in their own
region (`v3-map.png`). Random pairs share a group 71% of the time because
83% of the classes are birds.

**Placement radius.** The median distance from a class to its 3rd nearest
class is 0.0104 map units, against 0.0149 on the V2.4 map, so the radius
is 0.15 x 0.0104 / 0.0149 = 0.105 (`BIRDNET_V3_RADIUS` in
`src/classifier/species_map.rs`).

**Recordings on the map** (`v3_place_check.py`, windows of 3 s end to end
as Analyse makes them, placed by the app's rule at the default minimum
confidence of 0.25):

- Gentoo Penguin (XC540767, the demo file): 14 of 15 windows placed, every
  one with Gentoo Penguin first (0.44 to 0.95). On the t-SNE map they land
  among Gentoo, Adelie, Chinstrap and Western Rockhopper Penguins and the
  gannets; on the UMAP layout beside Gentoo Penguin, albatrosses and South
  Polar Skua. On the t-SNE map, 2nd and 3rd classes that were other
  penguins, gannets or kittiwakes were inside the radius and pulled the
  dot; unrelated ones (Chopi Blackbird, House Sparrow) were outside it and
  did not.
- Pauraque (XC1083752): all 8 windows, Pauraque first (0.92 to 0.98),
  land among Rufous, Little, White-tailed and Spot-tailed Nightjars on the
  t-SNE map, and beside Common Potoo and Silky-tailed Nightjar on UMAP.
  The flycatchers and kingbirds calling in the same windows (up to 0.90)
  are outside the radius and do not move the dot.
- Common Scoter (XC1086259): both windows, Common Scoter first (0.80 and
  0.93), land beside Common Scoter; on UMAP among geese, ducks, gulls and
  loons. On the t-SNE map its nearest classes are Buff-collared Nightjar
  and Common Poorwill, whose calls the model hears as close to the scoter's
  whistle.
- The embedding itself sits near the classes it sounds like: in 20 of the
  24 placed windows the top class's row is the 1st or 2nd nearest of all
  11,725 rows to the window's embedding by cosine (the others 5th to 25th).

**Licence.** BirdNET+ V3.0's weights are Apache 2.0, so the map is too.

**Commands** (run on 2026-11-21 with the Python venv of
`batchi-noisier2noise`, which has onnx, onnxruntime, scikit-learn,
matplotlib, scipy and soundfile; node runs umap-js):

```bash
M=~/.cache/oversample-research/birdnet-v3/zenodo/BirdNET+_V3.0_Global_12K_FP16_pruned.onnx
L=../birdnet-v3-bench/birdnet-v3-labels.tsv
X=path/to/batchi-extras
python -I project_v3.py $M $L .            # birdnet-v3-map.bin, v3-check.txt, v3-map.png; 3.5 min
# after ../live-embedding/v3_live_data.py has made birdnet-v3-umap.bin:
python -I v3_place_check.py $M $L birdnet-v3-map.bin ../live-embedding/birdnet-v3-umap.bin OUT_DIR \
    --radius 0.105 --umap-radius 0.07 \
    "$X/bat-demo-sounds/sounds/XC540767 - Gentoo Penguin - Pygoscelis papua.mp3" \
    "$X/no-bat-sounds/sounds/XC1083752 - Pauraque - Nyctidromus albicollis.wav" \
    "$X/no-bat-sounds/sounds/XC1086259 - Common Scoter - Melanitta nigra.wav" > v3-place-check.txt
```

## BattyBirdNET

The same method does not work for BattyBirdNET's regional classifiers. Their
rows give same-genus 2-NN shares of 0.07 (EU) and 0 to 0.17 (UK), and most
species' nearest rows are non-species classes such as "noise", "background"
and "audiomoth". Each head has 10 to 33 species and was trained to separate
them from noise, not to place them by similarity.

Proposed instead: the mean embedding of reference recordings per species (for
example Xeno-canto recordings run through the backbone at 256 kHz), with the
non-species classes left out.

## Output

`birdnet-map.bin`: 6,522 x 2 little-endian int16 (x then y per class, in
label order), 26 KB. `perch-map.bin`: the same for Perch's 14,795 classes,
59 KB, with (-32768, -32768) for a class with no position.
`birdnet-v3-map.bin`: the same for BirdNET+ V3.0's 11,725 classes, 47 KB.
The app ships all three (index.html copies them) and draws them in the Call
detection tab's Species map, for BirdNET V2.4, BirdNET+ V3.0 and Perch
results:
`src/components/file_sidebar/call_detection_panel/species_map.rs` and `species-map.js`, with
Apache ECharts 5.6.0 vendored in `vendor/`.

Each window at or above the minimum confidence is drawn at the
confidence-weighted mean of its top 3 classes' positions, counting only
classes within 0.15 map units of the top class (0.09 on the Perch map,
0.105 on the V3.0 map)
(`src/classifier/species_map.rs`), so an uncertain window sits between
related species and never in the gap between unrelated groups. Names come from the label file the app downloads with
the model. Each map is derived from its model's weights, so it carries the
model's licence: CC BY-NC-SA 4.0 for BirdNET V2.4, Apache 2.0 for BirdNET+
V3.0 and Perch.
