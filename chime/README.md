# chime

Decoder for the AudioMoth chime, the data-over-sound format by Open Acoustic Devices (MIT licence)
that sets a device's clock from a phone. See `src/lib.rs` for the format.

This is the one copy of the decoder. Oversample uses it (`oversample-core/src/dsp/chime_scan.rs`), and
so does the TASCAM Portacapture X8 bat patch in the `batchi-tascam-oversample-colab` repository, which
depends on this folder by path.

- `no_std`, no allocation, f32: it runs in the X8's firmware.
- `src/biquad.rs` is the decoder's own copy of the `Butterworth4` filter, so the crate needs nothing
  else.
- `src/encode.rs`, with the `std` feature: the encoder, a port of the phone page's JavaScript, for tests
  and test files.
- `format::RemoteCommand`: the X8 phone remote's start and stop commands in a chime's deployment ID.
- Examples: `decode_wav` (finds chimes in WAV files and compares an X8's clock with the phone),
  `chime_wavs` (writes phone remote test files), `cost` (times the decoder's stages, feature `profile`).
