//! Checks that the Rust port of the encoder in `support/gen.rs` matches the
//! JavaScript encoder, using a render from `tools/chime_render.mjs` in the `batchi-chime`
//! repository, which is beside this one.
//!
//! The fixture was made with this command, from this repository's root:
//!
//! ```text
//! node ../batchi-chime/tools/chime_render.mjs --rate 48000 --unix 1793000000 --tz 660 \
//!     --out crates/chime/tests/fixtures/js_48000_u1793000000_tz660.wav
//! ```
//!
//! That command printed:
//!
//! ```text
//! Payload: 4002df6a9402 (6 bytes, without the CRC)
//! Length: 47520 samples
//! Duration: 0.990000 s
//! ```
//!
//! The encoder appends the CRC b9 fa to that payload, so the message is 8 bytes.

mod support;

use support::generator::{self, Render, TUNE_TIME};

/// The payload bytes chime_render.mjs printed for the fixture.
const JS_PAYLOAD: [u8; 6] = [0x40, 0x02, 0xDF, 0x6A, 0x94, 0x02];
const UNIX: u32 = 1_793_000_000;
const TZ: i16 = 660;

/// The scale chime_render.mjs uses for 16-bit samples.
const INT16_SCALE: f64 = 32767.0;

fn read_fixture() -> (hound::WavSpec, Vec<i16>) {
    let path =
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/js_48000_u1793000000_tz660.wav");
    let mut reader = hound::WavReader::open(path).expect("fixture opens");
    let spec = reader.spec();
    let samples = reader.samples::<i16>().map(|s| s.expect("fixture sample")).collect();
    (spec, samples)
}

/// Finds the start of each symbol in a clean waveform and returns the distance
/// from each start to the next, in samples. The last symbol has no next start,
/// so the result has one entry fewer than there are symbols.
///
/// This reads the waveform directly, using neither gen.rs nor the crate's
/// decoder. A first difference removes most of the melody, and the RMS of the
/// last 3 carrier cycles gives the envelope, which falls to zero between symbols.
/// A symbol start is where the envelope rises through half its peak.
fn symbol_lengths(samples: &[f64], sample_rate: u32) -> Vec<usize> {
    let window = (3.0 * sample_rate as f64 / 18000.0).round() as usize;
    let differences: Vec<f64> = samples.windows(2).map(|pair| pair[1] - pair[0]).collect();
    let envelope: Vec<f64> = differences
        .windows(window)
        .map(|w| (w.iter().map(|d| d * d).sum::<f64>() / window as f64).sqrt())
        .collect();
    let peak = envelope.iter().cloned().fold(0.0, f64::max);
    let threshold = peak / 2.0;
    let starts: Vec<usize> = envelope
        .windows(2)
        .enumerate()
        .filter(|(_, pair)| pair[0] < threshold && pair[1] >= threshold)
        .map(|(index, _)| index)
        .collect();
    starts.windows(2).map(|pair| pair[1] - pair[0]).collect()
}

/// Turns symbol lengths into message bytes. Checks the start and stop symbols and
/// the Hamming codewords on the way.
fn bytes_from_symbol_lengths(lengths: &[usize], sample_rate: u32) -> Vec<u8> {
    const START_SYMBOLS: usize = chime::format::START_SYMBOLS;
    const STOP_SYMBOLS: usize = chime::format::STOP_SYMBOLS;
    const SYMBOLS_PER_BYTE: usize = chime::format::SYMBOLS_PER_BYTE;

    let samples_per_ms = sample_rate as f64 / 1000.0;
    let classify = |length: usize| -> u8 {
        let ms = length as f64 / samples_per_ms;
        if ms < 6.25 {
            0
        } else if ms < 8.75 {
            2
        } else {
            1
        }
    };

    let stop_intervals = STOP_SYMBOLS - 1;
    let data = &lengths[START_SYMBOLS..lengths.len() - stop_intervals];
    assert!(lengths[..START_SYMBOLS].iter().all(|&l| classify(l) == 2), "start symbols");
    assert!(lengths[lengths.len() - stop_intervals..].iter().all(|&l| classify(l) == 2), "stop");
    assert_eq!(data.len() % SYMBOLS_PER_BYTE, 0);

    data.chunks(SYMBOLS_PER_BYTE)
        .map(|symbols| {
            let mut words = [0u8; 2];
            for (index, &length) in symbols.iter().enumerate() {
                let bit = classify(length);
                assert!(bit < 2, "data symbol {index} has the length of a start symbol");
                words[index % 2] |= bit << (index / 2);
            }
            let (low, low_corrected) = chime::format::hamming_decode(words[0]);
            let (high, high_corrected) = chime::format::hamming_decode(words[1]);
            assert!(!low_corrected && !high_corrected, "a clean waveform needs no correction");
            low | (high << 4)
        })
        .collect()
}

/// The duration of a chime in seconds according to the format: start and stop
/// symbols of 7.5 ms, data symbols of 5 ms or 10 ms.
fn predicted_seconds(payload: &[u8]) -> f64 {
    let mut message = payload.to_vec();
    message.extend_from_slice(&chime::format::crc16(payload).to_le_bytes());
    let data_microseconds: u64 = generator::encode(&message)
        .iter()
        .map(|&bit| {
            (if bit == 1 { chime::format::DATA_1_US } else { chime::format::DATA_0_US }) as u64
        })
        .sum();
    let framing_microseconds = ((chime::format::START_SYMBOLS + chime::format::STOP_SYMBOLS)
        as u64)
        * chime::format::START_STOP_US as u64;
    (data_microseconds + framing_microseconds) as f64 / 1e6
}

fn render_fixture_message(sample_rate: u32) -> Render {
    generator::render(sample_rate, &generator::time_payload(UNIX, TZ), TUNE_TIME)
}

#[test]
fn rust_port_matches_the_fixture_within_one_lsb() {
    let (spec, fixture) = read_fixture();
    assert_eq!(spec.sample_rate, 48000);
    assert_eq!(spec.channels, 1);
    assert_eq!(spec.bits_per_sample, 16);

    let rendered = render_fixture_message(48000).mix();
    assert_eq!(rendered.len(), fixture.len(), "a length difference means a loop bound is wrong");

    let mut worst: f64 = 0.0;
    for (index, (&expected, &ours)) in fixture.iter().zip(&rendered).enumerate() {
        let error = (expected as f64 - ours * INT16_SCALE).abs();
        worst = worst.max(error);
        assert!(error <= 1.0, "sample {index}: fixture {expected}, port {}", ours * INT16_SCALE);
    }
    println!("Largest difference from the fixture: {worst:.3} LSB of 16-bit");
}

#[test]
fn payload_matches_what_the_javascript_printed() {
    assert_eq!(generator::time_payload(UNIX, TZ), JS_PAYLOAD);
}

#[test]
fn crate_crc_matches_the_port_of_the_javascript_crc() {
    let expected = generator::crc16(&JS_PAYLOAD);
    assert_eq!(chime::format::crc16(&JS_PAYLOAD).to_le_bytes(), expected);
    assert_eq!(chime::format::crc16(b"123456789").to_le_bytes(), generator::crc16(b"123456789"));
}

#[test]
fn fixture_symbols_contain_the_payload_and_crc() {
    let (_, fixture) = read_fixture();
    let samples: Vec<f64> = fixture.iter().map(|&s| s as f64 / INT16_SCALE).collect();

    let bytes = bytes_from_symbol_lengths(&symbol_lengths(&samples, 48000), 48000);

    let crc = chime::format::crc16(&JS_PAYLOAD).to_le_bytes();
    assert_eq!(bytes[..6], JS_PAYLOAD);
    assert_eq!(bytes[6..], crc);
    assert_eq!(bytes[6..], [0xB9, 0xFA]);
}

#[test]
fn fixture_duration_is_what_the_format_predicts() {
    let (_, fixture) = read_fixture();
    let predicted = predicted_seconds(&JS_PAYLOAD);
    assert!((predicted - 0.990).abs() < 1e-9, "predicted {predicted} s");
    let predicted_samples = predicted * 48000.0;
    let difference = fixture.len() as f64 - predicted_samples;
    assert!(difference.abs() <= 2.0, "{} samples, predicted {predicted_samples}", fixture.len());
}

#[test]
fn carrier_and_melody_lengths_and_symbol_starts_agree() {
    let render = render_fixture_message(48000);
    let symbols = 16 + 8 * 14 + 8;
    assert_eq!(render.symbol_starts.len(), symbols);
    assert_eq!(render.symbol_starts[0], 0);
    assert!(render.carrier.len().abs_diff(render.melody.len()) <= 8);
}

#[test]
fn longer_payload_layouts_and_tunes() {
    let mut payload = generator::time_payload(UNIX, TZ);
    payload.extend_from_slice(&generator::location_bytes(51.5, -0.1));
    assert_eq!(
        payload,
        [0x40, 0x02, 0xDF, 0x6A, 0x94, 0x02, 0xE0, 0xD3, 0x11, 0x03, 0xCB, 0xF3, 0xFF],
        "bytes printed by chime_render.mjs --location 51.5,-0.1"
    );

    let mut payload = generator::time_payload(UNIX, TZ);
    payload.extend_from_slice(&generator::deployment_bytes([
        0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF,
    ]));
    assert_eq!(
        payload,
        [0x40, 0x02, 0xDF, 0x6A, 0x94, 0x02, 0xEF, 0xCD, 0xAB, 0x89, 0x67, 0x45, 0x23, 0x01],
        "bytes printed by chime_render.mjs --deployment 0123456789abcdef"
    );

    assert_eq!(generator::tune_for_payload(6).len(), 4);
    assert_eq!(generator::tune_for_payload(13).len(), 7);
    assert_eq!(generator::tune_for_payload(14).len(), 8);
    assert_eq!(generator::tune_for_payload(21).len(), 12);
}

/// Compares the port with another render from chime_render.mjs, which is not kept
/// in the repository. Set CHIME_JS_WAV to its path. If the render has a different
/// payload from the fixture, also set CHIME_JS_PAYLOAD to the hex that
/// chime_render.mjs printed. Then run:
///
/// ```text
/// CHIME_JS_WAV=file.wav cargo test --release -p chime --test generator -- --ignored --nocapture
/// ```
#[test]
#[ignore = "needs a render from chime_render.mjs, named by CHIME_JS_WAV"]
fn rust_port_matches_another_js_render() {
    let path = std::env::var("CHIME_JS_WAV").expect("CHIME_JS_WAV is set");
    let mut reader = hound::WavReader::open(&path).expect("render opens");
    let spec = reader.spec();
    let fixture: Vec<f64> = match spec.sample_format {
        hound::SampleFormat::Int => {
            reader.samples::<i16>().map(|s| s.unwrap() as f64 / INT16_SCALE).collect()
        }
        hound::SampleFormat::Float => reader.samples::<f32>().map(|s| s.unwrap() as f64).collect(),
    };

    let render = match std::env::var("CHIME_JS_PAYLOAD") {
        Ok(hex) => {
            let payload: Vec<u8> = (0..hex.len() / 2)
                .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).expect("hex payload"))
                .collect();
            generator::render(spec.sample_rate, &payload, generator::tune_for_payload(payload.len()))
        }
        Err(_) => render_fixture_message(spec.sample_rate),
    };
    let rendered = render.mix();
    println!(
        "{path}: {} Hz, JS {} samples, port {} samples (carrier {}, melody {})",
        spec.sample_rate,
        fixture.len(),
        rendered.len(),
        render.carrier.len(),
        render.melody.len()
    );
    assert_eq!(rendered.len(), fixture.len());

    let worst = fixture
        .iter()
        .zip(&rendered)
        .map(|(js, ours)| (js - ours).abs())
        .fold(0.0, f64::max);
    println!("Largest difference: {worst:.3e}, or {:.3} LSB of 16-bit", worst * INT16_SCALE);
    assert!(worst * INT16_SCALE <= 1.0);
}
