//! Audio contexts made at a wanted sample rate.
//!
//! A browser can make a context at a rate without an error and still never
//! start it: Firefox 160 does this at 500 kHz. The context's state stays
//! "suspended", its clock stays at 0, the promise from `resume()` never
//! settles, and nothing plays. So the rate asked for is capped
//! ([`context_rate_for`]), and a context that has not started within
//! [`START_WAIT_MS`] is replaced by one at the browser's default rate when
//! that one does start.

use crate::audio::streaming_playback::context_rate_for;
use crate::web_util::sleep_ms;
use std::cell::Cell;
use wasm_bindgen::JsValue;
use web_sys::{AudioContext, AudioContextOptions, AudioContextState};

/// How long a new context is given to reach "running" after `resume()`.
/// A context that starts does so within a few tens of milliseconds.
const START_WAIT_MS: i32 = 1000;
/// How often the context's state is read while waiting.
const START_POLL_MS: i32 = 25;

thread_local! {
    /// Set when a context did not start and a default one did: the rate
    /// that did not start, and the default context's rate. Later contexts
    /// at that rate or above are asked for at the default rate.
    static DID_NOT_START: Cell<Option<(u32, u32)>> = const { Cell::new(None) };
}

/// The number in `window.__oversample_max_context_rate` when a test has set
/// one: the highest rate a context is asked for, in place of the built-in
/// cap, so a test can ask a browser for a rate it does not start.
fn test_max_rate() -> Option<u32> {
    web_sys::window()
        .and_then(|w| js_sys::Reflect::get(&w, &"__oversample_max_context_rate".into()).ok())
        .and_then(|v| v.as_f64())
        .map(|n| n as u32)
}

/// The rate to ask a context for, given the cap a test set (if any) and
/// the rate found not to start (if any) with the default rate used instead.
fn pick_rate(buffer_rate: u32, test_max: Option<u32>, did_not_start: Option<(u32, u32)>) -> u32 {
    let capped = match test_max {
        Some(max) => buffer_rate.min(max),
        None => context_rate_for(buffer_rate),
    };
    match did_not_start {
        Some((bad_rate, default_rate)) if capped >= bad_rate => default_rate,
        _ => capped,
    }
}

/// The sample rate to ask of an audio context that plays buffers, or reads
/// a mic, at `buffer_rate`.
pub(crate) fn context_rate(buffer_rate: u32) -> u32 {
    pick_rate(buffer_rate, test_max_rate(), DID_NOT_START.with(Cell::get))
}

/// A context at [`context_rate`] of `buffer_rate`, or at the browser's
/// default rate when the browser refuses that rate.
pub(crate) fn new_context(buffer_rate: u32) -> Result<AudioContext, JsValue> {
    let opts = AudioContextOptions::new();
    opts.set_sample_rate(context_rate(buffer_rate) as f32);
    AudioContext::new_with_context_options(&opts).or_else(|e| {
        log::warn!(
            "AudioContext at {} Hz refused ({e:?}); using the default rate",
            context_rate(buffer_rate)
        );
        AudioContext::new()
    })
}

/// Asks `ctx` to start and waits up to [`START_WAIT_MS`] for it. The
/// promise from `resume()` is not awaited, because it never settles in a
/// context that does not start.
async fn starts(ctx: &AudioContext) -> bool {
    let _ = ctx.resume();
    let mut waited = 0;
    loop {
        if ctx.state() == AudioContextState::Running {
            return true;
        }
        if waited >= START_WAIT_MS {
            return false;
        }
        sleep_ms(START_POLL_MS).await;
        waited += START_POLL_MS;
    }
}

/// Waits for `ctx` to start. Returns `None` when it does. When it does
/// not, tries a context at the browser's default rate: if that one starts
/// it is returned, running, for the caller to use in place of `ctx` (the
/// caller closes `ctx`), and the rate is remembered so it is not asked for
/// again.
///
/// When neither starts, the browser is holding all audio back, usually
/// until the user presses something on the page. `ctx` is then the right
/// context to keep, and `None` is returned.
pub(crate) async fn default_if_not_started(ctx: &AudioContext) -> Option<AudioContext> {
    if starts(ctx).await {
        return None;
    }
    let default_ctx = AudioContext::new().ok()?;
    let rate = ctx.sample_rate() as u32;
    let default_rate = default_ctx.sample_rate() as u32;
    if starts(&default_ctx).await {
        // The log goes to the browser console.
        log::warn!(
            "An audio context at {rate} Hz did not start in this browser. \
             Using one at {default_rate} Hz."
        );
        DID_NOT_START.with(|d| d.set(Some((rate, default_rate))));
        return Some(default_ctx);
    }
    let _ = default_ctx.close();
    None
}

#[cfg(test)]
mod tests {
    use super::pick_rate;

    #[test]
    fn the_rate_asked_for_is_capped_at_384_khz() {
        assert_eq!(pick_rate(500_000, None, None), 384_000);
        assert_eq!(pick_rate(256_000, None, None), 256_000);
        assert_eq!(pick_rate(48_000, None, None), 48_000);
    }

    #[test]
    fn a_test_can_set_another_cap() {
        assert_eq!(pick_rate(500_000, Some(500_000), None), 500_000);
        assert_eq!(pick_rate(500_000, Some(96_000), None), 96_000);
    }

    #[test]
    fn a_rate_that_did_not_start_is_replaced_by_the_default_rate() {
        let did_not_start = Some((384_000, 48_000));
        assert_eq!(pick_rate(500_000, None, did_not_start), 48_000);
        assert_eq!(pick_rate(384_000, None, did_not_start), 48_000);
        // A lower rate is still asked for: it may start.
        assert_eq!(pick_rate(256_000, None, did_not_start), 256_000);
    }
}
