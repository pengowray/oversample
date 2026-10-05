//! WAV edge cases: unusual but valid layouts, recorder quirks, and encodings
//! that should fail with a clear message.
//!
//! Every readable file is checked two ways, and the two must agree:
//!  - `load_audio` on the whole file (the in-memory path), and
//!  - `parse_wav_header_with_file_size` on the first 64 KB plus the file size,
//!    then `decode_pcm` on the samples it points at (the streaming path).
//!
//! Fixtures are in `tests/fixtures/wav` (see the README there). A few files are
//! built in code, and one real D500X recording is read from the qubero-samples
//! collection when it is present.

use oversample_core::audio::loader::{load_audio, parse_wav_header_with_file_size};
use oversample_core::audio::wav::{
    data_pad_len, decode_pcm, header_bytes, parse_wav_header, SampleEncoding, WavWriteFormat,
    WRITE_HEADER_LEN,
};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> Vec<u8> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/wav")
        .join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

struct Expect {
    rate: u32,
    channels: u16,
    frames: u64,
    encoding: SampleEncoding,
    valid_bits: Option<u16>,
}

const fn ok(
    rate: u32,
    channels: u16,
    frames: u64,
    encoding: SampleEncoding,
    valid_bits: Option<u16>,
) -> Expect {
    Expect {
        rate,
        channels,
        frames,
        encoding,
        valid_bits,
    }
}

use SampleEncoding::*;

const READABLE: &[(&str, Expect)] = &[
    (
        "broadcast-pcm16-bext-peak.wav",
        ok(48000, 2, 12000, I16, None),
    ),
    ("data-before-fmt-pcm16.wav", ok(8000, 1, 4, I16, None)),
    (
        "guano-past-riff-size-pcm16.wav",
        ok(8000, 1, 400, I16, None),
    ),
    (
        "ieee-float32-stereo-48000.wav",
        ok(48000, 2, 12000, F32, None),
    ),
    (
        "ieee-float64-mono-44100.wav",
        ok(44100, 1, 11025, F64, None),
    ),
    ("multiple-data-chunks-pcm16.wav", ok(8000, 1, 4, I16, None)),
    ("odd-final-data-no-pad-pcm-u8.wav", ok(8000, 1, 3, U8, None)),
    ("pcm-12bit-container16.wav", ok(11025, 1, 16, I16, Some(12))),
    ("pcm-s16le-stereo-44100.wav", ok(44100, 2, 11025, I16, None)),
    ("pcm-s24le-stereo-48000.wav", ok(48000, 2, 12000, I24, None)),
    ("pcm-s32le-mono-96000.wav", ok(96000, 1, 24000, I32, None)),
    ("pcm-u8-mono-8000.wav", ok(8000, 1, 2000, U8, None)),
    ("rf64-pcm-s16le-stereo.wav", ok(44100, 2, 11025, I16, None)),
    (
        "wave-extensible-float32-stereo.wav",
        ok(48000, 2, 16, F32, None),
    ),
    (
        "wave-extensible-pcm-5.1-48000.wav",
        ok(48000, 6, 12000, I24, None),
    ),
    (
        "wave-extensible-pcm24-valid20-5.1.wav",
        ok(48000, 6, 16, I24, Some(20)),
    ),
];

/// Files that must fail, and a piece of the message that says why.
const UNREADABLE: &[(&str, &str)] = &[
    ("g711-alaw-mono-8000.wav", "A-law"),
    ("g711-mulaw-mono-8000.wav", "µ-law"),
    ("gsm610-structural-header-only.wav", "GSM 6.10"),
    ("ima-adpcm-mono-22050.wav", "IMA ADPCM"),
    ("mp3-in-wav-mono-22050.wav", "MP3"),
    ("ms-adpcm-stereo-22050.wav", "Microsoft ADPCM"),
    ("rifx-pcm-s16be.wav", "RIFX"),
    ("unknown-format-tag-1234.wav", "0x1234"),
];

/// Check one readable file both ways. Returns the decoded interleaved samples.
fn check_both_paths(name: &str, bytes: &[u8], e: &Expect) -> Vec<f32> {
    let audio = load_audio(bytes).unwrap_or_else(|err| panic!("{name}: load_audio: {err}"));
    assert_eq!(audio.sample_rate, e.rate, "{name}: rate");
    assert_eq!(audio.channels, e.channels as u32, "{name}: channels");
    assert_eq!(audio.samples.len() as u64, e.frames, "{name}: frames");
    let details = audio.metadata.wav.as_ref().expect("WAV details");
    assert_eq!(details.valid_bits, e.valid_bits, "{name}: valid bits");
    assert_eq!(audio.metadata.is_float, e.encoding.is_float(), "{name}");
    assert_eq!(
        audio.metadata.bits_per_sample as usize,
        e.encoding.bytes() * 8,
        "{name}: sample width"
    );

    let head = &bytes[..bytes.len().min(65536)];
    let h = parse_wav_header_with_file_size(head, Some(bytes.len() as u64))
        .unwrap_or_else(|err| panic!("{name}: header: {err}"));
    assert_eq!(h.sample_rate, e.rate, "{name}: header rate");
    assert_eq!(h.channels, e.channels, "{name}: header channels");
    assert_eq!(h.total_frames, e.frames, "{name}: header frames");
    assert_eq!(h.encoding, e.encoding, "{name}: header encoding");
    assert_eq!(Some(h.data_offset), audio.metadata.data_offset, "{name}");
    assert_eq!(Some(h.data_size), audio.metadata.data_size, "{name}");

    let start = h.data_offset as usize;
    let streamed = decode_pcm(&bytes[start..start + h.data_size as usize], h.encoding);
    let in_memory: Vec<f32> = match &audio
        .source
        .as_any()
        .downcast_ref::<oversample_core::audio::source::InMemorySource>(
    ) {
        Some(src) => src.raw_samples.as_deref().unwrap_or(&src.samples).clone(),
        None => panic!("{name}: expected an in-memory source"),
    };
    assert_eq!(
        streamed, in_memory,
        "{name}: streamed and in-memory samples differ"
    );
    streamed
}

#[test]
fn readable_fixtures_agree_on_both_paths() {
    for (name, e) in READABLE {
        let samples = check_both_paths(name, &fixture(name), e);
        // FFmpeg's sine fixtures (0.25 s or longer) peak near 0.1. A wrong
        // scale or stride shows up as silence or as values past 1.
        if e.frames >= 2000 {
            let peak = samples.iter().fold(0f32, |m, s| m.max(s.abs()));
            assert!(peak > 0.01 && peak <= 1.0, "{name}: peak {peak}");
        }
    }
}

#[test]
fn unreadable_fixtures_say_why() {
    for (name, why) in UNREADABLE {
        let bytes = fixture(name);
        let err = load_audio(&bytes)
            .err()
            .unwrap_or_else(|| panic!("{name} loaded"));
        assert!(err.contains(why), "{name}: {err:?} should mention {why:?}");
        let head = &bytes[..bytes.len().min(65536)];
        let err = parse_wav_header_with_file_size(head, Some(bytes.len() as u64))
            .err()
            .unwrap_or_else(|| panic!("{name}: header parsed"));
        assert!(err.contains(why), "{name}: {err:?} should mention {why:?}");
    }
}

#[test]
fn layout_details_are_reported() {
    let h = parse_wav_header(&fixture("rf64-pcm-s16le-stereo.wav")).unwrap();
    assert!(h.details.rf64);
    let h = parse_wav_header(&fixture("wave-extensible-pcm24-valid20-5.1.wav")).unwrap();
    assert!(h.details.extensible);
    assert_eq!(h.details.channel_mask, Some(0x3F));
    let h = parse_wav_header(&fixture("multiple-data-chunks-pcm16.wav")).unwrap();
    assert_eq!(h.details.notes.len(), 1, "{:?}", h.details.notes);
    let h = parse_wav_header(&fixture("guano-past-riff-size-pcm16.wav")).unwrap();
    assert!(h.guano.is_some());
    let h = parse_wav_header(&fixture("pcm-s16le-stereo-44100.wav")).unwrap();
    assert!(h.details.notes.is_empty(), "{:?}", h.details.notes);
}

// ─── Files built in code ────────────────────────────────────────────────────

const MONO16: WavWriteFormat = WavWriteFormat {
    sample_rate: 500_000,
    channels: 1,
    bits_per_sample: 16,
    is_float: false,
};

fn ramp(n: usize) -> Vec<u8> {
    (0..n)
        .flat_map(|i| ((i * 7) as i16).to_le_bytes())
        .collect()
}

/// A D500X-style file: 44-byte header, then a metadata block at the front of
/// `data` that the data size leaves out.
fn d500x_file(block_len: usize, samples: usize, size_counts_block: bool) -> Vec<u8> {
    let mut block = vec![0u8; block_len];
    block[0..4].copy_from_slice(&(block_len as u32).to_le_bytes());
    block[0xA4..0xAE].copy_from_slice(b"M01671.WAV");
    block[0xC4..0xE1].copy_from_slice(b"D500X V2.2.6 140516, 17:19:14");
    block[0xF4..0x109].copy_from_slice(b"f=500 PRE=OFF LEN=0.3");
    block[0x12C..0x134].copy_from_slice(b"PROFILE0");
    let lines = b"File Name:       M01671.WAV\r\nS/N:             01059\r\n";
    block[0x1D4..0x1D4 + lines.len()].copy_from_slice(lines);

    let audio = ramp(samples);
    let data_size = audio.len() + if size_counts_block { block_len } else { 0 };
    let mut f = Vec::new();
    f.extend_from_slice(b"RIFF");
    f.extend_from_slice(&((36 + block_len + audio.len()) as u32).to_le_bytes());
    f.extend_from_slice(b"WAVEfmt ");
    f.extend_from_slice(&16u32.to_le_bytes());
    f.extend_from_slice(&[1, 0, 1, 0]);
    f.extend_from_slice(&500_000u32.to_le_bytes());
    f.extend_from_slice(&1_000_000u32.to_le_bytes());
    f.extend_from_slice(&[2, 0, 16, 0]);
    f.extend_from_slice(b"data");
    f.extend_from_slice(&(data_size as u32).to_le_bytes());
    f.extend_from_slice(&block);
    f.extend_from_slice(&audio);
    f
}

#[test]
fn d500x_block_is_skipped_and_every_sample_read() {
    for counts_block in [false, true] {
        let f = d500x_file(980, 1000, counts_block);
        let e = ok(500_000, 1, 1000, I16, None);
        let samples = check_both_paths("d500x", &f, &e);
        assert_eq!(samples[1], 7.0 / 32768.0, "first samples are the ramp");
        let h = parse_wav_header(&f).unwrap();
        assert_eq!(h.data_offset, 44 + 980);
        let block = h.details.recorder_block.expect("D500X block");
        assert_eq!(block.recorder, "Pettersson D500X");
        let get = |k: &str| {
            block
                .fields
                .iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("S/N"), Some("01059"));
        assert_eq!(get("Profile"), Some("PROFILE0"));
        assert_eq!(get("Profile settings"), Some("f=500 PRE=OFF LEN=0.3"));
        // The recorder's own layout gets a note; a corrected size does not.
        assert_eq!(
            h.details.notes.len(),
            usize::from(!counts_block),
            "{:?}",
            h.details.notes
        );
    }
}

#[test]
fn ordinary_data_is_not_mistaken_for_a_d500x_block() {
    // A file whose first sample happens to look like a block length.
    let mut f = header_bytes(&MONO16, 2000, 0);
    let mut audio = ramp(1000);
    audio[0..4].copy_from_slice(&980u32.to_le_bytes());
    f.extend_from_slice(&audio);
    let h = parse_wav_header(&f).unwrap();
    assert_eq!(h.data_offset, WRITE_HEADER_LEN as u64);
    assert!(h.details.recorder_block.is_none());
}

#[test]
fn file_cut_short_reads_what_is_there() {
    let mut f = header_bytes(&MONO16, 2000, 0);
    f.extend_from_slice(&ramp(600)); // 1200 of the promised 2000 bytes
    f.push(0x55); // and half a sample
    let e = ok(500_000, 1, 600, I16, None);
    check_both_paths("cut short", &f, &e);
    let h = parse_wav_header(&f).unwrap();
    assert_eq!(h.details.notes.len(), 1, "{:?}", h.details.notes);
}

#[test]
fn wrapped_data_size_reads_to_end_unless_a_chunk_follows() {
    // Size field far too small (as when a u32 size wraps past 4 GB).
    let samples = 600_000;
    let mut f = header_bytes(&MONO16, 100, 0);
    f.extend_from_slice(&ramp(samples));
    let h = parse_wav_header(&f).unwrap();
    assert_eq!(h.total_frames, samples as u64);

    // Same small size, but a real chunk follows the declared end: the size
    // is right, and the rest is that chunk.
    let mut f = header_bytes(&MONO16, 100, 0);
    f.extend_from_slice(&ramp(50));
    let junk = vec![0u8; 1_100_000];
    f.extend_from_slice(b"JUNK");
    f.extend_from_slice(&(junk.len() as u32).to_le_bytes());
    f.extend_from_slice(&junk);
    let h = parse_wav_header(&f).unwrap();
    assert_eq!(h.total_frames, 50);
}

#[test]
fn written_files_read_back() {
    for (fmt, encoding) in [
        (MONO16, I16),
        (
            WavWriteFormat {
                sample_rate: 384_000,
                channels: 1,
                bits_per_sample: 24,
                is_float: false,
            },
            I24,
        ),
        (
            WavWriteFormat {
                sample_rate: 48_000,
                channels: 2,
                bits_per_sample: 32,
                is_float: true,
            },
            F32,
        ),
    ] {
        // An odd sample count, so 24-bit mono has an odd data length and
        // needs the pad byte before the GUANO chunk.
        let frames = 11u64;
        let data_bytes = frames * fmt.block_align() as u64;
        let guano = b"GUANO|Version: 1.0\nMake: Test\n";
        let after = data_pad_len(data_bytes) + 8 + guano.len() as u64;
        let mut f = header_bytes(&fmt, data_bytes, after);
        f.extend((0..data_bytes).map(|i| (i * 3) as u8));
        f.extend(std::iter::repeat_n(0u8, data_pad_len(data_bytes) as usize));
        f.extend_from_slice(b"guan");
        f.extend_from_slice(&(guano.len() as u32).to_le_bytes());
        f.extend_from_slice(guano);
        assert_eq!(
            u32::from_le_bytes(f[4..8].try_into().unwrap()) as usize,
            f.len() - 8
        );

        let h = parse_wav_header(&f).unwrap();
        assert_eq!(h.encoding, encoding);
        assert_eq!(h.total_frames, frames);
        assert_eq!(h.data_offset, WRITE_HEADER_LEN as u64);
        assert!(h.guano.is_some(), "{fmt:?}");
        assert!(h.details.notes.is_empty(), "{:?}", h.details.notes);
    }
}

#[test]
fn over_4gb_is_written_as_rf64() {
    let data_bytes = 5_000_000_000u64;
    let h = header_bytes(&MONO16, data_bytes, 40);
    assert_eq!(&h[0..4], b"RF64");
    assert_eq!(&h[12..16], b"ds64");
    let parsed = parse_wav_header_with_file_size(&h, Some(80 + data_bytes + 40)).unwrap();
    assert!(parsed.details.rf64);
    assert_eq!(parsed.data_size, data_bytes);
    assert_eq!(parsed.total_frames, data_bytes / 2);
    assert!(
        parsed.details.notes.is_empty(),
        "{:?}",
        parsed.details.notes
    );

    // Under 4 GB: plain RIFF with the JUNK chunk reserved, same length.
    let h = header_bytes(&MONO16, 1000, 0);
    assert_eq!(&h[0..4], b"RIFF");
    assert_eq!(&h[12..16], b"JUNK");
    assert_eq!(h.len(), WRITE_HEADER_LEN);
}

// ─── Real recording from the qubero-samples collection ──────────────────────

/// The qubero-samples collection: `QUBERO_SAMPLES_DIR` (environment or the
/// workspace `.env`), else `../qubero-samples` beside this checkout.
fn qubero_samples() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let from_env = std::env::var("QUBERO_SAMPLES_DIR").ok().or_else(|| {
        let env = std::fs::read_to_string(root.join(".env")).ok()?;
        env.lines().find_map(|l| {
            let v = l.trim().strip_prefix("QUBERO_SAMPLES_DIR")?.trim_start();
            Some(v.strip_prefix('=')?.trim().trim_matches('"').to_string())
        })
    });
    let dir = match from_env {
        Some(p) if Path::new(&p).is_absolute() => PathBuf::from(p),
        Some(p) => root.join(p),
        None => root.join("../qubero-samples"),
    };
    dir.is_dir().then_some(dir)
}

#[test]
fn real_d500x_recording() {
    let Some(dir) = qubero_samples() else {
        eprintln!("SKIP real_d500x_recording: qubero-samples not found (set QUBERO_SAMPLES_DIR)");
        return;
    };
    let path = dir.join("wav/xc1060673-kuhls-pipistrelle-data-size-short.wav");
    let bytes = std::fs::read(&path).unwrap();
    let e = ok(500_000, 1, 150_000, I16, None);
    let samples = check_both_paths("xc1060673", &bytes, &e);
    // The samples start right after the 980-byte block, and the last one is
    // the last two bytes of the file.
    let first = i16::from_le_bytes([bytes[44 + 980], bytes[44 + 981]]);
    let n = bytes.len();
    let last = i16::from_le_bytes([bytes[n - 2], bytes[n - 1]]);
    assert_eq!(samples[0], first as f32 / 32768.0);
    assert_eq!(*samples.last().unwrap(), last as f32 / 32768.0);

    let h = parse_wav_header(&bytes).unwrap();
    let block = h.details.recorder_block.unwrap();
    let get = |k: &str| {
        block
            .fields
            .iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.as_str())
    };
    assert_eq!(get("FW Version"), Some("D500X V2.2.6 140516, 17:19:14"));
    // D500X note, and the RIFF size that runs 8 bytes past the end.
    assert_eq!(h.details.notes.len(), 2, "{:?}", h.details.notes);
}

#[test]
fn recording_headers_are_located_before_sizes_are_filled_in() {
    use oversample_core::audio::wav::locate_samples;
    // Current layout: 80-byte header with JUNK, sizes still 0.
    let fmt = WavWriteFormat {
        sample_rate: 384_000,
        channels: 1,
        bits_per_sample: 24,
        is_float: false,
    };
    let mut f = header_bytes(&fmt, 0, 0);
    f.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
    assert_eq!(locate_samples(&f), Some((fmt, WRITE_HEADER_LEN as u64)));

    // The 44-byte header written before 0.5.57.
    let mut old = Vec::new();
    old.extend_from_slice(b"RIFF\0\0\0\0WAVEfmt ");
    old.extend_from_slice(&16u32.to_le_bytes());
    old.extend_from_slice(&[1, 0, 1, 0]);
    old.extend_from_slice(&384_000u32.to_le_bytes());
    old.extend_from_slice(&(384_000u32 * 3).to_le_bytes());
    old.extend_from_slice(&[3, 0, 24, 0]);
    old.extend_from_slice(b"data\0\0\0\0");
    assert_eq!(old.len(), 44);
    old.extend_from_slice(&[1, 2, 3]);
    assert_eq!(locate_samples(&old), Some((fmt, 44)));
}
