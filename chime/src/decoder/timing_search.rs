//! The timing search: finds the symbol boundaries and rejects most false alarms.

use super::cplx::Cplx;
use super::front_end::Ring;
use crate::format::START_STOP_US;

/// Length of the window that the timing search folds, in seconds.
const FOLD_WINDOW_S: f64 = 0.050;

/// One slot for each baseband sample in a 7.5 ms period at 8.8 kHz.
pub(super) const MAX_FOLD_SLOTS: usize = 67;

/// Lowest coherence that lets a walk start. Coherence is the power of the folded
/// signal over the power of all samples in the window, as a fraction of what a
/// clean run of start symbols gives.
const COHERENCE_GATE: f32 = 0.12;

/// The walk starts this many start symbols before the newest sample.
const WALK_BACK_SYMBOLS: f64 = 5.0;

/// What the timing search found.
pub(super) struct Found {
    /// Where the walk starts, as a position in baseband samples.
    pub(super) start: f64,
    pub(super) coherence: f32,
    /// The carrier frequency error, in radians per bucket.
    pub(super) carrier_frequency: f32,
}

/// Finds the symbol boundaries in the newest 50 ms. Returns `None` if the window does
/// not look like start symbols.
///
/// The search flips the sign of every other 7.5 ms period of the window and adds
/// the periods together slot by slot. With true boundaries at slot t in each
/// period, the slots before t then all hold one polarity and the slots from t on
/// hold the other. So t is the split that maximises the size of (sum of the
/// slots from t on) minus (sum of the slots before t).
///
/// The window is short enough for the phase to change little between periods
/// unless the carrier is many Hz off. The detector's lag product (`lag_p`, `lag_q`)
/// gives the phase change per period, which the search removes.
///
/// `slots` is working space, and `power_limit` is the limit on the power of a sample.
pub(super) fn search(
    ring: &Ring,
    slots: &mut [Cplx; MAX_FOLD_SLOTS],
    baseband_rate: f64,
    (lag_p, lag_q): (f32, f32),
    power_limit: f32,
) -> Option<Found> {
    let period = START_STOP_US as f64 * 1e-6 * baseband_rate;
    let window = ((FOLD_WINDOW_S * baseband_rate) as u64).min(ring.count()) as usize;
    let first = ring.count() - window as u64;

    let phase_per_period = libm::atan2f(lag_q, lag_p);
    let rotation = Cplx { re: libm::cosf(phase_per_period), im: -libm::sinf(phase_per_period) };

    let slot_count = libm::ceil(period) as usize;
    let slots = &mut slots[..slot_count];
    slots.fill(Cplx::ZERO);
    let mut position = 0.0f32;
    let period_f32 = period as f32;
    // The sign flip of each period is part of this phasor.
    let mut phasor = Cplx::ONE;
    let mut energy = 0.0f32;
    for offset in 0..window {
        let z = ring.at(first + offset as u64).limited(power_limit);
        slots[position as usize] += z.mul(phasor);
        energy += z.power();
        position += 1.0;
        if position >= period_f32 {
            position -= period_f32;
            phasor = -phasor.mul(rotation);
        }
    }

    let mut total = Cplx::ZERO;
    for &slot in slots.iter() {
        total += slot;
    }
    let mut before = Cplx::ZERO;
    let mut best_score = -1.0f32;
    let mut best_slot = 0usize;
    for (t, &slot) in slots.iter().enumerate() {
        let score = (total - before * 2.0).power();
        if score > best_score {
            best_score = score;
            best_slot = t;
        }
        before += slot;
    }

    let coherence = best_score / (window as f32 * energy.max(1e-30));
    if coherence < COHERENCE_GATE {
        return None;
    }

    // Start the walk on the boundary 5 to 6 symbols before the newest sample. The
    // window is 6.67 symbols long, so that boundary is inside it.
    let newest = ring.count() as f64;
    let boundary = first as f64 + best_slot as f64;
    let periods = libm::floor((newest - boundary) / period - WALK_BACK_SYMBOLS).max(0.0);
    Some(Found { start: boundary + periods * period, coherence, carrier_frequency: phase_per_period / 3.0 })
}
