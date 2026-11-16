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
59 KB, with (-32768, -32768) for a class with no position. The app ships both
(index.html copies them) and draws them in the Call detection tab's Species
map, for BirdNET and Perch results:
`src/components/file_sidebar/call_detection_panel/species_map.rs` and `species-map.js`, with
Apache ECharts 5.6.0 vendored in `vendor/`.

Each window at or above the minimum confidence is drawn at the
confidence-weighted mean of its top 3 classes' positions, counting only
classes within 0.15 map units of the top class (0.09 on the Perch map)
(`src/classifier/species_map.rs`), so an uncertain window sits between
related species and never in the gap between unrelated groups. Names come from the label file the app downloads with
the model. Each map is derived from its model's weights, so it carries the
model's licence: CC BY-NC-SA 4.0 for BirdNET, Apache 2.0 for Perch.
