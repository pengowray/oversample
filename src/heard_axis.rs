//! Axis labels that give the values heard in playback.
//!
//! With XForm on and its Xform row applied, the spectrogram is meant to look
//! like a recording of what the speakers play, and its axes give the values
//! heard.
//!
//! Time expansion (TE) only plays the recording at another speed. Its
//! picture is the recording's own, and the frequency and time axes are
//! labelled with the values heard: at ×10, each frequency is the recording's
//! divided by 10 and each time is the recording's multiplied by 10. Only the
//! labels are scaled. Pointer positions, the HFR band, selections and
//! annotations stay in the recording's Hz and seconds.
//!
//! Pitch shift (PS, and its phase vocoder) changes the audio: the picture is
//! of the shifted audio, so a 50 kHz call is drawn at 5 kHz at ÷10. The
//! frequency labels are the picture's own, marked as heard values. Time is
//! unchanged and its strip stays the recording's.

use crate::dsp::pitch_shift::PitchFactor;
use crate::state::store_fields::*;
use crate::state::{AppState, PlaybackMode};
use leptos::prelude::*;

/// "r,g,b" of labels and notes that give heard values: the green of the
/// HEAR bar's "OUT ÷10" button (`.layer-btn.active`, #8fc), which gives the
/// same factor. Not the amber of the HFR band's text beside it, which gives
/// frequencies of the recording. The recording's own values are white.
pub const HEARD_RGB: &str = "136,255,204";
/// "r,g,b" of labels that give the recording's own values.
pub const RECORDING_RGB: &str = "255,255,255";

/// Which audio the spectrogram is a picture of.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Picture {
    /// The recording. The axis labels are scaled to give heard values (TE).
    Recording,
    /// The audio heard. The axis labels are the picture's own (PS).
    Heard,
}

/// The playback mode the heard values come from.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    TimeExpansion,
    PitchShift,
}

/// The axis labels when they give heard values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeardAxis {
    /// Heard frequency / recording frequency (0.1 at TE ×10 and PS ÷10).
    pub freq_scale: f64,
    /// Heard time / recording time (10 at TE ×10, 1 for PS).
    pub time_scale: f64,
    /// Which audio the picture is of.
    pub picture: Picture,
    /// Hz taken off every heard frequency after scaling: the PS output
    /// shift. 0 for TE.
    shift_hz: f64,
    mode: Mode,
    /// The factor that was asked for, when playback cannot play at it and
    /// `freq_scale` and `time_scale` are of the factor it plays at instead.
    asked_factor: Option<f64>,
}

impl HeardAxis {
    /// The scale for a TE factor as `transform.te_factor` stores it: above 1
    /// slows playback by that factor, below -1 speeds it up. `None` for a
    /// factor that leaves the speed unchanged.
    pub fn for_te_factor(te_factor: f64) -> Option<Self> {
        if !te_factor.is_finite() || te_factor.abs() <= 1.0 {
            return None;
        }
        let magnitude = te_factor.abs();
        let (freq_scale, time_scale) = if te_factor > 0.0 {
            (1.0 / magnitude, magnitude)
        } else {
            (magnitude, 1.0 / magnitude)
        };
        Some(Self {
            freq_scale,
            time_scale,
            picture: Picture::Recording,
            shift_hz: 0.0,
            mode: Mode::TimeExpansion,
            asked_factor: None,
        })
    }

    /// The scale for TE playback of a file at `sample_rate`: of the factor
    /// playback really applies (`te_applied_factor`), which is smaller than
    /// `te_factor` when the output rate would leave 8 to 384 kHz.
    pub fn for_te(sample_rate: u32, te_factor: f64) -> Option<Self> {
        let applied = crate::audio::streaming_playback::te_applied_factor(sample_rate, te_factor);
        let mut axis = Self::for_te_factor(applied)?;
        if (applied - te_factor).abs() > 0.005 * te_factor.abs() {
            axis.asked_factor = Some(te_factor.abs());
        }
        Some(axis)
    }

    /// The scale for pitch shift by `factor`, with `shift_hz` taken off the
    /// output afterwards. `None` for a factor that changes nothing.
    pub fn for_pitch_shift(factor: PitchFactor, shift_hz: f64) -> Option<Self> {
        if factor.is_bypass() {
            return None;
        }
        Some(Self {
            freq_scale: if factor.is_up() {
                factor.magnitude()
            } else {
                1.0 / factor.magnitude()
            },
            time_scale: 1.0,
            picture: Picture::Heard,
            shift_hz: if shift_hz >= 1.0 { shift_hz } else { 0.0 },
            mode: Mode::PitchShift,
            asked_factor: None,
        })
    }

    /// What a frequency of the picture is multiplied by to give its label.
    pub fn label_scale(self) -> f64 {
        match self.picture {
            Picture::Recording => self.freq_scale,
            Picture::Heard => 1.0,
        }
    }

    /// True when the time labels differ from the recording's.
    pub fn scales_time(self) -> bool {
        (self.time_scale - 1.0).abs() > 1e-9
    }

    /// "recording ÷10", or "(recording ÷10) − 1.5" when an output shift of
    /// 1.5 kHz is set: a heard frequency in kHz, from the recording's. The
    /// brackets keep it from reading as ÷(10 − 1.5).
    fn heard_from_recording(self) -> String {
        let scaled = format!("recording {}", op(self.freq_scale));
        if self.shift_hz > 0.0 {
            format!(
                "({scaled}) \u{2212} {}",
                crate::format_units::khz_value(self.shift_hz.round() as i64, 2)
            )
        } else {
            scaled
        }
    }

    /// "×10" or "÷10": what a recording's time is changed by.
    fn time_op(self) -> String {
        op(self.time_scale)
    }

    /// The note in the spectrogram's top left corner, beside the frequency
    /// strip. It gives the unit, which the axis labels leave out.
    pub fn freq_caption(self) -> String {
        format!("heard kHz = {}", self.heard_from_recording())
    }

    /// The note at the end of the time strip. `None` when the times are the
    /// recording's own.
    pub fn time_caption(self) -> Option<String> {
        self.scales_time()
            .then(|| format!("heard time = recording {}", self.time_op()))
    }

    /// The sentences in the Display Processing menu.
    pub fn blurb(self) -> String {
        let factor = number(self.time_scale.max(self.freq_scale));
        match self.mode {
            Mode::PitchShift => format!(
                "The frequency axis shows heard kHz: {}.",
                self.heard_from_recording()
            ),
            Mode::TimeExpansion => {
                let (freq, time) = (op(self.freq_scale), self.time_op());
                match self.asked_factor {
                    None => format!(
                        "Axes show what is heard at {factor}\u{00D7} time-expansion: frequency {freq}, time {time}."
                    ),
                    // Slower than playback can go, or faster. The limit
                    // comes first: it is why the labels are not of the
                    // factor that was set.
                    Some(asked) => {
                        let limit = if self.time_scale > 1.0 {
                            "the lowest playback sample rate is 8 kHz"
                        } else {
                            "the highest playback sample rate is 384 kHz"
                        };
                        format!(
                            "This file plays at {factor}\u{00D7}, not {}\u{00D7}: {limit}. Axes show what is heard: frequency {freq}, time {time}.",
                            number(asked)
                        )
                    }
                }
            }
        }
    }
}

/// A factor with no trailing ".0": 10 gives "10", 2.5 gives "2.5".
fn number(factor: f64) -> String {
    if (factor - factor.round()).abs() < 0.005 {
        format!("{factor:.0}")
    } else {
        format!("{factor:.1}")
    }
}

/// "×10" for a scale of 10, "÷10" for a scale of 0.1.
fn op(scale: f64) -> String {
    if scale >= 1.0 {
        format!("\u{00D7}{}", number(scale))
    } else {
        format!("\u{00F7}{}", number(1.0 / scale))
    }
}

/// What a caller of [`mode_axis`] subscribes to.
#[derive(Clone, Copy, PartialEq)]
enum Track {
    /// Nothing.
    No,
    /// The settings and which file is open.
    Settings,
    /// Those and the list of files, which changes whenever any file's
    /// details do. For callers that only redraw labels.
    SettingsAndFiles,
}

/// The open file's sample rate.
fn current_sample_rate(state: &AppState, track: Track) -> Option<u32> {
    let rate_at = |idx: usize, files: &Vec<crate::state::LoadedFile>| {
        files.get(idx).map(|f| f.audio.sample_rate)
    };
    let idx = if track == Track::No {
        state.library.current_index().get_untracked()
    } else {
        state.library.current_index().get()
    };
    if track == Track::SettingsAndFiles {
        state.library.files().with(|files| rate_at(idx?, files))
    } else {
        state
            .library
            .files()
            .with_untracked(|files| rate_at(idx?, files))
    }
}

/// The heard axis of the playback mode, whether or not XForm is on. Every
/// signal is read on each run, so a tracking caller is subscribed to all of
/// them.
fn mode_axis(state: &AppState, track: Track) -> Option<HeardAxis> {
    macro_rules! read {
        ($signal:expr) => {
            if track != Track::No {
                $signal.get()
            } else {
                $signal.get_untracked()
            }
        };
    }
    let mode = read!(state.playback.mode());
    let te_factor = read!(state.transform.te_factor());
    let ps_factor = read!(state.transform.ps_factor());
    let pv_factor = read!(state.transform.pv_factor());
    let stored_shift = read!(state.transform.ps_shift_hz());
    let band_lo = read!(state.filter.band_ff_freq_lo());
    let sample_rate = current_sample_rate(state, track);
    // The shift playback applies, which is the stored one limited by the
    // band's lower edge.
    let shift = |factor: PitchFactor| {
        crate::components::output_range_button::effective_ps_shift(
            stored_shift,
            band_lo,
            factor.signed(),
        )
    };
    match mode {
        PlaybackMode::TimeExpansion => match sample_rate {
            Some(rate) => HeardAxis::for_te(rate, te_factor),
            None => HeardAxis::for_te_factor(te_factor),
        },
        PlaybackMode::PitchShift => HeardAxis::for_pitch_shift(ps_factor, shift(ps_factor)),
        PlaybackMode::PhaseVocoder => HeardAxis::for_pitch_shift(pv_factor, shift(pv_factor)),
        _ => None,
    }
}

fn heard_axis_inner(state: &AppState, tracked: bool) -> Option<HeardAxis> {
    // Everything is read before the early return, so a tracked caller is
    // subscribed to every signal on each run.
    let (view, transform) = if tracked {
        (
            state.viewmode.main_view().get(),
            state.display.transform().get(),
        )
    } else {
        (
            state.viewmode.main_view().get_untracked(),
            state.display.transform().get_untracked(),
        )
    };
    let track = if tracked {
        Track::SettingsAndFiles
    } else {
        Track::No
    };
    let axis = mode_axis(state, track).filter(|_| transform)?;
    // XForm does nothing on the Wave and ZC views, which share the time
    // strip, or on the live waterfall.
    let live = (state.mic.recording().get_untracked() || state.mic.listening().get_untracked())
        && crate::canvas::live_waterfall::is_active();
    (view.is_spectrogram() && !live).then_some(axis)
}

/// The scale of the axis labels when they give heard values, or `None` when
/// they give the recording's own. Subscribes the calling Effect or Memo.
pub fn heard_axis(state: &AppState) -> Option<HeardAxis> {
    heard_axis_inner(state, true)
}

/// [`heard_axis`] without subscribing.
pub fn heard_axis_untracked(state: &AppState) -> Option<HeardAxis> {
    heard_axis_inner(state, false)
}

/// A Resam target rate, given as the rate of the audio heard, as a rate of
/// the picture's audio, for a display whose Xform row is applied. Under TE
/// ×10 the picture is the recording's, and the output's 48 kHz holds what
/// the recording has up to 480 kHz. Other modes keep the rate (a PS picture
/// is already of the audio heard), and 0 (no resampling) stays 0. Reads the
/// mode, the TE factor and which file is open tracked. It does not subscribe
/// to anything else (the list of files, the PS settings), since every run
/// of the calling Effect redraws the whole picture, and it does not read
/// `display.transform`, which that Effect writes.
pub fn resample_target_in_recording(state: &AppState, heard_rate: u32) -> u32 {
    let mode = state.playback.mode().get();
    let factor = state.transform.te_factor().get();
    let sample_rate = current_sample_rate(state, Track::Settings);
    let scale = (mode == PlaybackMode::TimeExpansion)
        .then(|| match sample_rate {
            Some(rate) => HeardAxis::for_te(rate, factor),
            None => HeardAxis::for_te_factor(factor),
        })
        .flatten()
        .map_or(1.0, |axis| axis.label_scale());
    (f64::from(heard_rate) / scale)
        .round()
        .clamp(0.0, f64::from(u32::MAX)) as u32
}

/// Steps between the frequency labels of an axis in heard values, in Hz.
const HEARD_STEPS: [f64; 8] = [
    100.0, 200.0, 500.0, 1_000.0, 2_000.0, 5_000.0, 10_000.0, 20_000.0,
];

/// The step between frequency labels on an axis in heard values: the
/// smallest of [`HEARD_STEPS`] that keeps labels at least 40 px apart.
///
/// The recording's own axes step by 10 kHz over a bat recording's range. A
/// heard range is a fraction of that (12.8 kHz for a 256 kHz file at ×10),
/// where those axes' rule (5 kHz steps up to 25 kHz, never under 1 kHz)
/// would leave two labels, or none.
pub fn heard_div_interval(range_hz: f64, height_px: f64) -> f64 {
    let hz_per_px = range_hz.max(1.0) / height_px.max(1.0);
    HEARD_STEPS
        .into_iter()
        .find(|step| step / hz_per_px >= 40.0)
        .unwrap_or(HEARD_STEPS[HEARD_STEPS.len() - 1])
}

/// A frequency label in kHz for a division step: whole kHz for steps of
/// 1 kHz or more ("12"), one decimal for finer steps ("0.5").
pub fn div_label(freq_hz: f64, div_interval: f64) -> String {
    if div_interval < 1_000.0 {
        crate::format_units::khz_value(freq_hz.round() as i64, 1)
    } else {
        crate::canvas::colors::freq_marker_label(freq_hz)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn te_slower_divides_frequency_and_multiplies_time() {
        let axis = HeardAxis::for_te_factor(10.0).unwrap();
        assert!((axis.freq_scale - 0.1).abs() < 1e-12);
        assert_eq!(axis.time_scale, 10.0);
        assert_eq!(axis.freq_caption(), "heard kHz = recording ÷10");
        assert_eq!(
            axis.time_caption().as_deref(),
            Some("heard time = recording ×10")
        );
        assert_eq!(axis.picture, Picture::Recording);
        assert!((axis.label_scale() - 0.1).abs() < 1e-12);
        assert_eq!(
            axis.blurb(),
            "Axes show what is heard at 10× time-expansion: frequency ÷10, time ×10."
        );
    }

    #[test]
    fn te_faster_multiplies_frequency_and_divides_time() {
        let axis = HeardAxis::for_te_factor(-4.0).unwrap();
        assert_eq!(axis.freq_scale, 4.0);
        assert_eq!(axis.time_scale, 0.25);
        assert_eq!(axis.freq_caption(), "heard kHz = recording ×4");
        assert_eq!(
            axis.time_caption().as_deref(),
            Some("heard time = recording ÷4")
        );
    }

    /// Pitch shift: the picture is of the shifted audio, so its labels are
    /// not scaled again, and the times are the recording's.
    #[test]
    fn pitch_shift_keeps_the_pictures_frequencies_and_the_recordings_times() {
        let axis = HeardAxis::for_pitch_shift(PitchFactor::down(10.0), 0.0).unwrap();
        assert!((axis.freq_scale - 0.1).abs() < 1e-12);
        assert_eq!(axis.time_scale, 1.0);
        assert_eq!(axis.picture, Picture::Heard);
        assert_eq!(axis.label_scale(), 1.0);
        assert_eq!(axis.freq_caption(), "heard kHz = recording ÷10");
        assert_eq!(axis.time_caption(), None);
        assert_eq!(
            axis.blurb(),
            "The frequency axis shows heard kHz: recording ÷10."
        );

        let up = HeardAxis::for_pitch_shift(PitchFactor::up(4.0), 0.0).unwrap();
        assert_eq!(up.freq_caption(), "heard kHz = recording ×4");

        assert_eq!(
            HeardAxis::for_pitch_shift(PitchFactor::from_signed(1.0), 0.0),
            None
        );
    }

    /// The PS output shift is taken off after the division.
    #[test]
    fn pitch_shift_with_an_output_shift_names_the_shift() {
        let axis = HeardAxis::for_pitch_shift(PitchFactor::down(10.0), 1_500.0).unwrap();
        assert_eq!(
            axis.freq_caption(),
            "heard kHz = (recording ÷10) \u{2212} 1.5"
        );
    }

    /// TE labels use the factor playback really applies. A 44.1 kHz file at
    /// ×10 would need a 4.41 kHz output: playback uses 8 kHz, which is
    /// ×5.5125. A 256 kHz file sped up ×2 would need 512 kHz: playback uses
    /// 384 kHz, which is ×1.5.
    #[test]
    fn te_labels_use_the_factor_playback_applies() {
        let axis = HeardAxis::for_te(44_100, 10.0).unwrap();
        assert!((axis.time_scale - 5.5125).abs() < 1e-9);
        assert!((axis.freq_scale - 1.0 / 5.5125).abs() < 1e-9);
        assert_eq!(axis.freq_caption(), "heard kHz = recording ÷5.5");
        assert_eq!(
            axis.blurb(),
            "This file plays at 5.5×, not 10×: the lowest playback sample rate is 8 kHz. \
             Axes show what is heard: frequency ÷5.5, time ×5.5."
        );

        let fast = HeardAxis::for_te(256_000, -2.0).unwrap();
        assert!((fast.freq_scale - 1.5).abs() < 1e-9);
        assert_eq!(fast.freq_caption(), "heard kHz = recording ×1.5");
        assert_eq!(
            fast.blurb(),
            "This file plays at 1.5×, not 2×: the highest playback sample rate is 384 kHz. \
             Axes show what is heard: frequency ×1.5, time ÷1.5."
        );

        // Inside the output's range the factor is the one asked for.
        let plain = HeardAxis::for_te(256_000, 10.0).unwrap();
        assert_eq!(plain, HeardAxis::for_te_factor(10.0).unwrap());
        // 192 kHz at ×7 is 27 428.57 Hz, cut to 27 428: still "÷7".
        let cut = HeardAxis::for_te(192_000, 7.0).unwrap();
        assert_eq!(cut.freq_caption(), "heard kHz = recording ÷7");
        assert!(!cut.blurb().contains("This file"));
    }

    #[test]
    fn a_factor_of_one_keeps_the_recordings_values() {
        assert_eq!(HeardAxis::for_te_factor(1.0), None);
        assert_eq!(HeardAxis::for_te_factor(-1.0), None);
        assert_eq!(HeardAxis::for_te_factor(0.0), None);
    }

    #[test]
    fn fractional_factors_keep_one_decimal() {
        let axis = HeardAxis::for_te_factor(2.5).unwrap();
        assert_eq!(axis.freq_caption(), "heard kHz = recording ÷2.5");
    }

    #[test]
    fn heard_steps_keep_labels_apart() {
        // A 256 kHz file at ×10 on a 530 px axis: 1 kHz steps, 41 px apart.
        assert_eq!(heard_div_interval(12_800.0, 530.0), 1_000.0);
        // A 384 kHz file at ×10: 1 kHz steps would be 27 px apart.
        assert_eq!(heard_div_interval(19_200.0, 530.0), 2_000.0);
        // Zoomed in to 2 kHz heard.
        assert_eq!(heard_div_interval(2_000.0, 530.0), 200.0);
        assert_eq!(heard_div_interval(300.0, 530.0), 100.0);
        // The recording's range at ÷2, as heard after speeding up.
        assert_eq!(heard_div_interval(192_000.0, 530.0), 20_000.0);
        assert_eq!(div_label(1_500.0, 500.0), "1.5");
        assert_eq!(div_label(2_000.0, 500.0), "2");
        assert_eq!(div_label(12_000.0, 1_000.0), "12");
    }
}
