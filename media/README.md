# Media for web pages

Short silent videos of Oversample for web pages and posts: 1280 × 720 MP4,
each with a `<name>-poster.jpg` (its last frame; the tour's is from the bat
book step). A caption above the app says what each clip shows.

| Clip | Length | Shows |
|---|---|---|
| `tour.mp4` | 56 s | Most of the clips below in one session, with a title and an end card |
| `open-recording.mp4` | 12 s | Find recordings: search for "soprano pip" and load a Xeno-canto recording |
| `zoom-pan.mp4` | 8 s | Ctrl + scroll wheel to zoom out and back in, then drag along the recording |
| `hfr-demo.mp4` | 13 s | Turn on HFR, drag the frequency strip and the time strip, press Play |
| `playback-modes.mp4` | 18 s | XForm on, then HET, TE, PS and ZC |
| `views.mp4` | 8 s | Spec, Wave and Dots |
| `mark-call.mp4` | 14 s | Zoom in, Selection tool, box round a call, Annotate Region, type a label |
| `call-detection.mp4` | 9 s | Open the Call detection panel; its boxes appear round the calls |
| `bat-book.mp4` | 7 s | Bat book on, then a second species to compare |

```html
<video src="tour.mp4" poster="tour-poster.jpg" width="1280" height="720"
       autoplay muted loop playsinline></video>
```

Re-record them with `scripts/demo-clips/record.mjs` (instructions at the top
of the script), for example after a change to the parts of the app they show.
`--gif` also writes GIFs for places that can't play a video; they are 3 to 7
times the size of the MP4.

## Recording shown

Every clip shows [XC1037039](https://xeno-canto.org/1037039), Soprano
Pipistrelle (*Pipistrellus pygmaeus*), recorded in Cornwall, UK, by Stephan
Risch, licensed
[CC BY-NC-SA 4.0](https://creativecommons.org/licenses/by-nc-sa/4.0/). A page
that uses a clip credits the recordist and keeps the licence.
