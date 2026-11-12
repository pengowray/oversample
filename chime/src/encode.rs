//! The chime encoder, for tests and test files (feature `std`): a Rust port of the encoder in
//! `audiomothchime.js` in the `batchi-chime` repository and of the payload and tune code in
//! `audiomothchime_connector.js`, by Open Acoustic Devices (MIT licence).
//!
//! The port keeps the JavaScript's f64 expressions and their order, because the
//! sample counts are not rounded: the encoder compares a loop counter against
//! products such as `0.0005 * sampleRate`, and floating point error in those
//! products decides whether a symbol gets one sample more. The test file
//! `tests/generator.rs` checks the result against a render from the JavaScript.

use std::f64::consts::PI;
use std::vec;
use std::vec::Vec;

const CARRIER_FREQUENCY: f64 = 18000.0;
const NUMBER_OF_STOP_BITS: usize = 8;
const NUMBER_OF_START_BITS: usize = 16;

const BIT_RISE: f64 = 0.0005;
const BIT_FALL: f64 = 0.0005;
const LOW_BIT_SUSTAIN: f64 = 0.004;
const HIGH_BIT_SUSTAIN: f64 = 0.009;
const START_STOP_BIT_SUSTAIN: f64 = 0.0065;

const NOTE_RISE_DURATION: f64 = 0.030;
const NOTE_FALL_DURATION: f64 = 0.030;
const NOTE_LONG_FALL_DURATION: f64 = 0.090;

const HAMMING_CODES: [[u8; 7]; 16] = [
    [0, 0, 0, 0, 0, 0, 0],
    [1, 1, 1, 0, 0, 0, 0],
    [1, 0, 0, 1, 1, 0, 0],
    [0, 1, 1, 1, 1, 0, 0],
    [0, 1, 0, 1, 0, 1, 0],
    [1, 0, 1, 1, 0, 1, 0],
    [1, 1, 0, 0, 1, 1, 0],
    [0, 0, 1, 0, 1, 1, 0],
    [1, 1, 0, 1, 0, 0, 1],
    [0, 0, 1, 1, 0, 0, 1],
    [0, 1, 0, 0, 1, 0, 1],
    [1, 0, 1, 0, 1, 0, 1],
    [1, 0, 0, 0, 0, 1, 1],
    [0, 1, 1, 0, 0, 1, 1],
    [0, 0, 0, 1, 1, 1, 1],
    [1, 1, 1, 1, 1, 1, 1],
];

/// A note name and its relative duration, written "C5:1" in the JavaScript.
pub type Note = (&'static str, u32);

/// Tune for a payload of time only (6 bytes).
pub const TUNE_TIME: &[Note] = &[("C5", 1), ("D5", 1), ("E5", 1), ("C5", 3)];

/// Tune for a payload of time and location (13 bytes).
pub const TUNE_TIME_LOCATION: &[Note] = &[
    ("C5", 1),
    ("D5", 1),
    ("E5", 1),
    ("C5", 3),
    ("D5", 1),
    ("E5", 1),
    ("C5", 3),
];

/// Tune for a payload of time and deployment ID (14 bytes).
pub const TUNE_DEPLOYMENT: &[Note] = &[
    ("Eb5", 1),
    ("G5", 1),
    ("D5", 1),
    ("F#5", 1),
    ("Db5", 1),
    ("F5", 1),
    ("C5", 1),
    ("E5", 5),
];

/// Tune for a payload of time, location and deployment ID (21 bytes).
pub const TUNE_DEPLOYMENT_LOCATION: &[Note] = &[
    ("Eb5", 1),
    ("G5", 1),
    ("D5", 1),
    ("F#5", 1),
    ("Db5", 1),
    ("F5", 1),
    ("C5", 1),
    ("E5", 5),
    ("Db5", 1),
    ("F5", 1),
    ("C5", 1),
    ("E5", 4),
];

/// The tune the connector plays for a payload of `payload_len` bytes. Panics for
/// a length the connector never produces.
pub fn tune_for_payload(payload_len: usize) -> &'static [Note] {
    match payload_len {
        6 => TUNE_TIME,
        13 => TUNE_TIME_LOCATION,
        14 => TUNE_DEPLOYMENT,
        21 => TUNE_DEPLOYMENT_LOCATION,
        _ => panic!("the connector has no tune for a payload of {payload_len} bytes"),
    }
}

/// The two parts of a chime before they are mixed. Each part has amplitude 1.
pub struct Render {
    /// `waveform1` in the JavaScript: the 18 kHz symbols.
    pub carrier: Vec<f64>,
    /// `waveform2` in the JavaScript: the melody.
    pub melody: Vec<f64>,
    /// The index in `carrier` where each symbol starts: start symbols, data
    /// symbols, then stop symbols.
    pub symbol_starts: Vec<usize>,
}

impl Render {
    /// The waveform the encoder plays: carrier / 4 + melody / 2 over the length
    /// of the shorter part.
    pub fn mix(&self) -> Vec<f64> {
        self.carrier
            .iter()
            .zip(&self.melody)
            .map(|(c, m)| c / 4.0 + m / 2.0)
            .collect()
    }

    /// The same mix with other gains, for changing the balance of carrier and
    /// melody. `mix_with_gains(0.25, 0.5)` equals `mix()`.
    pub fn mix_with_gains(&self, carrier_gain: f64, melody_gain: f64) -> Vec<f64> {
        self.carrier
            .iter()
            .zip(&self.melody)
            .map(|(c, m)| c * carrier_gain + m * melody_gain)
            .collect()
    }
}

/// The JavaScript's `state` object. The carrier keeps one state across all its
/// symbols and the melody keeps one across all its notes, so the oscillators run
/// without a phase jump. The polarity flip of the carrier is the `phase` argument
/// of `create_waveform_component`.
struct OscillatorState {
    amplitude_phase: f64,
    x: f64,
    y: f64,
}

impl OscillatorState {
    fn new() -> Self {
        Self {
            amplitude_phase: 0.0,
            x: 1.0,
            y: 0.0,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn create_waveform_component(
    waveform: &mut Vec<f64>,
    state: &mut OscillatorState,
    sample_rate: f64,
    frequency: f64,
    phase: f64,
    ramp_up: f64,
    sustain: f64,
    ramp_down: f64,
) {
    let samples_in_ramp_up = ramp_up * sample_rate;
    let samples_in_sustain = sustain * sample_rate;
    let samples_in_ramp_down = ramp_down * sample_rate;
    let theta = 2.0 * PI * frequency / sample_rate;
    let (sin_theta, cos_theta) = (theta.sin(), theta.cos());

    let mut k = 0.0;
    while k < samples_in_ramp_up + samples_in_sustain + samples_in_ramp_down {
        if k < samples_in_ramp_up {
            state.amplitude_phase =
                (PI / 2.0).min(state.amplitude_phase + PI / 2.0 / samples_in_ramp_up);
        }
        if k >= samples_in_ramp_up + samples_in_sustain {
            state.amplitude_phase =
                (0.0f64).max(state.amplitude_phase - PI / 2.0 / samples_in_ramp_down);
        }

        let volume = state.amplitude_phase.sin().powi(2);
        waveform.push(volume * phase * state.x);

        let x = state.x * cos_theta - state.y * sin_theta;
        let y = state.x * sin_theta + state.y * cos_theta;
        state.x = x;
        state.y = y;

        k += 1.0;
    }
}

/// The frequency of a note such as "F#5", from `generateFrequencyLookup`. The
/// JavaScript rounds it to a whole number of Hz.
fn note_frequency(note: &str) -> f64 {
    const NOTE_PREFIXES: [&str; 17] = [
        "C", "C#", "Db", "D", "D#", "Eb", "E", "F", "F#", "Gb", "G", "G#", "Ab", "A", "A#", "Bb",
        "B",
    ];
    const NOTE_DISTANCES: [i32; 17] = [
        -9, -8, -8, -7, -6, -6, -5, -4, -3, -3, -2, -1, -1, 0, 1, 1, 2,
    ];

    let (prefix, octave) = note.split_at(note.len() - 1);
    let octave: i32 = octave
        .parse()
        .expect("a note name ends with an octave digit");
    let index = NOTE_PREFIXES
        .iter()
        .position(|p| *p == prefix)
        .expect("known note name");
    let exponent = (octave - 4) as f64 + NOTE_DISTANCES[index] as f64 / 12.0;
    (440.0 * 2.0f64.powf(exponent) + 0.5).floor()
}

/// Port of `crc16`. Returns the low byte, then the high byte.
pub fn crc16(bytes: &[u8]) -> [u8; 2] {
    fn crc_update(crc_in: u32, incr: bool) -> u32 {
        let xor = (crc_in >> 15) & 65535;
        let mut out = (crc_in << 1) & 65535;
        if incr {
            out += 1;
        }
        if xor > 0 {
            out ^= 0x1021;
        }
        out
    }

    let mut crc = 0;
    for &byte in bytes {
        for j in (0..8).rev() {
            crc = crc_update(crc, byte & (1 << j) != 0);
        }
    }
    for _ in 0..16 {
        crc = crc_update(crc, false);
    }
    [(crc & 255) as u8, ((crc >> 8) & 255) as u8]
}

/// Port of `encode`: the data symbols for `bytes`, 14 for each byte.
pub fn encode(bytes: &[u8]) -> Vec<u8> {
    let mut bits = Vec::with_capacity(bytes.len() * 14);
    for &byte in bytes {
        let low = (byte & 0x0F) as usize;
        let high = (byte >> 4) as usize;
        for (low_bit, high_bit) in HAMMING_CODES[low].iter().zip(&HAMMING_CODES[high]) {
            bits.push(*low_bit);
            bits.push(*high_bit);
        }
    }
    bits
}

/// Port of `createWaveform` for a chime with data. `bytes` is the payload without
/// its CRC, which this function appends.
pub fn render(sample_rate: u32, bytes: &[u8], notes: &[Note]) -> Render {
    let rate = sample_rate as f64;

    let (frequencies, note_durations): (Vec<f64>, Vec<f64>) = if notes.is_empty() {
        (vec![523.0], vec![1.0])
    } else {
        notes
            .iter()
            .map(|(name, length)| (note_frequency(name), *length as f64))
            .unzip()
    };

    let mut carrier = Vec::new();
    let mut symbol_starts = Vec::new();
    let mut state = OscillatorState::new();
    let mut phase = 1.0;

    let mut carrier_symbol = |sustain: f64, carrier: &mut Vec<f64>, state: &mut OscillatorState| {
        symbol_starts.push(carrier.len());
        create_waveform_component(
            carrier,
            state,
            rate,
            CARRIER_FREQUENCY,
            phase,
            BIT_RISE,
            sustain,
            BIT_FALL,
        );
        phase *= -1.0;
    };

    for _ in 0..NUMBER_OF_START_BITS {
        carrier_symbol(START_STOP_BIT_SUSTAIN, &mut carrier, &mut state);
    }

    let mut message = bytes.to_vec();
    message.extend_from_slice(&crc16(bytes));
    for bit in encode(&message) {
        let sustain = if bit == 1 {
            HIGH_BIT_SUSTAIN
        } else {
            LOW_BIT_SUSTAIN
        };
        carrier_symbol(sustain, &mut carrier, &mut state);
    }

    for _ in 0..NUMBER_OF_STOP_BITS {
        carrier_symbol(START_STOP_BIT_SUSTAIN, &mut carrier, &mut state);
    }

    let mut state = OscillatorState::new();

    let sum_of_note_durations: f64 = note_durations.iter().sum();
    let mut note_duration = carrier.len() as f64 / rate
        - note_durations.len() as f64 * (NOTE_RISE_DURATION + NOTE_FALL_DURATION)
        + NOTE_FALL_DURATION
        - NOTE_LONG_FALL_DURATION;
    note_duration /= sum_of_note_durations;

    let mut melody = Vec::new();
    for i in 0..note_durations.len() {
        let note_fall = if i == note_durations.len() - 1 {
            NOTE_LONG_FALL_DURATION
        } else {
            NOTE_FALL_DURATION
        };
        create_waveform_component(
            &mut melody,
            &mut state,
            rate,
            frequencies[i],
            1.0,
            NOTE_RISE_DURATION,
            note_duration * note_durations[i],
            note_fall,
        );
    }

    Render {
        carrier,
        melody,
        symbol_starts,
    }
}

/// Port of the connector's `encodeTime`. The connector clamps the time zone to
/// -720..=840 minutes.
pub fn time_payload(unix: u32, tz_minutes: i16) -> Vec<u8> {
    let tz = tz_minutes.clamp(-720, 840);
    let mut bytes = unix.to_le_bytes().to_vec();
    bytes.extend_from_slice(&tz.to_le_bytes());
    bytes
}

/// Port of the connector's `encodeLocation`: the 7 location bytes.
pub fn location_bytes(latitude: f64, longitude: f64) -> [u8; 7] {
    let latitude = (latitude.clamp(-90.0, 90.0) * 1_000_000.0).round() as i32;
    let longitude = (longitude.clamp(-180.0, 180.0) * 500_000.0).round() as i32;
    [
        (latitude & 0xFF) as u8,
        ((latitude >> 8) & 0xFF) as u8,
        ((latitude >> 16) & 0xFF) as u8,
        (((latitude >> 24) & 0x0F) | ((longitude & 0x0F) << 4)) as u8,
        ((longitude >> 4) & 0xFF) as u8,
        ((longitude >> 12) & 0xFF) as u8,
        ((longitude >> 20) & 0xFF) as u8,
    ]
}

/// Port of the connector's `encodeDeploymentID`: the ID's bytes in reverse order.
pub fn deployment_bytes(deployment_id: [u8; 8]) -> [u8; 8] {
    let mut bytes = deployment_id;
    bytes.reverse();
    bytes
}
