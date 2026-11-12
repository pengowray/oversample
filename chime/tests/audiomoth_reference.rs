//! Checks the Rust port of AudioMoth's decoder in `support/audiomoth_port.rs`, and
//! uses it to cross-check the format code in `chime::format` and the encoder port in
//! `support/gen.rs` against the firmware that AudioMoth devices run.
//!
//! The port's events have NaN level and SNR fields, so these tests compare events
//! field by field.

mod support;

// `support/mod.rs` does not declare this module yet, so this file includes it by path.
#[path = "support/audiomoth_port.rs"]
mod audiomoth_port;

use audiomoth_port::{
    AudioMothPort, CostasLoop, HAMMING_CONVERSION, SINE_TABLE, Timing, calculate_crc, check_crc,
};
use chime::format::{self, TimeMessage};
use chime::{Chime, Event, Unreadable, UnreadableReason};
use support::channel::{self, Xorshift};
use support::generator::{self, tune_for_payload};

const FIXTURE_PAYLOAD: [u8; 6] = [0x40, 0x02, 0xDF, 0x6A, 0x94, 0x02];

/// The CRC the JavaScript encoder appended to `FIXTURE_PAYLOAD`: b9 fa, low byte first.
const FIXTURE_CRC: u16 = 0xFAB9;

const UNIX: u32 = 1_793_000_000;
const TZ: i16 = 660;

fn read_fixture() -> Vec<f32> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/js_48000_u1793000000_tz660.wav"
    );
    let mut reader = hound::WavReader::open(path).expect("fixture opens");
    assert_eq!(reader.spec().sample_rate, 48_000);
    // Dividing by a power of two keeps the exact value, so the port's multiplication
    // by 32768 gives the Costas loop the same numbers an AudioMoth would see.
    reader
        .samples::<i16>()
        .map(|s| s.expect("fixture sample") as f32 / 32768.0)
        .collect()
}

fn run(sample_rate: u32, samples: &[f32]) -> Vec<Event> {
    let mut port = AudioMothPort::new(sample_rate);
    samples
        .iter()
        .filter_map(|&sample| port.push(sample))
        .collect()
}

/// Asserts that the events are exactly one chime with this payload, and returns it.
fn expect_one_chime(events: &[Event], payload: &[u8], context: &str) -> Chime {
    assert_eq!(events.len(), 1, "{context}: events {events:?}");
    let Event::Chime(chime) = events[0] else {
        panic!("{context}: expected a chime, got {:?}", events[0]);
    };
    assert_eq!(&chime.payload[..chime.payload_len], payload, "{context}");
    chime
}

/// The four payloads the connector can build, with lengths 6, 13, 14 and 21.
fn connector_payloads() -> [Vec<u8>; 4] {
    let time = generator::time_payload(UNIX, TZ);
    let location = generator::location_bytes(-33.8688, 151.2093);
    let deployment = generator::deployment_bytes([0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF]);

    let with_location = [time.as_slice(), &location].concat();
    let with_deployment = [time.as_slice(), &deployment].concat();
    let with_both = [time.as_slice(), &location, &deployment].concat();
    [time, with_location, with_deployment, with_both]
}

/// Renders a chime as the connector would play it, with silence on both sides.
/// Returns the samples and the index of the first start symbol.
fn render_padded(sample_rate: u32, payload: &[u8]) -> (Vec<f32>, usize) {
    let render = generator::render(sample_rate, payload, tune_for_payload(payload.len()));
    let before = sample_rate as usize / 5;
    let padded = channel::pad(render.mix(), before, sample_rate as usize / 5);
    (channel::to_f32(&padded), before + render.symbol_starts[0])
}

// -- Decoding real chimes ----------------------------------------------------

#[test]
fn decodes_the_javascript_fixture() {
    let silence = vec![0.0; 9600];
    let samples = [silence.as_slice(), &read_fixture(), &silence].concat();

    let chime = expect_one_chime(&run(48_000, &samples), &FIXTURE_PAYLOAD, "fixture");

    assert_eq!(
        calculate_crc(&chime.payload[..chime.payload_len]),
        FIXTURE_CRC
    );
    assert_eq!(chime.corrected_bits, 0);

    let message = TimeMessage::parse(&chime.payload[..chime.payload_len]).expect("6 bytes");
    assert_eq!(message.unix_time, UNIX);
    assert_eq!(message.timezone_minutes, TZ);
}

#[test]
fn decodes_all_payload_lengths_from_generator_renders() {
    for sample_rate in [48_000, 192_000] {
        for payload in connector_payloads() {
            let (samples, _) = render_padded(sample_rate, &payload);
            let context = format!("{sample_rate} Hz, {} bytes", payload.len());
            let chime = expect_one_chime(&run(sample_rate, &samples), &payload, &context);
            assert_eq!(chime.corrected_bits, 0, "{context}");
        }
    }
}

#[test]
fn decodes_at_other_sample_rates() {
    let payload = generator::time_payload(UNIX, TZ);
    for sample_rate in [44_100, 96_000] {
        let (samples, _) = render_padded(sample_rate, &payload);
        expect_one_chime(
            &run(sample_rate, &samples),
            &payload,
            &format!("{sample_rate} Hz"),
        );
    }
}

#[test]
fn decodes_inverted_polarity() {
    let payload = generator::time_payload(UNIX, TZ);
    for sample_rate in [48_000, 192_000] {
        let (samples, _) = render_padded(sample_rate, &payload);
        let inverted: Vec<f32> = samples.iter().map(|s| -s).collect();
        expect_one_chime(
            &run(sample_rate, &inverted),
            &payload,
            &format!("{sample_rate} Hz"),
        );
    }
}

/// A pad of a few more samples moves the carrier to another phase at the mixer. The
/// other tests use a pad that starts the Costas loop in quadrature with the carrier,
/// which is the hardest start, so this checks the other phases.
#[test]
fn decodes_at_every_carrier_phase_on_the_sample_grid() {
    let payload = generator::time_payload(UNIX, TZ);
    for (sample_rate, phases) in [(48_000, 8), (192_000, 32)] {
        let render = generator::render(sample_rate, &payload, tune_for_payload(payload.len()));
        for offset in 0..phases {
            let padded = channel::pad(render.mix(), 1000 + offset, sample_rate as usize / 5);
            let context = format!("{sample_rate} Hz, offset {offset}");
            expect_one_chime(
                &run(sample_rate, &channel::to_f32(&padded)),
                &payload,
                &context,
            );
        }
    }
}

/// +10 dB in the 17 to 19 kHz band is well above where this decoder starts to fail,
/// which is about +2 dB, so every chime here should decode at both rates.
#[test]
fn decodes_with_white_noise_at_both_rates() {
    let payload = generator::time_payload(UNIX, TZ);
    for sample_rate in [48_000, 192_000] {
        let render = generator::render(sample_rate, &payload, tune_for_payload(payload.len()));
        let padded = channel::pad(
            render.mix(),
            sample_rate as usize / 5,
            sample_rate as usize / 5,
        );
        for seed in 0..5 {
            let noisy = channel::add_white_noise(padded.clone(), sample_rate, 10.0, 100 + seed);
            let context = format!("{sample_rate} Hz, seed {seed}");
            expect_one_chime(
                &run(sample_rate, &channel::to_f32(&noisy)),
                &payload,
                &context,
            );
        }
    }
}

#[test]
fn start_sample_is_close_to_the_first_start_symbol() {
    for sample_rate in [48_000, 192_000] {
        for payload in connector_payloads() {
            let (samples, true_start) = render_padded(sample_rate, &payload);
            let context = format!("{sample_rate} Hz, {} bytes", payload.len());
            let chime = expect_one_chime(&run(sample_rate, &samples), &payload, &context);

            let error_ms =
                (chime.start_sample as f64 - true_start as f64) * 1000.0 / sample_rate as f64;
            println!("{context}: start_sample error {error_ms:+.2} ms");
            assert!(
                error_ms.abs() < 15.0,
                "{context}: start_sample error {error_ms} ms"
            );
        }
    }
}

// -- The port's assumptions about its input scale ------------------------------

/// The peak of the in-phase channel over the last 50 ms of a 200 ms tone at 18 kHz.
/// The Costas loop takes samples in int16 units, as `AudioMothPort::push` provides.
fn locked_in_phase_peak(sample_rate: u32, amplitude_in_loop_units: f64) -> f32 {
    let mut costas_loop = CostasLoop::new(sample_rate);
    let length = sample_rate as usize / 5;
    let mut peak = 0.0f32;
    for n in 0..length {
        // The phase offset starts the loop away from lock in both directions of
        // the C's sine table index.
        let phase = 2.0 * std::f64::consts::PI * 18_000.0 * n as f64 / sample_rate as f64 + 1.0;
        let output = costas_loop.update((amplitude_in_loop_units * phase.sin()) as f32);
        if n >= length * 3 / 4 {
            peak = peak.max(output.abs());
        }
    }
    peak
}

/// In int16 units, an 18 kHz tone at any level above the AGC floor of 100 gives the
/// same in-phase level. The AGC divides by the mean rectified level, 2/pi of the peak,
/// so the level is pi/2 before the mixer and pi/4 after it. A +-1.0 sample that skipped
/// the port's multiplication by 32768 would be far below the floor.
#[test]
fn agc_floor_applies_in_int16_units() {
    for sample_rate in [48_000, 192_000] {
        let expected = std::f32::consts::FRAC_PI_4;
        for amplitude in [1_000.0, 8_192.0, 30_000.0] {
            let peak = locked_in_phase_peak(sample_rate, amplitude);
            assert!(
                (peak - expected).abs() < 0.05 * expected,
                "{sample_rate} Hz, amplitude {amplitude}: in-phase peak {peak}"
            );
        }

        let unscaled = locked_in_phase_peak(sample_rate, 0.25);
        assert!(
            unscaled < 0.002,
            "{sample_rate} Hz: in-phase peak {unscaled}"
        );
    }
}

/// A carrier with a clock error needs the Costas loop to track it. Below the AGC floor
/// the loop's control signal falls with the square of the level, and a 50 ppm error
/// (0.9 Hz) stops being tracked when the carrier peak is about 28 in int16 units. The
/// gains here put the peak at about 164 and 10.
#[test]
fn clock_error_is_tracked_above_the_agc_floor_and_not_far_below_it() {
    let payload = generator::time_payload(UNIX, TZ);
    for sample_rate in [48_000, 192_000] {
        let render = generator::render(sample_rate, &payload, tune_for_payload(payload.len()));
        let padded = channel::pad(
            render.mix(),
            sample_rate as usize / 5,
            sample_rate as usize / 5,
        );
        let shifted = channel::rate_error_ppm(padded, 50.0);

        let loud = channel::gain_db(shifted.clone(), -34.0);
        let context = format!("{sample_rate} Hz, loud");
        expect_one_chime(
            &run(sample_rate, &channel::to_f32(&loud)),
            &payload,
            &context,
        );

        let quiet = channel::gain_db(shifted, -58.0);
        let events = run(sample_rate, &channel::to_f32(&quiet));
        let decoded = events.iter().any(|event| matches!(event, Event::Chime(_)));
        assert!(!decoded, "{sample_rate} Hz, quiet: events {events:?}");
    }
}

// -- Cross-checks against our format code --------------------------------------

#[test]
fn hamming_decode_agrees_with_the_c_table_on_every_input() {
    for word in 0..128u8 {
        let (nibble, corrected) = format::hamming_decode(word);
        assert_eq!(nibble, HAMMING_CONVERSION[word as usize], "word {word}");

        let is_codeword = (0..7).all(|bit| {
            HAMMING_CONVERSION[(word ^ (1 << bit)) as usize] == HAMMING_CONVERSION[word as usize]
        });
        assert_eq!(corrected, !is_codeword, "word {word}");
    }
}

#[test]
fn hamming_codewords_are_the_ones_the_c_table_implies() {
    for (nibble, &codeword) in format::HAMMING_CODEWORDS.iter().enumerate() {
        assert_eq!(HAMMING_CONVERSION[codeword as usize], nibble as u8);
    }
}

#[test]
fn crc16_agrees_with_the_c_on_random_data() {
    assert_eq!(calculate_crc(b"123456789"), 0x31C3);

    let mut random = Xorshift::new(0x4352_4331);
    for trial in 0..20_000 {
        let length = (random.next_u64() % 41) as usize;
        let bytes: Vec<u8> = (0..length).map(|_| random.next_u64() as u8).collect();
        assert_eq!(
            format::crc16(&bytes),
            calculate_crc(&bytes),
            "trial {trial}: {bytes:02x?}"
        );
    }
}

#[test]
fn crc_of_the_encoder_port_passes_the_c_check() {
    let mut random = Xorshift::new(0x4352_4332);
    for _ in 0..1_000 {
        let length = (random.next_u64() % 22) as usize;
        let mut message: Vec<u8> = (0..length).map(|_| random.next_u64() as u8).collect();
        message.extend_from_slice(&generator::crc16(&message));
        assert!(check_crc(&message), "{message:02x?}");
    }
}

// -- Rate-dependent constants -----------------------------------------------------

#[test]
fn timing_at_48000_is_the_c_constants() {
    let timing = Timing::new(48_000);
    assert_eq!(timing.min_bit, 120);
    assert_eq!(timing.max_bit, 600);
    assert_eq!(timing.low_bit, 240);
    assert_eq!(timing.high_bit, 480);
    assert_eq!(timing.mid_bit, 360);
    assert_eq!(timing.start_stop_bit, 360);
    assert_eq!(timing.tolerance, 60);
}

#[test]
fn timing_at_192000_is_four_times_the_c_constants() {
    let timing = Timing::new(192_000);
    assert_eq!(timing.min_bit, 480);
    assert_eq!(timing.max_bit, 2400);
    assert_eq!(timing.low_bit, 960);
    assert_eq!(timing.high_bit, 1920);
    assert_eq!(timing.mid_bit, 1440);
    assert_eq!(timing.start_stop_bit, 1440);
    assert_eq!(timing.tolerance, 240);
}

#[test]
fn vco_constants_scale_with_the_rate() {
    let at_48000 = CostasLoop::new(48_000);
    assert_eq!(at_48000.phase_increment, 96.0);
    assert_eq!(at_48000.vco_gain, 0.5);

    let at_192000 = CostasLoop::new(192_000);
    assert_eq!(at_192000.phase_increment, 24.0);
    assert_eq!(at_192000.vco_gain, 0.125);
}

#[test]
fn sine_table_has_the_values_the_c_lists() {
    assert_eq!(SINE_TABLE[0], 0.0);
    assert_eq!(SINE_TABLE[64], 1.0);
    assert_eq!(SINE_TABLE[128], -0.000000087423);
    assert_eq!(SINE_TABLE[192], -1.0);
    for (index, &value) in SINE_TABLE.iter().enumerate() {
        let exact = (2.0 * std::f64::consts::PI * index as f64 / 256.0).sin();
        assert!((value as f64 - exact).abs() < 1e-6, "index {index}");
    }
}

// -- Filter design -----------------------------------------------------------------

/// Output amplitude for a steady tone through a filter given as a closure, measured
/// by RMS over the last half of 1 second. A peak would be too low at 48 kHz, where the
/// samples of an 18 kHz tone fall on only 8 points of its cycle.
fn steady_state_amplitude(
    sample_rate: u32,
    frequency: f64,
    mut filter: impl FnMut(f32) -> f32,
) -> f64 {
    let samples = sample_rate as usize;
    let mut sum_of_squares = 0.0f64;
    for n in 0..samples {
        let phase = 2.0 * std::f64::consts::PI * frequency * n as f64 / sample_rate as f64;
        let output = filter(phase.cos() as f32) as f64;
        if n >= samples / 2 {
            sum_of_squares += output * output;
        }
    }
    (2.0 * sum_of_squares / (samples - samples / 2) as f64).sqrt()
}

fn band_pass_gain(sample_rate: u32, frequency: f64) -> f64 {
    let coefficients = audiomoth_port::butterworth_design_band_pass(sample_rate, 17_000, 19_000);
    let [yc0, yc1] = coefficients.yc;
    let (mut x1, mut x2, mut y1, mut y2) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    steady_state_amplitude(sample_rate, frequency, |sample| {
        let x0 = sample * coefficients.gain;
        let y0 = x0 - x2 + yc0 * y2 + yc1 * y1;
        (x2, x1) = (x1, x0);
        (y2, y1) = (y1, y0);
        y0
    })
}

#[test]
fn carrier_filter_has_unit_gain_at_18_khz_and_half_power_at_its_edges() {
    for sample_rate in [48_000, 96_000, 192_000] {
        let centre = band_pass_gain(sample_rate, 18_000.0);
        assert!(
            (centre - 1.0).abs() < 0.001,
            "{sample_rate} Hz: gain {centre}"
        );

        for edge in [17_000.0, 19_000.0] {
            let gain = band_pass_gain(sample_rate, edge);
            let half_power = std::f64::consts::FRAC_1_SQRT_2;
            assert!(
                (gain - half_power).abs() < 0.01,
                "{sample_rate} Hz, {edge} Hz: gain {gain}"
            );
        }
    }
}

#[test]
fn agc_filter_has_unit_gain_at_dc_and_half_power_at_1_khz() {
    for sample_rate in [48_000, 96_000, 192_000] {
        let coefficients = audiomoth_port::butterworth_design_low_pass(sample_rate, 1_000);
        let mut filter = {
            let (mut x, mut y) = (0.0f32, 0.0f32);
            move |sample: f32| {
                let x0 = sample * coefficients.gain;
                y = x + x0 + coefficients.yc[0] * y;
                x = x0;
                y
            }
        };

        let mut output = 0.0;
        for _ in 0..sample_rate {
            output = filter(1.0);
        }
        assert!(
            (output - 1.0).abs() < 0.001,
            "{sample_rate} Hz: DC gain {output}"
        );

        let gain = steady_state_amplitude(sample_rate, 1_000.0, &mut filter);
        let half_power = std::f64::consts::FRAC_1_SQRT_2;
        assert!(
            (gain - half_power).abs() < 0.005,
            "{sample_rate} Hz: gain at 1 kHz {gain}"
        );
    }
}

/// The channel filter is the RBJ low-pass form with a bandwidth of 2 octaves, which
/// gives Q = 2/3 at low frequencies, so its gain at the 100 Hz cutoff is 2/3.
#[test]
fn channel_filter_has_gain_q_at_its_cutoff() {
    for (sample_rate, allowed_error) in [(48_000, 0.002), (192_000, 0.02)] {
        let c = audiomoth_port::biquad_design_low_pass(sample_rate, 100, 2.0);
        let (mut x1, mut x2, mut y1, mut y2) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        let gain = steady_state_amplitude(sample_rate, 100.0, |x0| {
            let y0 = c.b0_a0 * x0 + c.b1_a0 * x1 + c.b2_a0 * x2 - c.a1_a0 * y1 - c.a2_a0 * y2;
            (x2, x1) = (x1, x0);
            (y2, y1) = (y1, y0);
            y0
        });
        println!("channel filter gain at 100 Hz, {sample_rate} Hz: {gain:.4}");
        assert!(
            (gain - 2.0 / 3.0).abs() < allowed_error,
            "{sample_rate} Hz: gain {gain}"
        );
    }
}

/// The DC gain of the channel filter is off by more at 192 kHz, because its
/// coefficients have only f32 precision and its cutoff is 1/1920 of the sample rate.
#[test]
fn channel_filter_has_near_unit_gain_at_dc() {
    for (sample_rate, allowed_error) in [(48_000, 0.001), (96_000, 0.005), (192_000, 0.03)] {
        let c = audiomoth_port::biquad_design_low_pass(sample_rate, 100, 2.0);
        let gain = (c.b0_a0 as f64 + c.b1_a0 as f64 + c.b2_a0 as f64)
            / (1.0 + c.a1_a0 as f64 + c.a2_a0 as f64);
        println!("channel filter DC gain at {sample_rate} Hz: {gain:.5}");
        assert!(
            (gain - 1.0).abs() < allowed_error,
            "{sample_rate} Hz: gain {gain}"
        );
    }
}

// -- State machine, driven with chosen periods -------------------------------------

const LOW: u64 = 240;
const HIGH: u64 = 480;
const START_STOP: u64 = 360;

/// Feeds the state machine a Costas loop output with a zero crossing after each
/// period, and returns the events.
fn feed_periods(periods: &[u64]) -> Vec<Event> {
    let mut port = AudioMothPort::new(48_000);
    let mut events = Vec::new();
    let mut sign = 1.0f32;
    for &period in periods {
        for _ in 0..period {
            events.extend(port.process_costas_loop_output(sign));
        }
        sign = -sign;
    }
    events.extend(port.process_costas_loop_output(sign));
    events
}

fn data_periods(message: &[u8]) -> Vec<u64> {
    generator::encode(message)
        .iter()
        .map(|&bit| if bit == 1 { HIGH } else { LOW })
        .collect()
}

/// The periods of a whole chime: start, payload, CRC, stop.
fn chime_periods(payload: &[u8], start_periods: usize, stop_periods: usize) -> Vec<u64> {
    let message = [payload, &generator::crc16(payload)].concat();
    let mut periods = vec![START_STOP; start_periods];
    periods.extend(data_periods(&message));
    periods.extend(vec![START_STOP; stop_periods]);
    periods
}

fn expect_unreadable(events: &[Event], context: &str) -> Unreadable {
    assert_eq!(events.len(), 1, "{context}: events {events:?}");
    let Event::Unreadable(unreadable) = events[0] else {
        panic!(
            "{context}: expected an unreadable chime, got {:?}",
            events[0]
        );
    };
    unreadable
}

#[test]
fn state_machine_decodes_exact_periods() {
    let events = feed_periods(&chime_periods(&FIXTURE_PAYLOAD, 16, 8));
    let chime = expect_one_chime(&events, &FIXTURE_PAYLOAD, "exact periods");
    assert_eq!(chime.corrected_bits, 0);
    assert_eq!(chime.start_sample, 0);
}

/// The first period enters `START_BITS` without being counted, and the C needs
/// `bitCount > 4 && bitCount < 24` when the first data symbol ends.
#[test]
fn start_run_must_have_6_to_24_periods() {
    for start_periods in 0..30 {
        let events = feed_periods(&chime_periods(&FIXTURE_PAYLOAD, start_periods, 8));
        let decoded = events.iter().any(|event| matches!(event, Event::Chime(_)));
        // The stop periods of the chime can start a new run, but no data follows it.
        assert_eq!(
            decoded,
            (6..=24).contains(&start_periods),
            "{start_periods} start periods"
        );
    }
}

/// The C finishes at the fourth stop period, and it needs all four.
#[test]
fn message_ends_at_the_fourth_stop_period() {
    let payload = FIXTURE_PAYLOAD;
    assert!(feed_periods(&chime_periods(&payload, 16, 3)).is_empty());

    let periods = chime_periods(&payload, 16, 4);
    let chime = expect_one_chime(&feed_periods(&periods), &payload, "4 stop periods");
    assert_eq!(chime.end_sample, periods.iter().sum::<u64>());
}

/// With 23 bytes the C checks the CRC at the last byte, so it needs no stop periods.
#[test]
fn message_of_23_bytes_ends_without_stop_periods() {
    let payload: Vec<u8> = (0..21).map(|i| 0x35 + 7 * i as u8).collect();
    let periods = chime_periods(&payload, 16, 0);
    let chime = expect_one_chime(&feed_periods(&periods), &payload, "23 bytes");
    assert_eq!(chime.end_sample, periods.iter().sum::<u64>());
}

/// A message of 22 or more payload bytes reaches the limit before its CRC, so the
/// C reports a CRC error at the 23rd byte. It does not report too many bytes.
#[test]
fn message_over_the_limit_is_a_crc_error_at_byte_23() {
    let payload = [0x5Au8; 30];
    let unreadable =
        expect_unreadable(&feed_periods(&chime_periods(&payload, 16, 8)), "over limit");
    assert_eq!(unreadable.reason, UnreadableReason::BadCrc);
    assert_eq!(unreadable.bytes_received, 23);
}

/// The C checks for stop periods only after more than 2 bytes, so 2 bytes followed
/// by stop periods are not a message.
#[test]
fn no_message_ends_after_only_2_bytes() {
    assert!(feed_periods(&chime_periods(&[], 16, 8)).is_empty());
}

#[test]
fn hamming_corrects_one_wrong_symbol() {
    let mut periods = chime_periods(&FIXTURE_PAYLOAD, 16, 8);
    let symbol = 16 + 3 * 14 + 5;
    periods[symbol] = if periods[symbol] == LOW { HIGH } else { LOW };

    let chime = expect_one_chime(
        &feed_periods(&periods),
        &FIXTURE_PAYLOAD,
        "one wrong symbol",
    );
    assert_eq!(chime.corrected_bits, 1);
}

#[test]
fn two_wrong_symbols_in_one_codeword_are_a_crc_error() {
    let mut periods = chime_periods(&FIXTURE_PAYLOAD, 16, 8);
    // Symbols of one byte alternate between its two codewords, so symbols 0 and 2
    // are bits of the same codeword.
    for symbol in [16 + 3 * 14, 16 + 3 * 14 + 2] {
        periods[symbol] = if periods[symbol] == LOW { HIGH } else { LOW };
    }

    let unreadable = expect_unreadable(&feed_periods(&periods), "two wrong symbols");
    assert_eq!(unreadable.reason, UnreadableReason::BadCrc);
    assert_eq!(unreadable.bytes_received, FIXTURE_PAYLOAD.len() + 2);
}

/// A period at or below the minimum, or at or above the maximum, makes the C report a
/// bit error and return to `IDLE`.
#[test]
fn period_out_of_range_is_a_bit_error() {
    for bad_period in [20, 120, 600, 900] {
        let mut periods = chime_periods(&FIXTURE_PAYLOAD, 16, 8);
        periods[16 + 2 * 14 + 1] = bad_period;

        let events = feed_periods(&periods);
        let unreadable = expect_unreadable(&events, &format!("period {bad_period}"));
        assert_eq!(unreadable.reason, UnreadableReason::BadSymbol);
        assert_eq!(unreadable.bytes_received, 2);
    }
}

/// A period just inside the limits is data, and it counts as a 0 or a 1 on the side of
/// 360 samples it falls on. If that is the wrong value, the Hamming code corrects it.
#[test]
fn periods_just_inside_the_range_are_data() {
    for period in [121, 599] {
        let mut periods = chime_periods(&FIXTURE_PAYLOAD, 16, 8);
        let symbol = 16 + 14;
        let was_high = periods[symbol] == HIGH;
        periods[symbol] = period;

        let chime = expect_one_chime(&feed_periods(&periods), &FIXTURE_PAYLOAD, "in range");
        let expected_corrections = if was_high == (period > 360) { 0 } else { 1 };
        assert_eq!(
            chime.corrected_bits, expected_corrections,
            "period {period}"
        );
    }
}

/// `IN_PERIOD` is strict, so 300 and 420 are neither start periods nor data periods
/// while the state is `START_BITS`, and either of them ends the run. A chime that has
/// it among its last 5 start periods then has too short a run to start the message.
#[test]
fn start_run_ends_at_the_exact_period_limits() {
    for (period, decodes) in [(300, false), (301, true), (419, true), (420, false)] {
        let mut periods = chime_periods(&FIXTURE_PAYLOAD, 16, 8);
        periods[13] = period;
        let decoded = feed_periods(&periods)
            .iter()
            .any(|event| matches!(event, Event::Chime(_)));
        assert_eq!(decoded, decodes, "period {period}");
    }
}

/// A data symbol that measures between 300 and 420 samples right after the start
/// symbols counts as one more start symbol. The message then starts one symbol late.
/// This is why the first data symbol has half the margin of later ones.
#[test]
fn first_data_symbol_near_the_start_period_slips_the_message() {
    let mut periods = chime_periods(&FIXTURE_PAYLOAD, 16, 8);
    periods[16] = 350;

    let events = feed_periods(&periods);
    let decoded = events.iter().any(|event| matches!(event, Event::Chime(_)));
    assert!(!decoded, "events {events:?}");
}

/// A stop symbol that measures outside 300 to 420 samples ends the chance of stop
/// detection, and there are not enough symbols left to complete a byte. The message
/// is lost without a CRC check, and the C reports it only at the next crossing.
#[test]
fn stop_symbol_outside_the_start_period_loses_the_message() {
    let mut periods = chime_periods(&FIXTURE_PAYLOAD, 16, 8);
    let first_stop = periods.len() - 8;
    periods[first_stop] = 290;
    periods.push(50_000);

    let unreadable = expect_unreadable(&feed_periods(&periods), "bad stop symbol");
    assert_eq!(unreadable.reason, UnreadableReason::BadSymbol);
    assert_eq!(unreadable.bytes_received, FIXTURE_PAYLOAD.len() + 2);
}

/// A lost crossing merges two symbols. A 5 ms and a 5 ms symbol become one 10 ms
/// symbol, so every later symbol moves by one place and the Hamming code cannot help.
#[test]
fn one_lost_crossing_loses_the_message() {
    let mut periods = chime_periods(&FIXTURE_PAYLOAD, 16, 8);
    let first = (16..periods.len() - 9)
        .find(|&i| periods[i] == LOW && periods[i + 1] == LOW)
        .expect("the fixture has two low symbols in a row");
    periods[first] = HIGH;
    periods.remove(first + 1);
    periods.push(50_000);

    let events = feed_periods(&periods);
    assert!(
        !events.iter().any(|event| matches!(event, Event::Chime(_))),
        "events {events:?}"
    );
}

/// The channel filter rings after a click at about 7.5 ms per half cycle, because its
/// Q of 2/3 gives a damped frequency of 66 Hz. That falls inside the window for start
/// symbols, from 6.25 to 8.75 ms. In digital silence nothing masks the ringing, so one
/// click produces enough start periods to begin a false message. Any noise floor masks
/// the ringing after a few crossings, because it falls by a factor of about 35 at each
/// crossing. A simulation that combines exact silence with clicks therefore shows
/// failures that a real input would not.
#[test]
fn a_click_in_digital_silence_rings_at_the_start_symbol_period() {
    for sample_rate in [48_000, 192_000] {
        let mut costas_loop = CostasLoop::new(sample_rate);
        let timing = Timing::new(sample_rate);
        let window =
            (timing.start_stop_bit - timing.tolerance)..=(timing.start_stop_bit + timing.tolerance);

        let (mut last_output, mut last_crossing) = (0.0f32, 0usize);
        let mut periods = Vec::new();
        for n in 0..sample_rate as usize / 4 {
            let output = costas_loop.update(if n == 0 { 3000.0 } else { 0.0 });
            if (last_output >= 0.0 && output < 0.0) || (last_output < 0.0 && output >= 0.0) {
                periods.push((n - last_crossing) as u64);
                last_crossing = n;
            }
            last_output = output;
        }

        let in_window = periods
            .iter()
            .filter(|period| window.contains(period))
            .count();
        assert!(in_window >= 6, "{sample_rate} Hz: periods {periods:?}");
    }
}
