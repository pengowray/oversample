# Media for web pages

Short silent videos of Oversample for web pages and posts, each with a
`<name>-poster.jpg` (its last frame; the tours' are from the bat book step).
A caption above the app says what each clip shows.

## Desktop (1280 × 720)

| Clip | Length | Shows |
|---|---|---|
| `tour.mp4` | 53 s | Most of the clips below in one session, with a title and an end card |
| `open-recording.mp4` | 12 s | Find recordings: search for "soprano pip" and load a Xeno-canto recording |
| `zoom-pan.mp4` | 8 s | Ctrl + scroll wheel to zoom out and back in, then drag along the recording |
| `hfr-demo.mp4` | 13 s | Turn on HFR, drag the frequency strip and the time strip, press Play |
| `playback-modes.mp4` | 18 s | XForm on, then HET, TE, PS and ZC |
| `views.mp4` | 8 s | Spec, Wave and Dots |
| `mark-call.mp4` | 14 s | Zoom in, Selection tool, box round a call, Annotate Region, type a label |
| `call-detection.mp4` | 9 s | Open the Call detection panel; its boxes appear round the calls |
| `bat-book.mp4` | 7 s | Bat book on, then a second species to compare |
| `view-3d.mp4` | 23 s | Selection round a few calls, View in 3D, drag to turn the surface |
| `timeline.mp4` | 9 s | Files tab: Select, pick three recordings, Create Timeline |
| `live.mp4` | 9 s | Live listening, fed by a replayed recording (the title bar says "Test signal: Replay"); HFR on and a band chosen |

```html
<video src="tour.mp4" poster="tour-poster.jpg" width="1280" height="720"
       autoplay muted loop playsinline></video>
```

## Phone (824 × 1678, portrait)

The same steps in the phone layout, with a fingertip instead of a pointer.

| Clip | Length | Shows |
|---|---|---|
| `tour-phone.mp4` | 27 s | Open a recording, HFR, bat book, with a title and an end card |
| `open-recording-phone.mp4` | 12 s | Files, Find recordings, search and load |
| `hfr-demo-phone.mp4` | 13 s | HFR, frequency strip, time strip, Play |
| `bat-book-phone.mp4` | 7 s | Bat book on, then a second species |
| `live-phone.mp4` | 9 s | Live listening from a replayed recording, HFR on and a band chosen |

```html
<video src="tour-phone.mp4" poster="tour-phone-poster.jpg" width="412" height="839"
       autoplay muted loop playsinline></video>
```

## Re-recording

Re-record them with `scripts/demo-clips/record.mjs` (instructions at the top
of the script), for example after a change to the parts of the app they show.
`--phone` records the phone clips. `--gif` also writes GIFs for places that
can't play a video; they are 3 to 7 times the size of the MP4.

## Recordings shown

Every clip except the timeline shows
[XC1037039](https://xeno-canto.org/1037039), Soprano Pipistrelle
(*Pipistrellus pygmaeus*), recorded in Cornwall, UK, by Stephan Risch,
licensed [CC BY-NC-SA 4.0](https://creativecommons.org/licenses/by-nc-sa/4.0/).
The live clips replay it through the live view.

`timeline.mp4` shows [XC1084197](https://xeno-canto.org/1084197),
[XC1084199](https://xeno-canto.org/1084199) and
[XC1084195](https://xeno-canto.org/1084195), recorded on one night in Sweden
by Alessandra Munari, licensed
[CC BY-NC-SA 4.0](https://creativecommons.org/licenses/by-nc-sa/4.0/).

A page that uses a clip credits the recordist and keeps the licence.
