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
3. **"Change in sound"**: the cosine distance between each reading's embedding
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

## Tried and dropped

- **PCA "slider" axes.** `pca_axes.py` fits PCA on the class rows
  (`pca_axes.txt` has the output). The first 6 components explain 1.8%, 1.3%,
  1.1%, 0.9%, 0.9% and 0.8% of the variance, and their ends are regional bird
  groups (Amazonian antbirds against larks, waterbirds against Andean tanagers,
  North American warblers against African larks), not sound qualities. No
  axis has a name a user would recognise, so none were added. PCA on
  embeddings of recordings was not tried; the change trace already shows "the
  sound is changing", which was the remaining use for unnamed axes.
- **UMAP with out-of-sample projection.** Not built. It kept less of the
  genus grouping than t-SNE (0.19 against 0.33 same-genus share), shipping
  the fitted model to the browser costs megabytes, and placing the marker from
  the class scores already puts it among the right species (it agrees with
  the classifier by construction). It would only buy positions for sounds that
  no class scores, which the dashed "not near anything" ring covers.
- **Fixed non-bird rows in the strip** (see 2).
- **A row label column.** The sidebar is too narrow; names are written over
  the left end of each row instead.

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
