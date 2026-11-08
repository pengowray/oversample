//! Head-to-head comparison of `chime::Decoder` and the port of AudioMoth's decoder in
//! `support/audiomoth_port.rs`, over simulated channels.
//!
//! The tables and the cost figures are ignored tests, because they take minutes:
//!
//! ```text
//! cargo test --release -p chime --test robustness -- --ignored --nocapture
//! ```
//!
//! The tests that are not ignored check a few cells of the matrix, so that a regression
//! in `chime::Decoder` fails. They cover a moving phone, an overloaded input, loud bursts
//! in the band, echoes after a good read, sounds that are not chimes, and failed chimes.
//!
//! # How a trial is built
//!
//! Each trial renders a chime with a time payload, which is what
//! the page in the `batchi-chime` repository sends, with a Unix time, a time zone and random seeds
//! that depend on the row and the trial. The signal then goes through the effects of
//! its row in this order:
//!
//! 1. Render with the row's balance of carrier and melody.
//! 2. Upsample from 48 kHz, for the rows that do.
//! 3. Rate error, speaker low-pass, speaker nonlinearity, then reverb or a random room.
//! 4. Pad with 0.5 s and a random number of samples before, and 0.7 s after.
//! 5. Gain to the row's carrier level.
//! 6. White noise for the row's in-band SNR, measured on the signal at this point.
//! 7. Clicks, bat calls, hum and other added sounds.
//! 8. DC offset, overdrive with hard clipping at +-1.0, polarity inversion.
//!
//! Rows with no SNR have no added noise, so in those rows only the bit pattern, the
//! start position and the seeds of the added sounds change from trial to trial.
//!
//! "Carrier level" is the carrier's peak amplitude in dBFS, with the melody at twice
//! the carrier as usual. The encoder's normal carrier is -12 dBFS peak, and its RMS
//! level during a chime is about 3.4 dB lower.
//!
//! "Melody +N dB" lowers the carrier by N dB and leaves the melody at amplitude 1/2,
//! because that is what a phone speaker that is weak at 18 kHz does.
//!
//! Clicks are at 0 dBFS, the most an input can hold.
//!
//! The table of conditions the decoder was not tuned on includes a phone that moves, and
//! its "UTC" rows use a time zone of 0. The two zero bytes of that time zone are 28 data
//! 0 symbols in a row. Its rows for a phone that moves in a room apply the motion first,
//! so that each echo carries the motion from when it left the phone.
//!
//! # What counts as a pass
//!
//! A trial passes if exactly one `Chime` event carries the exact payload and no `Chime`
//! event carries anything else. A `Chime` with a wrong payload has passed the CRC, and
//! it is counted in its own column. `Unreadable` events do not fail a trial, and they
//! are counted in their own column too.
//!
//! # The other columns
//!
//! AudioMoth's decoder is designed for 48 kHz. The "Port at 48 kHz" column feeds the port
//! the same channel output after a brick-wall low-pass at 22 kHz and decimation, so that
//! a weak result in the "Port at row's rate" column can be told apart from the cost of
//! running the port away from its design rate.
//!
//! The "Either" column counts the trials that `chime::Decoder` or the port at the row's
//! rate passed, for a design that runs both decoders and lets the CRC choose.

mod support;

// `support/mod.rs` does not declare this module, so this file includes it by path.
#[path = "support/audiomoth_port.rs"]
mod audiomoth_port;

use std::f64::consts::PI;
use std::hint::black_box;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use audiomoth_port::AudioMothPort;
use chime::{Decoder, Event};
use support::channel::{self, ReverbProfile, Xorshift};
use support::generator::{self, tune_for_payload};

/// The sample rate of the X8's bat recordings, and of the matrix.
const RATE: u32 = 192_000;

/// The encoder's carrier and melody amplitudes.
const CARRIER_GAIN: f64 = 0.25;
const MELODY_GAIN: f64 = 0.5;

const TRIALS: u32 = 20;

/// The two ignored tests run one after the other, so that the matrix does not disturb
/// the timing and the two outputs do not interleave.
static RUN_ALONE: Mutex<()> = Mutex::new(());

fn amplitude_from_db(level_db: f64) -> f64 {
    10f64.powf(level_db / 20.0)
}

// -- Channel conditions --------------------------------------------------------

/// An effect that the simulator does not have: input signal, sample rate, seed.
type Effect = fn(Vec<f64>, u32, u64) -> Vec<f64>;

#[derive(Clone, Copy)]
enum Upsample {
    Sinc,
    ZeroOrderHold,
}

/// One channel condition. The fields are listed in the order the effects apply.
#[derive(Clone, Copy)]
struct Condition {
    melody_db: f64,
    upsample_from_48k: Option<Upsample>,
    rate_ppm: f64,
    lowpass_hz: Option<f64>,
    nonlinearity: bool,
    reverb: Option<ReverbProfile>,
    /// A replacement for `reverb`, or another effect before the padding.
    acoustic: Option<Effect>,
    carrier_peak_dbfs: Option<f64>,
    snr_db: Option<f64>,
    clicks: bool,
    bat_calls: bool,
    hum: bool,
    /// Another sound added after the noise.
    added: Option<Effect>,
    dc: f64,
    overdrive_db: Option<f64>,
    invert: bool,
    /// Send the 21-byte payload, with location and deployment ID.
    long_payload: bool,
    /// A fixed time zone in minutes, in place of a random one for each trial.
    time_zone: Option<i16>,
}

const CLEAN: Condition = Condition {
    melody_db: 0.0,
    upsample_from_48k: None,
    rate_ppm: 0.0,
    lowpass_hz: None,
    nonlinearity: false,
    reverb: None,
    acoustic: None,
    carrier_peak_dbfs: None,
    snr_db: None,
    clicks: false,
    bat_calls: false,
    hum: false,
    added: None,
    dc: 0.0,
    overdrive_db: None,
    invert: false,
    long_payload: false,
    time_zone: None,
};

const FIELD_A: Condition = Condition {
    reverb: Some(ReverbProfile::Small),
    melody_db: 20.0,
    snr_db: Some(10.0),
    rate_ppm: 100.0,
    ..CLEAN
};

const FIELD_B: Condition = Condition {
    reverb: Some(ReverbProfile::Medium),
    melody_db: 30.0,
    lowpass_hz: Some(16_000.0),
    nonlinearity: true,
    snr_db: Some(3.0),
    rate_ppm: -100.0,
    ..CLEAN
};

const FIELD_C: Condition = Condition {
    reverb: Some(ReverbProfile::Large),
    melody_db: 30.0,
    snr_db: Some(0.0),
    bat_calls: true,
    hum: true,
    ..CLEAN
};

/// The carrier is 20 dB weaker than usual, and the melody clips the input hard.
const OVERLOADED: Condition = Condition { melody_db: 20.0, overdrive_db: Some(40.0), ..CLEAN };

/// A condition with a label, a sample rate and a number of trials.
#[derive(Clone)]
struct Row {
    label: String,
    rate: u32,
    trials: u32,
    condition: Condition,
}

fn row(label: &str, condition: Condition) -> Row {
    Row { label: label.to_string(), rate: RATE, trials: TRIALS, condition }
}

/// The impairments of the matrix that do not set an SNR.
fn impairment_rows() -> Vec<Row> {
    vec![
        row("carrier -40 dBFS", Condition { carrier_peak_dbfs: Some(-40.0), ..CLEAN }),
        row("carrier -60 dBFS", Condition { carrier_peak_dbfs: Some(-60.0), ..CLEAN }),
        row("carrier -80 dBFS", Condition { carrier_peak_dbfs: Some(-80.0), ..CLEAN }),
        row("melody +20 dB", Condition { melody_db: 20.0, ..CLEAN }),
        row("melody +40 dB", Condition { melody_db: 40.0, ..CLEAN }),
        row("speaker low-pass 16 kHz", Condition { lowpass_hz: Some(16_000.0), ..CLEAN }),
        row("speaker nonlinearity", Condition { nonlinearity: true, ..CLEAN }),
        row("reverb small", Condition { reverb: Some(ReverbProfile::Small), ..CLEAN }),
        row("reverb medium", Condition { reverb: Some(ReverbProfile::Medium), ..CLEAN }),
        row("reverb large", Condition { reverb: Some(ReverbProfile::Large), ..CLEAN }),
        row("rate +100 ppm", Condition { rate_ppm: 100.0, ..CLEAN }),
        row("rate -100 ppm", Condition { rate_ppm: -100.0, ..CLEAN }),
        row("rate +500 ppm", Condition { rate_ppm: 500.0, ..CLEAN }),
        row("rate -500 ppm", Condition { rate_ppm: -500.0, ..CLEAN }),
        row("DC offset 0.1", Condition { dc: 0.1, ..CLEAN }),
        row("hum 50 Hz -20 dBFS", Condition { hum: true, ..CLEAN }),
        row("clipping, 20 dB overdrive", Condition { overdrive_db: Some(20.0), ..CLEAN }),
        row("clipping, 40 dB overdrive", Condition { overdrive_db: Some(40.0), ..CLEAN }),
        row("clicks 20/s 0 dBFS", Condition { clicks: true, ..CLEAN }),
        row("bat calls 20/s -20 dBFS", Condition { bat_calls: true, ..CLEAN }),
        row("48 kHz render, sinc upsample", Condition { upsample_from_48k: Some(Upsample::Sinc), ..CLEAN }),
        row("48 kHz render, zero-order hold", Condition { upsample_from_48k: Some(Upsample::ZeroOrderHold), ..CLEAN }),
        row("inverted polarity", Condition { invert: true, ..CLEAN }),
    ]
}

/// Table 1: the matrix.
fn matrix_rows() -> Vec<Row> {
    let mut rows = vec![row("clean", CLEAN)];
    rows.extend(impairment_rows());
    for snr_db in [20.0, 10.0, 5.0, 0.0, -5.0, -10.0, -15.0] {
        rows.push(row(&format!("SNR {snr_db:+} dB"), Condition { snr_db: Some(snr_db), ..CLEAN }));
    }
    rows.push(row("FIELD-A", FIELD_A));
    rows.push(row("FIELD-B", FIELD_B));
    rows.push(row("FIELD-C", FIELD_C));
    for rate in [44_100, 48_000, 96_000] {
        rows.push(Row { rate, ..row(&format!("clean at {rate} Hz"), CLEAN) });
        rows.push(Row { rate, ..row(&format!("FIELD-A at {rate} Hz"), FIELD_A) });
    }
    rows
}

/// Table 2: the impairments again, with noise at +10 dB in the band.
fn noisy_impairment_rows() -> Vec<Row> {
    impairment_rows()
        .into_iter()
        .map(|row| Row {
            label: format!("{}, SNR +10", row.label),
            condition: Condition { snr_db: Some(10.0), ..row.condition },
            ..row
        })
        .collect()
}

/// Table 3: conditions that the shipped decoder's constants were not tuned on, and a
/// quiet carrier against sounds that have a fixed level.
fn untuned_rows() -> Vec<Row> {
    let carrier_40 = Condition { carrier_peak_dbfs: Some(-40.0), ..CLEAN };
    let snr_10 = Condition { snr_db: Some(10.0), ..CLEAN };
    let utc = Condition { time_zone: Some(0), ..CLEAN };
    let utc_snr_10 = Condition { time_zone: Some(0), ..snr_10 };
    vec![
        row("random room small", Condition { acoustic: Some(random_room_small), ..CLEAN }),
        row("random room medium", Condition { acoustic: Some(random_room_medium), ..CLEAN }),
        row("random room large", Condition { acoustic: Some(random_room_large), ..CLEAN }),
        row("random room small, SNR +10", Condition { acoustic: Some(random_room_small), ..snr_10 }),
        row("random room medium, SNR +10", Condition { acoustic: Some(random_room_medium), ..snr_10 }),
        row("random room large, SNR +10", Condition { acoustic: Some(random_room_large), ..snr_10 }),
        row("phone moves 2 cm at 2 Hz", Condition { acoustic: Some(hand_wobble::<20>), ..CLEAN }),
        row("phone moves 2 cm at 2 Hz, SNR +10", Condition { acoustic: Some(hand_wobble::<20>), ..snr_10 }),
        row("phone moves 2 cm at 2 Hz, UTC", Condition { acoustic: Some(hand_wobble::<20>), ..utc }),
        row("phone moves 2 cm at 2 Hz, UTC, SNR +10", Condition { acoustic: Some(hand_wobble::<20>), ..utc_snr_10 }),
        row("phone moves 1 cm at 2 Hz, UTC, SNR +10", Condition { acoustic: Some(hand_wobble::<10>), ..utc_snr_10 }),
        row("phone moves 5 mm at 2 Hz, UTC, SNR +10", Condition { acoustic: Some(hand_wobble::<5>), ..utc_snr_10 }),
        row("phone still, UTC, SNR +10", utc_snr_10),
        row("reverb small, moves 2 cm, UTC, SNR +10", Condition { acoustic: Some(moves_in_small_room), ..utc_snr_10 }),
        row("reverb medium, moves 5 mm, SNR +10", Condition { acoustic: Some(moves_in_medium_room), ..snr_10 }),
        row("steady 18.3 kHz tone, carrier -6 dB", Condition { added: Some(steady_tone), ..CLEAN }),
        row("insect pulses 17.8 kHz", Condition { added: Some(insect_pulses), ..CLEAN }),
        row("carrier -40, clicks 0 dBFS", Condition { clicks: true, ..carrier_40 }),
        row("carrier -40, damped rings 0 dBFS", Condition { added: Some(damped_rings::<0>), ..carrier_40 }),
        row("carrier -40, damped rings -20 dBFS", Condition { added: Some(damped_rings::<20>), ..carrier_40 }),
        row("carrier -40, bat calls -20 dBFS", Condition { bat_calls: true, ..carrier_40 }),
        row("carrier -40, hum -20 dBFS", Condition { hum: true, ..carrier_40 }),
        row("melody +30 dB, 20 dB overdrive", Condition { melody_db: 30.0, overdrive_db: Some(20.0), ..CLEAN }),
        row("melody +30 dB, 20 dB overdrive, SNR +20", Condition { melody_db: 30.0, overdrive_db: Some(20.0), snr_db: Some(20.0), ..CLEAN }),
        row("melody +20 dB, 40 dB overdrive", OVERLOADED),
        row("melody +30 dB, 30 dB overdrive", Condition { melody_db: 30.0, overdrive_db: Some(30.0), ..CLEAN }),
        row("melody +30 dB, 40 dB overdrive", Condition { melody_db: 30.0, overdrive_db: Some(40.0), ..CLEAN }),
        row("21-byte payload, clean", Condition { long_payload: true, ..CLEAN }),
        row("21-byte payload, SNR 0", Condition { long_payload: true, snr_db: Some(0.0), ..CLEAN }),
        row("21-byte payload, FIELD-A", Condition { long_payload: true, ..FIELD_A }),
    ]
}

/// Table 4: success against SNR in steps of 1 dB.
fn sensitivity_rows() -> Vec<Row> {
    (-12..=8)
        .rev()
        .map(|snr| Row {
            trials: 60,
            ..row(&format!("SNR {snr:+} dB"), Condition { snr_db: Some(snr as f64), ..CLEAN })
        })
        .collect()
}

// -- Effects the simulator does not have ---------------------------------------

/// Ranges for a random room. Each reflection has a random delay, gain and polarity.
struct RoomKind {
    reflections: usize,
    longest_delay_s: f64,
    strongest_db: f64,
    weakest_db: f64,
    /// RT60 and energy relative to the direct sound of a noise tail, if it has one.
    tail: Option<(f64, f64)>,
}

/// A new impulse response for each trial, unlike the fixed profiles in `channel.rs`
/// that the shipped decoder was developed against.
fn random_room(x: Vec<f64>, rate: u32, seed: u64, kind: &RoomKind) -> Vec<f64> {
    let mut random = Xorshift::new(seed ^ 0x524F_4F4D);
    let rate = rate as f64;
    let tail_length_s = kind.tail.map_or(0.0, |(rt60, _)| 1.2 * rt60);
    let mut response = vec![0.0; (kind.longest_delay_s.max(tail_length_s) * rate) as usize + 1];
    response[0] = 1.0;
    for _ in 0..kind.reflections {
        let delay = 0.001 + random.uniform() * (kind.longest_delay_s - 0.001);
        let gain_db = kind.strongest_db + random.uniform() * (kind.weakest_db - kind.strongest_db);
        let polarity = if random.uniform() < 0.5 { -1.0 } else { 1.0 };
        response[(delay * rate) as usize] += polarity * amplitude_from_db(gain_db);
    }
    if let Some((rt60, energy_db)) = kind.tail {
        // The same scaling as in `channel::impulse_response`, for a tail from 5 ms on.
        let decay = 1000f64.ln() / rt60;
        let start = 0.005;
        let energy = 10f64.powf(energy_db / 10.0);
        let amplitude = (energy * 2.0 * decay * (2.0 * decay * start).exp() / rate).sqrt();
        for (n, sample) in response.iter_mut().enumerate().skip((start * rate) as usize) {
            *sample += amplitude * (-decay * n as f64 / rate).exp() * random.gaussian();
        }
    }
    channel::convolve(&x, &response)
}

fn random_room_small(x: Vec<f64>, rate: u32, seed: u64) -> Vec<f64> {
    let kind = RoomKind { reflections: 6, longest_delay_s: 0.010, strongest_db: -6.0, weakest_db: -20.0, tail: None };
    random_room(x, rate, seed, &kind)
}

fn random_room_medium(x: Vec<f64>, rate: u32, seed: u64) -> Vec<f64> {
    let kind = RoomKind {
        reflections: 8,
        longest_delay_s: 0.040,
        strongest_db: -4.0,
        weakest_db: -16.0,
        tail: Some((0.200, -9.0)),
    };
    random_room(x, rate, seed, &kind)
}

fn random_room_large(x: Vec<f64>, rate: u32, seed: u64) -> Vec<f64> {
    let kind = RoomKind {
        reflections: 8,
        longest_delay_s: 0.020,
        strongest_db: -2.0,
        weakest_db: -6.0,
        tail: Some((0.600, -3.0)),
    };
    random_room(x, rate, seed, &kind)
}

/// A phone held by hand that moves `MILLIMETRES` either way at 2 Hz. At 343 m/s, 2 cm
/// changes the delay by 58 us either way, and the peak speed of 25 cm/s is a Doppler
/// shift of 733 ppm.
fn hand_wobble<const MILLIMETRES: u32>(x: Vec<f64>, rate: u32, seed: u64) -> Vec<f64> {
    let mut random = Xorshift::new(seed ^ 0x4841_4E44);
    let phase = 2.0 * PI * random.uniform();
    let rate = rate as f64;
    let swing = MILLIMETRES as f64 * 0.001 / 343.0 * rate;
    let at = |n: i64| if n >= 0 && (n as usize) < x.len() { x[n as usize] } else { 0.0 };
    (0..x.len())
        .map(|n| {
            let position = n as f64 + swing * (2.0 * PI * 2.0 * n as f64 / rate + phase).sin();
            let whole = position.floor();
            let t = position - whole;
            let k = whole as i64;
            // 4-point Lagrange interpolation.
            let (a, b, c, d) = (at(k - 1), at(k), at(k + 1), at(k + 2));
            -t * (t - 1.0) * (t - 2.0) / 6.0 * a + (t + 1.0) * (t - 1.0) * (t - 2.0) / 2.0 * b
                - (t + 1.0) * t * (t - 2.0) / 2.0 * c
                + (t + 1.0) * t * (t - 1.0) / 6.0 * d
        })
        .collect()
}

/// A phone that moves 2 cm either way at 2 Hz in a room with the small profile.
fn moves_in_small_room(x: Vec<f64>, rate: u32, seed: u64) -> Vec<f64> {
    channel::reverb(hand_wobble::<20>(x, rate, seed), rate, ReverbProfile::Small)
}

/// A phone that moves 5 mm either way at 2 Hz in a room with the medium profile.
fn moves_in_medium_room(x: Vec<f64>, rate: u32, seed: u64) -> Vec<f64> {
    channel::reverb(hand_wobble::<5>(x, rate, seed), rate, ReverbProfile::Medium)
}

/// A tone inside the decoders' band for the whole stream, 6 dB below the carrier.
fn steady_tone(mut x: Vec<f64>, rate: u32, seed: u64) -> Vec<f64> {
    let phase = 2.0 * PI * Xorshift::new(seed ^ 0x544F_4E45).uniform();
    for (n, sample) in x.iter_mut().enumerate() {
        *sample += 0.5 * CARRIER_GAIN * (2.0 * PI * 18_300.0 * n as f64 / rate as f64 + phase).sin();
    }
    x
}

/// Tone pulses inside the band, 5 ms long and 20 ms apart, as loud as the carrier. An
/// insect call can look like this.
fn insect_pulses(mut x: Vec<f64>, rate: u32, seed: u64) -> Vec<f64> {
    let phase = 2.0 * PI * Xorshift::new(seed ^ 0x494E_5345).uniform();
    let rate = rate as f64;
    for (n, sample) in x.iter_mut().enumerate() {
        let t = n as f64 / rate + phase;
        if t.rem_euclid(0.020) < 0.005 {
            *sample += CARRIER_GAIN * (2.0 * PI * 17_800.0 * t).sin();
        }
    }
    x
}

/// Damped resonances at 20 per second, each with a random frequency from 12 to 30 kHz
/// and a time constant of 0.3 ms, starting `ATTENUATION_DB` below full scale. A
/// single-sample click has little energy in the 2 kHz band, and this has more.
fn damped_rings<const ATTENUATION_DB: u32>(mut x: Vec<f64>, rate: u32, seed: u64) -> Vec<f64> {
    let amplitude = amplitude_from_db(-(ATTENUATION_DB as f64));
    let mut random = Xorshift::new(seed ^ 0x5249_4E47);
    let rate = rate as f64;
    let mean_gap = rate / 20.0;
    let length = (0.003 * rate) as usize;
    let mut position = -(1.0 - random.uniform()).ln() * mean_gap;
    while (position as usize) < x.len() {
        let frequency = 12_000.0 + random.uniform() * 18_000.0;
        for k in 0..length {
            let Some(sample) = x.get_mut(position as usize + k) else { break };
            let t = k as f64 / rate;
            *sample += amplitude * (-t / 0.0003).exp() * (2.0 * PI * frequency * t).sin();
        }
        position += -(1.0 - random.uniform()).ln() * mean_gap;
    }
    x
}

/// Tone bursts near the carrier at random times, on average 20 per second, each with a
/// random frequency from 17.5 to 18.5 kHz and a random length from 3 to 33 ms. Real
/// recordings with no chime in them have sounds like these, which start walks.
fn tone_bursts(mut x: Vec<f64>, rate: u32, amplitude: f64, seed: u64) -> Vec<f64> {
    let mut random = Xorshift::new(seed);
    let rate = rate as f64;
    let mean_gap = rate / 20.0;
    let mut position = -(1.0 - random.uniform()).ln() * mean_gap;
    while (position as usize) < x.len() {
        let length = (0.003 + random.uniform() * 0.030) * rate;
        let frequency = 17_500.0 + random.uniform() * 1_000.0;
        let phase = 2.0 * PI * random.uniform();
        for k in 0..length as usize {
            let Some(sample) = x.get_mut(position as usize + k) else { break };
            let envelope = (PI * k as f64 / length).sin();
            *sample += amplitude * envelope * (2.0 * PI * frequency * k as f64 / rate + phase).sin();
        }
        position += -(1.0 - random.uniform()).ln() * mean_gap;
    }
    x
}

// -- Building a trial's signal -------------------------------------------------

/// Real time zone offsets, in minutes.
const TIME_ZONES: [i16; 13] = [-720, -600, -480, -300, -210, 0, 60, 330, 345, 480, 570, 660, 840];

/// Seed for one trial of one row. It depends on the label and not on the row's place
/// in a table, so that adding a row does not change the results of the others.
fn trial_seed(label: &str, rate: u32, trial: u32) -> u64 {
    let mut hash = 0xCBF2_9CE4_8422_2325u64 ^ rate as u64;
    for byte in label.bytes() {
        hash = (hash ^ byte as u64).wrapping_mul(0x0000_0100_0000_01B3);
    }
    hash ^ (trial as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

fn trial_payload(condition: &Condition, random: &mut Xorshift) -> Vec<u8> {
    // Dates from 2025 to 2033.
    let unix = 1_740_000_000 + (random.next_u64() % 260_000_000) as u32;
    let random_zone = TIME_ZONES[(random.next_u64() % TIME_ZONES.len() as u64) as usize];
    let time_zone = condition.time_zone.unwrap_or(random_zone);
    let mut payload = generator::time_payload(unix, time_zone);
    if condition.long_payload {
        let id = (random.next_u64()).to_le_bytes();
        payload.extend_from_slice(&generator::location_bytes(
            -90.0 + 180.0 * random.uniform(),
            -180.0 + 360.0 * random.uniform(),
        ));
        payload.extend_from_slice(&generator::deployment_bytes(id));
    }
    payload
}

struct TrialSignal {
    samples: Vec<f64>,
    payload: Vec<u8>,
    /// The input sample where the first start symbol begins.
    true_start: usize,
}

fn build_signal(condition: &Condition, rate: u32, seed: u64) -> TrialSignal {
    let mut random = Xorshift::new(seed);
    let payload = trial_payload(condition, &mut random);
    let notes = tune_for_payload(payload.len());
    let carrier_gain = CARRIER_GAIN * amplitude_from_db(-condition.melody_db);

    let mut x = match condition.upsample_from_48k {
        None => generator::render(rate, &payload, notes).mix_with_gains(carrier_gain, MELODY_GAIN),
        Some(kind) => {
            assert!(rate.is_multiple_of(48_000));
            let factor = (rate / 48_000) as usize;
            let rendered = generator::render(48_000, &payload, notes).mix_with_gains(carrier_gain, MELODY_GAIN);
            match kind {
                Upsample::Sinc => channel::upsample_sinc(rendered, factor),
                Upsample::ZeroOrderHold => channel::zoh_upsample(rendered, factor),
            }
        }
    };

    if condition.rate_ppm != 0.0 {
        x = channel::rate_error_ppm(x, condition.rate_ppm);
    }
    if let Some(cutoff_hz) = condition.lowpass_hz {
        x = channel::speaker_lowpass(x, rate, cutoff_hz);
    }
    if condition.nonlinearity {
        x = channel::speaker_nonlinearity(x, 0.1, 0.1);
    }
    if let Some(profile) = condition.reverb {
        x = channel::reverb(x, rate, profile);
    }
    if let Some(effect) = condition.acoustic {
        x = effect(x, rate, random.next_u64());
    }

    // A random start, so that symbol boundaries and carrier phase differ from the
    // decoders' decimation and mixer from trial to trial.
    let before = rate as usize / 2 + (random.next_u64() % (rate as u64 / 100)) as usize;
    let after = 7 * rate as usize / 10;
    x = channel::pad(x, before, after);

    if let Some(level_dbfs) = condition.carrier_peak_dbfs {
        x = channel::gain_db(x, level_dbfs - channel::db(carrier_gain));
    }
    if let Some(snr_db) = condition.snr_db {
        x = channel::add_white_noise(x, rate, snr_db, random.next_u64());
    }
    if condition.clicks {
        x = channel::clicks(x, rate, 20.0, 0.0, random.next_u64());
    }
    if condition.bat_calls {
        x = channel::bat_calls(x, rate, 20.0, -20.0, random.next_u64());
    }
    if condition.hum {
        x = channel::hum(x, rate, 50.0, -20.0);
    }
    if let Some(effect) = condition.added {
        x = effect(x, rate, random.next_u64());
    }
    if condition.dc != 0.0 {
        x = channel::dc_offset(x, condition.dc);
    }
    if let Some(gain) = condition.overdrive_db {
        x = channel::clip(channel::gain_db(x, gain), 1.0);
    }
    if condition.invert {
        x = channel::invert(x);
    }
    TrialSignal { samples: x, payload, true_start: before }
}

// -- Running the decoders ------------------------------------------------------

fn decoder_events(rate: u32, samples: &[f32]) -> Vec<Event> {
    let mut decoder = Decoder::new(rate).expect("rate above 40 kHz");
    samples.iter().filter_map(|&sample| decoder.push(sample)).collect()
}

fn port_events(rate: u32, samples: &[f32]) -> Vec<Event> {
    let mut port = AudioMothPort::new(rate);
    samples.iter().filter_map(|&sample| port.push(sample)).collect()
}

/// The channel output as a 48 kHz recorder would have it, if `rate` allows that.
fn decimated_to_48k(x: &[f64], rate: u32) -> Option<Vec<f32>> {
    if rate == 48_000 || !rate.is_multiple_of(48_000) {
        return None;
    }
    let filtered = channel::band_limited(x, rate, 0.0, 22_000.0);
    Some(filtered.iter().step_by((rate / 48_000) as usize).map(|&s| s as f32).collect())
}

/// Totals for one decoder over the trials of a row.
#[derive(Clone, Default)]
struct Tally {
    passes: u32,
    wrong_payloads: u32,
    /// Trials with more than one `Chime` that has the exact payload.
    duplicates: u32,
    unreadable_events: u32,
    /// The error of `start_sample` in ms, for each trial that passed.
    start_errors_ms: Vec<f64>,
}

impl Tally {
    fn add_trial(&mut self, events: &[Event], payload: &[u8], true_start: usize, rate: u32) {
        let mut exact = 0;
        let mut wrong = 0;
        let mut start = None;
        for event in events {
            match event {
                Event::Chime(chime) if chime.payload[..chime.payload_len] == *payload => {
                    exact += 1;
                    start.get_or_insert(chime.start_sample);
                }
                Event::Chime(_) => wrong += 1,
                Event::Unreadable(_) => self.unreadable_events += 1,
            }
        }
        self.wrong_payloads += wrong;
        if exact > 1 {
            self.duplicates += 1;
        }
        if exact == 1 && wrong == 0 {
            self.passes += 1;
            let start = start.expect("one exact chime");
            self.start_errors_ms.push((start as f64 - true_start as f64) * 1000.0 / rate as f64);
        }
    }
}

struct ErrorStats {
    mean: f64,
    mean_size: f64,
    largest_size: f64,
}

fn error_stats(errors: &[f64]) -> Option<ErrorStats> {
    if errors.is_empty() {
        return None;
    }
    let count = errors.len() as f64;
    Some(ErrorStats {
        mean: errors.iter().sum::<f64>() / count,
        mean_size: errors.iter().map(|e| e.abs()).sum::<f64>() / count,
        largest_size: errors.iter().fold(0.0, |largest: f64, e| largest.max(e.abs())),
    })
}

#[derive(Clone, Default)]
struct RowResult {
    shipped: Tally,
    port: Tally,
    port_48k: Tally,
    /// Trials that `chime::Decoder` or the port at the row's rate passed, for a design
    /// that runs both and lets the CRC choose.
    either_passes: u32,
}

/// Prints each chime whose payload is wrong, because each one is a serious failure.
fn report_wrong_payloads(decoder: &str, row: &Row, trial: u32, payload: &[u8], events: &[Event]) {
    for event in events {
        if let Event::Chime(chime) = event
            && chime.payload[..chime.payload_len] != *payload
        {
            println!(
                "WRONG PAYLOAD: {decoder}, row \"{}\", trial {trial}: sent {payload:02x?}, read {:02x?}",
                row.label,
                &chime.payload[..chime.payload_len]
            );
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Decoders {
    ShippedOnly,
    All,
}

/// Runs every trial of every row on worker threads.
fn run_rows(rows: &[Row], decoders: Decoders) -> Vec<RowResult> {
    let jobs: Vec<(usize, u32)> =
        rows.iter().enumerate().flat_map(|(r, row)| (0..row.trials).map(move |trial| (r, trial))).collect();
    let next_job = AtomicUsize::new(0);
    let results = Mutex::new(vec![RowResult::default(); rows.len()]);
    let workers = std::thread::available_parallelism().map_or(4, |count| count.get().saturating_sub(2).max(1));

    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                while let Some(&(r, trial)) = jobs.get(next_job.fetch_add(1, Ordering::Relaxed)) {
                    let row = &rows[r];
                    let seed = trial_seed(&row.label, row.rate, trial);
                    let signal = build_signal(&row.condition, row.rate, seed);
                    let samples = channel::to_f32(&signal.samples);
                    let shipped = decoder_events(row.rate, &samples);
                    let (port, port_48k) = if decoders == Decoders::All {
                        let decimated = decimated_to_48k(&signal.samples, row.rate);
                        (
                            Some(port_events(row.rate, &samples)),
                            decimated.map(|samples| port_events(48_000, &samples)),
                        )
                    } else {
                        (None, None)
                    };

                    report_wrong_payloads("chime::Decoder", row, trial, &signal.payload, &shipped);
                    if let Some(events) = &port {
                        report_wrong_payloads("Port", row, trial, &signal.payload, events);
                    }
                    if let Some(events) = &port_48k {
                        report_wrong_payloads("Port at 48 kHz", row, trial, &signal.payload, events);
                    }

                    let mut results = results.lock().unwrap();
                    let result = &mut results[r];
                    let shipped_passes_before = result.shipped.passes;
                    let port_passes_before = result.port.passes;
                    result.shipped.add_trial(&shipped, &signal.payload, signal.true_start, row.rate);
                    if let Some(events) = port {
                        result.port.add_trial(&events, &signal.payload, signal.true_start, row.rate);
                    }
                    if result.shipped.passes > shipped_passes_before || result.port.passes > port_passes_before {
                        result.either_passes += 1;
                    }
                    if let Some(events) = port_48k {
                        let start_at_48k = signal.true_start / (row.rate / 48_000) as usize;
                        result.port_48k.add_trial(&events, &signal.payload, start_at_48k, 48_000);
                    }
                }
            });
        }
    });
    results.into_inner().unwrap()
}

// -- Printing ------------------------------------------------------------------

fn tally_cells(tally: &Tally) -> String {
    format!(
        "{:>4} {:>5} {:>3} {:>6}",
        tally.passes, tally.wrong_payloads, tally.duplicates, tally.unreadable_events
    )
}

const LABEL_WIDTH: usize = 47;

fn print_table(title: &str, rows: &[Row], results: &[RowResult]) {
    println!();
    println!("{title}");
    println!(
        "{:<LABEL_WIDTH$} | {:<21} | {:<21} | {:<21} | Either | start_sample error, ms",
        "", "chime::Decoder", "Port at row's rate", "Port at 48 kHz"
    );
    println!(
        "{:<LABEL_WIDTH$} | {c} | {c} | {c} | {:>6} | {:>6} {:>6} {:>6}",
        "Condition (trials)",
        "pass",
        "mean",
        "|mean|",
        "|max|",
        c = "pass wrong dup unread"
    );
    for (row, result) in rows.iter().zip(results) {
        let label = format!("{} ({})", row.label, row.trials);
        let port_48k = if row.rate == 48_000 || !row.rate.is_multiple_of(48_000) {
            format!("{:^21}", "not run")
        } else {
            tally_cells(&result.port_48k)
        };
        let errors = match error_stats(&result.shipped.start_errors_ms) {
            Some(stats) => format!("{:>6.2} {:>6.2} {:>6.2}", stats.mean, stats.mean_size, stats.largest_size),
            None => format!("{:>6} {:>6} {:>6}", "-", "-", "-"),
        };
        println!(
            "{label:<LABEL_WIDTH$} | {} | {} | {port_48k} | {:>6} | {errors}",
            tally_cells(&result.shipped),
            tally_cells(&result.port),
            result.either_passes
        );
    }
}

fn print_start_error_summary(name: &str, results: &[&[RowResult]], select: fn(&RowResult) -> &Tally) {
    let errors: Vec<f64> =
        results.iter().flat_map(|table| table.iter()).flat_map(|result| select(result).start_errors_ms.iter().copied()).collect();
    match error_stats(&errors) {
        Some(stats) => println!(
            "{name}: {} decoded trials, mean error {:.3} ms, mean size of error {:.3} ms, largest size of error {:.3} ms",
            errors.len(),
            stats.mean,
            stats.mean_size,
            stats.largest_size
        ),
        None => println!("{name}: no decoded trials"),
    }
}

// -- The tables ----------------------------------------------------------------

#[test]
#[ignore]
fn comparison_tables() {
    let _alone = RUN_ALONE.lock().unwrap();

    let matrix = matrix_rows();
    let matrix_results = run_rows(&matrix, Decoders::All);
    print_table("Table 1: the matrix", &matrix, &matrix_results);

    let noisy = noisy_impairment_rows();
    let noisy_results = run_rows(&noisy, Decoders::All);
    print_table("Table 2: the impairments of table 1 with noise at +10 dB in the band", &noisy, &noisy_results);

    let untuned = untuned_rows();
    let untuned_results = run_rows(&untuned, Decoders::All);
    print_table("Table 3: conditions the shipped decoder was not tuned on", &untuned, &untuned_results);

    let sensitivity = sensitivity_rows();
    let sensitivity_results = run_rows(&sensitivity, Decoders::All);
    print_table("Table 4: success against SNR", &sensitivity, &sensitivity_results);

    println!();
    println!("start_sample error over table 1:");
    print_start_error_summary("chime::Decoder", &[&matrix_results], |result| &result.shipped);
    print_start_error_summary("Port at the row's rate", &[&matrix_results], |result| &result.port);
    println!("start_sample error over tables 1 to 4:");
    let all: [&[RowResult]; 4] = [&matrix_results, &noisy_results, &untuned_results, &sensitivity_results];
    print_start_error_summary("chime::Decoder", &all, |result| &result.shipped);
    print_start_error_summary("Port at the row's rate", &all, |result| &result.port);
}

// -- Cost ----------------------------------------------------------------------

/// Pushes `segments` in order. Returns the time spent on the segments marked true and
/// the time spent on the others.
fn timed_pass(mut push: impl FnMut(f32) -> Option<Event>, segments: &[(&[f32], bool)]) -> (Duration, Duration) {
    let mut kept = Duration::ZERO;
    let mut other = Duration::ZERO;
    for &(segment, is_kept) in segments {
        let started = Instant::now();
        for &sample in segment {
            black_box(push(black_box(sample)));
        }
        if is_kept {
            kept += started.elapsed();
        } else {
            other += started.elapsed();
        }
    }
    (kept, other)
}

/// The longest single call of `push`, in ns. Reading the clock around each call adds
/// to it, and an interrupt of the host can land in one call, so the smallest result
/// of a few runs is the fair figure.
fn longest_push_ns(mut push: impl FnMut(f32) -> Option<Event>, samples: &[f32]) -> u128 {
    let mut longest = 0;
    for &sample in samples {
        let started = Instant::now();
        black_box(push(black_box(sample)));
        longest = longest.max(started.elapsed().as_nanos());
    }
    longest
}

#[test]
#[ignore]
fn decoder_cost() {
    let _alone = RUN_ALONE.lock().unwrap();
    const PASSES: usize = 5;

    println!();
    println!("size of chime::Decoder: {} bytes", std::mem::size_of::<Decoder>());
    println!("size of AudioMothPort:  {} bytes", std::mem::size_of::<AudioMothPort>());

    // White noise at -40 dBFS RMS, and the same noise with chimes in it.
    let gap = RATE as usize / 2;
    let chime = generator::render(RATE, &generator::time_payload(1_793_000_000, 660), tune_for_payload(6)).mix();
    let chime_len = chime.len();
    let mut stream = Vec::new();
    for _ in 0..8 {
        stream.extend(std::iter::repeat_n(0.0, gap));
        stream.extend_from_slice(&chime);
    }
    stream.extend(std::iter::repeat_n(0.0, gap));
    let with_noise = channel::to_f32(&channel::add_white_noise_rms(stream, 0.01, 77));
    let noise = channel::to_f32(&channel::add_white_noise_rms(vec![0.0; 10 * RATE as usize], 0.01, 78));
    // Noise first, so that the filter states hold values that decay to subnormal numbers
    // during the silence that follows it. Starting from silence would leave them at 0.
    let burst = channel::to_f32(&channel::add_white_noise_rms(vec![0.0; RATE as usize / 10], 0.01, 79));
    let silence = vec![0.0f32; 10 * RATE as usize];
    let silence_segments: [(&[f32], bool); 2] = [(&burst, false), (&silence, true)];

    let mut chime_segments: Vec<(&[f32], bool)> = Vec::new();
    let mut position = 0;
    for _ in 0..8 {
        chime_segments.push((&with_noise[position..position + gap], false));
        position += gap;
        chime_segments.push((&with_noise[position..position + chime_len], true));
        position += chime_len;
    }
    chime_segments.push((&with_noise[position..], false));

    let ns_per_sample = |time: Duration, samples: usize| time.as_nanos() as f64 / samples as f64;
    let share_of_real_time = |ns: f64| ns * RATE as f64 * 1e-9 * 100.0;

    /// The smallest time of each kind over several runs.
    fn smallest(results: impl Iterator<Item = (Duration, Duration)>) -> (Duration, Duration) {
        results.fold((Duration::MAX, Duration::MAX), |best, run| (best.0.min(run.0), best.1.min(run.1)))
    }

    let chime_samples = 8 * chime_len;
    let gap_samples = with_noise.len() - chime_samples;

    let shipped_noise = smallest((0..PASSES).map(|_| {
        let mut decoder = Decoder::new(RATE).unwrap();
        timed_pass(|sample| decoder.push(sample), &[(&noise, true)])
    }));
    let port_noise = smallest((0..PASSES).map(|_| {
        let mut port = AudioMothPort::new(RATE);
        timed_pass(|sample| port.push(sample), &[(&noise, true)])
    }));
    let shipped_chimes = smallest((0..PASSES).map(|_| {
        let mut decoder = Decoder::new(RATE).unwrap();
        timed_pass(|sample| decoder.push(sample), &chime_segments)
    }));
    let port_chimes = smallest((0..PASSES).map(|_| {
        let mut port = AudioMothPort::new(RATE);
        timed_pass(|sample| port.push(sample), &chime_segments)
    }));
    let shipped_silence = smallest((0..PASSES).map(|_| {
        let mut decoder = Decoder::new(RATE).unwrap();
        timed_pass(|sample| decoder.push(sample), &silence_segments)
    }));
    let port_silence = smallest((0..PASSES).map(|_| {
        let mut port = AudioMothPort::new(RATE);
        timed_pass(|sample| port.push(sample), &silence_segments)
    }));

    let rows = [
        ("white noise, -40 dBFS RMS", ns_per_sample(shipped_noise.0, noise.len()), ns_per_sample(port_noise.0, noise.len())),
        (
            "noise between chimes",
            ns_per_sample(shipped_chimes.1, gap_samples),
            ns_per_sample(port_chimes.1, gap_samples),
        ),
        ("during chimes", ns_per_sample(shipped_chimes.0, chime_samples), ns_per_sample(port_chimes.0, chime_samples)),
        (
            "digital silence after noise",
            ns_per_sample(shipped_silence.0, silence.len()),
            ns_per_sample(port_silence.0, silence.len()),
        ),
    ];
    println!("Host time per input sample at 192 kHz, smallest of {PASSES} runs:");
    println!("{:<38} {:>16} {:>8} {:>16} {:>8}", "", "Decoder ns", "% RT", "Port ns", "% RT");
    for (label, shipped, port) in rows {
        println!(
            "{label:<38} {shipped:>16.1} {:>8.2} {port:>16.1} {:>8.2}",
            share_of_real_time(shipped),
            share_of_real_time(port)
        );
    }

    let shipped_longest = (0..PASSES)
        .map(|_| {
            let mut decoder = Decoder::new(RATE).unwrap();
            longest_push_ns(|sample| decoder.push(sample), &with_noise)
        })
        .min()
        .unwrap();
    let port_longest = (0..PASSES)
        .map(|_| {
            let mut port = AudioMothPort::new(RATE);
            longest_push_ns(|sample| port.push(sample), &with_noise)
        })
        .min()
        .unwrap();
    println!("Longest single push, smallest of {PASSES} runs: Decoder {shipped_longest} ns, Port {port_longest} ns");
}

// -- Regression check ----------------------------------------------------------

/// A few cells of the matrix, with the number of passes out of 6 that the shipped
/// decoder must reach. The seeds are fixed, so the result changes only when the decoder
/// or the simulator changes. Each noisy cell allows one miss below what the matrix shows.
#[test]
fn shipped_decoder_holds_its_expected_performance() {
    let utc_snr_10 = Condition { time_zone: Some(0), snr_db: Some(10.0), ..CLEAN };
    let carrier_40 = Condition { carrier_peak_dbfs: Some(-40.0), ..CLEAN };
    let cells = [
        ("clean", CLEAN, 6),
        ("rate -500 ppm", Condition { rate_ppm: -500.0, ..CLEAN }, 6),
        ("melody +40 dB", Condition { melody_db: 40.0, ..CLEAN }, 6),
        ("clipping, 40 dB overdrive", Condition { overdrive_db: Some(40.0), ..CLEAN }, 6),
        ("reverb medium", Condition { reverb: Some(ReverbProfile::Medium), ..CLEAN }, 6),
        ("SNR -4 dB", Condition { snr_db: Some(-4.0), ..CLEAN }, 5),
        ("FIELD-A", FIELD_A, 5),
        // A moving phone. The first three read 7, 15 and 19 of 20 in table 3 with one
        // trellis and with echo taps that learned during runs of equal symbols.
        ("phone moves 2 cm at 2 Hz, UTC, SNR +10", Condition { acoustic: Some(hand_wobble::<20>), ..utc_snr_10 }, 5),
        ("phone moves 1 cm at 2 Hz, UTC, SNR +10", Condition { acoustic: Some(hand_wobble::<10>), ..utc_snr_10 }, 6),
        ("phone moves 5 mm at 2 Hz, UTC, SNR +10", Condition { acoustic: Some(hand_wobble::<5>), ..utc_snr_10 }, 6),
        ("reverb small, moves 2 cm, UTC, SNR +10", Condition { acoustic: Some(moves_in_small_room), ..utc_snr_10 }, 5),
        // An overloaded input. These read 15 of 20 and 0 of 40 without the limit on loud
        // samples and the second trellis.
        ("melody +30 dB, 20 dB overdrive", Condition { melody_db: 30.0, overdrive_db: Some(20.0), ..CLEAN }, 6),
        ("melody +20 dB, 40 dB overdrive", OVERLOADED, 5),
        // Loud bursts in the band. This read 7 of 20 without the limit on loud samples.
        ("carrier -40, damped rings 0 dBFS", Condition { added: Some(damped_rings::<0>), ..carrier_40 }, 5),
    ];
    let rows: Vec<Row> =
        cells.iter().map(|&(label, condition, _)| Row { trials: 6, ..row(label, condition) }).collect();
    let results = run_rows(&rows, Decoders::ShippedOnly);
    for ((label, _, required), result) in cells.iter().zip(&results) {
        let tally = &result.shipped;
        assert_eq!(tally.wrong_payloads, 0, "{label}: a wrong payload passed the CRC");
        assert!(tally.passes >= *required, "{label}: {} of 6 passed", tally.passes);
        if let Some(stats) = error_stats(&tally.start_errors_ms) {
            assert!(stats.largest_size <= 2.0, "{label}: start_sample is {} ms off", stats.largest_size);
        }
    }
}

/// The echoes of a chime that was read must not count as a second chime. A hard room with
/// no noise is the worst case, because the echoes stay above the noise for longest. This
/// gave one `Unreadable` event for each chime read when the wait after a chime was a
/// fixed 300 ms.
#[test]
fn echoes_after_a_read_give_no_event() {
    let cells = [
        ("reverb large", Condition { reverb: Some(ReverbProfile::Large), ..CLEAN }),
        ("reverb large, UTC", Condition { reverb: Some(ReverbProfile::Large), time_zone: Some(0), ..CLEAN }),
    ];
    let rows: Vec<Row> = cells.iter().map(|&(label, condition)| Row { trials: 8, ..row(label, condition) }).collect();
    let results = run_rows(&rows, Decoders::ShippedOnly);
    for ((label, _), result) in cells.iter().zip(&results) {
        let tally = &result.shipped;
        assert!(tally.passes >= 6, "{label}: {} of 8 passed", tally.passes);
        // Only a trial that failed may give an `Unreadable` event.
        assert!(
            tally.unreadable_events <= 8 - tally.passes,
            "{label}: {} passed and {} unreadable",
            tally.passes,
            tally.unreadable_events
        );
    }
}

/// Runs a stream through the shipped decoder and counts the events of each kind.
fn event_counts(rate: u32, stream: &[f64]) -> (usize, usize) {
    let events = decoder_events(rate, &channel::to_f32(stream));
    let chimes = events.iter().filter(|event| matches!(event, Event::Chime(_))).count();
    (chimes, events.len() - chimes)
}

/// A sound with no chime in it must not be reported as a chime that failed. Tone bursts
/// near 18 kHz start many walks. These three streams of 60 s gave 12, 12 and 2 `Unreadable`
/// events before such an event needed evidence of a chime, which is the rate of the worst
/// real recording. A long tone at the right frequency can look like start symbols, so
/// this test sets a low limit on the count and does not demand none.
#[test]
fn sounds_with_no_chime_give_few_unreadable_events() {
    let rate = 48_000;
    let seconds = 60;
    let mut total = 0;
    for (trial, amplitude) in [0.01, 0.1, 0.003].into_iter().enumerate() {
        let quiet = vec![0.0; seconds * rate as usize];
        let noise = channel::add_white_noise_rms(quiet, 0.003, 900 + trial as u64);
        let stream = tone_bursts(noise, rate, amplitude, 910 + trial as u64);
        let (chimes, unreadable) = event_counts(rate, &stream);
        assert_eq!(chimes, 0, "amplitude {amplitude}");
        total += unreadable;
    }
    assert!(total <= 2, "{total} unreadable events in {} s", 3 * seconds);
}

/// A chime that fails must still be reported, because that tells a person to move the
/// phone closer. Each cell gives the share of the trials that did not pass that must
/// have given an `Unreadable` event, in percent. The noisy cells are below 100 because
/// a chime far below the noise can show too little to be told from noise.
#[test]
fn failed_chimes_are_still_reported() {
    let cells = [
        ("reverb large, SNR +3", Condition { reverb: Some(ReverbProfile::Large), snr_db: Some(3.0), ..CLEAN }, 90),
        ("FIELD-B", FIELD_B, 80),
        ("SNR -8 dB", Condition { snr_db: Some(-8.0), ..CLEAN }, 25),
    ];
    let rows: Vec<Row> = cells.iter().map(|&(label, condition, _)| Row { trials: 20, ..row(label, condition) }).collect();
    let results = run_rows(&rows, Decoders::ShippedOnly);
    for ((label, _, percent), result) in cells.iter().zip(&results) {
        let tally = &result.shipped;
        let failed = 20 - tally.passes;
        assert!(failed >= 4, "{label}: {} of 20 passed, which leaves too few failures to judge", tally.passes);
        assert!(
            tally.unreadable_events * 100 >= failed * percent,
            "{label}: {} unreadable events for {failed} failures",
            tally.unreadable_events
        );
    }

    // A clean chime cut off at any point after its start symbols.
    let rate = 48_000u32;
    let whole = generator::render(rate, &generator::time_payload(1_793_000_000, 0), tune_for_payload(6)).mix();
    for tenths in 2..=9 {
        let cut = whole[..whole.len() * tenths / 10].to_vec();
        let stream = channel::add_white_noise_rms(channel::pad(cut, rate as usize / 2, rate as usize), 1e-3, tenths as u64);
        let (chimes, unreadable) = event_counts(rate, &stream);
        assert_eq!((chimes, unreadable), (0, 1), "cut off at {tenths} tenths");
    }
}
