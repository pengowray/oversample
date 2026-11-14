//! Read-only state snapshot exposed on `window.__oversample_test()` for end-to-
//! end (Playwright) assertions. Installed once at startup.
//!
//! This is intentionally always-on (the e2e harness runs the `--release`
//! build), but it is harmless in production: it only *reads* signals via
//! `get_untracked` and returns a plain JS object — there are no setters and no
//! reactive subscriptions, so it can't affect app behavior.
//!
//! `install` also installs `window.__oversample_classifier`
//! (`crate::classifier::test_hook`), which is not read-only: it can start model
//! downloads and runs through the classifier API.

use crate::state::store_fields::*;
use crate::state::AppState;
use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

fn set(obj: &js_sys::Object, key: &str, val: JsValue) {
    let _ = js_sys::Reflect::set(obj, &JsValue::from_str(key), &val);
}

fn opt_f64(v: Option<f64>) -> JsValue {
    match v {
        Some(x) => JsValue::from_f64(x),
        None => JsValue::NULL,
    }
}

/// Install `window.__oversample_test`, a zero-arg function returning a snapshot
/// of view, current-file and live-capture state used by the e2e specs.
pub fn install(state: AppState) {
    crate::classifier::test_hook::install(state);
    let Some(window) = web_sys::window() else {
        return;
    };

    let cb = Closure::wrap(Box::new(move || -> JsValue {
        use crate::canvas::live_waterfall as wf;
        let o = js_sys::Object::new();
        // View
        set(
            &o,
            "scrollOffset",
            JsValue::from_f64(state.view.scroll_offset().get_untracked()),
        );
        set(
            &o,
            "zoom",
            JsValue::from_f64(state.view.zoom_level().get_untracked()),
        );
        set(
            &o,
            "minFreq",
            opt_f64(state.view.min_display_freq().get_untracked()),
        );
        set(
            &o,
            "maxFreq",
            opt_f64(state.view.max_display_freq().get_untracked()),
        );
        // HFR band and playback mode
        set(
            &o,
            "hfrEnabled",
            JsValue::from_bool(state.viewmode.hfr_enabled().get_untracked()),
        );
        set(
            &o,
            "bandLo",
            JsValue::from_f64(state.filter.band_ff_freq_lo().get_untracked()),
        );
        set(
            &o,
            "bandHi",
            JsValue::from_f64(state.filter.band_ff_freq_hi().get_untracked()),
        );
        set(
            &o,
            "playbackMode",
            JsValue::from_str(&format!("{:?}", state.playback.mode().get_untracked())),
        );
        set(
            &o,
            "hetFreq",
            JsValue::from_f64(state.transform.het_frequency().get_untracked()),
        );
        set(
            &o,
            "hetCutoff",
            JsValue::from_f64(state.transform.het_cutoff().get_untracked()),
        );
        // The current file: its rate and duration, the top of its
        // spectrogram's frequency axis, and what is drawn on it.
        let current = state.library.current_index().get_untracked().and_then(|i| {
            state.library.files().with_untracked(|files| {
                files.get(i).map(|f| {
                    (
                        f.id,
                        f.audio.sample_rate,
                        f.audio.duration_secs,
                        f.spectrogram.max_freq,
                    )
                })
            })
        });
        if let Some((id, rate, duration, max_freq)) = current {
            set(&o, "sampleRate", JsValue::from_f64(f64::from(rate)));
            set(&o, "duration", JsValue::from_f64(duration));
            set(&o, "fileMaxFreq", JsValue::from_f64(max_freq));
            let lo = state.view.min_display_freq().get_untracked().unwrap_or(0.0);
            let hi = state
                .view
                .max_display_freq()
                .get_untracked()
                .unwrap_or(max_freq);
            set(&o, "displayFreqLo", JsValue::from_f64(lo));
            set(&o, "displayFreqHi", JsValue::from_f64(hi));
            let anns = js_sys::Array::new();
            state.annotations.store().with_untracked(|store| {
                for a in store
                    .get(id)
                    .map(|s| s.annotations.as_slice())
                    .unwrap_or(&[])
                {
                    if let crate::annotations::AnnotationKind::Region(r) = &a.kind {
                        let ao = js_sys::Object::new();
                        set(&ao, "timeStart", JsValue::from_f64(r.time_start));
                        set(&ao, "timeEnd", JsValue::from_f64(r.time_end));
                        set(&ao, "freqLow", opt_f64(r.freq_low));
                        set(&ao, "freqHigh", opt_f64(r.freq_high));
                        anns.push(&ao);
                    }
                }
            });
            set(&o, "regions", anns.into());
        }
        let selection = state.interaction.selection().get_untracked();
        set(
            &o,
            "selection",
            match selection {
                Some(sel) => {
                    let so = js_sys::Object::new();
                    set(&so, "timeStart", JsValue::from_f64(sel.time_start));
                    set(&so, "timeEnd", JsValue::from_f64(sel.time_end));
                    set(&so, "freqLow", opt_f64(sel.freq_low));
                    set(&so, "freqHigh", opt_f64(sel.freq_high));
                    so.into()
                }
                None => JsValue::NULL,
            },
        );
        set(
            &o,
            "isPlaying",
            JsValue::from_bool(state.playback.is_playing().get_untracked()),
        );
        // The HFR band in force (0 to 0 when HFR is off).
        set(
            &o,
            "hfrBandLo",
            JsValue::from_f64(state.filter.band_ff_freq_lo().get_untracked()),
        );
        set(
            &o,
            "hfrBandHi",
            JsValue::from_f64(state.filter.band_ff_freq_hi().get_untracked()),
        );
        // Live capture
        let listening = state.mic.listening().get_untracked();
        let recording = state.mic.recording().get_untracked();
        let live_active = wf::is_active();
        set(&o, "listening", JsValue::from_bool(listening));
        set(&o, "recording", JsValue::from_bool(recording));
        set(&o, "liveActive", JsValue::from_bool(live_active));
        set(&o, "liveTotalTime", JsValue::from_f64(wf::total_time()));
        set(
            &o,
            "liveDataCols",
            JsValue::from_f64(state.mic.live_data_cols().get_untracked() as f64),
        );
        set(
            &o,
            "recordingTargetScroll",
            JsValue::from_f64(state.mic.recording_target_scroll().get_untracked()),
        );
        let pan_until = state.mic.scroll_user_pan_until().get_untracked();
        set(&o, "scrollUserPanUntil", JsValue::from_f64(pan_until));
        // Derived: is the live waterfall auto-follow currently engaged?
        let now = js_sys::Date::now();
        let following = (listening || recording) && live_active && now >= pan_until;
        set(&o, "following", JsValue::from_bool(following));
        // The overview's displayed window [axisStart, axisStart+span] (live only).
        if let Some((axis_start, span)) = crate::components::overview::live_overview_window(&state)
        {
            set(&o, "overviewAxisStart", JsValue::from_f64(axis_start));
            set(&o, "overviewSpan", JsValue::from_f64(span));
        }
        o.into()
    }) as Box<dyn Fn() -> JsValue>);

    let _ = js_sys::Reflect::set(
        &window,
        &JsValue::from_str("__oversample_test"),
        cb.as_ref().unchecked_ref(),
    );
    cb.forget(); // keep the closure alive for the page lifetime
}
