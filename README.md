# Oversample

Oversample is a bat call analysis tool. Open a recording (WAV, FLAC, OGG or MP3) to see its spectrogram and waveform, and listen to ultrasonic calls slowed down or shifted into the range of human hearing. It can also record from a microphone, including USB ultrasonic microphones.

All decoding and analysis happens on your own device. Recordings are not uploaded.

## Use it

- **In a browser:** [app.oversample.com](https://app.oversample.com). Nothing to install.
- **Windows, Linux and macOS:** download an installer from the [latest release](https://github.com/pengowray/oversample/releases/latest). The macOS build is untested.
- **Android:** in closed testing. See [oversample.com/beta](https://oversample.com/beta).

## Build it

Oversample is written in Rust: a [Leptos](https://leptos.dev) frontend compiled to WebAssembly, wrapped with [Tauri](https://tauri.app) for desktop and Android.

```sh
# Web app, served at http://localhost:21259
trunk serve --release --port 21259

# Desktop app
cargo tauri dev
```

[CLAUDE.md](CLAUDE.md) has the full build commands, the source layout and the tests.

## License

The apps are licensed under GPL-3.0. The library crates (`oversample-core`, `xc-lib`) are licensed under MIT or Apache-2.0, at your option. See [CONTRIBUTING.md](CONTRIBUTING.md) for details and for how to contribute.
