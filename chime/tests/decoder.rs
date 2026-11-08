//! Tests of the chime decoder on rendered chimes and simulated channels.
//!
//! `cargo test --release -p chime --test decoder -- --ignored --nocapture` also prints
//! a table of decoding success against SNR, for use when changing the decoder's
//! thresholds.

mod support;

use std::f64::consts::PI;
use std::mem::MaybeUninit;

use chime::format::{self, TimeMessage};
use chime::{Chime, Decoder, Event, Unreadable, UnreadableReason};
use support::channel::{self, ReverbProfile};
use support::generator::{self, TUNE_TIME};

const UNIX: u32 = 1_793_000_000;
const TZ: i16 = 660;

/// Half a second of padding is enough for the noise floor estimate.
const PAD_SECONDS: f64 = 0.5;

fn run(rate: u32, samples: &[f32]) -> Vec<Event> {
    let mut decoder = Decoder::new(rate).unwrap();
    samples.iter().filter_map(|&sample| decoder.push(sample)).collect()
}

fn time_payload() -> Vec<u8> {
    generator::time_payload(UNIX, TZ)
}

fn location_payload() -> Vec<u8> {
    let mut payload = time_payload();
    payload.extend_from_slice(&generator::location_bytes(-33.8688, 151.2093));
    payload
}

fn deployment_payload() -> Vec<u8> {
    let mut payload = time_payload();
    payload.extend_from_slice(&generator::deployment_bytes([1, 2, 3, 4, 5, 6, 7, 8]));
    payload
}

fn location_and_deployment_payload() -> Vec<u8> {
    let mut payload = location_payload();
    payload.extend_from_slice(&generator::deployment_bytes([1, 2, 3, 4, 5, 6, 7, 8]));
    payload
}

fn mix(rate: u32, payload: &[u8]) -> Vec<f64> {
    generator::render(rate, payload, generator::tune_for_payload(payload.len())).mix()
}

fn pad_samples(rate: u32) -> usize {
    (PAD_SECONDS * rate as f64) as usize
}

fn the_chime(events: &[Event]) -> Chime {
    assert_eq!(events.len(), 1, "expected exactly one event: {events:?}");
    match events[0] {
        Event::Chime(chime) => chime,
        Event::Unreadable(unreadable) => panic!("the chime was not readable: {unreadable:?}"),
    }
}

fn assert_start_within_2_ms(chime: &Chime, rate: u32, true_start: usize) {
    let error_ms = (chime.start_sample as f64 - true_start as f64) * 1000.0 / rate as f64;
    assert!(error_ms.abs() <= 2.0, "start_sample is {error_ms:.2} ms from the true start at {rate} Hz");
}

fn read_fixture_as_f32() -> (u32, Vec<f32>) {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/js_48000_u1793000000_tz660.wav");
    let mut reader = hound::WavReader::open(path).expect("fixture opens");
    let rate = reader.spec().sample_rate;
    let samples = reader.samples::<i16>().map(|s| s.expect("fixture sample") as f32 / 32768.0).collect();
    (rate, samples)
}

/// A carrier with the given symbol lengths in ms and the encoder's envelope and polarity
/// flips, at the encoder's amplitude. For test signals that the encoder does not make.
fn carrier_with_symbol_lengths(rate: u32, lengths_ms: &[f64]) -> Vec<f64> {
    let rate = rate as f64;
    let mut output = Vec::new();
    let mut polarity = 0.25;
    let mut n = 0.0;
    for &length_ms in lengths_ms {
        let samples = (length_ms * 0.001 * rate).round() as usize;
        let ramp = 0.0005 * rate;
        for k in 0..samples {
            let edge = (k as f64 + 1.0).min((samples - k) as f64) / ramp;
            let envelope = (edge.min(1.0) * PI / 2.0).sin().powi(2);
            output.push(polarity * envelope * (2.0 * PI * 18000.0 * n / rate).cos());
            n += 1.0;
        }
        polarity = -polarity;
    }
    output
}

/// The symbol lengths in ms for the given message bytes, CRC included.
fn message_lengths_ms(message: &[u8], stop_symbols: usize) -> Vec<f64> {
    let mut lengths = vec![7.5; 16];
    lengths.extend(generator::encode(message).iter().map(|&bit| if bit == 1 { 10.0 } else { 5.0 }));
    lengths.extend(std::iter::repeat_n(7.5, stop_symbols));
    lengths
}

fn with_crc(payload: &[u8]) -> Vec<u8> {
    let mut message = payload.to_vec();
    message.extend_from_slice(&generator::crc16(payload));
    message
}

// -- The JavaScript encoder's output -------------------------------------------

#[test]
fn js_fixture_decodes_to_the_exact_bytes() {
    let (rate, samples) = read_fixture_as_f32();
    let chime = the_chime(&run(rate, &samples));
    assert_eq!(&chime.payload[..chime.payload_len], &[0x40, 0x02, 0xDF, 0x6A, 0x94, 0x02]);
    assert_eq!(chime.corrected_bits, 0);
    assert_start_within_2_ms(&chime, rate, 0);
    let message = TimeMessage::parse(&chime.payload[..chime.payload_len]).unwrap();
    assert_eq!(message.unix_time, UNIX);
    assert_eq!(message.timezone_minutes, TZ);
}

// -- Sample rates and timing ---------------------------------------------------

#[test]
fn renders_decode_at_all_common_rates() {
    let payload = time_payload();
    for rate in [44_100, 48_000, 96_000, 192_000] {
        let pad = pad_samples(rate);
        let x = channel::pad(mix(rate, &payload), pad, pad);
        let chime = the_chime(&run(rate, &channel::to_f32(&x)));
        assert_eq!(&chime.payload[..chime.payload_len], &payload[..], "{rate} Hz");
        assert_eq!(chime.corrected_bits, 0, "{rate} Hz");
        assert_start_within_2_ms(&chime, rate, pad);
    }
}

#[test]
fn renders_decode_at_rates_that_are_not_multiples_of_8_khz() {
    let payload = time_payload();
    for rate in [40_000, 43_999, 50_000, 88_200, 176_400, 384_000] {
        let pad = pad_samples(rate);
        let x = channel::pad(mix(rate, &payload), pad, pad);
        let chime = the_chime(&run(rate, &channel::to_f32(&x)));
        assert_eq!(&chime.payload[..chime.payload_len], &payload[..], "{rate} Hz");
        assert_start_within_2_ms(&chime, rate, pad);
    }
}

#[test]
fn a_chime_played_at_one_rate_decodes_at_another() {
    let payload = location_and_deployment_payload();
    // The encoder lengthens symbols most at 44.1 kHz, by 11 us for each data 0, and this
    // message lasts more than 2 s. Upsampling by 4 gives the rate of an X8 recording.
    let x = channel::upsample_sinc(mix(44_100, &payload), 4);
    let rate = 176_400;
    let pad = pad_samples(rate);
    let x = channel::pad(x, pad, pad);
    let chime = the_chime(&run(rate, &channel::to_f32(&x)));
    assert_eq!(&chime.payload[..chime.payload_len], &payload[..]);
    assert_start_within_2_ms(&chime, rate, pad);
}

#[test]
fn inverted_polarity_decodes() {
    let payload = time_payload();
    let rate = 48_000;
    let pad = pad_samples(rate);
    let x = channel::invert(channel::pad(mix(rate, &payload), pad, pad));
    let chime = the_chime(&run(rate, &channel::to_f32(&x)));
    assert_eq!(&chime.payload[..chime.payload_len], &payload[..]);
    assert_start_within_2_ms(&chime, rate, pad);
}

#[test]
fn start_sample_is_within_2_ms_with_noise_at_every_rate() {
    let payload = time_payload();
    for rate in [44_100, 48_000, 96_000, 192_000] {
        // 12345 samples at each rate: the start is not on any baseband sample boundary.
        let pad = pad_samples(rate) + 12345;
        let x = channel::pad(mix(rate, &payload), pad, pad);
        let x = channel::add_white_noise(x, rate, 3.0, 11);
        let chime = the_chime(&run(rate, &channel::to_f32(&x)));
        assert_eq!(&chime.payload[..chime.payload_len], &payload[..], "{rate} Hz");
        assert_start_within_2_ms(&chime, rate, pad);
    }
}

#[test]
fn time_at_a_later_sample_comes_from_start_sample() {
    let payload = time_payload();
    for rate in [44_100, 192_000] {
        let pad = pad_samples(rate) + 777;
        let x = channel::pad(mix(rate, &payload), pad, pad);
        let chime = the_chime(&run(rate, &channel::to_f32(&x)));

        let message = TimeMessage::parse(&chime.payload[..chime.payload_len]).unwrap();
        let sample = chime.end_sample + 12_345;
        let seconds_after_start = (sample as i64 - chime.start_sample) as f64 / rate as f64;
        let time = message.unix_time as f64 + seconds_after_start;

        // The encoded time is that of the first sample of the chime.
        let expected = UNIX as f64 + (sample as f64 - pad as f64) / rate as f64;
        assert!((time - expected).abs() < 0.002, "{time} against {expected} at {rate} Hz");
    }
}

// -- Streams with more in them -------------------------------------------------

#[test]
fn several_chimes_in_one_stream_are_each_decoded_once() {
    let rate = 48_000;
    let spacing = 3 * rate as usize;
    let times = [UNIX, UNIX + 3, UNIX + 6, UNIX + 9];
    let mut stream = vec![0.0; spacing * times.len() + rate as usize];
    let mut starts = Vec::new();
    for (i, &time) in times.iter().enumerate() {
        let start = rate as usize / 2 + spacing * i;
        starts.push(start);
        let chime = mix(rate, &generator::time_payload(time, TZ));
        for (slot, value) in stream[start..].iter_mut().zip(&chime) {
            *slot = *value;
        }
    }
    let stream = channel::add_white_noise(stream, rate, 15.0, 5);
    let events = run(rate, &channel::to_f32(&stream));
    assert_eq!(events.len(), times.len(), "{events:?}");
    for ((event, &time), &start) in events.iter().zip(&times).zip(&starts) {
        let Event::Chime(chime) = event else { panic!("{event:?}") };
        let message = TimeMessage::parse(&chime.payload[..chime.payload_len]).unwrap();
        assert_eq!(message.unix_time, time);
        assert_start_within_2_ms(chime, rate, start);
    }
}

/// `push_block` gives the same events as `push`, at the same samples, whatever the block
/// lengths.
#[test]
fn blocks_give_the_same_events_as_single_samples() {
    let rate = 192_000;
    let spacing = 4 * rate as usize;
    let mut stream = vec![0.0; 3 * spacing];
    for (i, time) in [UNIX, UNIX + 4].into_iter().enumerate() {
        let chime = mix(rate, &generator::time_payload(time, TZ));
        let start = rate as usize / 2 + spacing * i;
        for (slot, value) in stream[start..].iter_mut().zip(&chime) {
            *slot = *value * 0.01;
        }
    }
    let samples = channel::to_f32(&channel::add_white_noise(stream, rate, 0.0, 9));

    let mut decoder = Decoder::new(rate).unwrap();
    let one_at_a_time: Vec<(usize, Event)> =
        samples.iter().enumerate().filter_map(|(i, &x)| decoder.push(x).map(|event| (i, event))).collect();
    assert_eq!(one_at_a_time.len(), 2, "{one_at_a_time:?}");

    for lengths in [&[1024][..], &[1, 23, 24, 25, 1000], &[7]] {
        let mut decoder = Decoder::new(rate).unwrap();
        let mut events = Vec::new();
        let mut at = 0;
        for &length in lengths.iter().cycle() {
            if at == samples.len() {
                break;
            }
            let block = &samples[at..(at + length).min(samples.len())];
            decoder.push_block(block, |i, event| events.push((at + i, event)));
            at += block.len();
        }
        assert_eq!(events, one_at_a_time, "blocks of {lengths:?}");
    }
}

#[test]
fn a_chime_soon_after_another_is_read_while_the_echoes_are_still_fading() {
    // The recorder setup page leaves 1 s between chimes. With an RT60 of 500 ms and a low
    // noise floor, the echoes of the first chime have not faded when the second one starts.
    let rate = 48_000;
    for gap_seconds in [1.0, 0.5] {
        let first = mix(rate, &generator::time_payload(UNIX, TZ));
        let second = mix(rate, &generator::time_payload(UNIX + 3, TZ));
        let second_start = first.len() + (gap_seconds * rate as f64) as usize;
        let mut chimes = vec![0.0; second_start + second.len()];
        chimes[..first.len()].copy_from_slice(&first);
        chimes[second_start..].copy_from_slice(&second);
        let x = padded(rate, channel::reverb(chimes, rate, ReverbProfile::Large));
        let x = channel::add_white_noise_rms(x, 1e-5, 41);
        let events = run(rate, &channel::to_f32(&x));
        let read: Vec<u32> = events
            .iter()
            .filter_map(|event| match event {
                Event::Chime(chime) => TimeMessage::parse(&chime.payload[..chime.payload_len]).map(|m| m.unix_time),
                Event::Unreadable(_) => None,
            })
            .collect();
        assert_eq!(read, [UNIX, UNIX + 3], "gap {gap_seconds} s: {events:?}");
    }
}

#[test]
fn a_chime_starting_at_sample_0_decodes() {
    let payload = time_payload();
    let rate = 48_000;
    let x = channel::pad(mix(rate, &payload), 0, rate as usize);
    let chime = the_chime(&run(rate, &channel::to_f32(&x)));
    assert_eq!(&chime.payload[..chime.payload_len], &payload[..]);
    assert_start_within_2_ms(&chime, rate, 0);
}

#[test]
fn a_chime_starting_after_10_s_of_noise_decodes() {
    let payload = time_payload();
    let rate = 48_000;
    let pad = 10 * rate as usize;
    let x = channel::pad(mix(rate, &payload), pad, rate as usize / 2);
    let x = channel::add_white_noise(x, rate, 6.0, 21);
    let chime = the_chime(&run(rate, &channel::to_f32(&x)));
    assert_eq!(&chime.payload[..chime.payload_len], &payload[..]);
    assert_start_within_2_ms(&chime, rate, pad);
    assert!((chime.snr_db - 6.0).abs() < 2.5, "snr_db {}", chime.snr_db);
}

#[test]
fn all_payload_lengths_from_the_connector_decode() {
    let rate = 48_000;
    let pad = pad_samples(rate);
    let payloads =
        [time_payload(), location_payload(), deployment_payload(), location_and_deployment_payload()];
    for (payload, expected_len) in payloads.iter().zip([6, 13, 14, 21]) {
        assert_eq!(payload.len(), expected_len);
        let x = channel::pad(mix(rate, payload), pad, pad);
        let chime = the_chime(&run(rate, &channel::to_f32(&x)));
        assert_eq!(&chime.payload[..chime.payload_len], &payload[..]);
        assert_eq!(chime.corrected_bits, 0, "payload of {expected_len} bytes");
        assert!(TimeMessage::parse(&chime.payload[..chime.payload_len]).is_some());
    }
}

#[test]
fn a_crc_match_with_another_length_is_not_a_chime() {
    let rate = 48_000;
    for payload_len in [5, 7, 10, 12, 15, 20] {
        let payload: Vec<u8> = (1..=payload_len).collect();
        let carrier = carrier_with_symbol_lengths(rate, &message_lengths_ms(&with_crc(&payload), 8));
        let x = channel::pad(carrier, pad_samples(rate), pad_samples(rate));
        let events = run(rate, &channel::to_f32(&x));
        assert!(!events.iter().any(|event| matches!(event, Event::Chime(_))), "{payload_len} bytes: {events:?}");
    }
}

/// A chime with a good CRC, and a time zone that the encoder clamps away.
#[test]
fn a_crc_match_with_a_time_zone_out_of_range_is_not_a_chime() {
    let rate = 48_000;
    for time_zone in [30_000i16, -30_000, 841, -721] {
        // The generator clamps the time zone, as the encoder does.
        let mut payload = time_payload();
        payload[4..6].copy_from_slice(&time_zone.to_le_bytes());
        let carrier = carrier_with_symbol_lengths(rate, &message_lengths_ms(&with_crc(&payload), 8));
        let x = channel::pad(carrier, pad_samples(rate), pad_samples(rate));
        let unreadable = the_unreadable(&run(rate, &channel::to_f32(&x)));
        assert_eq!(unreadable.reason, UnreadableReason::MessageRefused, "time zone {time_zone}");
        assert_eq!(unreadable.bytes_received, 8);
    }
}

/// Symbol lengths for a message in which one bit of each of the first `flipped` codewords
/// is wrong, so that the Hamming code corrects them all and the CRC still passes.
fn lengths_with_flipped_codewords(message: &[u8], flipped: usize) -> Vec<f64> {
    let mut bits = generator::encode(message);
    for word in 0..flipped {
        // Codewords alternate bit by bit, and each byte has two, of 7 bits.
        bits[word / 2 * 14 + word % 2] ^= 1;
    }
    let mut lengths = vec![7.5; 16];
    lengths.extend(bits.iter().map(|&bit| if bit == 1 { 10.0 } else { 5.0 }));
    lengths.extend(std::iter::repeat_n(7.5, 8));
    lengths
}

#[test]
fn a_message_with_too_many_corrected_words_is_refused_and_says_so() {
    let rate = 48_000;
    let message = with_crc(&time_payload());
    let render = |flipped: usize| {
        let carrier = carrier_with_symbol_lengths(rate, &lengths_with_flipped_codewords(&message, flipped));
        channel::to_f32(&channel::pad(carrier, pad_samples(rate), pad_samples(rate)))
    };
    let chime = the_chime(&run(rate, &render(10)));
    assert_eq!(chime.payload[..chime.payload_len], message[..6]);
    assert_eq!(chime.corrected_bits, 10);

    let unreadable = the_unreadable(&run(rate, &render(11)));
    assert_eq!(unreadable.reason, UnreadableReason::MessageRefused);
    assert_eq!(unreadable.bytes_received, 8);
}

// -- Levels --------------------------------------------------------------------

#[test]
fn a_carrier_at_minus_80_dbfs_decodes_over_a_quiet_noise_floor() {
    let payload = time_payload();
    let rate = 48_000;
    let carrier_gain = 1e-4; // -80 dBFS peak.
    let render = generator::render(rate, &payload, TUNE_TIME);
    let x = render.mix_with_gains(carrier_gain, 2.0 * carrier_gain);
    let pad = pad_samples(rate);
    let x = channel::pad(x, pad, pad);
    // -115 dBFS RMS over the whole band from 0 to 24 kHz.
    let x = channel::add_white_noise_rms(x, 10f64.powf(-115.0 / 20.0), 3);
    let chime = the_chime(&run(rate, &channel::to_f32(&x)));
    assert_eq!(&chime.payload[..chime.payload_len], &payload[..]);
    assert_start_within_2_ms(&chime, rate, pad);
    assert!((chime.level_dbfs + 83.4).abs() < 1.0, "level_dbfs {}", chime.level_dbfs);
}

#[test]
fn level_matches_the_carrier_rms_of_the_channel_simulator() {
    let payload = time_payload();
    let rate = 48_000;
    let pad = pad_samples(rate);
    let x = channel::pad(mix(rate, &payload), pad, pad);
    let expected = channel::db(channel::carrier_rms(&x, rate)) as f32;
    let chime = the_chime(&run(rate, &channel::to_f32(&x)));
    assert!((chime.level_dbfs - expected).abs() < 0.3, "{} against {expected}", chime.level_dbfs);
}

#[test]
fn a_full_scale_carrier_reads_about_minus_3_dbfs() {
    let payload = time_payload();
    let rate = 48_000;
    let render = generator::render(rate, &payload, TUNE_TIME);
    let x = channel::pad(render.mix_with_gains(1.0, 0.0), pad_samples(rate), pad_samples(rate));
    let chime = the_chime(&run(rate, &channel::to_f32(&x)));
    assert!((chime.level_dbfs + 3.4).abs() < 0.5, "level_dbfs {}", chime.level_dbfs);
}

#[test]
fn samples_above_full_scale_decode() {
    let payload = time_payload();
    let rate = 48_000;
    let x = channel::pad(mix(rate, &payload), pad_samples(rate), pad_samples(rate));
    let x = channel::gain_db(x, 24.0);
    let chime = the_chime(&run(rate, &channel::to_f32(&x)));
    assert_eq!(&chime.payload[..chime.payload_len], &payload[..]);
    assert!((chime.level_dbfs - 8.6).abs() < 0.5, "level_dbfs {}", chime.level_dbfs);
}

#[test]
fn snr_reads_close_to_the_snr_in_the_2_khz_band() {
    let payload = time_payload();
    let rate = 48_000;
    let pad = pad_samples(rate);
    for snr_db in [0.0, 10.0, 20.0, 30.0] {
        let x = channel::pad(mix(rate, &payload), pad, pad);
        let x = channel::add_white_noise(x, rate, snr_db, 31);
        let chime = the_chime(&run(rate, &channel::to_f32(&x)));
        assert!((chime.snr_db as f64 - snr_db).abs() < 1.5, "{} at {snr_db} dB", chime.snr_db);
    }
}

/// The noise is not known when a chime starts too soon after the first sample to measure
/// it, and digital silence has none to measure. A number here would be made up.
#[test]
fn snr_is_nan_when_the_noise_was_not_measured() {
    let payload = time_payload();
    let rate = 48_000;
    let clean = mix(rate, &payload);

    for pad_ms in [0, 20, 40] {
        let pad = rate as usize * pad_ms / 1000;
        let x = channel::add_white_noise(channel::pad(clean.clone(), pad, rate as usize), rate, 10.0, 51);
        let chime = the_chime(&run(rate, &channel::to_f32(&x)));
        assert!(chime.snr_db.is_nan(), "{pad_ms} ms: {chime:?}");
    }

    let x = channel::pad(clean.clone(), pad_samples(rate), pad_samples(rate));
    let chime = the_chime(&run(rate, &channel::to_f32(&x)));
    assert!(chime.snr_db.is_nan(), "after digital silence: {chime:?}");

    let x = channel::add_white_noise(channel::pad(clean, rate as usize / 10, rate as usize), rate, 10.0, 52);
    let chime = the_chime(&run(rate, &channel::to_f32(&x)));
    assert!((chime.snr_db - 10.0).abs() < 3.0, "after 100 ms of noise: {chime:?}");
}

/// A real noise floor far below -120 dBFS in the band still gives the true SNR.
#[test]
fn snr_is_right_with_a_very_quiet_carrier_and_noise() {
    let payload = time_payload();
    let rate = 48_000;
    let carrier_gain = 2e-5; // -94 dBFS peak.
    let render = generator::render(rate, &payload, TUNE_TIME);
    let x = render.mix_with_gains(carrier_gain, 2.0 * carrier_gain);
    let pad = pad_samples(rate);
    let x = channel::add_white_noise(channel::pad(x, pad, pad), rate, 30.0, 53);
    let chime = the_chime(&run(rate, &channel::to_f32(&x)));
    assert!((chime.snr_db - 30.0).abs() < 2.0, "{chime:?}");
}

#[test]
fn band_level_reads_a_tone_and_white_noise() {
    let rate = 48_000u32;
    let mut decoder = Decoder::new(rate).unwrap();
    for n in 0..(3 * rate) {
        let t = n as f64 / rate as f64;
        decoder.push((0.5 * (2.0 * PI * 18_100.0 * t).sin()) as f32);
    }
    // An amplitude of 0.5 is -9 dBFS RMS.
    assert!((decoder.band_level_dbfs() + 9.03).abs() < 0.5, "tone: {}", decoder.band_level_dbfs());

    // White noise has 1/12 of its power in the 2 kHz band out of 24 kHz.
    let mut decoder = Decoder::new(rate).unwrap();
    let noise = channel::add_white_noise_rms(vec![0.0; 3 * rate as usize], 0.1, 17);
    for sample in channel::to_f32(&noise) {
        decoder.push(sample);
    }
    let expected = -20.0 - 10.0 * 12f32.log10();
    assert!((decoder.band_level_dbfs() - expected).abs() < 1.0, "noise: {}", decoder.band_level_dbfs());

    let mut decoder = Decoder::new(rate).unwrap();
    decoder.push(0.0);
    assert!(decoder.band_level_dbfs() <= -100.0);
}

// -- Signals that are not chimes -----------------------------------------------

#[test]
fn a_steady_18_khz_tone_gives_no_events() {
    let rate = 48_000u32;
    let tone: Vec<f64> =
        (0..10 * rate).map(|n| 0.25 * (2.0 * PI * 18_000.0 * n as f64 / rate as f64).sin()).collect();
    let tone = channel::add_white_noise_rms(tone, 1e-4, 9);
    assert_eq!(run(rate, &channel::to_f32(&tone)), Vec::new());
}

#[test]
fn white_noise_gives_no_events() {
    let rate = 48_000;
    for (scale, seed) in [(0.01, 1), (1e-5, 2), (3.0, 3)] {
        let noise = channel::add_white_noise_rms(vec![0.0; 60 * rate as usize], scale, seed);
        assert_eq!(run(rate, &channel::to_f32(&noise)), Vec::new(), "RMS level {scale}");
    }
}

#[test]
fn digital_silence_gives_no_events() {
    assert_eq!(run(48_000, &vec![0.0; 480_000]), Vec::new());
}

/// The library's `tone` function: alternating symbols of 10 ms and 5 ms, with the melody.
#[test]
fn tone_mode_gives_no_chime() {
    let rate = 48_000u32;
    let lengths: Vec<f64> = [10.0, 5.0].iter().copied().cycle().take(2 * 5000 / 15).collect();
    let mut x = carrier_with_symbol_lengths(rate, &lengths);
    for (n, sample) in x.iter_mut().enumerate() {
        *sample += 0.5 * (2.0 * PI * 523.0 * n as f64 / rate as f64).sin();
    }
    let x = channel::add_white_noise_rms(channel::pad(x, 24_000, 24_000), 1e-3, 4);
    let events = run(rate, &channel::to_f32(&x));
    assert!(!events.iter().any(|event| matches!(event, Event::Chime(_))), "{events:?}");
}

/// A decoder built in place must not depend on what the memory held before, and it must
/// read like one from `Decoder::new`.
#[test]
fn a_decoder_built_in_place_reads_the_same_whatever_the_memory_held() {
    let payload = time_payload();
    let rate = 48_000;
    let x = channel::add_white_noise(channel::pad(mix(rate, &payload), pad_samples(rate), pad_samples(rate)), rate, 0.0, 41);
    let x = channel::to_f32(&x);
    let expected = run(rate, &x);
    assert_eq!(the_chime(&expected).payload[..6], payload[..]);

    for pattern in [0x00u8, 0xFF, 0xA5] {
        let mut slot = Box::new(MaybeUninit::<Decoder>::uninit());
        // SAFETY: writing bytes to memory that this function owns and has not read.
        unsafe { slot.as_mut_ptr().write_bytes(pattern, 1) };
        let decoder = Decoder::init_in_place(&mut slot, rate).unwrap();
        let events: Vec<Event> = x.iter().filter_map(|&sample| decoder.push(sample)).collect();
        assert_eq!(events, expected, "memory filled with {pattern:#x}");
    }
}

#[test]
fn a_decoder_can_be_rebuilt_in_place_at_another_rate() {
    let payload = time_payload();
    let mut slot = Box::new(MaybeUninit::<Decoder>::uninit());
    assert!(Decoder::init_in_place(&mut slot, 39_999).is_none());
    for rate in [48_000, 192_000, 44_100] {
        let x = channel::pad(mix(rate, &payload), pad_samples(rate), pad_samples(rate));
        let decoder = Decoder::init_in_place(&mut slot, rate).unwrap();
        let events: Vec<Event> = channel::to_f32(&x).iter().filter_map(|&sample| decoder.push(sample)).collect();
        assert_eq!(the_chime(&events).payload[..6], payload[..], "{rate} Hz");
    }
}

#[test]
fn rates_below_40_khz_give_no_decoder() {
    assert!(Decoder::new(39_999).is_none());
    assert!(Decoder::new(32_000).is_none());
    assert!(Decoder::new(0).is_none());
    assert!(Decoder::new(40_000).is_some());
}

// -- Failed chimes -------------------------------------------------------------

fn the_unreadable(events: &[Event]) -> Unreadable {
    assert_eq!(events.len(), 1, "expected exactly one event: {events:?}");
    match events[0] {
        Event::Unreadable(unreadable) => unreadable,
        Event::Chime(chime) => panic!("expected an unreadable chime: {chime:?}"),
    }
}

#[test]
fn a_chime_cut_off_in_its_data_is_signal_lost_and_the_next_chime_decodes() {
    let payload = time_payload();
    let rate = 48_000;
    let pad = pad_samples(rate);
    let whole = mix(rate, &payload);
    let cut = channel::pad(whole[..whole.len() / 2].to_vec(), pad, 2 * rate as usize);
    let second_start = cut.len();
    let mut x = cut;
    x.extend_from_slice(&whole);
    x.extend(std::iter::repeat_n(0.0, pad));
    let x = channel::add_white_noise_rms(x, 1e-4, 8);
    let events = run(rate, &channel::to_f32(&x));
    assert_eq!(events.len(), 2, "{events:?}");
    let unreadable = the_unreadable(&events[..1]);
    assert_eq!(unreadable.reason, UnreadableReason::SignalLost);
    assert!((2..=5).contains(&unreadable.bytes_received), "{unreadable:?}");
    assert_eq!(the_chime(&events[1..]).payload[..6], payload[..]);
    assert_start_within_2_ms(&the_chime(&events[1..]), rate, second_start);
}

/// A recording that starts during the start symbols: the start of the chime is before the
/// first sample, and the time in the chime belongs to that moment.
#[test]
fn a_chime_that_started_before_the_first_sample_has_a_negative_start() {
    let payload = time_payload();
    let rate = 192_000usize;
    let clean = mix(rate as u32, &payload);
    let symbol = rate * 75 / 10_000;
    for kept_symbols in [15, 8, 4] {
        let dropped = (16 - kept_symbols) * symbol;
        let x = channel::pad(clean[dropped..].to_vec(), 0, rate);
        let x = channel::add_white_noise(x, rate as u32, 20.0, 61);
        let chime = the_chime(&run(rate as u32, &channel::to_f32(&x)));
        assert_eq!(chime.payload[..chime.payload_len], payload[..]);
        let error_ms = (chime.start_sample + dropped as i64) as f64 * 1000.0 / rate as f64;
        assert!(error_ms.abs() <= 2.0, "{kept_symbols} start symbols kept: {error_ms} ms off");
    }
}

/// The time after a chime is cut off is not part of what was received. The walk goes on
/// for about 240 ms after the cut before it counts the signal as lost.
#[test]
fn a_chime_cut_off_reports_what_arrived_before_the_cut() {
    let rate = 192_000u32;
    let start_symbols_ms = 120.0;
    let cases: [(&[u8], f64); 4] = [
        // Payload, and the milliseconds of data symbols to keep.
        (&[0xD8, 0x1D, 0xF8, 0x74, 0x00, 0x00], 0.0),
        (&[0xD8, 0x1D, 0xF8, 0x74, 0x00, 0x00], 60.0),
        (&[0xD8, 0x1D, 0xF8, 0x74, 0x00, 0x00], 400.0),
        (&location_and_deployment_payload(), 900.0),
    ];
    for (payload, data_ms) in cases {
        let message = with_crc(payload);
        let lengths = message_lengths_ms(&message, 8);
        // The bytes that the kept part holds.
        let mut bytes_sent = 0;
        let mut elapsed = 0.0;
        for (symbol, length) in lengths[16..lengths.len() - 8].iter().enumerate() {
            elapsed += length;
            if elapsed > data_ms {
                break;
            }
            bytes_sent = (symbol + 1) / 14;
        }
        let whole = carrier_with_symbol_lengths(rate, &lengths);
        let keep = ((start_symbols_ms + data_ms) * 0.001 * rate as f64) as usize;
        let expected = channel::db(channel::carrier_rms(&whole[..keep], rate)) as f32;
        let x = channel::pad(whole[..keep].to_vec(), pad_samples(rate), rate as usize);
        let x = channel::add_white_noise_rms(x, 1e-4, 62);
        let unreadable = the_unreadable(&run(rate, &channel::to_f32(&x)));
        assert_eq!(unreadable.reason, UnreadableReason::SignalLost, "{data_ms} ms of data");
        assert!(
            unreadable.bytes_received.abs_diff(bytes_sent) <= 1,
            "{data_ms} ms of data: {bytes_sent} bytes sent, {unreadable:?}"
        );
        assert!(
            (unreadable.level_dbfs - expected).abs() <= 1.0,
            "{data_ms} ms of data: level should be {expected}, {unreadable:?}"
        );
    }
}

/// An estimate of the noise left high by a loud sound must not count a quieter chime as
/// lost. The estimate has to follow the input down, however quiet it gets.
#[test]
fn a_loud_sound_does_not_hide_quiet_chimes_after_it() {
    for rate in [48_000u32, 192_000] {
        for gap_noise_rms in [0.0, 1e-7, 1e-6] {
            let chime = channel::gain_db(mix(rate, &time_payload()), -20.0);
            let mut x = channel::add_white_noise_rms(vec![0.0; rate as usize / 2], 0.17, 71);
            for _ in 0..6 {
                x.extend(std::iter::repeat_n(0.0, rate as usize));
                x.extend_from_slice(&chime);
            }
            x.extend(std::iter::repeat_n(0.0, rate as usize));
            let loud_end = rate as usize / 2;
            let mut tail = x.split_off(loud_end);
            if gap_noise_rms > 0.0 {
                tail = channel::add_white_noise_rms(tail, gap_noise_rms, 72);
            }
            x.extend_from_slice(&tail);
            let events = run(rate, &channel::to_f32(&x));
            let read = events.iter().filter(|event| matches!(event, Event::Chime(_))).count();
            assert_eq!(read, 6, "{rate} Hz, gap noise {gap_noise_rms}: {events:?}");
        }
    }
}

#[test]
fn a_wrong_crc_is_bad_crc() {
    let rate = 48_000;
    let mut message = with_crc(&time_payload());
    let last = message.len() - 1;
    message[last] ^= 0x10;
    let carrier = carrier_with_symbol_lengths(rate, &message_lengths_ms(&message, 8));
    let x = channel::pad(carrier, pad_samples(rate), pad_samples(rate));
    let unreadable = the_unreadable(&run(rate, &channel::to_f32(&x)));
    assert_eq!(unreadable.reason, UnreadableReason::BadCrc);
    assert_eq!(unreadable.bytes_received, message.len());
    assert_start_within_2_ms(
        &Chime {
            payload: [0; 21],
            payload_len: 0,
            start_sample: unreadable.start_sample,
            end_sample: 0,
            corrected_bits: 0,
            level_dbfs: 0.0,
            snr_db: 0.0,
        },
        rate,
        pad_samples(rate),
    );
}

#[test]
fn a_message_longer_than_the_format_allows_is_too_many_bytes() {
    let rate = 48_000;
    let message: Vec<u8> = (1..=30).collect();
    let carrier = carrier_with_symbol_lengths(rate, &message_lengths_ms(&message, 8));
    let x = channel::pad(carrier, pad_samples(rate), pad_samples(rate));
    let events = run(rate, &channel::to_f32(&x));
    // The rest of the long message can start another failed walk after the first one.
    let unreadable = the_unreadable(&events[..1]);
    assert_eq!(unreadable.reason, UnreadableReason::TooManyBytes);
    // One more than the longest message, which is the byte that showed the message was
    // too long.
    assert_eq!(unreadable.bytes_received, format::MAX_MESSAGE_BYTES + 1);
    assert!(!events.iter().any(|event| matches!(event, Event::Chime(_))), "{events:?}");
}

#[test]
fn a_message_of_only_zeros_is_not_a_chime() {
    // The CRC of zeros is zero, so the CRC does not protect this message.
    let rate = 48_000;
    let message = [0u8; 8];
    let carrier = carrier_with_symbol_lengths(rate, &message_lengths_ms(&message, 8));
    let x = channel::pad(carrier, pad_samples(rate), pad_samples(rate));
    let events = run(rate, &channel::to_f32(&x));
    assert!(!events.iter().any(|event| matches!(event, Event::Chime(_))), "{events:?}");
}

#[test]
fn nan_and_infinite_samples_do_not_stop_the_decoder() {
    let payload = time_payload();
    // At 192 kHz the front end has two stages, and at 48 kHz one.
    for rate in [48_000u32, 192_000] {
        let mut decoder = Decoder::new(rate).unwrap();
        for _ in 0..1000 {
            assert_eq!(decoder.push(f32::NAN), None);
            assert_eq!(decoder.push(f32::INFINITY), None);
            assert_eq!(decoder.push(f32::NEG_INFINITY), None);
            assert_eq!(decoder.push(3e20), None);
        }
        let x = channel::pad(mix(rate, &payload), pad_samples(rate), pad_samples(rate));
        let events: Vec<Event> = channel::to_f32(&x).iter().filter_map(|&sample| decoder.push(sample)).collect();
        assert_eq!(the_chime(&events).payload[..6], payload[..], "{rate} Hz");
    }
}

// -- The budget ----------------------------------------------------------------

#[test]
fn decoder_is_small() {
    let size = std::mem::size_of::<Decoder>();
    assert!(size <= 20 * 1024, "Decoder is {size} bytes");
}

// -- Decoding through simulated channels ---------------------------------------

/// Number of trials out of `trials` that decode to the exact bytes, one trial for each
/// noise seed. `make_signal` returns the signal without noise, with the padding in it.
fn successes(
    rate: u32,
    payload: &[u8],
    snr_db: f64,
    trials: u64,
    make_signal: impl Fn() -> Vec<f64>,
) -> u64 {
    let clean = make_signal();
    (0..trials)
        .filter(|&seed| {
            let x = channel::add_white_noise(clean.clone(), rate, snr_db, seed + 1000);
            run(rate, &channel::to_f32(&x)).iter().any(|event| {
                matches!(event, Event::Chime(chime) if chime.payload[..chime.payload_len] == *payload)
            })
        })
        .count() as u64
}

fn padded(rate: u32, x: Vec<f64>) -> Vec<f64> {
    channel::pad(x, pad_samples(rate), pad_samples(rate))
}

#[test]
fn decodes_below_0_db_snr() {
    // At -3 dB in the 2 kHz band, AudioMoth's own decoder has stopped working.
    let payload = time_payload();
    let rate = 48_000;
    let good = successes(rate, &payload, -3.0, 20, || padded(rate, mix(rate, &payload)));
    assert!(good >= 19, "{good} of 20");
}

#[test]
fn decodes_with_carrier_frequency_error() {
    let payload = time_payload();
    let rate = 48_000;
    for ppm in [-1000.0, -200.0, 200.0, 1000.0] {
        let good = successes(rate, &payload, 0.0, 6, || {
            padded(rate, channel::rate_error_ppm(mix(rate, &payload), ppm))
        });
        assert_eq!(good, 6, "{ppm} ppm");
    }
}

#[test]
fn decodes_through_reverb() {
    let payload = time_payload();
    let rate = 48_000;
    for (profile, snr_db) in
        [(ReverbProfile::Small, 0.0), (ReverbProfile::Medium, 10.0), (ReverbProfile::Large, 25.0)]
    {
        let good = successes(rate, &payload, snr_db, 6, || {
            padded(rate, channel::reverb(mix(rate, &payload), rate, profile))
        });
        assert_eq!(good, 6, "{profile:?} at {snr_db} dB");
    }
}

#[test]
fn decodes_a_clipped_input() {
    let rate = 48_000;
    for payload in [time_payload(), location_and_deployment_payload()] {
        for gain in [6.0, 20.0] {
            let good = successes(rate, &payload, 20.0, 4, || {
                padded(rate, channel::clip(channel::gain_db(mix(rate, &payload), gain), 0.5))
            });
            assert_eq!(good, 4, "{} bytes, gain {gain} dB", payload.len());
        }
    }
}

/// A table of decoding success against SNR. Not an assertion. It takes about 30 s in a
/// release build.
#[test]
#[ignore]
fn sensitivity_table() {
    let payload = time_payload();
    let rate = 48_000;
    println!("size of Decoder: {} bytes", std::mem::size_of::<Decoder>());
    for snr_db in [3.0, 0.0, -2.0, -3.0, -4.0, -5.0, -6.0, -7.0, -8.0] {
        let trials: u64 = 240;
        let threads = 12;
        let total: u64 = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..threads)
                .map(|thread| {
                    let payload = payload.clone();
                    scope.spawn(move || {
                        let clean = padded(rate, mix(rate, &payload));
                        (0..trials / threads)
                            .filter(|&trial| {
                                let seed = thread * 100_000 + trial;
                                let x = channel::add_white_noise(clean.clone(), rate, snr_db, seed);
                                run(rate, &channel::to_f32(&x)).iter().any(|event| {
                                    matches!(event, Event::Chime(chime) if chime.payload[..chime.payload_len] == *payload)
                                })
                            })
                            .count() as u64
                    })
                })
                .collect();
            handles.into_iter().map(|handle| handle.join().unwrap()).sum()
        });
        println!("{snr_db:5.1} dB in the 2 kHz band: {total} of {trials}");
    }
}
