# Live view (experimental): BirdNET and Perch 2.0

A view in the Call detection tab (BirdNET or Perch 2.0 picked, "Live view"
switched on) that shows what the model hears in the last 3 s (BirdNET V2.4)
or 5 s (Perch 2.0) of audio, a few times a second, while a file plays or the
microphone listens. Most of this page describes BirdNET, the first model the
view ran; [Perch 2.0](#perch-20) lists what differs with Perch. Code:
`src/live_embedding.rs` (worker, requests, placement, strip rows, and the
`BIRDNET` and `PERCH` model settings), `live-embedding.js` (charts, Apache
ECharts), and `src/components/file_sidebar/call_detection_panel/live_view.rs`
(switch and status line). The worker message is `ToWorker::Live` in
`oversample-ipc/src/classifier.rs`.

## How it runs

- The view starts a second copy of the BirdNET worker with the backbone
  loaded, so it never waits for a classifier run, Perch or the denoiser, and
  they never wait for it. Turning the switch off, or leaving the tab, stops the
  worker (about 300 MB) and the request loop. Off costs nothing.
- Each request sends the last 3 s of mono audio at its own rate: the 3 s
  ending at the playhead during playback, or the tail of the live buffer while
  the microphone listens or records. The worker resamples to 48 kHz, pads short
  audio with zeros at the front, runs the backbone and answers with the
  1,024-value embedding and all 6,522 scores.
- One request is in flight at a time. The next goes out after the answer and
  at least 250 ms after the last one, so a slow device updates less often and
  nothing queues.
- While a file is open and nothing plays or listens, the view makes one
  reading of the 3 s that Play would start with (the paused reading, see 8).

## Measured

| What | Value |
|---|---|
| Backbone + resampling, native release build (192 kHz source) | 55 to 58 ms per reading |
| Same in the browser (WASM, desktop Chrome) | about 180 ms per reading |
| Interval between readings in the browser | 0.25 to 0.30 s |
| Live reading vs the classifier's window on the same 3 s (Pauraque, 4 windows) | same top class, same score to 3 decimals |

The parity check is `live_matches_the_run` in
`birdnet-worker/tests/real_models.rs`.

In the app, the live view and the classifier agreed on the test recordings:

| Recording | Classifier's best window | Live view |
|---|---|---|
| XC1099081 Common Swift (384 kHz) | Pallid Swift 0.92 | Pallid Swift 0.75 at the moment checked |
| XC1160798 Lesser Whistling Duck (192 kHz) | Watercock 0.85 | Watercock 0.58, Lesser Whistling Duck in the strip |
| XC1078076 Northern Screamer | | No species at 0.50 or above; Scrub Greenlet 0.25 highest |

## Perch 2.0

With Perch 2.0 picked as the method, the same view runs Perch (added
2026-11-20). Everything in the sections below works the same way, with these
values (`PERCH` and `BIRDNET` in `src/live_embedding.rs`):

| | BirdNET V2.4 | Perch 2.0 |
|---|---|---|
| Reading | last 3 s, resampled to 48 kHz | last 5 s, resampled to 32 kHz |
| Answer | 1,024-value embedding, 6,522 scores (sigmoid) | 1,536-value embedding, 14,795 softmax shares (the numbers a Perch run gives) |
| Minimum confidence | BirdNET's setting | Perch's own setting (default 0.30) |
| Shortest gap between requests | 250 ms | 500 ms |
| Request without an answer given up after (and longest gap counted in the interval) | 5 s | 15 s |
| Readings averaged in the component graphs | 4 (about 1 s) | 2 (about 1 s) |
| t-SNE map and placement radius | `birdnet-map.bin`, 0.15 | `perch-map.bin`, 0.09 |
| UMAP layout and placement radius | `birdnet-umap.bin`, 0.11 | `perch-umap.bin`, 0.06 |
| Component maps | `birdnet-pc-rows.bin` (12), or `birdnet-pc.bin` with `?pc=rec` | `perch-pc-rows.bin` (12); no recordings file, so `?pc=rec` does nothing |
| Score for a row in the strip | 0.1 | 0.1 |
| Note for bat recordings | "BirdNET hears up to 15 kHz, so bat calls do not show here." | "Perch 2.0 hears up to 16 kHz, so bat calls do not show here." |

- **One model at a time.** The worker holds the model picked in the tab while
  the live view is shown. Picking the other method stops the worker, clears
  the readings and the chart, and starts it again with the other model (the
  chart then gets that model's species). The worker loads Perch's two files
  (embedding model, then classifier), about 230 MB of WASM memory.
- **Live explorer** stays on BirdNET: on its own it runs BirdNET as before.
  While the live view in the Call detection tab runs Perch, the explorer keeps
  no readings and shows the spectrum only, with the status line "Only the
  spectrum updates while the live view in the Call detection tab uses Perch
  2.0."
- **Row threshold, 0.1.** Perch's scores are softmax shares, so classes in
  one window compete and a second class rarely gets far when the first is
  high. 0.1 is a third of Perch's default minimum (0.30). In
  `research/perch.md`, animals Perch named in recordings of rain had a median
  share of 0.02, traffic 0.07 and wind 0.20; correct top classes had medians
  of 0.72 (birds) and 0.47 (grasshoppers and crickets), at least 0.03. So 0.1
  leaves most rain and traffic guesses out of the strip and keeps most real
  calls; wind guesses get rows, as BirdNET's weak guesses do at the same
  value.
- **Parity:** `perch_live_matches_the_run` in
  `birdnet-worker/tests/real_models.rs` (`PERCH_MODELS_DIR`): on the Pauraque
  recording, a Live reading of a run's 5 s window has the same top class, the
  same share to within 0.001 (live 0.862, 0.805, 0.767 and 0.823; the run
  0.862, 0.805, 0.768 and 0.823), and
  embedding cosine 0.99999 or more. 267 to 319 ms per reading natively
  (release build, run beside the BirdNET test).
- **Time per reading in the browser** (release build, headless Chromium,
  Pauraque recording at 192 kHz playing, 2026-11-20): 540 to 690 ms per
  reading (resampling, embedding model and classifier), readings every 0.65
  to 0.75 s. `research/perch.md` measured 391 to 428 ms per window for the
  models alone in a quiet test page; here the app plays and draws at the same
  time. BirdNET in the same run: 190 ms, every 0.25 s, as before. The
  readings stay under a second, so the 500 ms gap rarely holds one back; the
  status line reports the measured interval as for BirdNET. Checked in the
  same run: download of both models, the paused reading ("The 5 s from where
  Play starts"), Perch's readings with 12 component maps on the UMAP layout,
  switching to BirdNET (its own species, layout and readings) and back, and
  that `perch-umap.bin` and `perch-pc-rows.bin` are served.

### Perch data files

Made by `perch_live_data.py` (output in `perch_live_data.txt`), the Perch
version of `species_umap.py` and `pc_bands.py`:

```
python -I perch_live_data.py perch_v2_head.npz labels.csv perch_v2_ebird_classes.csv \
    birdnet_labels.txt ../species-map/perch-map.bin . \
    HSN_hp/emb_raw.npz POW/emb_raw.npz --spatial HSN_hp/spatial_raw.npy
```

- **Class vectors:** the weighted mean of each class's 4 prototypes, scaled
  to length 1, as `research/species-map/project_perch.py` uses for
  `perch-map.bin`. The 53 classes whose prototype weights are all 0 are left
  out (no position; middle rank on every component).
- **UMAP layout** (`perch-umap.bin`, same format as `perch-map.bin`): umap-learn
  is not installed on this machine, so the script runs the app's vendored
  umap-js (`umap_js.cjs`, seeded, so the file is the same each run) on the
  class vectors after PCA to 50 dimensions, scaled back to length 1 so
  euclidean distance there follows cosine distance. 15 neighbours, minimum
  distance 0.1: 20 s. 50 neighbours: 42 s, slightly less compact. Classes sit
  closer on it than on the t-SNE map (10th nearest class at 0.0134 map units
  against 0.0201), so the placement radius is 0.09 x 0.67 = 0.06.
- **Components** (`perch-pc-rows.bin`, the OPC1 layout of `pc_bands.py` with
  1,536 dimensions and 14,795 classes): PCA of the class vectors. Shares of
  variance, PC1 to PC12: 2.32, 1.23, 1.05, 0.94, 0.93, 0.83, 0.77, 0.73,
  0.66, 0.64, 0.62 and 0.62% (the same PCA with each column shuffled: 0.21%
  for the first). Kept by the rule BirdNET's file used: while the share of a
  class's 10 nearest neighbours on the UMAP layout in the same half stays at
  0.65 or more. PC1 to PC12: 0.67, 0.70, 0.76, 0.72, 0.73, 0.73, 0.70, 0.70,
  0.69, 0.68, 0.66, 0.67; PC13 0.64 (t-SNE 0.65 to 0.75 for PC1 to PC12).
- **Percentiles of a reading:** BirdSet's HSN and POW test sets already had
  Perch embeddings (`emb_raw.npz`, key `perch`, from
  `batchi-noisier2noise/birdset_embed.py`), so the reference quantiles come
  from 16,560 real recordings' windows, as for BirdNET. As with BirdNET, the
  readings sit to one side of the class vectors on some axes: the share of
  readings outside the classes' 5 to 95% range is 0.38 on PC2, 0.26 on PC8
  and 0.17 on PC7 (0.1 if spread alike).
- **Check** (1,500 HSN windows, full logits from the stored spatial
  embeddings; the 208 with a class at 0.3 or more): reading percentile
  against its top class's percentile, mean gap / shuffled gap / correlation.
  PC1 0.458 / 0.451 / -0.03, PC2 0.245 / 0.253 / 0.14, PC3 0.178 / 0.204 /
  0.18, PC4 0.287 / 0.290 / 0.13, PC5 0.212 / 0.269 / 0.32, PC6 0.248 /
  0.273 / 0.53, PC7 0.253 / 0.291 / 0.30, PC8 0.195 / 0.261 / 0.45, PC9
  0.284 / 0.285 / 0.01, PC10 0.344 / 0.360 / 0.18, PC11 0.230 / 0.286 /
  0.49, PC12 0.162 / 0.184 / 0.26. Like BirdNET's class-row components, a
  reading's percentile follows its top class's only loosely; on PC1 and PC9
  not at all. HSN is one North American set, all birds.
- **Component ends** (the tooltip's "Low end" and "High end"), from the 12
  classes at each end and the share of classes with an eBird code (birds) in
  each outer tenth: PC1 mostly frogs, insects and mammals / mostly birds;
  PC2 tropical birds and frogs / European bush-crickets; PC3 Amazonian frogs
  and forest birds / Andean hummingbirds and tanagers; PC4 drongos, cats and
  deer / tropical American hummingbirds and forest birds; PC5 cicadas and
  katydids / large mammals (horses, camels, deer, pigs); PC6 drongos and
  other Asian forest birds / cicadas; PC7 wheatears, larks and shrikes /
  cicadas and katydids; PC8 tropical American forest birds / larks and other
  open-country birds; PC9 cicadas / frogs; PC10 a mix (seedeaters, antwren,
  manakin, flamingo) / owls, frogmouths and other night birds; PC11 a mix
  (frogs, owl, potoo, seedeaters) / crickets; PC12 cicadas and snakes / a mix
  (hornbills, oropendola, woodpeckers). Where Perch's labels have no English
  name the script prints the scientific name only, so the insect, frog and
  mammal groups are named from their genera.

## What the view shows, and why

1. **The top species and its score, as text**, at the top. In a 220 px
   sidebar this is the part that reads at a glance. Below the minimum
   confidence (the same setting as the results) it says "No species at 0.50 or
   above" and lists the highest scores in grey, so a weak match never looks
   like a hit. A bird that is unlikely at the place and month gets the
   app's flag after its name (see 9).
2. **A strip of scores over the last 20 s**, one row per class, coloured by
   score, newest on the right. A class gets a row when it scores 0.1 or more
   anywhere in the 20 s and keeps its place while it is in that window; with
   more than 8, the ones with the lowest peak drop out. Rows never reorder,
   so the strip scrolls like a spectrogram. Cells at or above the minimum
   confidence step up in opacity. BirdNET's non-bird classes (human voice,
   engine, dog, and so on) appear in the same rows when they score; a fixed set
   of non-bird rows was dropped because most of the time they are empty rows
   that push the species rows down.
3. **"Change in latent space"**: the cosine distance between each reading's embedding
   and the one before. A steady sound stays under 0.1; a new call or a change
   of scene reaches 0.2 to 0.5. It shows change that the scores miss, for
   example a new sound that BirdNET has no class for.
4. **The species map with a marker**: the class rows on the UMAP layout (or
   the t-SNE map of `research/species-map`, see 5), with the current reading placed by the same
   rule as the map's windows (score-weighted mean of the top 3 classes within
   0.15 map units of the top one) and a fading 15 s trail. A hit is a filled
   green dot sized by the score; below the minimum it is a large dashed ring.
   The trail line breaks on jumps longer than 0.15 map units, so it never
   draws across the gap between unrelated groups. At sidebar size the map
   shows which part of the bird world the sound is in and whether it is
   moving; the name comes from the text above it.
   Pointing anywhere on the map (hover with a mouse, or tap) lists the 5
   species nearest that point, nearest first, with the nearest ringed; a
   click or tap keeps the list (see 7). The 3
   species nearest the reading are named beside the marker, and once
   zoomed in to half the map's width or less, up to 12 species nearest the
   middle of the view are named too, without overlapping. The trail's
   "Reading N s ago" tooltip was dropped: the readout above already names
   the reading's species. Names and lookups are a canvas and DOM layer over
   the chart, about 0.5 to 1 ms per reading.

5. **Component maps (prototype)**: small copies of the species map, one per
   principal component, under the main map: 12 for the class-row components
   (the default), 8 for `?pc=rec`. The count comes from the file's header.
   The sidebar has 4 to a row; wider places (full screen) have more, as many
   as fit at 70 px or wider, with the rows filled evenly (12 maps: 3 rows of
   4 in the sidebar, 2 rows of 6 in the 440 px full-screen column).
   - Layout: the main map and the small maps share one layout, UMAP of the
     class rows (`species_umap.py`, `birdnet-umap.bin`) by default or the
     t-SNE of `research/species-map`, chosen with the Layout buttons above
     the map and saved on the device (`?pcmap=tsne` or `?pcmap=umap` sets it
     for one visit). UMAP of 6,522 class rows (cosine) takes 12 s with 50
     neighbours and 34 to 41 s with 15, so it is precomputed. The marker is
     placed on each layout by the same rule; on UMAP the radius is 0.11 map
     units instead of 0.15, because species sit closer there (10th nearest
     neighbour at 0.023 against 0.031). The UMAP file is centred and scaled
     so its longer side fills the map.
   - Compactness (`species_umap.txt`): the share of each species' 10 nearest
     neighbours on the layout that are in the same half on a component
     (0.5 is chance). Class-row PCA: t-SNE 0.77, UMAP 0.71 to 0.74.
     Recording PCA: t-SNE 0.56, UMAP 0.52 to 0.54. The two layouts are about
     as compact as each other; the class-row components are far more compact
     than the recording ones on both.
   - Components: PCA of BirdNET's class rows (the species' own vectors) by
     default, since the maps show where a reading is among the species, and
     those components form compact areas. `?pc=rec` uses the PCA of
     recordings' embeddings instead, which follows a recording's sound
     better (table below) but scatters over the map.
   - Colours: the species are split into five equal fifths per component
     (blue `#4a8ee0`, `#2f66b8`, grey `#6a6963`, orange `#b25e18`, `#e8843c`
     from the low end to the high end). Zoomed out, each species takes the
     colour of the fifth that is concentrated around it on the map (its
     share of the species in the surrounding 3 x 3 grid cells, 24 cells a
     side, furthest above 1 in 5), stronger the further above; species where
     no fifth stands out (strength under 0.15) are a faint grey. Zoomed in
     (from 60% of the map's width in view down to 30%), the colours fade to
     each species' own fifth, which is speckled. Blue against orange
     survives red-green colour blindness; the two steps within each arm are
     only 10 to 11 apart on the dataviz check (15 is its target), which the
     ordered key beside each graph covers.
   - The small maps show the same part of the map as the main one, so they
     zoom and pan with it. Pressing one colours the main map by that
     component (a key appears in the map's corner); pressing it again
     removes the colours. For this the main map's species are drawn on a
     canvas under the ECharts layers, which now hold only the marker and
     trail.
   - The reading on a small map is a dot at the marker's place, in the colour
     of the nearest species' own fifth, which can differ from the colour of
     the area around it.
   - Under each small map, a graph of the reading's percentile on that
     component over the last 20 s, newest on the right like the strip, each
     stretch in the colour of the fifth it is in, with the five colours as a
     key at the right edge and a white mark at the newest value. The
     percentile is the mean of the last 4 readings (about 1 s). This replaced
     lighting up the reading's fifth on the map, which could not be told
     from the rest without watching for the change.
   - Beside each name, a small pie shows the component's share of the
     variation (1.8% for PC1 of the class rows, so a thin slice; 21% for PC1
     of the recordings).
   - 12 class-row components (was 8, since 2026-11-20). Shares of the class
     rows' variance, PC1 to PC12: 1.79, 1.33, 1.10, 0.95, 0.88, 0.77, 0.69,
     0.64, 0.60, 0.57, 0.57 and 0.53% (10.4% together). The same PCA on the
     rows with each column shuffled gives 0.18% per component, so every one of
     the first 24 is above noise; variance does not say where to stop.
     Compactness does: the share of each species' 10 nearest neighbours on
     the UMAP layout in the same half of a component is 0.84, 0.82, 0.84,
     0.72, 0.68, 0.70, 0.67, 0.70, 0.65, 0.69, 0.68 and 0.65 for PC1 to PC12
     (t-SNE: 0.85 to 0.68), then 0.62, 0.63, 0.63 and 0.58 for PC13 to PC16
     (0.5 is chance). Past PC12 the maps are close to speckle, so the file
     stops at 12. The first 8 are the same axes as before (cosine 0.997 or
     more, same sign), so their colours and ends are unchanged.
   - Ends of the new components (from `pc_bands.txt`): PC9 South Asian
     laughingthrushes and babblers against Australian and New Guinea birds;
     PC10 a mix (harrier, crake, thornbird, hornero, plover) against European
     crows, jays and tits; PC11 South American pigeons, tinamou and motmot
     against a mix (hummingbird, booby, avocet, grebe); PC12 Himalayan
     warblers, treecreepers and tits against a mix (antpitta, frogmouth,
     myna, drongo).
   - Pointing at a small map says what the colour there stands for: the
     share of the species around the pointer that are in the area's fifth.
     Pointing at a graph gives the percentile and time of that reading.
   - Cost: a reading redraws eight map copies and graphs in about 0.3 ms; a
     zoom or pan redraws all species once per frame. Not measured again with
     12 maps.
   - Removed: the `?pclight=` looks (regions, regions5, two, outer, halves,
     fine, typical). Zooming in now gives the `regions` look.

6. **Full screen**: the button above the map fills the window with the view
   (the map as large as the window's height allows), and the screen where
   the browser allows it. Escape or the same button leaves it.

7. **Species list**: a click or tap on the map keeps the list of nearest
   species. A kept list has a Close button and, beside each species, a
   button that opens Find recordings with that species' scientific name. It
   also closes on Escape, on a press outside it, and on a pan.

8. **Paused reading**: while a file is open and nothing plays or listens,
   the view reads the 3 s that Play would start with, so the map shows where
   playback will begin. The start follows the Play button's start setting
   (Auto, All, Here, Sel) by the same rules as the button
   (`audio::playback::play_start_time`): the start of the file, the dashed
   "here" line, or the start of the selection. The window is the 3 s
   starting there; within 3 s of the end of the file it is the last 3 s.
   This differs on purpose from the live reading, which is the 3 s ending at
   the playhead: the paused reading is what is heard first. One request goes
   out 300 ms after the start stops moving (scrolling, a new selection, a
   new file), and only while the live view is on. It does not go into the
   strip, the change trace, the trail, the component graphs or the Live
   explorer. On the maps it is a blue play triangle, in the blue of the
   spectrogram's "here" line; the readout shows its species with the window
   in front ("12.4s to 15.4s" or "1m02.4s to 1m05.4s", the app's position
   format, in the same blue), and the status line says "The 3 s from where
   Play starts (blue triangle on the map)". The trail of earlier readings is
   hidden meanwhile. While the start moves and its reading is not in yet,
   the map has no marker and the readout is blank, so the reading from
   before the pause is never taken for the play start. A reading that does
   not arrive is asked for again after 10 s. When playback or the microphone
   starts, the triangle goes at once and the live readings take over. The
   classifier's stored windows were not used: a stored window that contains
   the position is not the 3 s starting at it, so the marker would sit
   elsewhere. Timelines get no paused reading.

9. **Unlikely here**: with "Mark birds unlikely here" on, the app's flag
   (as in the results lists) follows a bird's name in the strip and in the
   readout, with "Unlikely at this place and month" as its tooltip. The
   place is the one the classifier results use: during file playback (or
   paused) the file's own place, or this device's position (if "Use my
   location" gave one this session) in the file's week; while the microphone
   listens, this device's position in the current week, and no flags without
   it. The location list is worked out the first time it is needed for a
   place (`classifier::location::scores_at`), which downloads the Geomodel
   (16 MB) if it is not on the device yet, and starts the classifier's
   worker for it. A place whose list fails is tried again after 60 s.

   **Second version (`?pc=rec` in the address): PCA of recordings' embeddings**
   (`pc_rec.py`, output and checks in `pc_rec.txt`, data `birdnet-pc.bin`).
   - Fitted, centred, on about 50,000 BirdNET embeddings of BirdSet's HSN and
     POW test sets (both North American; no other set is on this machine).
     Shares: 21.4, 7.0, 4.1, 2.7, 2.3, 2.2, 2.0 and 1.9%, against 1 to 2% for
     the class rows.
   - A reading is a plain projection on the same axes, turned into a
     percentile among the same recordings.
   - Species are not recordings, so each needs a place on each component.
     45 species are the top class (score 0.5 or more) of at least 5
     reference readings; their place is the median score of those readings.
     Their class rows projected onto the same axes agree with that
     (correlation 0.89, 0.58, 0.75, 0.87, 0.83, 0.69 and 0.76 on PC1 to PC3
     and PC5 to PC8), so on those components every species is ranked by its
     class row alone. On PC4 they agree only 0.46, so PC4 lights only the 45
     species and the other 6,477 stay dim.
   - Mixing the two sources in one ranking was tried first: the 45
     recording-placed species then took the ends of every axis (all 45 in
     PC1's outer tenth) and lit the wrong species.
   - Check, fitted on half the recordings and checked on the other half: see
     the table below. It is much better than the first version, but it tests
     the same 45 North American species the places came from.

   **First version (shown by default): PCA of the class rows**
   (`pc_bands.py`, `pc_bands.txt`, `birdnet-pc-rows.bin`), the same fit as
   `pca_axes.py`. The rows and the embedding share the 1,024 dimensions but
   not their spread (the embedding is non-negative and sits to one side), so
   the reading's percentile among the BirdSet recordings is matched to the
   species percentile. The light was on the top class's own band little
   more often than chance, and without smoothing it jumped by 0.3 to 0.5
   between readings.

   Reading percentile against its top class's percentile, readings with a
   class at 0.5 or more (mean gap, shuffled gap, correlation):

   | PC | Class rows | Recordings, held out |
   |---|---|---|
   | 1 | 0.51, 0.52, 0.43 | 0.32, 0.48, 0.83 |
   | 2 | 0.33, 0.35, 0.21 | 0.27, 0.29, 0.23 |
   | 3 | 0.25, 0.26, 0.06 | 0.36, 0.50, 0.63 |
   | 4 | 0.38, 0.44, 0.58 | 0.28, 0.37, 0.44 |
   | 5 | 0.38, 0.39, 0.32 | 0.25, 0.45, 0.71 |
   | 6 | 0.23, 0.32, 0.61 | 0.24, 0.41, 0.65 |
   | 7 | 0.45, 0.46, 0.06 | 0.23, 0.33, 0.48 |
   | 8 | 0.45, 0.46, 0.15 | 0.19, 0.29, 0.55 |
   | 9 | 0.31, 0.30, -0.14 | |
   | 10 | 0.28, 0.31, 0.20 | |
   | 11 | 0.36, 0.32, -0.26 | |
   | 12 | 0.28, 0.31, 0.22 | |

   On PC9 and PC11 the reading's percentile runs against its top class's
   (negative correlation), so on those two the graph under the map can sit
   at the other end from the species the marker is among.

## Tried and dropped

- **PCA "slider" axes.** `pca_axes.py` fits PCA on the class rows
  (`pca_axes.txt` has the output). The first 6 components explain 1.8%, 1.3%,
  1.1%, 0.9%, 0.9% and 0.8% of the variance, and their ends are regional bird
  groups (Amazonian antbirds against larks, waterbirds against Andean tanagers,
  North American warblers against African larks), not sound qualities. No
  axis has a name a user would recognise, so none were added. PCA on
  embeddings of recordings was not tried; the change trace already shows "the
  sound is changing", which was the remaining use for unnamed axes.
- **UMAP of the class rows, with new readings projected onto it** (for the
  live view's map). Not built: the UMAP layout that is now the default (5)
  places readings from the class scores, as the t-SNE one does. It kept less of the
  genus grouping than t-SNE (0.19 against 0.33 same-genus share), shipping
  the fitted model to the browser costs megabytes, and placing the marker from
  the class scores already puts it among the right species (it agrees with
  the classifier by construction). It would only buy positions for sounds that
  no class scores, which the dashed "not near anything" ring covers.
- **Fixed non-bird rows in the strip** (see 2).
- **A row label column.** The sidebar is too narrow; names are written over
  the left end of each row instead.

## Live explorer tab

A separate right-sidebar tab, "Live explorer (beta)", for exploring the
readings rather than reading off a species (`src/live_explorer.rs`,
`live-explorer.js`, `src/components/live_explorer.rs`). It shares the live
view's worker: the worker runs while either view is shown, and the tab keeps
the last 5 minutes of readings (1,200). Panels:

1. **Frequency peaks**: Welch power spectrum of the same last 3 s
   (`oversample_core::dsp::psd`, about 25 Hz per bin, every 0.5 s), with the
   strongest peaks marked. Works without BirdNET and for bat recordings. The
   axis ends at the highest frequency within 40 dB of the loudest bin.
2. **Top classes now**: the newest reading's scores of 0.02 or more, up to 15,
   with the minimum confidence as a dashed line.
3. **This session's readings**: every reading as a dot. PCA is fitted on the
   session's own embeddings (refitted as the session grows by a tenth, with
   axis signs kept, so the picture does not flip), and each new reading is
   projected. On the Pauraque recording PC1 and PC2 hold 36% and 18% of the
   variation, against 1.8% and 1.3% for PCA of the class rows: fitted on one
   recording, the first axes do separate its sounds (call against gap).
   "UMAP" runs umap-js (vendored, MIT, loaded on first use) on the session's
   embeddings once, as a snapshot; it needs 10 or more readings. This is a
   different use from the class-row UMAP dropped above: it costs 83 KB of
   script and a second or two, only when asked for.
4. **Similarity, last 15 s**: cosine similarity between every pair of the
   last 60 readings. A repeated call shows as bright blocks away from the
   diagonal. The colour scale starts at the lowest similarity in view.
5. **Embedding, last 15 s**: the 48 of the 1,024 values with the most variance
   in the last 60 readings, one row each, as z-scores per row (blue below the
   row's mean, amber above).

Redraws are spaced (spectrum 0.45 s, map 0.9 s, the two heatmaps 1.5 s):
redrawing all five at every reading dropped 4% of playback frames; spaced, the
Pauraque test dropped 6 of 1,117 frames, and the bat and duck tests none.

Each reading covers 3 s, so a short call changes the embedding twice: when it
enters the window and 3 s later when it leaves. The change trace and the
similarity matrix both show this double step.

Not done yet: adding the file's classifier windows
(`state.classifier.embeddings()`) to the session map, which share the same
space (see the parity test).

## Limits

- The paused reading needs the region decoded: for a long file still
  streaming in, it waits until that part is loaded.
- Bat and other ultrasonic recordings: BirdNET hears 0 to 15 kHz, Perch 60 Hz
  to 16 kHz. For audio at 96 kHz or more, when nothing reaches the minimum
  confidence, the view says "BirdNET hears up to 15 kHz, so bat calls do not
  show here." (or the same for Perch 2.0 at 16 kHz). It still runs, since
  such recordings often have birds or insects in the audible band too.
- During playback the view reads the file's original audio, not what the
  playback mode (heterodyne, time expansion) makes of it.
- When the window is in the background the browser slows its animation
  frames, the playhead advances in steps, and readings repeat.
- The real microphone was not tested here (the test browser blocks it). The
  live path was tested with the app's synthetic microphone, which feeds the
  same live buffer.
