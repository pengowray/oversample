//! Compose a tiered "effective bit depth" verdict from the lower-level
//! analyses, leading with the strongest claim we can make and falling
//! back through weaker evidence.
//!
//! The user's primary question is "what's the *true* bit depth of this
//! recording?" The container bit depth (e.g. 16-bit WAV) is often
//! misleading — many recorders zero-pad or DSP-shape a lower-bit-depth
//! ADC stream into a higher-bit-depth container. This module merges:
//!
//! - **Value coverage** (counting distinct sample values) — gives a
//!   mathematically certain upper bound: a 16-bit file using only 4083
//!   distinct values is *provably* ≤ 12-bit effective.
//! - **LSB stride GCD** — if every sample is a multiple of 2^N, the low
//!   N bits are literally zero, also mathematically certain.
//! - **LSB autocorrelation chi² / autocorrelation** — statistical
//!   evidence that the low bits carry deterministic DSP residue rather
//!   than analog noise.
//! - **Firmware-family detectors** (`pipistrelle`, `audiomoth`) — soft
//!   hints that the file's structure matches a known 12-bit + DSP firmware.
//!
//! The output is a `BitDepthCertainty` with:
//! - A single **headline** statement (the strongest single claim).
//! - A `certainty` band (Certain / HighConfidence / Suggestive).
//! - The strongest **upper bound** (and best estimate) we can prove.
//! - A list of supporting `Fact`s, certainty-strongest-first.

use crate::dsp::audiomoth::{AudioMothResult, AudioMothVerdict};
use crate::dsp::bit_analysis::BitAnalysis;
use crate::dsp::float_grid::{FloatGridResult, FloatGridVerdict};
use crate::dsp::lsb_autocorr::{LsbAutocorrResult, LsbVerdict};
use crate::dsp::pipistrelle::{PipistrelleResult, PipistrelleVerdict};

/// Returns `true` for lossy-compressed formats whose decoded samples
/// don't carry meaningful bit-depth information about the original
/// recording. Lossy codecs (MP3, AAC, Vorbis, ADPCM) decode to f32 /
/// re-quantise to 16-bit container; the LSB stride / autocorrelation /
/// inverse-filter tests are all destroyed by the codec.
///
/// FLAC and WAV are lossless; all bit-depth tests are meaningful.
pub fn is_lossy_compressed(format: &str) -> bool {
    matches!(format, "MP3" | "OGG" | "M4A" | "W4V")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CertaintyLevel {
    /// Mathematically certain (counting / arithmetic). Cannot be wrong.
    /// E.g. stride GCD, value-coverage upper bound.
    Certain,
    /// Strong statistical evidence with explicit thresholds (chi²
    /// p < 0.001, autocorrelation well above noise floor, etc.).
    HighConfidence,
    /// Soft hint — firmware-family signature matches, value coverage
    /// suggestive but not definitive.
    Suggestive,
}

impl CertaintyLevel {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Certain => "mathematically certain",
            Self::HighConfidence => "high confidence",
            Self::Suggestive => "suggestive",
        }
    }

    /// Order strongest first.
    fn rank(&self) -> u8 {
        match self {
            Self::Certain => 0,
            Self::HighConfidence => 1,
            Self::Suggestive => 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fact {
    pub certainty: CertaintyLevel,
    /// One-line statement with specific numbers ("Only 4083 of 65536
    /// values used → log₂(4083) = 12.0 bits effective ceiling").
    pub statement: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BitDepthCertainty {
    /// Container bit depth (e.g. 16 for a 16-bit WAV).
    pub container_bits: u16,
    /// Tightest mathematically-certain upper bound on effective bits.
    pub upper_bound: u16,
    /// Best single estimate of effective bit depth. Equal to
    /// `upper_bound` unless statistical signals suggest going lower.
    pub best_estimate: u16,
    /// Certainty about `best_estimate`.
    pub certainty: CertaintyLevel,
    /// Single user-facing headline line, e.g.
    /// "Effective bit depth: ≤ 12 (mathematically certain)".
    pub headline: String,
    /// Detail facts, certainty-strongest-first.
    pub facts: Vec<Fact>,
}

pub fn compose(
    container_bits: u16,
    is_float: bool,
    format: &str,
    bit: &BitAnalysis,
    lsb: &LsbAutocorrResult,
    pip: Option<&PipistrelleResult>,
    am: Option<&AudioMothResult>,
    float_grid: &[FloatGridResult],
) -> BitDepthCertainty {
    // Lossy-compressed audio: all bit-depth signatures are destroyed by
    // the codec. We can still report the container depth but should
    // flag clearly that the analysis isn't measuring the original ADC.
    if is_lossy_compressed(format) {
        return BitDepthCertainty {
            container_bits,
            upper_bound: container_bits,
            best_estimate: container_bits,
            certainty: CertaintyLevel::Suggestive,
            headline: format!(
                "Lossy {} \u{2014} bit-depth analysis doesn't apply (codec re-quantises \
                 to {}-bit, destroying any original ADC signature)",
                format, container_bits,
            ),
            facts: vec![Fact {
                certainty: CertaintyLevel::Certain,
                statement: format!(
                    "{} is a lossy compressed format. Decoded samples come from \
                     re-synthesised audio (the codec discards original LSBs, \
                     applies its own dither, and may add quantisation noise). \
                     LSB stride / autocorrelation / inverse-filter tests run on \
                     these samples are not meaningful as a probe of the original \
                     recorder's bit depth.",
                    format,
                ),
            }],
        };
    }

    // Float files are a different beast — we don't reduce their depth via these tests.
    if is_float {
        if let Some(verdict) = compose_float_grid(container_bits, float_grid) {
            return verdict;
        }
        return BitDepthCertainty {
            container_bits,
            upper_bound: container_bits,
            best_estimate: container_bits,
            certainty: CertaintyLevel::Certain,
            headline: format!(
                "{}-bit float \u{2014} bit-depth-reduction tests don't apply",
                container_bits,
            ),
            facts: Vec::new(),
        };
    }

    let mut facts: Vec<Fact> = Vec::new();
    let mut upper_bound = container_bits;
    let mut best_estimate = container_bits;

    // ── 1. Mathematically certain: stride GCD / LSB zero-pad ───────────
    if let LsbVerdict::ZeroPaddedNBit {
        padding_bits,
        effective_bits,
    } = lsb.verdict
    {
        upper_bound = upper_bound.min(effective_bits);
        best_estimate = effective_bits;
        let multiple = 1u64 << padding_bits;
        facts.push(Fact {
            certainty: CertaintyLevel::Certain,
            statement: format!(
                "Every sample is a multiple of {} — the low {} bit{} of every \
                 sample {} literally zero. Effective bit depth = exactly {} bits.",
                multiple,
                padding_bits,
                if padding_bits == 1 { "" } else { "s" },
                if padding_bits == 1 { "is" } else { "are" },
                effective_bits,
            ),
        });
    }

    // ── 2. Value-coverage upper bound ─────────────────────────────────
    // Value coverage is a hard ceiling on the FILE's information content
    // (mathematically certain). But it doesn't directly bound the
    // RECORDER's hardware bit depth — a quiet recording on a 16-bit
    // recorder also uses few distinct values. We word the fact carefully
    // so the user knows what they're being told.
    if let Some(ref vc) = bit.value_coverage {
        let ceil_bits = vc.resolution_bits.ceil() as u16;
        if ceil_bits < upper_bound {
            upper_bound = ceil_bits;
            best_estimate = ceil_bits;
            facts.push(Fact {
                certainty: CertaintyLevel::Certain,
                statement: format!(
                    "Only {} distinct sample values observed out of {} possible — \
                     log₂({}) = {:.2}, so the signal carries ≤ {} bits of \
                     information (this could be a {}-bit recorder, or a quiet \
                     signal on a higher-bit recorder; see other facts for \
                     disambiguation).",
                    vc.unique_count,
                    vc.value_space,
                    vc.unique_count,
                    vc.resolution_bits,
                    ceil_bits,
                    ceil_bits,
                ),
            });
        } else if (vc.coverage_pct - 100.0).abs() < 0.01
            || vc.unique_count as u64 == vc.value_space as u64
        {
            // Full value-space coverage. Note this affirmatively.
            facts.push(Fact {
                certainty: CertaintyLevel::Certain,
                statement: format!(
                    "All {} possible {}-bit values are used — no value-coverage \
                     reduction detected.",
                    vc.value_space, container_bits,
                ),
            });
        } else if vc.coverage_pct > 50.0 {
            // Substantial coverage. Suggests the container bit depth is genuine.
            facts.push(Fact {
                certainty: CertaintyLevel::HighConfidence,
                statement: format!(
                    "{} of {} possible values used ({:.1}% coverage, log₂ = {:.2} bits) — \
                     consistent with a genuine {}-bit recorder.",
                    vc.unique_count,
                    vc.value_space,
                    vc.coverage_pct,
                    vc.resolution_bits,
                    container_bits,
                ),
            });
        }
    }

    // ── 3. High-confidence: LSB autocorrelation + DSP residue ──────────
    match lsb.verdict {
        LsbVerdict::QuietSectionZeroPadded {
            effective_bits_in_quiet,
            padding_bits,
        } => {
            facts.push(Fact {
                certainty: CertaintyLevel::HighConfidence,
                statement: format!(
                    "In the quietest 4096-sample window, low {} bits are literally \
                     zero on every sample (chi² = {:.0}, p < 10⁻⁶) — recorder \
                     has a noise-gate / AGC that drops effective bit depth to \
                     {} in quiet sections, while loud sections may use the full \
                     {} bits.",
                    padding_bits, lsb.quiet_lsb_chi2, effective_bits_in_quiet, container_bits,
                ),
            });
            // Don't refine upper_bound from this (loud sections may use full bits).
        }
        LsbVerdict::DspPaddedLowBitDepth {
            effective_bits_guess,
        } => {
            facts.push(Fact {
                certainty: CertaintyLevel::HighConfidence,
                statement: format!(
                    "Low {} bits show statistical structure (chi² = {:.0} vs \
                     uniform-baseline threshold ~38; lag-1 autocorrelation = \
                     {:+.3}) — consistent with on-device DSP filtering of a \
                     ~{}-bit ADC stream rather than analog noise.",
                    lsb.n_low, lsb.quiet_lsb_chi2, lsb.quiet_lsb_lag1_acf, effective_bits_guess,
                ),
            });
            // Soft refinement of best_estimate, but not upper_bound.
            if best_estimate > effective_bits_guess {
                best_estimate = effective_bits_guess;
            }
        }
        LsbVerdict::ConsistentWithClaimedBitDepth => {
            facts.push(Fact {
                certainty: CertaintyLevel::HighConfidence,
                statement: format!(
                    "Low {} bits look like analog noise (chi² = {:.0}, lag-1 = \
                     {:+.3}) — consistent with a genuine {}-bit recorder, no \
                     bit-depth padding detected.",
                    lsb.n_low, lsb.quiet_lsb_chi2, lsb.quiet_lsb_lag1_acf, container_bits,
                ),
            });
        }
        LsbVerdict::Inconclusive => {
            facts.push(Fact {
                certainty: CertaintyLevel::Suggestive,
                statement: format!(
                    "LSB tests inconclusive — quietest window stdev = {:.1} LSB, \
                     above the {:.0}-LSB threshold where analog noise dithers \
                     out any DSP signature.",
                    lsb.quietest_window_stdev, 16.0,
                ),
            });
        }
        _ => {}
    }

    // ── 4. Suggestive: firmware-family detectors ──────────────────────
    if let Some(p) = pip {
        if matches!(p.verdict, PipistrelleVerdict::Match) {
            facts.push(Fact {
                certainty: CertaintyLevel::Suggestive,
                statement: format!(
                    "Inverse-filter test with Pipistrelle firmware's dBcut=12 \
                     preset gives {:.2}% RMS error with every recovered value in \
                     the 12-bit [0, 4095] range — consistent with a 12-bit ADC \
                     + DSP firmware family (Pipistrelle / AudioMoth / Pettersson \
                     D1000X / SM Mini Bat fw2+).",
                    p.best_normalized_residual * 100.0,
                ),
            });
            if best_estimate > 12 {
                best_estimate = 12;
            }
        }
    }
    if let Some(a) = am {
        if matches!(a.verdict, AudioMothVerdict::Match) {
            if let Some(s) = a.best.as_ref() {
                facts.push(Fact {
                    certainty: CertaintyLevel::Suggestive,
                    statement: format!(
                        "Sample differences cluster on integer multiples of G = {:.1} \
                         after AudioMoth-style 1-pole HPF inverse (cutoff ≈ {} Hz, \
                         mean |frac| = {:.3} vs 0.25 uniform baseline) — consistent \
                         with AudioMoth or similar 12-bit-ADC firmware.",
                        s.gain_total, s.cutoff_hz, s.mean_abs_frac,
                    ),
                });
                if best_estimate > 12 {
                    best_estimate = 12;
                }
            }
        }
    }

    // ── Headline ───────────────────────────────────────────────────────
    //
    // Categorise the evidence:
    // - "Recorder-confirming": GCD / zero-pad / firmware-family — these
    //   are about the recorder's hardware bit depth and are decisive.
    // - "Signal-only": value-coverage by itself just bounds the file's
    //   information content. Could be a smaller recorder, or a quiet
    //   signal on a larger recorder.
    // - "Full coverage": all (or most) possible values used — strong
    //   evidence that the recorder is at least the container depth.
    let has_recorder_confirming = matches!(
        lsb.verdict,
        LsbVerdict::ZeroPaddedNBit { .. }
            | LsbVerdict::QuietSectionZeroPadded { .. }
            | LsbVerdict::DspPaddedLowBitDepth { .. }
    ) || pip
        .is_some_and(|p| matches!(p.verdict, PipistrelleVerdict::Match))
        || am.is_some_and(|a| matches!(a.verdict, AudioMothVerdict::Match));
    let any_certain_below_container = facts.iter().any(|f| {
        f.certainty == CertaintyLevel::Certain
            && (f.statement.contains("≤") || f.statement.contains("= exactly"))
    });
    let high_conf_full_coverage = facts.iter().any(|f| {
        f.certainty == CertaintyLevel::HighConfidence
            && f.statement.contains("consistent with a genuine")
    });

    let (certainty, headline) = if upper_bound < container_bits && has_recorder_confirming {
        // Recorder-confirming evidence + bounded upper. Strong claim.
        (
            CertaintyLevel::Certain,
            format!(
                "Recorder bit depth: ≤ {} (mathematically certain \u{2014} \
                 {}-bit container with confirmed reduction)",
                upper_bound, container_bits,
            ),
        )
    } else if upper_bound < container_bits && any_certain_below_container {
        // Signal-information-only bound. Word the headline so users
        // don't conflate "signal content" with "recorder hardware".
        (
            CertaintyLevel::Certain,
            format!(
                "Signal information: \u{2264} {} bits (mathematically certain). \
                 Recorder could be {}-bit with a quiet signal, OR a narrower ADC.",
                upper_bound, container_bits,
            ),
        )
    } else if upper_bound < container_bits {
        (
            CertaintyLevel::HighConfidence,
            format!(
                "Effective bit depth: ~{} ({}-bit container, but evidence \
                 suggests reduction)",
                best_estimate, container_bits,
            ),
        )
    } else if has_recorder_confirming {
        // Reduction signature without coverage limit (e.g. noise-gated D1000).
        (
            CertaintyLevel::HighConfidence,
            format!(
                "Effective bit depth: ~{} (loud sections may use full {} bits, \
                 but DSP / noise-gate signatures point to a lower-bit ADC)",
                best_estimate, container_bits,
            ),
        )
    } else if high_conf_full_coverage {
        (
            CertaintyLevel::HighConfidence,
            format!(
                "Recorder bit depth: {} (genuine \u{2014} no reduction detected)",
                container_bits
            ),
        )
    } else {
        (
            CertaintyLevel::Suggestive,
            format!(
                "Effective bit depth: {} (container claim; reduction tests inconclusive)",
                container_bits,
            ),
        )
    };

    // Sort facts certainty-strongest-first (stable sort preserves insertion order within tier).
    facts.sort_by_key(|f| f.certainty.rank());

    BitDepthCertainty {
        container_bits,
        upper_bound,
        best_estimate,
        certainty,
        headline,
        facts,
    }
}

/// Verdict for a float file from the per-channel grid results, or `None` when
/// no channel could be tested yet. Reports the channel with the most structure,
/// because a nearly silent channel says little about the recorder.
fn compose_float_grid(
    container_bits: u16,
    channels: &[FloatGridResult],
) -> Option<BitDepthCertainty> {
    fn rank(r: &FloatGridResult) -> Option<u8> {
        match r.verdict {
            FloatGridVerdict::TwoSteps => Some(0),
            FloatGridVerdict::OneStep => Some(1),
            FloatGridVerdict::IntegerSource { .. } => Some(2),
            FloatGridVerdict::NoFixedStep => Some(3),
            FloatGridVerdict::TooFewSamples | FloatGridVerdict::NotApplicable => None,
        }
    }
    let (index, chosen) = channels
        .iter()
        .enumerate()
        .filter(|(_, r)| rank(r).is_some())
        .min_by_key(|(_, r)| rank(r))?;

    let mut facts = Vec::new();
    let fact = |certainty, statement: String| Fact {
        certainty,
        statement,
    };

    let channels_differ = channels
        .iter()
        .filter(|r| rank(r).is_some())
        .any(|r| !same_grid(r, chosen));
    if channels_differ {
        facts.push(fact(
            CertaintyLevel::Certain,
            format!(
                "Showing channel {} only. The channels gave different results.",
                index + 1
            ),
        ));
    }

    let (certainty, headline, upper_bound, best_estimate) = match chosen.verdict {
        FloatGridVerdict::IntegerSource { bits } => (
            CertaintyLevel::Certain,
            format!(
                "32-bit float file, but exactly {bits}-bit integers. \
                 Converting to {bits}-bit integer is lossless."
            ),
            bits,
            bits,
        ),
        FloatGridVerdict::NoFixedStep => (
            CertaintyLevel::Suggestive,
            "32-bit float file with no fixed step found. Mixing or filtering can hide one."
                .to_string(),
            container_bits,
            container_bits,
        ),
        _ => {
            let fine = chosen.fine?;
            let resolution = whole_bits_at_full_scale(fine.step);
            let best = chosen.path_bits.or(resolution).unwrap_or(container_bits);
            let headline = if let Some(coarse) = chosen.coarse {
                facts.push(fact(
                    CertaintyLevel::HighConfidence,
                    "Samples came from integers, not a continuous float signal.".to_string(),
                ));
                facts.push(fact(
                    CertaintyLevel::HighConfidence,
                    format!(
                        "Fine step {}, coarse step {}; coarse step is {:.1} dB larger.",
                        step_text(fine.step),
                        step_text(coarse.step),
                        chosen.step_ratio_db().unwrap_or(0.0),
                    ),
                ));
                facts.push(fact(
                    CertaintyLevel::Suggestive,
                    "Consistent with recorders that use one ADC path for quiet sound \
                     and another for loud sound."
                        .to_string(),
                ));
                "32-bit float file, but most samples are multiples of two fixed steps".to_string()
            } else {
                format!(
                    "32-bit float file with one fixed step, {}: integers multiplied by a gain",
                    step_text(fine.step),
                )
            };
            if let Some(bits) = resolution {
                let name = if chosen.coarse.is_some() {
                    "Fine step"
                } else {
                    "This step"
                };
                facts.push(fact(
                    CertaintyLevel::HighConfidence,
                    format!(
                        "{name} equals {bits}-bit integer resolution across full scale, \
                         sign included."
                    ),
                ));
            }
            (
                CertaintyLevel::HighConfidence,
                headline,
                container_bits,
                best,
            )
        }
    };

    if let Some(ceiling) = chosen.flat_ceiling {
        let mut statement = format!(
            "Clipped: {} samples at the peak of {:+.1} dBFS.",
            ceiling.samples,
            20.0 * ceiling.value.log10(),
        );
        if let Some(bits) = chosen.path_bits {
            let name = if chosen.coarse.is_some() {
                "coarse "
            } else {
                ""
            };
            statement.push_str(&format!(
                " The peak is exactly 2^{} {name}steps from zero, which is the range of a \
                 {bits}-bit integer.",
                bits - 1,
            ));
        }
        facts.push(fact(CertaintyLevel::HighConfidence, statement));
    }

    if chosen.fine.is_some() {
        facts.push(fact(
            CertaintyLevel::Suggestive,
            "A gain change rescales the step, so step size alone can't show the \
             recorder's original step."
                .to_string(),
        ));
    }

    facts.sort_by_key(|f| f.certainty.rank());
    Some(BitDepthCertainty {
        container_bits,
        upper_bound,
        best_estimate,
        certainty,
        headline,
        facts,
    })
}

/// Bit width whose full-scale step this is, when it is within 0.1% of a power of 2.
fn whole_bits_at_full_scale(step: f64) -> Option<u16> {
    let exponent = (1.0 / step).log2();
    let near_whole = (2f64.powf(exponent.round()) * step - 1.0).abs() < 1e-3;
    (near_whole && (7.0..=31.0).contains(&exponent.round())).then_some(exponent.round() as u16 + 1)
}

fn step_text(step: f64) -> String {
    format!("2^{:.2}", step.log2())
}

fn same_grid(a: &FloatGridResult, b: &FloatGridResult) -> bool {
    fn same_step(
        a: Option<crate::dsp::float_grid::GridStep>,
        b: Option<crate::dsp::float_grid::GridStep>,
    ) -> bool {
        match (a, b) {
            (Some(x), Some(y)) => (x.step / y.step - 1.0).abs() < 1e-3,
            (None, None) => true,
            _ => false,
        }
    }
    a.verdict == b.verdict && same_step(a.fine, b.fine) && same_step(a.coarse, b.coarse)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::bit_analysis::{BitAnalysis, ValueCoverage};
    use crate::dsp::float_grid::{FlatCeiling, GridStep};
    use crate::dsp::lsb_autocorr::{LsbAutocorrResult, LsbVerdict};

    fn default_bit_analysis(bits: u16, vc: Option<ValueCoverage>) -> BitAnalysis {
        BitAnalysis {
            bits_per_sample: bits,
            is_float: false,
            total_samples: 16384,
            duration_secs: 1.0,
            bit_stats: vec![],
            bit_cautions: vec![],
            effective_bits: bits,
            effective_bits_f64: bits as f64,
            summary: String::new(),
            warnings: vec![],
            positive_counts: vec![],
            negative_counts: vec![],
            positive_total: 0,
            negative_total: 0,
            zero_total: 0,
            pair_counts: vec![],
            headroom_bits: 0,
            noise_floor_db: -80.0,
            value_coverage: vc,
        }
    }

    fn default_lsb(verdict: LsbVerdict) -> LsbAutocorrResult {
        LsbAutocorrResult {
            verdict,
            explanation: String::new(),
            n_low: 4,
            window_size: 4096,
            gcd_nonzero: 1,
            quietest_window_idx: 0,
            quietest_window_stdev: 5.0,
            quiet_lsb_nonzero_frac: 0.5,
            quiet_lsb_chi2: 10.0,
            quiet_lsb_lag1_acf: 0.0,
            quiet_lsb_lag256_acf: 0.0,
            low_bit_period: None,
            low_bit_period_match: 0.0,
        }
    }

    #[test]
    fn zero_padded_gives_certain_upper_bound() {
        let bit = default_bit_analysis(16, None);
        let lsb = default_lsb(LsbVerdict::ZeroPaddedNBit {
            effective_bits: 12,
            padding_bits: 4,
        });
        let r = compose(16, false, "WAV", &bit, &lsb, None, None, &[]);
        assert_eq!(r.upper_bound, 12);
        assert_eq!(r.best_estimate, 12);
        assert_eq!(r.certainty, CertaintyLevel::Certain);
        assert!(r.headline.contains("≤ 12"));
        assert!(r.headline.contains("mathematically certain"));
        assert!(r
            .facts
            .iter()
            .any(|f| f.statement.contains("multiple of 16")));
    }

    #[test]
    fn value_coverage_caps_upper_bound() {
        let bit = default_bit_analysis(
            16,
            Some(ValueCoverage {
                unique_count: 4096,
                value_space: 65536,
                coverage_pct: 6.25,
                resolution_bits: 12.0,
            }),
        );
        let lsb = default_lsb(LsbVerdict::Inconclusive);
        let r = compose(16, false, "WAV", &bit, &lsb, None, None, &[]);
        assert_eq!(r.upper_bound, 12);
        assert!(r.headline.contains("≤ 12"), "got: {}", r.headline);
    }

    #[test]
    fn genuine_16bit_when_full_coverage() {
        let bit = default_bit_analysis(
            16,
            Some(ValueCoverage {
                unique_count: 60000,
                value_space: 65536,
                coverage_pct: 91.5,
                resolution_bits: 15.87,
            }),
        );
        let lsb = default_lsb(LsbVerdict::ConsistentWithClaimedBitDepth);
        let r = compose(16, false, "WAV", &bit, &lsb, None, None, &[]);
        assert_eq!(r.upper_bound, 16);
        assert_eq!(r.certainty, CertaintyLevel::HighConfidence);
        assert!(r.headline.contains("16"));
        assert!(
            r.headline.contains("genuine") || r.headline.contains("no reduction"),
            "headline: {}",
            r.headline
        );
    }

    #[test]
    fn quiet_section_zero_pad_high_confidence_only() {
        let bit = default_bit_analysis(16, None);
        let lsb = default_lsb(LsbVerdict::QuietSectionZeroPadded {
            effective_bits_in_quiet: 12,
            padding_bits: 4,
        });
        let r = compose(16, false, "WAV", &bit, &lsb, None, None, &[]);
        // Don't downgrade upper bound (loud sections may use full bits)
        assert_eq!(r.upper_bound, 16);
        assert!(r.headline.contains("noise-gate") || r.headline.contains("loud sections"));
    }

    #[test]
    fn lossy_formats_short_circuit_with_caveat() {
        let bit = default_bit_analysis(
            16,
            Some(ValueCoverage {
                unique_count: 4096,
                value_space: 65536,
                coverage_pct: 6.25,
                resolution_bits: 12.0,
            }),
        );
        let lsb = default_lsb(LsbVerdict::ZeroPaddedNBit {
            effective_bits: 12,
            padding_bits: 4,
        });
        for fmt in ["MP3", "OGG", "M4A", "W4V"] {
            let r = compose(16, false, fmt, &bit, &lsb, None, None, &[]);
            assert!(
                r.headline.starts_with(&format!("Lossy {}", fmt)),
                "format={} headline={}",
                fmt,
                r.headline
            );
            assert_eq!(r.upper_bound, 16, "lossy must NOT claim reduction");
            assert!(
                r.facts
                    .iter()
                    .any(|f| f.statement.contains("not meaningful")),
                "lossy explanation missing for {}",
                fmt
            );
        }
    }

    #[test]
    fn lossless_formats_run_full_analysis() {
        let bit = default_bit_analysis(
            16,
            Some(ValueCoverage {
                unique_count: 4096,
                value_space: 65536,
                coverage_pct: 6.25,
                resolution_bits: 12.0,
            }),
        );
        let lsb = default_lsb(LsbVerdict::ZeroPaddedNBit {
            effective_bits: 12,
            padding_bits: 4,
        });
        for fmt in ["WAV", "FLAC"] {
            let r = compose(16, false, fmt, &bit, &lsb, None, None, &[]);
            assert_eq!(r.upper_bound, 12, "format={} should claim reduction", fmt);
        }
    }

    #[test]
    fn float_is_passthrough() {
        let bit = default_bit_analysis(32, None);
        let lsb = default_lsb(LsbVerdict::NotApplicable);
        let r = compose(32, true, "WAV", &bit, &lsb, None, None, &[]);
        assert_eq!(r.upper_bound, 32);
        assert!(r.headline.contains("float"));
        assert!(r.facts.is_empty());
    }

    fn grid(verdict: FloatGridVerdict, fine: Option<f64>, coarse: Option<f64>) -> FloatGridResult {
        let step = |s: f64| GridStep { step: s, fit: 1.0 };
        FloatGridResult {
            verdict,
            fine: fine.map(step),
            coarse: coarse.map(step),
            flat_ceiling: None,
            path_bits: None,
            negative_zeros: 0,
            samples_checked: 100_000,
        }
    }

    fn compose_float(channels: &[FloatGridResult]) -> BitDepthCertainty {
        let bit = default_bit_analysis(32, None);
        let lsb = default_lsb(LsbVerdict::NotApplicable);
        compose(32, true, "WAV", &bit, &lsb, None, None, channels)
    }

    #[test]
    fn float_with_two_steps_reports_both_and_the_path_width() {
        let fine = 0.99997 * 2f64.powi(-21);
        let coarse = fine * 10.0;
        let mut channel = grid(FloatGridVerdict::TwoSteps, Some(fine), Some(coarse));
        channel.flat_ceiling = Some(FlatCeiling {
            value: 10.0,
            samples: 1500,
        });
        channel.path_bits = Some(22);
        let r = compose_float(&[channel]);
        assert!(r.headline.contains("two fixed steps"), "{}", r.headline);
        assert_eq!(r.certainty, CertaintyLevel::HighConfidence);
        assert_eq!(r.upper_bound, 32);
        assert_eq!(r.best_estimate, 22);
        let all: Vec<&str> = r.facts.iter().map(|f| f.statement.as_str()).collect();
        assert!(all.iter().any(|s| s.contains("20.0 dB larger")), "{all:?}");
        assert!(
            all.iter().any(|s| s.contains("exactly 2^21 coarse steps")),
            "{all:?}"
        );
        assert!(all.iter().any(|s| s.contains("+20.0 dBFS")), "{all:?}");
    }

    #[test]
    fn float_of_exact_integers_is_certain_and_lowers_the_bound() {
        let r = compose_float(&[grid(
            FloatGridVerdict::IntegerSource { bits: 16 },
            None,
            None,
        )]);
        assert_eq!(r.certainty, CertaintyLevel::Certain);
        assert_eq!(r.upper_bound, 16);
        assert!(r.headline.contains("16-bit"));
    }

    #[test]
    fn float_with_no_step_keeps_the_container_depth_and_a_soft_claim() {
        let r = compose_float(&[grid(FloatGridVerdict::NoFixedStep, None, None)]);
        assert_eq!(r.certainty, CertaintyLevel::Suggestive);
        assert_eq!(r.upper_bound, 32);
        assert!(r.headline.contains("no fixed step"));
    }

    #[test]
    fn one_step_gain_shows_the_step_without_a_bit_width() {
        let r = compose_float(&[grid(FloatGridVerdict::OneStep, Some(1.343_871e-7), None)]);
        assert!(r.headline.contains("one fixed step"));
        assert!(r.headline.contains("2^-22.83"), "{}", r.headline);
        assert_eq!(r.best_estimate, 32);
    }

    #[test]
    fn silent_channel_does_not_count_as_a_difference() {
        let silent = grid(FloatGridVerdict::TooFewSamples, None, None);
        let active = grid(FloatGridVerdict::OneStep, Some(2f64.powi(-21)), None);
        let r = compose_float(&[silent, active]);
        assert!(r
            .facts
            .iter()
            .all(|f| !f.statement.contains("Showing channel")));
    }

    #[test]
    fn differing_channels_name_the_channel_shown() {
        let one = grid(FloatGridVerdict::OneStep, Some(2f64.powi(-21)), None);
        let none = grid(FloatGridVerdict::NoFixedStep, None, None);
        let r = compose_float(&[none, one]);
        assert!(r
            .facts
            .iter()
            .any(|f| f.statement.contains("Showing channel 2 only")));
    }

    #[test]
    fn facts_sorted_strongest_first() {
        let bit = default_bit_analysis(
            16,
            Some(ValueCoverage {
                unique_count: 4083,
                value_space: 65536,
                coverage_pct: 6.23,
                resolution_bits: 11.99,
            }),
        );
        let mut lsb = default_lsb(LsbVerdict::ZeroPaddedNBit {
            effective_bits: 12,
            padding_bits: 4,
        });
        lsb.quiet_lsb_chi2 = 61440.0;
        let r = compose(16, false, "WAV", &bit, &lsb, None, None, &[]);
        // Both ZeroPad and ValueCoverage should be Certain; order preserved.
        let cert_ranks: Vec<u8> = r.facts.iter().map(|f| f.certainty.rank()).collect();
        let sorted: Vec<u8> = {
            let mut s = cert_ranks.clone();
            s.sort();
            s
        };
        assert_eq!(cert_ranks, sorted, "facts not sorted strongest-first");
    }
}
