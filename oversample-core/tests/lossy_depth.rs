//! The bit depth the loader reports: none for lossy codecs (MP3, Ogg Vorbis,
//! AAC), and the stored depth for lossless ones (FLAC, WAV, ALAC).

use oversample_core::audio::loader::{load_audio, m4a_pcm_bits};
use oversample_core::types::FileMetadata;
use std::path::Path;

fn read(rel: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(rel);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn loader_reports_pcm_bits_by_codec() {
    for (file, bits) in [
        ("mp3_estimate/tone_48k_mono_xing.mp3", None),
        ("fixtures/tone_48k_mono.ogg", None),
        ("fixtures/tone_48k_mono.m4a", None),
        ("fixtures/tone_48k_mono.flac", Some(16)),
        ("fixtures/tone_48k_mono.wav", Some(16)),
        ("fixtures/m4a/tone_48k_mono_alac24.m4a", Some(24)),
    ] {
        let audio = load_audio(&read(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
        assert_eq!(audio.metadata.pcm_bits(), bits, "{file}");
        assert!(!audio.metadata.is_float, "{file}");
    }
}

/// The browser's decoder gives float samples whatever the codec, so the M4A
/// depth is read from the file's sample description.
#[test]
fn m4a_depth_from_the_sample_description() {
    let aac = read("fixtures/tone_48k_mono.m4a");
    assert_eq!(m4a_pcm_bits(&aac), FileMetadata::LOSSY_BITS);
    let alac = read("fixtures/m4a/tone_48k_mono_alac24.m4a");
    assert_eq!(m4a_pcm_bits(&alac), 24);
}
