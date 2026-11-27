//! The Channels row of the mic menu for a device with more than 2 channels:
//! a button showing the channels in use, which opens a list of every choice.
//! The list has each pair (1 and 2, 3 and 4, ...) in stereo, then channels 1
//! and 2 averaged into one and each channel alone in mono.
//!
//! On a device with channel names (`ChannelLabels`) a row is the name, then
//! in lighter text where the sound comes from when the device says ("EXT MIC,
//! 48 V"), and the USB channel numbers at the right. Any other device shows
//! "Channels 5 and 6". With nothing saved, channels 1 and 2 in stereo.
//!
//! This is a list of its own and not a `<select>` because a `<select>` can't
//! show part of an option in lighter text.

use crate::components::popup::{Align, PopupPanel, Side};
use crate::state::store_fields::*;
use crate::state::AppState;
use leptos::prelude::*;
use oversample_core::audio::channel_labels::ChannelLabels;
use wasm_bindgen::JsCast;

/// One choice in the Channels list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelChoice {
    /// The pick as saved: "5,6", "5" or "avg" (1-based).
    pub value: String,
    /// "Input 3 and 4", or "Channels 5 and 6" on a device without names.
    pub name: String,
    /// Where the sound comes from, when the device says: "EXT MIC, 48 V".
    pub source: Option<String>,
    /// "USB 5, 6", on a device with names.
    pub usb: Option<String>,
    /// Recorded as one channel.
    pub mono: bool,
}

impl ChannelChoice {
    /// The text of the button that opens the list: "Input 3 and 4 (stereo)".
    pub fn button_text(&self) -> String {
        let mode = if self.mono { "mono" } else { "stereo" };
        format!("{} ({mode})", self.name)
    }
}

/// Every choice for a device with `channels` channels: the stereo ones, then
/// the mono ones. `sources` is what each channel carries, from a patched X8's
/// input report.
pub fn channel_choices(
    channels: u32,
    labels: Option<&ChannelLabels>,
    sources: Option<&[Option<String>]>,
) -> (Vec<ChannelChoice>, Vec<ChannelChoice>) {
    let choice = |picked: &[u32], averaged: bool| -> ChannelChoice {
        let numbers = picked
            .iter()
            .map(|c| (c + 1).to_string())
            .collect::<Vec<_>>();
        let (name, source, usb) = match labels.and_then(|l| l.pick_parts(picked, sources)) {
            Some(p) => (
                p.name,
                p.source,
                Some(format!("USB {}", numbers.join(", "))),
            ),
            None if picked.len() == 1 => (format!("Channel {}", numbers[0]), None, None),
            None => (format!("Channels {}", numbers.join(" and ")), None, None),
        };
        ChannelChoice {
            value: if averaged {
                "avg".to_string()
            } else {
                numbers.join(",")
            },
            // Never "mix" or "mixed" for this: on a recorder "mix" is the
            // stereo mix on channels 1 and 2.
            name: if averaged {
                format!("{name} averaged")
            } else {
                name
            },
            source,
            usb,
            mono: averaged || picked.len() == 1,
        }
    };
    let stereo = (0..channels.saturating_sub(1))
        .step_by(2)
        .map(|a| choice(&[a, a + 1], false))
        .collect();
    let mono = std::iter::once(choice(&[0, 1], true))
        .chain((0..channels).map(|a| choice(&[a], false)))
        .collect();
    (stereo, mono)
}

#[component]
pub fn WideChannelRow(
    channels: u32,
    labels: Option<ChannelLabels>,
    /// What each channel carries, from a patched X8's input report.
    sources: Option<Vec<Option<String>>>,
) -> impl IntoView {
    let state = expect_context::<AppState>();
    let open = RwSignal::new(false);
    let anchor = NodeRef::<leptos::html::Div>::new();

    // The pick as saved ("5,6", "5" or "avg"), or "1,2" when nothing usable
    // is saved.
    let current = move || {
        let device = state.mic.device_name().get();
        let pick = state
            .mic
            .channel_picks()
            .with(|p| crate::state::channel_pick_for(p, device.as_deref()));
        if !pick.channels.is_empty() && pick.channels.iter().all(|&c| c < channels) {
            crate::state::format_channel_pick(&pick)
        } else {
            "1,2".to_string()
        }
    };
    let choose = move |value: String| {
        open.set(false);
        let Some(device) = state.mic.device_name().get_untracked() else {
            return;
        };
        crate::state::set_channel_pick(&state, &device, crate::state::parse_channel_pick(&value));
        super::bottom_toolbar::restart_listening(state);
    };

    let (stereo, mono) = channel_choices(channels, labels.as_ref(), sources.as_deref());
    let groups = StoredValue::new([("Stereo", stereo), ("Mono", mono)]);
    let button_text = move || {
        let value = current();
        groups.with_value(|g| {
            g.iter()
                .flat_map(|(_, choices)| choices)
                .find(|c| c.value == value)
                .map(ChannelChoice::button_text)
                .unwrap_or_default()
        })
    };

    // A press anywhere outside the list and its button closes the list. The
    // list is drawn under <body>, outside the mic menu, so the mic menu's own
    // handlers never see a press on it. `try_` reads: this listener can run
    // while the row is being removed.
    let outside = window_event_listener(leptos::ev::pointerdown, move |ev| {
        if open.try_get_untracked() != Some(true) {
            return;
        }
        let inside = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
            .and_then(|el| {
                el.closest(".channel-menu, .channel-menu-btn")
                    .ok()
                    .flatten()
            })
            .is_some();
        if !inside {
            open.try_set(false);
        }
    });
    on_cleanup(move || outside.remove());

    view! {
        <div class="layer-panel-slider-row het-text-row">
            <label style="font-size: 11px;" title="Channels used for listening and recording. The live spectrogram shows left and right averaged.">"Channels"</label>
            <div node_ref=anchor class="channel-menu-anchor">
                <button
                    class=move || if open.get() { "channel-menu-btn open" } else { "channel-menu-btn" }
                    aria-haspopup="listbox"
                    aria-expanded=move || if open.get() { "true" } else { "false" }
                    on:click=move |_| open.update(|o| *o = !*o)
                    on:keydown=move |ev: web_sys::KeyboardEvent| {
                        if ev.key() == "Escape" && open.get_untracked() {
                            ev.stop_propagation();
                            open.set(false);
                        }
                    }
                >
                    <span class="channel-menu-btn-text">{button_text}</span>
                    <span class="channel-menu-btn-caret" aria-hidden="true">"\u{25BE}"</span>
                </button>
            </div>
            // Above the mic menu, which is at z-index 1000 on phones.
            <PopupPanel
                is_open=Signal::derive(move || open.get())
                anchor=anchor
                preferred_side=Side::Above
                preferred_align=Align::End
                class="layer-panel channel-menu"
                extra_style="z-index: 1100; min-width: 230px;"
            >
                {move || groups.with_value(|g| g.iter().map(|(title, choices)| view! {
                    <div class="layer-panel-title">{*title}</div>
                    <div role="listbox" aria-label=*title>
                        {choices.iter().cloned().map(|c| {
                            let value = c.value.clone();
                            let selected = { let v = value.clone(); move || current() == v };
                            let is_selected = selected.clone();
                            view! {
                                <button
                                    role="option"
                                    aria-selected=move || if is_selected() { "true" } else { "false" }
                                    class=move || if selected() { "layer-panel-opt channel-opt sel" } else { "layer-panel-opt channel-opt" }
                                    on:click=move |_| choose(value.clone())
                                >
                                    <span class="channel-opt-text">
                                        <span class="channel-opt-name">{c.name}</span>
                                        {c.source.map(|s| view! { <span class="channel-opt-source">{s}</span> })}
                                    </span>
                                    {c.usb.map(|u| view! { <span class="channel-opt-usb">{u}</span> })}
                                </button>
                            }
                        }).collect::<Vec<_>>()}
                    </div>
                }).collect::<Vec<_>>())}
            </PopupPanel>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oversample_core::audio::channel_labels::{channel_labels, device_channel_labels};

    fn row(c: &ChannelChoice) -> (&str, &str, Option<&str>, Option<&str>) {
        (&c.value, &c.name, c.source.as_deref(), c.usb.as_deref())
    }

    #[test]
    fn a_known_device_shows_its_channel_names_and_the_usb_numbers() {
        let x8 = channel_labels("Portacapture X8", 8).unwrap();
        let (stereo, mono) = channel_choices(8, Some(&x8), None);
        assert_eq!(stereo.len(), 4);
        assert_eq!(
            row(&stereo[0]),
            ("1,2", "Mix L and R", None, Some("USB 1, 2"))
        );
        assert_eq!(
            row(&stereo[2]),
            ("5,6", "Input 3 and 4", None, Some("USB 5, 6"))
        );
        assert_eq!(stereo[2].button_text(), "Input 3 and 4 (stereo)");
        assert_eq!(mono.len(), 9);
        assert_eq!(
            row(&mono[0]),
            ("avg", "Mix L and R averaged", None, Some("USB 1, 2"))
        );
        assert_eq!(mono[0].button_text(), "Mix L and R averaged (mono)");
        assert_eq!(row(&mono[2]), ("2", "Mix R", None, Some("USB 2")));
        assert_eq!(mono[2].button_text(), "Mix R (mono)");
    }

    #[test]
    fn a_device_s_own_names_give_each_row_its_source() {
        let names: Vec<String> = [
            "Mix L",
            "Mix R",
            "Input 1: INT MIC",
            "Input 2: INT MIC",
            "Input 3: EXT MIC",
            "Input 4: EXT MIC",
            "Input 5: LINE IN",
            "Input 6: EXT MIC",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let x8 = device_channel_labels(&names, "Portacapture X8", 8).unwrap();
        let (stereo, mono) = channel_choices(8, Some(&x8), None);
        assert_eq!(
            row(&stereo[2]),
            ("5,6", "Input 3 and 4", Some("EXT MIC"), Some("USB 5, 6"))
        );
        assert_eq!(
            row(&stereo[3]),
            (
                "7,8",
                "Input 5 and 6",
                Some("5: LINE IN; 6: EXT MIC"),
                Some("USB 7, 8")
            )
        );
        assert_eq!(
            row(&mono[3]),
            ("3", "Input 1", Some("INT MIC"), Some("USB 3"))
        );

        // An input report's sources are used in place of the names' own.
        let mut sources: Vec<Option<String>> = vec![None; 8];
        sources[4] = Some("EXT MIC, 48 V".into());
        sources[5] = Some("EXT MIC, 48 V".into());
        let (stereo, _) = channel_choices(8, Some(&x8), Some(&sources));
        assert_eq!(stereo[2].source.as_deref(), Some("EXT MIC, 48 V"));
    }

    #[test]
    fn any_other_device_shows_numbers_only() {
        // 5 channels: two pairs, and channel 5 alone.
        let (stereo, mono) = channel_choices(5, None, None);
        assert_eq!(stereo.len(), 2);
        assert_eq!(row(&stereo[1]), ("3,4", "Channels 3 and 4", None, None));
        assert_eq!(stereo[1].button_text(), "Channels 3 and 4 (stereo)");
        assert_eq!(mono.len(), 6);
        assert_eq!(
            row(&mono[0]),
            ("avg", "Channels 1 and 2 averaged", None, None)
        );
        assert_eq!(row(&mono[5]), ("5", "Channel 5", None, None));
        assert_eq!(mono[5].button_text(), "Channel 5 (mono)");
    }
}
