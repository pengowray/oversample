use crate::audio::source::ChannelView;
use crate::audio::streaming_source::read_views_ready;
use crate::dsp::channel_delay::{
    right_leads_by_secs, MAX_ANALYSIS_SECS, MAX_DELAY_SECS, SPEED_OF_SOUND_M_S,
};
use crate::state::store_fields::*;
use crate::state::AppState;
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

/// The part of the current file to measure.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DelayRegion {
    pub time_start: f64,
    pub time_end: f64,
    pub freq_low: Option<f64>,
    pub freq_high: Option<f64>,
}

#[derive(Clone, Copy, PartialEq)]
enum Reading {
    Measuring,
    Finished(Option<f64>),
    /// Part of the selection's audio could not be loaded.
    Unreadable,
}

/// Rounds to the two decimals shown, so a delay shown as 0.00 ms never names a
/// channel as first.
fn delay_text(right_leads_by: f64) -> (String, Option<String>) {
    let ms = (right_leads_by.abs() * 1000.0 * 100.0).round() / 100.0;
    if ms == 0.0 {
        return ("L/R delay: 0.00 ms, L and R together".to_string(), None);
    }
    let first = if right_leads_by > 0.0 { "R" } else { "L" };
    let cm = right_leads_by.abs() * SPEED_OF_SOUND_M_S * 100.0;
    (
        format!("L/R delay: {ms:.2} ms, {first} first"),
        Some(format!("Path difference: {cm:.0} cm")),
    )
}

/// Time difference between the left and right channels within `region`, shown in
/// the "..." menu of a selection or annotation. It's a menu row because the status
/// bar, which also lists selection details, is hidden by default. It shows only for
/// 2-channel files, because other files have no L and R to compare.
#[component]
pub fn ChannelDelayRow(#[prop(into)] region: Signal<Option<DelayRegion>>) -> impl IntoView {
    let state = expect_context::<AppState>();
    let reading = RwSignal::new(Reading::Measuring);
    let generation = RwSignal::new(0u32);

    let current_source = move || {
        let index = state.library.current_index().get_untracked()?;
        state.library.files().with_untracked(|files| {
            let file = files.get(index)?;
            (file.audio.channels == 2).then(|| file.audio.source.clone())
        })
    };

    let is_stereo = Signal::derive(move || {
        let index = state.library.current_index().get();
        state.library.files().with(|files| {
            index
                .and_then(|i| files.get(i))
                .is_some_and(|file| file.audio.channels == 2)
        })
    });

    Effect::new(move || {
        let Some(region) = region.get() else {
            return;
        };
        if !is_stereo.get() {
            return;
        }
        let this_run = generation.get_untracked().wrapping_add(1);
        generation.set(this_run);
        reading.set(Reading::Measuring);

        spawn_local(async move {
            // The menu can close while this runs, which disposes these signals, so
            // reads use try_*.
            crate::web_util::yield_now().await;
            let is_latest = move || generation.try_get_untracked() == Some(this_run);
            if !is_latest() {
                return;
            }
            let Some(source) = current_source() else {
                return;
            };
            let sample_rate = source.sample_rate();
            let sr = sample_rate as f64;
            let start = (region.time_start * sr) as u64;
            let wanted_secs = (region.time_end - region.time_start).min(MAX_ANALYSIS_SECS);
            let len = (wanted_secs * sr) as usize;

            let views = [ChannelView::Channel(0), ChannelView::Channel(1)];
            let read = read_views_ready(&*source, &views, start, len).await;
            if !is_latest() {
                return;
            }
            let Some([left, right]) = read.views.and_then(|v| <[Vec<f32>; 2]>::try_from(v).ok())
            else {
                reading.set(Reading::Unreadable);
                return;
            };
            let band = region.freq_low.zip(region.freq_high);
            let result = right_leads_by_secs(&left, &right, sample_rate, band);
            reading.set(Reading::Finished(result));
        });
    });

    let hover_text = format!(
        "Assumes sound travels at {SPEED_OF_SOUND_M_S:.0} m/s (air at 20 \u{00B0}C). \
         Searches delays up to {:.0} ms.",
        MAX_DELAY_SECS * 1000.0
    );

    let measured_part_only = move || {
        region
            .get()
            .is_some_and(|r| r.time_end - r.time_start > MAX_ANALYSIS_SECS)
    };

    move || {
        if !is_stereo.get() || region.get().is_none() {
            return None;
        }
        let (headline, detail) = match reading.get() {
            Reading::Measuring => ("L/R delay: measuring\u{2026}".to_string(), None),
            Reading::Finished(Some(right_leads_by)) => delay_text(right_leads_by),
            Reading::Unreadable => ("L/R delay: couldn't read the audio".to_string(), None),
            Reading::Finished(None) => (
                "L/R delay: no clear match".to_string(),
                Some("Select one call and its frequency range".to_string()),
            ),
        };
        Some(view! {
            <div class="canvas-overflow-info channel-delay" title=hover_text.clone()>
                <div class="channel-delay-headline">{headline}</div>
                {detail.map(|text| view! { <div>{text}</div> })}
                {measured_part_only().then(|| view! {
                    <div>"Measured the first 2 s only"</div>
                })}
            </div>
        })
    }
}
