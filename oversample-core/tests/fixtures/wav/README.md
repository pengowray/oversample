# WAV edge-case fixtures

Copied from the `wav/` folder of the qubero-samples collection (commit
`85702a7`), which made them for the Qubero hex editor. All are CC0-1.0. Most
were generated with FFmpeg 4.2.3 and hold a 0.25 second sine wave; the
structural cases hold a few synthetic samples.

They are used by `tests/wav_edge_cases.rs`. That test also builds a few files
in code (a Pettersson D500X recording, truncated and over-4 GB headers) and
reads one real D500X recording, xeno-canto XC1060673, from the
qubero-samples collection when it is present. That recording is CC BY-NC-SA
4.0, so it is not copied here.

| File | Case |
|---|---|
| `broadcast-pcm16-bext-peak.wav` | Broadcast Wave `bext` and `PEAK` chunks before `data` |
| `data-before-fmt-pcm16.wav` | `data` chunk before `fmt ` |
| `g711-alaw-mono-8000.wav`, `g711-mulaw-mono-8000.wav` | G.711 A-law and µ-law (not supported) |
| `gsm610-structural-header-only.wav` | GSM 6.10 (not supported) |
| `guano-past-riff-size-pcm16.wav` | GUANO chunk after `data` that runs past the RIFF size |
| `ieee-float32-stereo-48000.wav` | float32, written by FFmpeg as WAVE_FORMAT_EXTENSIBLE |
| `ieee-float64-mono-44100.wav` | float64 |
| `ima-adpcm-mono-22050.wav`, `ms-adpcm-stereo-22050.wav` | ADPCM (not supported) |
| `mp3-in-wav-mono-22050.wav` | MP3 inside RIFF/WAVE (not supported) |
| `multiple-data-chunks-pcm16.wav` | Two `data` chunks |
| `odd-final-data-no-pad-pcm-u8.wav` | Odd-length final chunk with no pad byte |
| `pcm-12bit-container16.wav` | 12-bit samples in 16-bit slots |
| `pcm-s16le-stereo-44100.wav`, `pcm-s24le-stereo-48000.wav`, `pcm-s32le-mono-96000.wav`, `pcm-u8-mono-8000.wav` | Integer PCM widths |
| `rf64-pcm-s16le-stereo.wav` | RF64 with a `ds64` chunk |
| `rifx-pcm-s16be.wav` | Big-endian RIFX (not supported) |
| `unknown-format-tag-1234.wav` | Unknown format tag |
| `wave-extensible-float32-stereo.wav` | Minimal extensible float32 |
| `wave-extensible-pcm-5.1-48000.wav` | Extensible 24-bit 5.1 with a channel mask |
| `wave-extensible-pcm24-valid20-5.1.wav` | Extensible: 24-bit samples, 20 valid bits, 5.1 |
