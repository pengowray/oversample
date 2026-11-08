// This file is a port of code from AudioMoth-Firmware-Basic by Open Acoustic
// Devices, which has this licence:
//
// MIT License
//
// Copyright (c) 2017 OpenAcousticDevices
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

//! Rust port of the chime decoder in AudioMoth-Firmware-Basic by Open Acoustic
//! Devices. It covers `AudioConfig_listenForAudioConfigurationPackets`,
//! `updateCostasLoop`, `calculateCRC` and `checkCRC` from `src/audioconfig.c`, and
//! the filters from `src/biquad.c` and `src/butterworth.c` that those functions use.
//!
//! The tests keep this port as an independent reference for the format and as the
//! baseline to compare the crate's own decoder against. It shares no code with the
//! crate apart from the `Event` types that it returns.
//!
//! # What differs from the C
//!
//! The C runs at 48 kHz only. These values depend on the sample rate here and equal
//! the C's values at 48 kHz:
//!
//! - The period constants are in samples in the C: 240, 480, 360, tolerance 60,
//!   minimum 120 and maximum 600. [`Timing`] gives them in seconds and rounds their
//!   product with the sample rate to whole samples.
//! - `SINE_CARRIER_PHASE_INCREMENT` is a phase step per sample. The C macro uses
//!   integer division, which is exact at 48000 and 192000 Hz (96 and 24). Here it is
//!   a float, so that rates such as 44100 Hz have the correct carrier frequency.
//! - `VCO_GAIN` is a phase correction per sample, so it scales by 48000 / rate.
//!   Otherwise the loop bandwidth in Hz would change with the rate.
//! - The filters are designed in Hz for the given rate, with the C's formulas.
//!
//! The AudioMoth's samples are `int16_t`, and the AGC floor of 100 is in those units,
//! so [`AudioMothPort::push`] multiplies its input by 32768. A sample of an int16 file
//! divided by 32768 reaches the Costas loop as exactly the value the C would see. The
//! port does not clip or round, so a float input outside +-1.0 or with detail below
//! 1/32768 keeps values that an AudioMoth's ADC could not produce.
//!
//! The filter design uses f32 everywhere the C uses `float`, and f64 for the two
//! calls where the C uses a `double` function: `cos` for `b2` in the biquad and
//! `csqrt` in the band-pass. `math.h` may define `M_PI` as a double, depending on the
//! compiler options, which we do not know for the AudioMoth build. This port uses the
//! f32 value that `biquad.c` and `butterworth.c` define when `math.h` does not.
//! Complex division follows `__divsc3` in libgcc for a CPU with no double-precision
//! FPU, such as the AudioMoth's Cortex-M4F.
//!
//! The port leaves out what only drives the hardware: the LED events
//! `AC_EVENT_PULSE`, `AC_EVENT_START` and `AC_EVENT_BYTE`, the listening timeout, and
//! the restart after a microphone change.
//!
//! # Fields of the events
//!
//! - `level_dbfs` and `snr_db` are NaN, because the C measures neither. Comparing two
//!   events with `==` is therefore always false, so compare their fields.
//! - `start_sample` is an estimate and it is rough. When the first data symbol ends,
//!   the port counts back from the start of the run of start periods it saw, adding
//!   one nominal start symbol length for each of the 16 start symbols that is missing
//!   from the run. The crossings it counts from lag the symbol boundaries by the delay
//!   of the filters, so on a clean chime the estimate is about 2 ms late at both 48 and
//!   192 kHz. The port does not correct for this.
//! - `end_sample` is the sample whose crossing produced the event.
//! - `corrected_bits` is not in the C. The port counts the received 7-bit words that
//!   are not codewords, using only `hammingConversion`. A word with two wrong bits also
//!   counts as one.
//!
//! # How the C's outcomes map to events
//!
//! | C outcome                               | Event                           |
//! |-----------------------------------------|---------------------------------|
//! | `AudioConfig_handleAudioConfigurationPacket` | `Event::Chime`             |
//! | `AC_EVENT_CRC_ERROR`, after stop periods or at the 23-byte limit | `Unreadable` with `BadCrc` |
//! | `AC_EVENT_BIT_ERROR`                    | `Unreadable` with `BadSymbol`   |
//!
//! `TooManyBytes` never appears, because the C checks the CRC at the 23rd byte and
//! returns to `IDLE` whatever the result. `SignalLost` never appears, because the C
//! has no timeout: after the carrier stops, it stays in `DATA_BITS` until the next
//! zero crossing, however late, which then gives `BadSymbol`.

#![allow(dead_code)]

use chime::{Chime, Event, MAX_PAYLOAD_BYTES, Unreadable, UnreadableReason};
use std::f32::consts::LN_2;

// -- Constants from audioconfig.c that do not depend on the sample rate --------

const CONFIG_CARRIER_FREQUENCY: u32 = 18000;
const SINE_TABLE_LENGTH: u32 = 256;
const SINE_QUARTER_CYCLE_INCREMENT: u8 = 64;
const MAXIMUM_NUMBER_OF_BYTES: usize = 23;
const CRC_SIZE_IN_BYTES: usize = 2;
const ENCODED_BITS_IN_BYTE: u32 = 14;
const MIN_NUMBER_OF_START_STOP_PERIODS: u32 = 4;
const MAX_NUMBER_OF_START_STOP_PERIODS: u32 = 24;
const AGC_MINIMUM_AMPLITUDE: f32 = 100.0;
const AGC_FILTER_CUTOFF_FREQUENCY: u32 = 1000;
const CHANNEL_FILTER_CUTOFF_FREQUENCY: u32 = 100;
const CHANNEL_FILTER_BANDWIDTH: f32 = 2.0;
const CARRIER_FILTER_CUTOFF_FREQUENCY: u32 = 1000;

/// `VCO_GAIN` in the C, which applies at 48 kHz.
const VCO_GAIN_AT_48000: f32 = 0.5;

/// The sample rate that the C's sample-based constants assume.
const CONFIG_SAMPLE_RATE: f32 = 48000.0;

/// Converts a +-1.0 float input to the units of the AudioMoth's `int16_t` samples.
const INT16_SCALE: f32 = 32768.0;

/// Number of start symbols in a chime, from `NUMBER_OF_START_BITS` in
/// `audiomothchime.js`. The C does not use this number. The port needs it only for
/// the `start_sample` estimate.
const START_SYMBOLS_IN_CHIME: i64 = 16;

// The period constants of the C divided by 48000.
const MIN_BIT_SECONDS: f64 = 0.0025;
const MAX_BIT_SECONDS: f64 = 0.0125;
const LOW_BIT_SECONDS: f64 = 0.005;
const HIGH_BIT_SECONDS: f64 = 0.010;
const START_STOP_BIT_SECONDS: f64 = 0.0075;
const PERIOD_TOLERANCE_SECONDS: f64 = 0.00125;

/// `sineTable` from audioconfig.c, verbatim. It is not a clean sine: entry 128 is
/// -0.000000087423, and the second half is not the exact negative of the first.
#[allow(clippy::excessive_precision)]
pub const SINE_TABLE: [f32; 256] = [
    0.000000000000, 0.024541229010, 0.049067676067, 0.073564566672,
    0.098017141223, 0.122410677373, 0.146730467677, 0.170961901546,
    0.195090323687, 0.219101235271, 0.242980197072, 0.266712784767,
    0.290284663439, 0.313681751490, 0.336889863014, 0.359895050526,
    0.382683455944, 0.405241340399, 0.427555084229, 0.449611335993,
    0.471396744251, 0.492898225784, 0.514102756977, 0.534997642040,
    0.555570244789, 0.575808227062, 0.595699310303, 0.615231633186,
    0.634393334389, 0.653172850609, 0.671558976173, 0.689540565014,
    0.707106769085, 0.724247097969, 0.740951180458, 0.757208883762,
    0.773010432720, 0.788346409798, 0.803207516670, 0.817584812641,
    0.831469655037, 0.844853579998, 0.857728660107, 0.870086967945,
    0.881921291351, 0.893224298954, 0.903989315033, 0.914209783077,
    0.923879504204, 0.932992815971, 0.941544055939, 0.949528217316,
    0.956940352917, 0.963776051998, 0.970031261444, 0.975702106953,
    0.980785310268, 0.985277652740, 0.989176511765, 0.992479562759,
    0.995184719563, 0.997290432453, 0.998795449734, 0.999698817730,
    1.000000000000, 0.999698817730, 0.998795449734, 0.997290432453,
    0.995184719563, 0.992479503155, 0.989176511765, 0.985277652740,
    0.980785250664, 0.975702106953, 0.970031261444, 0.963776051998,
    0.956940293312, 0.949528157711, 0.941544055939, 0.932992815971,
    0.923879504204, 0.914209723473, 0.903989315033, 0.893224298954,
    0.881921231747, 0.870087027550, 0.857728600502, 0.844853520393,
    0.831469535828, 0.817584812641, 0.803207516670, 0.788346350193,
    0.773010492325, 0.757208824158, 0.740951061249, 0.724246978760,
    0.707106769085, 0.689540505409, 0.671558856964, 0.653172850609,
    0.634393274784, 0.615231513977, 0.595699131489, 0.575808167458,
    0.555570185184, 0.534997463226, 0.514102756977, 0.492898136377,
    0.471396625042, 0.449611365795, 0.427555054426, 0.405241221189,
    0.382683277130, 0.359895050526, 0.336889803410, 0.313681602478,
    0.290284723043, 0.266712725163, 0.242980077863, 0.219101071358,
    0.195090308785, 0.170961812139, 0.146730333567, 0.122410699725,
    0.098017096519, 0.073564447463, 0.049067486078, 0.024541210383,
    -0.000000087423, -0.024541385472, -0.049067661166, -0.073564618826,
    -0.098017267883, -0.122410871089, -0.146730497479, -0.170961990952,
    -0.195090487599, -0.219101235271, -0.242980241776, -0.266712903976,
    -0.290284872055, -0.313681781292, -0.336889952421, -0.359895199537,
    -0.382683426142, -0.405241400003, -0.427555233240, -0.449611514807,
    -0.471396774054, -0.492898285389, -0.514102876186, -0.534997642040,
    -0.555570304394, -0.575808346272, -0.595699310303, -0.615231633186,
    -0.634393393993, -0.653172969818, -0.671558976173, -0.689540624619,
    -0.707106888294, -0.724247097969, -0.740951180458, -0.757208764553,
    -0.773010432720, -0.788346469402, -0.803207576275, -0.817584931850,
    -0.831469774246, -0.844853758812, -0.857728540897, -0.870086967945,
    -0.881921291351, -0.893224358559, -0.903989374638, -0.914209842682,
    -0.923879683018, -0.932992756367, -0.941544055939, -0.949528217316,
    -0.956940352917, -0.963776111603, -0.970031321049, -0.975702226162,
    -0.980785250664, -0.985277652740, -0.989176511765, -0.992479562759,
    -0.995184719563, -0.997290492058, -0.998795449734, -0.999698817730,
    -1.000000000000, -0.999698817730, -0.998795449734, -0.997290432453,
    -0.995184719563, -0.992479503155, -0.989176511765, -0.985277652740,
    -0.980785250664, -0.975702106953, -0.970031201839, -0.963775992393,
    -0.956940233707, -0.949528217316, -0.941544055939, -0.932992756367,
    -0.923879444599, -0.914209663868, -0.903989136219, -0.893224120140,
    -0.881921291351, -0.870086967945, -0.857728540897, -0.844853460789,
    -0.831469476223, -0.817584633827, -0.803207576275, -0.788346409798,
    -0.773010432720, -0.757208764553, -0.740951001644, -0.724246919155,
    -0.707106530666, -0.689540624619, -0.671558976173, -0.653172791004,
    -0.634393155575, -0.615231454372, -0.595699071884, -0.575807929039,
    -0.555570304394, -0.534997642040, -0.514102697372, -0.492898076773,
    -0.471396535635, -0.449611067772, -0.427554786205, -0.405241370201,
    -0.382683426142, -0.359894961119, -0.336889714003, -0.313681542873,
    -0.290284395218, -0.266712397337, -0.242980226874, -0.219101220369,
    -0.195090234280, -0.170961722732, -0.146730244160, -0.122410371900,
    -0.098016768694, -0.073564596474, -0.049067638814, -0.024541122839,
];

/// `hammingConversion` from audioconfig.c, verbatim. It maps a received 7-bit word
/// to a nibble.
pub const HAMMING_CONVERSION: [u8; 128] = [
    0, 0, 0, 1, 0, 1, 1, 1, //
    0, 2, 4, 8, 9, 5, 3, 1, //
    0, 2, 10, 6, 7, 11, 3, 1, //
    2, 2, 3, 2, 3, 2, 3, 3, //
    0, 12, 4, 6, 7, 5, 13, 1, //
    4, 5, 4, 4, 5, 5, 4, 5, //
    7, 6, 6, 6, 7, 7, 7, 6, //
    14, 2, 4, 6, 7, 5, 3, 15, //
    0, 12, 10, 8, 9, 11, 13, 1, //
    9, 8, 8, 8, 9, 9, 9, 8, //
    10, 11, 10, 10, 11, 11, 10, 11, //
    14, 2, 10, 8, 9, 11, 3, 15, //
    12, 12, 13, 12, 13, 12, 13, 13, //
    14, 12, 4, 8, 9, 5, 13, 15, //
    14, 12, 10, 6, 7, 11, 13, 15, //
    14, 14, 14, 15, 14, 15, 15, 15,
];

// -- CRC ---------------------------------------------------------------------

/// `calculateCRC` from audioconfig.c.
#[allow(clippy::manual_rotate)]
pub fn calculate_crc(bytes: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &data in bytes {
        crc = (crc >> 8) | (crc << 8);
        crc ^= data as u16;
        crc ^= (crc & 0xFF) >> 4;
        crc ^= crc << 12;
        crc ^= (crc & 0xFF) << 5;
    }
    crc
}

/// `checkCRC` from audioconfig.c. `bytes` ends with the CRC, low byte first.
pub fn check_crc(bytes: &[u8]) -> bool {
    let length = bytes.len();
    let crc = calculate_crc(&bytes[..length - CRC_SIZE_IN_BYTES]);
    let low = (crc & 0xFF) as u8;
    let high = (crc >> 8) as u8;
    low == bytes[length - 2] && high == bytes[length - 1]
}

// -- Hamming code ------------------------------------------------------------

/// Whether `word` is a codeword, judged from [`HAMMING_CONVERSION`] alone: a codeword
/// decodes to the same nibble as all 7 words that are one bit away from it.
fn is_codeword(word: u8) -> bool {
    let nibble = HAMMING_CONVERSION[word as usize];
    (0..7).all(|bit| HAMMING_CONVERSION[(word ^ (1 << bit)) as usize] == nibble)
}

// -- Complex numbers for the Butterworth design -------------------------------

/// `complex float` in the C.
#[derive(Clone, Copy)]
struct Complex {
    re: f32,
    im: f32,
}

impl Complex {
    const fn real(re: f32) -> Self {
        Self { re, im: 0.0 }
    }

    fn add(self, other: Self) -> Self {
        Self { re: self.re + other.re, im: self.im + other.im }
    }

    fn sub(self, other: Self) -> Self {
        Self { re: self.re - other.re, im: self.im - other.im }
    }

    fn neg(self) -> Self {
        Self { re: -self.re, im: -self.im }
    }

    fn mul(self, other: Self) -> Self {
        Self {
            re: self.re * other.re - self.im * other.im,
            im: self.re * other.im + self.im * other.re,
        }
    }

    /// `__divsc3` in libgcc on a CPU with no double-precision FPU.
    fn div(self, other: Self) -> Self {
        let (a, b, c, d) = (self.re, self.im, other.re, other.im);
        if c.abs() < d.abs() {
            let ratio = c / d;
            let denominator = c * ratio + d;
            Self { re: (a * ratio + b) / denominator, im: (b * ratio - a) / denominator }
        } else {
            let ratio = d / c;
            let denominator = d * ratio + c;
            Self { re: (b * ratio + a) / denominator, im: (b - a * ratio) / denominator }
        }
    }

    /// `csqrt` in the C, which takes and returns `complex double`.
    fn sqrt(self) -> Self {
        let (a, b) = (self.re as f64, self.im as f64);
        let modulus = a.hypot(b);
        let re = ((modulus + a) / 2.0).sqrt();
        let im = ((modulus - a) / 2.0).sqrt().copysign(b);
        Self { re: re as f32, im: im as f32 }
    }
}

// -- butterworth.c -----------------------------------------------------------

/// `M_PI` as `biquad.c` and `butterworth.c` define it when `math.h` does not.
const M_PI: f32 = std::f32::consts::PI;
const M_TWOPI: f32 = 2.0 * M_PI;

fn blt(pz: Complex) -> Complex {
    Complex::real(2.0).add(pz).div(Complex::real(2.0).sub(pz))
}

/// Writes the coefficients of the polynomial with roots `pz` to `coeffs`, which has
/// one more entry than `pz`.
fn expand(pz: &[Complex], coeffs: &mut [Complex]) {
    let npz = pz.len();
    coeffs[0] = Complex::real(1.0);
    for coefficient in &mut coeffs[1..=npz] {
        *coefficient = Complex::real(0.0);
    }
    for &root in pz {
        let nw = root.neg();
        for i in (1..=npz).rev() {
            coeffs[i] = nw.mul(coeffs[i]).add(coeffs[i - 1]);
        }
        coeffs[0] = nw.mul(coeffs[0]);
    }
}

fn eval(coeffs: &[Complex], z: Complex) -> Complex {
    let mut sum = Complex::real(0.0);
    for &coefficient in coeffs.iter().rev() {
        sum = sum.mul(z).add(coefficient);
    }
    sum
}

fn evaluate(topco: &[Complex], botco: &[Complex], z: Complex) -> Complex {
    eval(topco, z).div(eval(botco, z))
}

/// `BW_filterCoefficients_t`.
#[derive(Clone, Copy, Debug)]
pub struct ButterworthCoefficients {
    pub gain: f32,
    pub yc: [f32; 2],
}

/// `Butterworth_designLowPassFilter`: a first-order low-pass filter.
pub fn butterworth_design_low_pass(sample_rate: u32, frequency: u32) -> ButterworthCoefficients {
    let alpha1 = frequency as f32 / sample_rate as f32;
    let warped_alpha1 = (M_PI * alpha1).tan() / M_PI;

    // The C writes this as M_TWOPI * warped_alpha1 * spoles[0], with spoles[0] = -1 + 0i.
    let spole = Complex::real(-(M_TWOPI * warped_alpha1));

    let zpoles = [blt(spole)];
    let zzeros = [Complex::real(-1.0)];

    let mut topcoeffs = [Complex::real(0.0); 2];
    let mut botcoeffs = [Complex::real(0.0); 2];
    expand(&zzeros, &mut topcoeffs);
    expand(&zpoles, &mut botcoeffs);

    let yc0 = -(botcoeffs[0].re / botcoeffs[1].re);
    let dc_gain = evaluate(&topcoeffs, &botcoeffs, Complex::real(1.0));
    let gain = 1.0 / dc_gain.re.hypot(dc_gain.im);

    ButterworthCoefficients { gain, yc: [yc0, 0.0] }
}

/// `Butterworth_designBandPassFilter`: a second-order band-pass filter from `freq1`
/// to `freq2`.
pub fn butterworth_design_band_pass(
    sample_rate: u32,
    freq1: u32,
    freq2: u32,
) -> ButterworthCoefficients {
    let alpha1 = freq1 as f32 / sample_rate as f32;
    let alpha2 = freq2 as f32 / sample_rate as f32;

    let warped_alpha1 = (M_PI * alpha1).tan() / M_PI;
    let warped_alpha2 = (M_PI * alpha2).tan() / M_PI;

    let w1 = M_TWOPI * warped_alpha1;
    let w2 = M_TWOPI * warped_alpha2;
    let w0 = (w1 * w2).sqrt();
    let bw = w2 - w1;

    // The C writes this as 0.5f * bw * spoles[0], with spoles[0] = -1 + 0i.
    let hba = Complex::real(-(0.5 * bw));

    let w0_over_hba = Complex::real(w0).div(hba);
    let temp = Complex::real(1.0).sub(w0_over_hba.mul(w0_over_hba)).sqrt();

    let spoles = [hba.mul(Complex::real(1.0).add(temp)), hba.mul(Complex::real(1.0).sub(temp))];

    let zpoles = [blt(spoles[0]), blt(spoles[1])];
    let zzeros = [blt(Complex::real(0.0)), Complex::real(-1.0)];

    let mut topcoeffs = [Complex::real(0.0); 3];
    let mut botcoeffs = [Complex::real(0.0); 3];
    expand(&zzeros, &mut topcoeffs);
    expand(&zpoles, &mut botcoeffs);

    let yc0 = -(botcoeffs[0].re / botcoeffs[2].re);
    let yc1 = -(botcoeffs[1].re / botcoeffs[2].re);

    let theta = M_TWOPI * 0.5 * (alpha1 + alpha2);
    let exp_theta = Complex { re: theta.cos(), im: theta.sin() };
    let fc_gain = evaluate(&topcoeffs, &botcoeffs, exp_theta);
    let gain = 1.0 / fc_gain.re.hypot(fc_gain.im);

    ButterworthCoefficients { gain, yc: [yc0, yc1] }
}

/// `BW_filter_t`.
#[derive(Clone, Copy, Default)]
struct ButterworthFilter {
    xv: [f32; 3],
    yv: [f32; 3],
}

impl ButterworthFilter {
    /// `Butterworth_applyLowPassFilter`.
    fn apply_low_pass(&mut self, sample: f32, coefficients: &ButterworthCoefficients) -> f32 {
        self.xv[0] = self.xv[1];
        self.xv[1] = sample * coefficients.gain;

        self.yv[0] = self.yv[1];
        self.yv[1] = self.xv[0] + self.xv[1] + coefficients.yc[0] * self.yv[0];

        self.yv[1]
    }

    /// `Butterworth_applyBandPassFilter`.
    fn apply_band_pass(&mut self, sample: f32, coefficients: &ButterworthCoefficients) -> f32 {
        self.xv[0] = self.xv[1];
        self.xv[1] = self.xv[2];
        self.xv[2] = sample * coefficients.gain;

        self.yv[0] = self.yv[1];
        self.yv[1] = self.yv[2];
        self.yv[2] = self.xv[2] - self.xv[0]
            + coefficients.yc[0] * self.yv[0]
            + coefficients.yc[1] * self.yv[1];

        self.yv[2]
    }
}

// -- biquad.c ----------------------------------------------------------------

/// `BQ_filterCoefficients_t`.
#[derive(Clone, Copy, Debug)]
pub struct BiquadCoefficients {
    pub b0_a0: f32,
    pub b1_a0: f32,
    pub b2_a0: f32,
    pub a1_a0: f32,
    pub a2_a0: f32,
}

/// `Biquad_designLowPassFilter`. `bandwidth` is in octaves.
pub fn biquad_design_low_pass(sample_rate: u32, frequency: u32, bandwidth: f32) -> BiquadCoefficients {
    let omega = 2.0 * M_PI * frequency as f32 / sample_rate as f32;
    let alpha = omega.sin() * (LN_2 / 2.0 * bandwidth * omega / omega.sin()).sinh();

    let b0 = (1.0 - omega.cos()) / 2.0;
    let b1 = 1.0 - omega.cos();
    // The C calls the double cos() for b2 and cosf() for b0. At 192 kHz the two
    // results differ by about 0.2%, because 1 - cos(omega) is only about 90 times the
    // rounding step of cosf().
    let b2 = ((1.0 - (omega as f64).cos()) / 2.0) as f32;

    let a0 = 1.0 + alpha;
    let a1 = -2.0 * omega.cos();
    let a2 = 1.0 - alpha;

    BiquadCoefficients {
        a1_a0: a1 / a0,
        a2_a0: a2 / a0,
        b0_a0: b0 / a0,
        b1_a0: b1 / a0,
        b2_a0: b2 / a0,
    }
}

/// `BQ_filter_t`.
#[derive(Clone, Copy, Default)]
struct BiquadFilter {
    xv: [f32; 3],
    yv: [f32; 3],
}

impl BiquadFilter {
    /// `Biquad_applyFilter`.
    fn apply(&mut self, sample: f32, coefficients: &BiquadCoefficients) -> f32 {
        self.xv[0] = self.xv[1];
        self.yv[0] = self.yv[1];
        self.xv[1] = self.xv[2];
        self.yv[1] = self.yv[2];

        self.xv[2] = sample;

        self.yv[2] = coefficients.b0_a0 * self.xv[2]
            + coefficients.b1_a0 * self.xv[1]
            + coefficients.b2_a0 * self.xv[0]
            - coefficients.a1_a0 * self.yv[1]
            - coefficients.a2_a0 * self.yv[0];

        self.yv[2]
    }
}

// -- Costas loop -------------------------------------------------------------

/// The state and filters of `updateCostasLoop`.
pub struct CostasLoop {
    pub carrier_coefficients: ButterworthCoefficients,
    pub agc_coefficients: ButterworthCoefficients,
    pub channel_coefficients: BiquadCoefficients,
    pub phase_increment: f32,
    pub vco_gain: f32,
    carrier_filter: ButterworthFilter,
    agc_filter: ButterworthFilter,
    channel1_filter: BiquadFilter,
    channel2_filter: BiquadFilter,
    omega_t: f32,
}

impl CostasLoop {
    pub fn new(sample_rate_hz: u32) -> Self {
        let rate = sample_rate_hz as f32;
        Self {
            carrier_coefficients: butterworth_design_band_pass(
                sample_rate_hz,
                CONFIG_CARRIER_FREQUENCY - CARRIER_FILTER_CUTOFF_FREQUENCY,
                CONFIG_CARRIER_FREQUENCY + CARRIER_FILTER_CUTOFF_FREQUENCY,
            ),
            agc_coefficients: butterworth_design_low_pass(sample_rate_hz, AGC_FILTER_CUTOFF_FREQUENCY),
            channel_coefficients: biquad_design_low_pass(
                sample_rate_hz,
                CHANNEL_FILTER_CUTOFF_FREQUENCY,
                CHANNEL_FILTER_BANDWIDTH,
            ),
            phase_increment: (CONFIG_CARRIER_FREQUENCY * SINE_TABLE_LENGTH) as f32 / rate,
            vco_gain: VCO_GAIN_AT_48000 * CONFIG_SAMPLE_RATE / rate,
            carrier_filter: ButterworthFilter::default(),
            agc_filter: ButterworthFilter::default(),
            channel1_filter: BiquadFilter::default(),
            channel2_filter: BiquadFilter::default(),
            omega_t: 0.0,
        }
    }

    /// `updateCostasLoop`. `sample` is in the units of an `int16_t` sample. Returns
    /// the output of the in-phase channel.
    pub fn update(&mut self, sample: f32) -> f32 {
        // Apply gain control

        let mut filtered_sample =
            self.carrier_filter.apply_band_pass(sample, &self.carrier_coefficients);

        let regulated_filtered_sample =
            if filtered_sample > 0.0 { filtered_sample } else { -filtered_sample };

        let agc_output =
            self.agc_filter.apply_low_pass(regulated_filtered_sample, &self.agc_coefficients);

        filtered_sample /=
            if agc_output > AGC_MINIMUM_AMPLITUDE { agc_output } else { AGC_MINIMUM_AMPLITUDE };

        // Demodulate input sound. Casting a negative float to u32 gives 0 in Rust,
        // as the Cortex-M4F's conversion does. It is undefined behaviour in C.

        let index = ((self.omega_t as u32) & (SINE_TABLE_LENGTH - 1)) as u8;

        let channel1 = self.channel1_filter.apply(
            SINE_TABLE[index as usize] * filtered_sample,
            &self.channel_coefficients,
        );

        let index = index.wrapping_sub(SINE_QUARTER_CYCLE_INCREMENT);

        let channel2 = self.channel2_filter.apply(
            SINE_TABLE[index as usize] * filtered_sample,
            &self.channel_coefficients,
        );

        let control_signal = channel1 * channel2;

        // Update the VCO

        self.omega_t += self.phase_increment - self.vco_gain * control_signal;

        let multiple = self.omega_t as u32 / SINE_TABLE_LENGTH;

        self.omega_t -= (multiple * SINE_TABLE_LENGTH) as f32;

        // Return in phase channel

        channel1
    }
}

// -- Timing of the state machine ----------------------------------------------

/// The period constants of the C in samples at a given sample rate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timing {
    /// `MIN_BIT_PERIOD`
    pub min_bit: u64,
    /// `MAX_BIT_PERIOD`
    pub max_bit: u64,
    /// `LOW_BIT_PERIOD`
    pub low_bit: u64,
    /// `HIGH_BIT_PERIOD`
    pub high_bit: u64,
    /// `MID_BIT_PERIOD`
    pub mid_bit: u64,
    /// `START_STOP_BIT_PERIOD`
    pub start_stop_bit: u64,
    /// `PERIOD_TOLERANCE`
    pub tolerance: u64,
}

impl Timing {
    pub fn new(sample_rate_hz: u32) -> Self {
        let samples = |seconds: f64| (seconds * sample_rate_hz as f64).round() as u64;
        let low_bit = samples(LOW_BIT_SECONDS);
        let high_bit = samples(HIGH_BIT_SECONDS);
        Self {
            min_bit: samples(MIN_BIT_SECONDS),
            max_bit: samples(MAX_BIT_SECONDS),
            low_bit,
            high_bit,
            mid_bit: low_bit / 2 + high_bit / 2,
            start_stop_bit: samples(START_STOP_BIT_SECONDS),
            tolerance: samples(PERIOD_TOLERANCE_SECONDS),
        }
    }

    /// The `IN_PERIOD` macro with `PERIOD_TOLERANCE`. Both comparisons are strict.
    fn in_period(&self, period: u64, mean: u64) -> bool {
        period > mean - self.tolerance && period < mean + self.tolerance
    }

    fn is_start_stop(&self, period: u64) -> bool {
        self.in_period(period, self.start_stop_bit)
    }

    fn is_low(&self, period: u64) -> bool {
        self.in_period(period, self.low_bit)
    }

    fn is_high(&self, period: u64) -> bool {
        self.in_period(period, self.high_bit)
    }
}

// -- The state machine --------------------------------------------------------

/// `state_t`.
#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Idle,
    StartBits,
    DataBits,
    DataOrStopBits,
}

/// Port of `AudioConfig_listenForAudioConfigurationPackets`, one sample at a time.
pub struct AudioMothPort {
    costas_loop: CostasLoop,
    timing: Timing,

    // Zero crossing variables
    last_value: f32,
    last_crossing: u64,

    // Receive state variables
    bit_count: u32,
    byte_count: usize,
    state: State,
    received_hamming_codes: [u8; 2],
    received_bytes: [u8; MAXIMUM_NUMBER_OF_BYTES],

    counter: u64,

    // These three are not in the C. They fill in the fields of the events.
    corrected_bits: u32,
    start_run_begin: u64,
    start_sample: u64,
}

impl AudioMothPort {
    /// Panics if the carrier filter's upper edge of 19 kHz is not below half the
    /// sample rate.
    pub fn new(sample_rate_hz: u32) -> Self {
        assert!(sample_rate_hz > 38_000, "the 17 to 19 kHz band must fit below half the sample rate");
        Self {
            costas_loop: CostasLoop::new(sample_rate_hz),
            timing: Timing::new(sample_rate_hz),
            last_value: 0.0,
            last_crossing: 0,
            bit_count: 0,
            byte_count: 0,
            state: State::Idle,
            received_hamming_codes: [0; 2],
            received_bytes: [0; MAXIMUM_NUMBER_OF_BYTES],
            counter: 0,
            corrected_bits: 0,
            start_run_begin: 0,
            start_sample: 0,
        }
    }

    pub fn timing(&self) -> Timing {
        self.timing
    }

    /// The Costas loop, so that a test can read its coefficients.
    pub fn costas_loop(&self) -> &CostasLoop {
        &self.costas_loop
    }

    /// The bytes received since the last start of a message, CRC bytes included. They
    /// stay readable after the event for that message, until the next message starts.
    pub fn received_bytes(&self) -> &[u8] {
        &self.received_bytes[..self.byte_count]
    }

    /// Feeds one sample, normally +-1.0. Returns an event when the C would report a
    /// packet, a CRC error or a bit error.
    pub fn push(&mut self, sample: f32) -> Option<Event> {
        let costas_loop_output = self.costas_loop.update(sample * INT16_SCALE);
        self.process_costas_loop_output(costas_loop_output)
    }

    /// The part of the C's main loop that follows `updateCostasLoop`. It is public so
    /// that a test can drive the state machine with chosen periods.
    pub fn process_costas_loop_output(&mut self, costas_loop_output: f32) -> Option<Event> {
        let mut event = None;
        let timing = self.timing;

        // Check thresholds

        if (self.last_value >= 0.0 && costas_loop_output < 0.0)
            || (self.last_value < 0.0 && costas_loop_output >= 0.0)
        {
            let period = self.counter - self.last_crossing;

            if self.state == State::Idle {
                if timing.is_start_stop(period) {
                    self.state = State::StartBits;
                    self.bit_count = 0;
                    self.start_run_begin = self.last_crossing;
                }
            } else if self.state == State::StartBits {
                if timing.is_start_stop(period) {
                    self.bit_count += 1;
                } else if timing.is_low(period) || timing.is_high(period) {
                    if self.bit_count > MIN_NUMBER_OF_START_STOP_PERIODS
                        && self.bit_count < MAX_NUMBER_OF_START_STOP_PERIODS
                    {
                        self.received_hamming_codes = [0; 2];
                        self.state = State::DataBits;
                        self.byte_count = 0;
                        self.start_sample = self.estimate_start_sample();
                        self.corrected_bits = 0;
                        self.bit_count = 0;
                    } else {
                        self.state = State::Idle;
                    }
                } else {
                    self.state = State::Idle;
                }
            }

            // This is a separate if, not an else, as in the C. The crossing that
            // moves the state from START_BITS to DATA_BITS also ends the first data
            // symbol, so the code below handles it as the first data bit.

            if self.state == State::DataBits || self.state == State::DataOrStopBits {
                if period > timing.min_bit && period < timing.max_bit {
                    // Determine the received bit

                    if period > timing.mid_bit {
                        let mask = 1u8 << (self.bit_count >> 1);
                        self.received_hamming_codes[(self.bit_count % 2) as usize] |= mask;
                    }

                    self.bit_count += 1;

                    // Check if this could still be a stop bit

                    if !timing.is_start_stop(period) {
                        self.state = State::DataBits;
                    }

                    // Check if stop condition met

                    if self.bit_count == MIN_NUMBER_OF_START_STOP_PERIODS
                        && self.state == State::DataOrStopBits
                        && self.byte_count > CRC_SIZE_IN_BYTES
                    {
                        event = Some(self.finish_message());
                        self.state = State::Idle;
                    }

                    // Check if full byte has been received

                    if self.bit_count == ENCODED_BITS_IN_BYTE {
                        let [low_word, high_word] = self.received_hamming_codes;
                        self.received_bytes[self.byte_count] = (HAMMING_CONVERSION
                            [high_word as usize]
                            << 4)
                            | HAMMING_CONVERSION[low_word as usize];
                        self.corrected_bits +=
                            !is_codeword(low_word) as u32 + !is_codeword(high_word) as u32;

                        self.byte_count += 1;

                        // Check the CRC if all bytes have been received

                        if self.byte_count == MAXIMUM_NUMBER_OF_BYTES {
                            event = Some(self.finish_message());
                            self.state = State::Idle;
                        } else {
                            self.received_hamming_codes = [0; 2];
                            self.state = State::DataOrStopBits;
                            self.bit_count = 0;
                        }
                    }
                } else {
                    event = Some(self.unreadable(UnreadableReason::BadSymbol));
                    self.state = State::Idle;
                }
            }

            self.last_crossing = self.counter;
        }

        // Update counters and status

        self.last_value = costas_loop_output;
        self.counter += 1;

        event
    }

    /// Counts back from the start of the run of start periods to where the first of
    /// the chime's 16 start symbols should have begun. The run has `bit_count + 1`
    /// periods, because the period that entered `START_BITS` did not count.
    fn estimate_start_sample(&self) -> u64 {
        let periods_in_run = self.bit_count as i64 + 1;
        let missing_symbols = START_SYMBOLS_IN_CHIME - periods_in_run;
        let estimate =
            self.start_run_begin as i64 - missing_symbols * self.timing.start_stop_bit as i64;
        estimate.max(0) as u64
    }

    /// The outcome of `checkCRC` on the first `byte_count` received bytes.
    fn finish_message(&self) -> Event {
        if check_crc(&self.received_bytes[..self.byte_count]) {
            let payload_len = self.byte_count - CRC_SIZE_IN_BYTES;
            let mut payload = [0u8; MAX_PAYLOAD_BYTES];
            payload[..payload_len].copy_from_slice(&self.received_bytes[..payload_len]);
            Event::Chime(Chime {
                payload,
                payload_len,
                start_sample: self.start_sample as i64,
                end_sample: self.counter,
                corrected_bits: self.corrected_bits,
                level_dbfs: f32::NAN,
                snr_db: f32::NAN,
            })
        } else {
            self.unreadable(UnreadableReason::BadCrc)
        }
    }

    fn unreadable(&self, reason: UnreadableReason) -> Event {
        Event::Unreadable(Unreadable {
            start_sample: self.start_sample as i64,
            end_sample: self.counter,
            reason,
            bytes_received: self.byte_count,
            level_dbfs: f32::NAN,
        })
    }
}
