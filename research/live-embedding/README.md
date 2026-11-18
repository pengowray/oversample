# Live BirdNET view (experimental)

A view in the Call detection tab (BirdNET picked, "Live view" switched on)
that shows what BirdNET V2.4 hears in the last 3 s of audio, a few times a
second, while a file plays or the microphone listens. Code:
`src/live_embedding.rs` (worker, requests, placement, strip rows),
`live-embedding.js` (charts, Apache ECharts), and
`src/components/file_sidebar/call_detection_panel/live_view.rs` (switch and
status line). The worker message is `ToWorker::Live` in
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

## What the view shows, and why

1. **The top species and its score, as text**, at the top. In a 220 px
   sidebar this is the part that reads at a glance. Below the minimum
   confidence (the same setting as the results) it says "No species at 0.50 or
   above" and lists the highest scores in grey, so a weak match never looks
   like a hit.
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
4. **The species map with a marker**: the existing t-SNE map of the class
   rows (`research/species-map`), with the current reading placed by the same
   rule as the map's windows (score-weighted mean of the top 3 classes within
   0.15 map units of the top one) and a fading 15 s trail. A hit is a filled
   green dot sized by the score; below the minimum it is a large dashed ring.
   The trail line breaks on jumps longer than 0.15 map units, so it never
   draws across the gap between unrelated groups. At sidebar size the map
   shows which part of the bird world the sound is in and whether it is
   moving; the name comes from the text above it.

5. **Component maps (prototype)**: eight small copies of the species map, four
   to a row, one per principal component, under the main map. Under each, a
   thin line shows the reading's percentile on that component (the tick is
   the median). Each band of species is pre-drawn to an offscreen canvas:
   all eight maps take about 0.5 ms per reading in Chrome, against about
   5 ms for the main map's ECharts update. The reading's percentile is the
   mean of the last 4 readings (about 1 s), so the light does not flicker.
   - Default look (`regions`): every species is always coloured by its
     fifth of the component (equal counts; blue `#4a8ee0`, `#2f66b8`, grey
     `#6a6963`, orange `#b25e18`, `#e8843c` from the low end to the high
     end), dimmed to 30%. The fifth the reading is in is drawn at full
     strength; nothing is while the reading is in the middle fifth. The line
     under each map has the same five colours and a white mark at the
     reading, so it is the key. Blue against orange survives red-green
     colour blindness; the two steps within each arm are only 10 to 11 apart
     on the dataviz check (15 is its target), which the ordered line covers.
     `?pclight=regions5` adds the 5% band at the reading in white; at this
     size it scatters over the whole map, so it is off. On the class-row
     PCA (`?pc=rows`) the fifths form clear regions on PC1 to PC3; on the
     recording PCA they mostly interleave.
   - Other looks, chosen with `?pclight=`:
     `typical`: of the third of the species the reading is in, only those
     sitting among others of the same third (their third's share of the
     surrounding grid cells, above 1 in 3) light in dark blue, and the 5%
     band at the reading lights in light yellow on top. The middle third is
     never lit (only the ends of an axis say much about a sound), but the 5%
     band still shows. `two`: the same without the "typical" filter.
     `outer`: only the outer thirds, in green. `halves`: the half below or
     above the median, nothing between the 40th and 60th percentiles.
     `fine`: 5% bands with a 10% fade. The two colours differ in lightness as
     well as hue.
   - `?pcmap=umap` draws the small maps on a UMAP layout of the class rows
     (`species_umap.py`, `birdnet-umap.bin`) instead of the main map's
     t-SNE. UMAP of 6,522 class rows (cosine) takes 12 s with 50 neighbours
     and 34 to 41 s with 15 (the first run includes compiling), so it has to
     be precomputed.
   - Compactness (`species_umap.txt`): the share of each species' 10 nearest
     neighbours on the layout that are in the same half on a component
     (0.5 is chance). Class-row PCA: t-SNE 0.77, UMAP 0.71 to 0.74.
     Recording PCA: t-SNE 0.56, UMAP 0.52 to 0.54. t-SNE is the more compact
     layout for every basis and setting, so it stays the default.
   - How often the heard species is lit against a random species, with the
     earlier two-band default (reference readings, not held out): 2 bands
     with a middle zone 0.44 to 0.76 against 0.33 to 0.43; 20 bands 0.10 to
     0.36 against 0.14 to 0.15.
   - Even the 5% band scatters over the whole small map with the recording
     PCA: on a map of species, its bands are not regions. Only PC1 and PC3
     show a clear area.

   **Second version (shown by default): PCA of recordings' embeddings**
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

   **First version (`?pc=rows` in the address): PCA of the class rows**
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
  live view's map). Not built. It kept less of the
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

- Bat and other ultrasonic recordings: BirdNET hears 0 to 15 kHz. For audio
  at 96 kHz or more the view says "BirdNET hears up to 15 kHz, so bat calls
  above that do not show here." It still runs, since such recordings often
  have birds or insects in the audible band too.
- During playback the view reads the file's original audio, not what the
  playback mode (heterodyne, time expansion) makes of it.
- When the window is in the background the browser slows its animation
  frames, the playhead advances in steps, and readings repeat.
- The real microphone was not tested here (the test browser blocks it). The
  live path was tested with the app's synthetic microphone, which feeds the
  same live buffer.
