//! The two kinds of box drawn over the spectrogram, and how each looks.
//!
//! - Animal: a bird, bat, insect, frog or mammal call. A solid line with a
//!   dark edge under it, a light fill when picked, and a bright label.
//! - Noise: anything else the detectors found: tones, electrical pulses,
//!   clicks and rustles (often handling noise), engines. A dashed grey line,
//!   no fill, and a muted label.
//!
//! The difference is the line (solid or dashed) and fill, not only the
//! colour, since colour already means species where BattyBirdNET names the
//! bat calls (`overlays::SPECIES_COLOURS`), and so it holds on every colour
//! map.

use crate::dsp::call_detect::CallKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxClass {
    Animal,
    Noise,
}

/// The call detector's kinds: bat calls, single ones included, are animal
/// calls; the rest are noise. An Audible detection ("probably a bird, voice
/// or footstep") is noise here because it is in the bat band only as a side
/// effect of a louder sound below it; the bird boxes show the bird itself.
pub fn call_class(kind: CallKind) -> BoxClass {
    match kind {
        CallKind::Bat | CallKind::Single => BoxClass::Animal,
        CallKind::Tone
        | CallKind::PeriodicPulse
        | CallKind::Broadband
        | CallKind::TooLow
        | CallKind::Audible => BoxClass::Noise,
    }
}

/// Lime: no other overlay uses it. Species are pink, sky blue and violet,
/// sound clusters dark grey, green, orange and yellow, annotations green and
/// amber, the selection amber, PC rows blue and orange. It was the bird box
/// colour; bat calls had a green of their own until both became one class.
pub const ANIMAL_STROKE: &str = "rgba(180, 240, 60, 0.95)";
pub const ANIMAL_FILL: &str = "rgba(180, 240, 60, 0.14)";
/// A bat call with no similar call near it: probably a bat, less sure.
pub const ANIMAL_UNSURE_STROKE: &str = "rgba(180, 240, 60, 0.55)";
/// Call trains: a line along the top of the spectrogram.
pub const TRAIN_STROKE: &str = "rgba(180, 240, 60, 0.6)";

pub const NOISE_STROKE: &str = "rgba(190, 190, 190, 0.6)";
pub const NOISE_LABEL: &str = "rgba(200, 200, 200, 0.85)";
/// Dash and gap of a noise box's line, in pixels.
pub const NOISE_DASH: (f64, f64) = (4.0, 3.0);

/// Set the line dash for `class` on `ctx`.
pub fn set_dash(ctx: &web_sys::CanvasRenderingContext2d, class: BoxClass) {
    let dash = match class {
        BoxClass::Animal => js_sys::Array::new(),
        BoxClass::Noise => js_sys::Array::of2(&NOISE_DASH.0.into(), &NOISE_DASH.1.into()),
    };
    let _ = ctx.set_line_dash(&dash);
}
