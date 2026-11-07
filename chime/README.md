# chime

A copy of the AudioMoth chime decoder from `crates/chime` in the `batchi-tascam-oversample-colab`
repository, at commit e85fc2a, so this repository builds on its own. That repository has no public remote yet.
Change the decoder there and copy it here: `src/` is the same apart from `src/biquad.rs`, which is the
`Butterworth4` filter from that repository's `bat-dsp` crate (its tests left out), and the one `use` line in
`src/decoder/front_end.rs` that imports it.

The format is the AudioMoth chime by Open Acoustic Devices (MIT licence). See `src/lib.rs`.
