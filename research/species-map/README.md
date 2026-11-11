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
label order), 26 KB. The app ships this file (index.html copies it) and draws
it in the Call detection tab's Species map (BirdNET results only):
`src/components/file_sidebar/call_detection_panel/species_map.rs` and `species-map.js`, with
Apache ECharts 5.6.0 vendored in `vendor/`.

Each window at or above the minimum confidence is drawn at the
confidence-weighted mean of its top 3 classes' positions, counting only
classes within 0.15 map units of the top class
(`src/classifier/species_map.rs`), so an uncertain window sits between
related species and never in the gap between unrelated groups. Names come from the label file the app downloads with
the model. The map is derived from the model's weights, so it carries the
model's licence, CC BY-NC-SA 4.0.
