//! The play button beside a selection's or annotation's "..." button, and the
//! "Play with" row of the annotation's "..." menu, which sets how that
//! annotation's play button plays it (`Annotation::play`).

use crate::annotations::{now_iso8601, AnnotationId, AnnotationKind, PlaySetting};
use crate::audio::playback;
use crate::components::icons::Icon;
use crate::state::store_fields::*;
use crate::state::{AppState, PlaybackMode, PlayingItem, Selection};
use leptos::prelude::*;

/// Width of the "..." and play buttons, in CSS pixels (`button.canvas-overflow-btn`,
/// `button.canvas-item-play`).
pub(crate) const BTN_SIZE: f64 = 22.0;
/// Space between the "..." and play buttons (`.canvas-overflow-anchor` gap).
pub(crate) const BTN_GAP: f64 = 4.0;

/// About how wide the play button is: square, or wider with the
/// annotation's own setting written on it ("HET 45 kHz"). For keeping the
/// buttons on the canvas and the annotation's label clear of them.
pub(crate) fn play_button_width(setting: Option<PlaySetting>) -> f64 {
    match setting.filter(|s| *s != PlaySetting::Unknown) {
        None => BTN_SIZE,
        // 11 px text, about 6.5 px a character, beside the 10 px icon.
        Some(s) => BTN_SIZE + 6.0 + s.short_label().chars().count() as f64 * 6.5,
    }
}

/// The time and frequency range of the selected annotations, as playback
/// plays them, and the one annotation's id and play setting when only one is
/// selected. Reactive. As [`playback::effective_selection`] for annotations.
pub(crate) fn selected_annotations_play(
    state: AppState,
) -> Option<(Selection, AnnotationId, Option<PlaySetting>)> {
    let ids = state.annotations.selected_ids().get();
    let file_id = state.current_file_id_tracked()?;
    state.annotations.store().with(|store| {
        let set = store.get(file_id)?;
        let mut range: Option<Selection> = None;
        let mut all_have_freq = true;
        let mut first: Option<(AnnotationId, Option<PlaySetting>)> = None;
        for ann in set.annotations.iter().filter(|a| ids.contains(&a.id)) {
            let AnnotationKind::Region(r) = &ann.kind else {
                continue;
            };
            first.get_or_insert_with(|| (ann.id.clone(), ann.play));
            all_have_freq &= r.freq_low.is_some() && r.freq_high.is_some();
            range = Some(match range {
                None => Selection {
                    time_start: r.time_start,
                    time_end: r.time_end,
                    freq_low: r.freq_low,
                    freq_high: r.freq_high,
                },
                Some(s) => Selection {
                    time_start: s.time_start.min(r.time_start),
                    time_end: s.time_end.max(r.time_end),
                    freq_low: s.freq_low.zip(r.freq_low).map(|(a, b)| a.min(b)),
                    freq_high: s.freq_high.zip(r.freq_high).map(|(a, b)| a.max(b)),
                },
            });
        }
        let mut range = range?;
        if !all_have_freq {
            range.freq_low = None;
            range.freq_high = None;
        }
        if range.time_end - range.time_start < 0.0001 {
            return None;
        }
        let (id, play) = first?;
        // Several annotations play together with the app's settings.
        let play = if ids.len() == 1 { play } else { None };
        Some((range, id, play))
    })
}

/// Play button beside a "..." button: plays `range` with the annotation's own
/// setting when it has one, then stops. Shows a stop button while it plays.
#[component]
pub(crate) fn ItemPlayButton(
    /// What the button plays.
    range: Selection,
    /// The annotation; None for the selection.
    annotation: Option<AnnotationId>,
    /// The annotation's own play setting.
    setting: Option<PlaySetting>,
) -> impl IntoView {
    let state = expect_context::<AppState>();
    let setting = setting.filter(|s| *s != PlaySetting::Unknown);
    let annotation_for_check = annotation.clone();
    let playing = Signal::derive(move || {
        state.playback.is_playing().get()
            && state
                .playback
                .item()
                .with(|i| i.as_ref().is_some_and(|i| i.annotation == annotation_for_check))
    });
    let label = setting.map(|s| s.short_label());
    let title = {
        let label = label.clone();
        move || match (playing.get(), &label) {
            (true, _) => "Stop".to_string(),
            (false, Some(l)) => format!("Play with {l}"),
            (false, None) => "Play".to_string(),
        }
    };
    let on_click = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        if playing.get_untracked() {
            playback::stop(&state);
        } else {
            playback::play_item(
                &state,
                range,
                PlayingItem {
                    annotation: annotation.clone(),
                    setting,
                },
            );
        }
    };
    view! {
        <button
            class="canvas-item-play"
            class:playing=move || playing.get()
            title=title
            on:click=on_click
        >
            {move || if playing.get() {
                view! { <Icon kind=Icon::Stop /> }
            } else {
                view! { <Icon kind=Icon::Play /> }
            }}
            {label.map(|l| view! { <span class="canvas-item-play-label">{l}</span> })}
        </button>
    }
}

/// Set or clear annotation `id`'s own play setting, with undo. A playing
/// annotation goes on playing with the new setting.
fn set_annotation_play(state: AppState, id: &AnnotationId, play: Option<PlaySetting>) {
    let Some(file_id) = state.current_file_id() else {
        return;
    };
    state.snapshot_annotations();
    state.annotations.store().update(|store| {
        if let Some(ann) = store
            .get_mut(file_id)
            .and_then(|set| set.annotations.iter_mut().find(|a| &a.id == id))
        {
            ann.play = play;
            ann.modified_at = now_iso8601();
        }
    });
    state.annotations.dirty().set(true);
    let playing_this = state.playback.is_playing().get_untracked()
        && state
            .playback
            .item()
            .with_untracked(|i| i.as_ref().is_some_and(|i| i.annotation.as_ref() == Some(id)));
    if playing_this {
        state.playback.item().update(|i| {
            if let Some(i) = i {
                i.setting = play;
            }
        });
        playback::schedule_replay_live(&state);
    }
}

/// The modes in the "Play with" list, after "Hear settings".
const MODES: [(PlaybackMode, &str); 5] = [
    (PlaybackMode::Normal, "1:1"),
    (PlaybackMode::Heterodyne, "HET"),
    (PlaybackMode::TimeExpansion, "TE"),
    (PlaybackMode::PitchShift, "PS"),
    (PlaybackMode::ZeroCrossing, "ZC"),
];

fn mode_key(mode: PlaybackMode) -> &'static str {
    match mode {
        PlaybackMode::Normal => "normal",
        PlaybackMode::Heterodyne => "het",
        PlaybackMode::TimeExpansion => "te",
        PlaybackMode::PitchShift => "ps",
        PlaybackMode::ZeroCrossing => "zc",
    }
}

/// The setting picked from the list: the mode with the app's current value
/// for it. HET takes the centre of the annotation's frequency range when it
/// has one, which is where playing it tunes with the app's settings.
fn setting_for(
    state: AppState,
    mode: PlaybackMode,
    band: Option<(f64, f64)>,
) -> PlaySetting {
    let t = state.transform;
    match mode {
        PlaybackMode::Normal => PlaySetting::Direct,
        PlaybackMode::Heterodyne => PlaySetting::Heterodyne {
            freq_hz: band
                .map(|(lo, hi)| ((lo + hi) / 2.0 / 100.0).round() * 100.0)
                .unwrap_or_else(|| t.het_frequency().get_untracked()),
        },
        PlaybackMode::TimeExpansion => PlaySetting::TimeExpansion {
            factor: t.te_factor().get_untracked(),
        },
        PlaybackMode::PitchShift => PlaySetting::PitchShift {
            factor: t.ps_factor().get_untracked().signed(),
        },
        PlaybackMode::ZeroCrossing => PlaySetting::ZeroCrossing {
            factor: t.zc_factor().get_untracked(),
        },
    }
}

/// "Play with" row of an annotation's "..." menu: "Hear settings" (the
/// app's playback settings) or a mode with its frequency or factor, which the
/// annotation's play button then uses.
#[component]
pub(crate) fn PlaySettingRow(
    id: AnnotationId,
    play: Option<PlaySetting>,
    /// The annotation's frequency range, when it has one.
    band: Option<(f64, f64)>,
) -> impl IntoView {
    let state = expect_context::<AppState>();
    let play = play.filter(|s| *s != PlaySetting::Unknown);
    let selected_key = play.and_then(|p| p.mode()).map_or("", mode_key);
    let id_select = id.clone();
    let on_select = move |ev: web_sys::Event| {
        let key = event_target_value(&ev);
        let setting = MODES
            .iter()
            .find(|(m, _)| mode_key(*m) == key)
            .map(|(m, _)| setting_for(state, *m, band));
        set_annotation_play(state, &id_select, setting);
    };
    let options = MODES
        .iter()
        .map(|(m, label)| {
            let key = mode_key(*m);
            view! { <option value=key selected=key == selected_key>{*label}</option> }
        })
        .collect_view();
    // The value of the chosen mode: kHz for HET, a factor for the others.
    let value_input = play.and_then(|p| {
        let value = p.value()?;
        let (before, after) = match p {
            PlaySetting::Heterodyne { .. } => ("", "kHz"),
            PlaySetting::TimeExpansion { .. } => ("\u{00D7}", ""),
            PlaySetting::PitchShift { factor } if factor < 0.0 => ("\u{00D7}", ""),
            _ => ("\u{00F7}", ""),
        };
        let sign = if value < 0.0 { -1.0 } else { 1.0 };
        let shown = value.abs();
        let id = id.clone();
        let input_ref = NodeRef::<leptos::html::Input>::new();
        let on_change = move |_| {
            let Some(input) = input_ref.get_untracked() else {
                return;
            };
            match input
                .value()
                .trim()
                .parse::<f64>()
                .ok()
                .and_then(|v| p.with_value(v * sign))
            {
                Some(new) if new != p => set_annotation_play(state, &id, Some(new)),
                // Not a value the mode can use: show the saved one again.
                _ => input.set_value(&format_value(shown)),
            }
        };
        let aria = match p {
            PlaySetting::Heterodyne { .. } => "HET frequency in kHz",
            _ => "Factor",
        };
        Some(view! {
            {(!before.is_empty()).then(|| view! { <span class="play-setting-unit">{before}</span> })}
            <input
                class="play-setting-value"
                type="text"
                inputmode="decimal"
                aria-label=aria
                node_ref=input_ref
                value=format_value(shown)
                on:change=on_change
                on:keydown=move |ev: web_sys::KeyboardEvent| {
                    // Keys typed here are not app shortcuts.
                    ev.stop_propagation();
                    if ev.key() == "Enter" {
                        if let Some(input) = input_ref.get_untracked() {
                            let _ = input.blur();
                        }
                    }
                }
            />
            {(!after.is_empty()).then(|| view! { <span class="play-setting-unit">{after}</span> })}
        })
    });
    view! {
        <div class="canvas-overflow-freq-row play-setting-row">
            <label class="canvas-overflow-freq-text" for="annotation-play-mode">"Play with"</label>
            <div class="canvas-overflow-btn-group">
                <select id="annotation-play-mode" class="play-setting-mode" on:change=on_select>
                    <option value="" selected=selected_key.is_empty()>"Hear settings"</option>
                    {options}
                </select>
                {value_input}
            </div>
        </div>
    }
}

/// A frequency in kHz or a factor, without a trailing ".0".
fn format_value(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if (r - r.round()).abs() < 1e-9 {
        format!("{}", r.round() as i64)
    } else {
        format!("{r}")
    }
}
