//! Axis labels that give the values heard in playback.
//!
//! Time expansion (TE) only plays the recording at another speed. With XForm
//! on and its Xform row applied, the spectrogram of a TE recording is the
//! recording's own picture, and the frequency and time axes are labelled
//! with the values heard: at ×10, each frequency is the recording's divided
//! by 10 and each time is the recording's multiplied by 10.
//!
//! Only the labels are scaled. Pointer positions, the HFR band, selections
//! and annotations stay in the recording's Hz and seconds.

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

/// The scale of the axis labels when they give heard values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeardAxis {
    /// Heard frequency / recording frequency (0.1 at TE ×10).
    pub freq_scale: f64,
    /// Heard time / recording time (10 at TE ×10).
    pub time_scale: f64,
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
        Some(if te_factor > 0.0 {
            Self {
                freq_scale: 1.0 / magnitude,
                time_scale: magnitude,
            }
        } else {
            Self {
                freq_scale: magnitude,
                time_scale: 1.0 / magnitude,
            }
        })
    }

    /// "÷10" or "×10": what a recording's frequency is changed by.
    fn freq_op(self) -> String {
        op(self.freq_scale)
    }

    /// "×10" or "÷10": what a recording's time is changed by.
    fn time_op(self) -> String {
        op(self.time_scale)
    }

    /// The note in the spectrogram's top left corner, beside the frequency
    /// strip. It gives the unit, which the axis labels leave out.
    pub fn freq_caption(self) -> String {
        format!("heard kHz = recording {}", self.freq_op())
    }

    /// The note at the end of the time strip.
    pub fn time_caption(self) -> String {
        format!("heard time = recording {}", self.time_op())
    }

    /// The sentence in the XForm menu.
    pub fn blurb(self) -> String {
        let factor = self.time_scale.max(self.freq_scale);
        format!(
            "Axes show what is heard at {}\u{00D7} time-expansion: frequency {}, time {}.",
            number(factor),
            self.freq_op(),
            self.time_op()
        )
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

/// The TE factor of a display that mirrors TE playback, when XForm applies
/// its Xform row. Tracked when `tracked`.
fn display_te_factor(state: &AppState, tracked: bool) -> Option<f64> {
    let (transform, mode, factor) = if tracked {
        (
            state.display.transform().get(),
            state.playback.mode().get(),
            state.transform.te_factor().get(),
        )
    } else {
        (
            state.display.transform().get_untracked(),
            state.playback.mode().get_untracked(),
            state.transform.te_factor().get_untracked(),
        )
    };
    (transform && mode == PlaybackMode::TimeExpansion).then_some(factor)
}

fn heard_axis_inner(state: &AppState, tracked: bool) -> Option<HeardAxis> {
    // The view is read before the early return, so a tracked caller is
    // subscribed to every signal on each run.
    let view = if tracked {
        state.viewmode.main_view().get()
    } else {
        state.viewmode.main_view().get_untracked()
    };
    let axis = HeardAxis::for_te_factor(display_te_factor(state, tracked)?)?;
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

/// A Resampling target rate, given as the rate of the audio heard, as a rate of
/// the recording, for a display whose Xform row is applied. Under TE ×10 the
/// output's 48 kHz holds what the recording has up to 480 kHz. Other modes
/// keep the rate, and 0 (no resampling) stays 0. Reads the mode and the TE
/// factor tracked; it does not read `display.transform`, which the calling
/// Effect writes.
pub fn resample_target_in_recording(state: &AppState, heard_rate: u32) -> u32 {
    let mode = state.playback.mode().get();
    let factor = state.transform.te_factor().get();
    let scale = (mode == PlaybackMode::TimeExpansion)
        .then(|| HeardAxis::for_te_factor(factor))
        .flatten()
        .map_or(1.0, |axis| axis.freq_scale);
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
        assert_eq!(axis.time_caption(), "heard time = recording ×10");
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
        assert_eq!(axis.time_caption(), "heard time = recording ÷4");
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
