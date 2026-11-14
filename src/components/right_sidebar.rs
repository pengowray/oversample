use crate::state::store_fields::*;
use crate::state::{AppState, RightSidebarTab};
use js_sys;
use leptos::prelude::*;
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::components::debug_panel::DebugPanel;
use crate::components::file_sidebar::{
    CallDetectionPanel, ExportPanel, HarmonicsPanel, MetadataPanel, PsdPanel, SelectionPanel,
    SidebarAnalysisPanel,
};

#[component]
pub fn RightSidebar() -> impl IntoView {
    let state = expect_context::<AppState>();

    // Resize drag logic (inverted: handle on left edge, dragging left increases width)
    let on_resize_start = move |ev: web_sys::MouseEvent| {
        ev.prevent_default();
        let start_x = ev.client_x() as f64;
        let start_width = state.panels.right_width().get_untracked();
        let doc = web_sys::window().unwrap().document().unwrap();
        let body = doc.body().unwrap();
        let _ = body.class_list().add_1("sidebar-resizing");

        let on_move =
            Closure::<dyn FnMut(web_sys::MouseEvent)>::new(move |ev: web_sys::MouseEvent| {
                let dx = ev.client_x() as f64 - start_x;
                let new_width = (start_width - dx).clamp(140.0, 500.0);
                state.panels.right_width().set(new_width);
            });
        let on_move_slot: Rc<RefCell<Option<Closure<dyn FnMut(web_sys::MouseEvent)>>>> =
            Rc::new(RefCell::new(Some(on_move)));
        let on_up_slot: Rc<RefCell<Option<Closure<dyn FnMut(web_sys::MouseEvent)>>>> =
            Rc::new(RefCell::new(None));

        let on_move_fn = on_move_slot
            .borrow()
            .as_ref()
            .unwrap()
            .as_ref()
            .unchecked_ref::<js_sys::Function>()
            .clone();
        let _ = doc.add_event_listener_with_callback("mousemove", &on_move_fn);

        let doc_for_up = doc.clone();
        let on_move_slot_for_up = Rc::clone(&on_move_slot);
        let on_up_slot_for_up = Rc::clone(&on_up_slot);
        let on_move_fn_for_up = on_move_fn.clone();
        let on_up =
            Closure::<dyn FnMut(web_sys::MouseEvent)>::new(move |_: web_sys::MouseEvent| {
                if let Some(body) = doc_for_up.body() {
                    let _ = body.class_list().remove_1("sidebar-resizing");
                }
                let _ =
                    doc_for_up.remove_event_listener_with_callback("mousemove", &on_move_fn_for_up);
                if let Some(on_up) = on_up_slot_for_up.borrow().as_ref() {
                    let on_up_fn = on_up.as_ref().unchecked_ref::<js_sys::Function>();
                    let _ = doc_for_up
                        .remove_event_listener_with_callback_and_bool("mouseup", on_up_fn, true);
                }
                on_move_slot_for_up.borrow_mut().take();
                on_up_slot_for_up.borrow_mut().take();
            });

        let on_up_fn = on_up.as_ref().unchecked_ref::<js_sys::Function>().clone();
        *on_up_slot.borrow_mut() = Some(on_up);
        let _ = doc.add_event_listener_with_callback_and_bool("mouseup", &on_up_fn, true);
    };

    let sidebar_class = move || {
        let mut cls = String::from("sidebar right-sidebar");
        if state.panels.right_collapsed().get() {
            cls.push_str(" collapsed");
        }
        if state.status.is_mobile().get() {
            cls.push_str(" mobile-overlay");
        }
        cls
    };

    let dropdown_open = state.panels.right_dropdown_open();

    let on_dropdown_toggle = move |_: web_sys::MouseEvent| {
        if state.panels.right_collapsed().get_untracked() {
            state.panels.right_collapsed().set(false);
        } else {
            dropdown_open.update(|v| *v = !*v);
        }
    };

    // Close the dropdown when focus leaves it. The check runs 150 ms later and
    // closes the menu only if focus is still outside: a press elsewhere (the
    // Info panel's "Correct for ×10" button, say) followed within 150 ms by a
    // press on the dropdown would otherwise close the menu that press opened.
    let on_dropdown_blur = move |ev: web_sys::FocusEvent| {
        let wrap = ev
            .current_target()
            .and_then(|t| t.dyn_into::<web_sys::Node>().ok());
        let handle = wasm_bindgen::closure::Closure::once(move || {
            let focus_inside = wrap.is_some_and(|wrap| {
                web_sys::window()
                    .and_then(|w| w.document())
                    .and_then(|d| d.active_element())
                    .is_some_and(|el| wrap.contains(Some(&el)))
            });
            if !focus_inside {
                dropdown_open.set(false);
            }
        });
        let _ = web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(
                handle.as_ref().unchecked_ref(),
                150,
            );
        handle.forget();
    };

    // Escape closes the menu, as it closes the app's other menus. It stops
    // there, so the window's Escape handler (which stops listening or
    // recording) does not also run.
    let on_dropdown_key = move |ev: web_sys::KeyboardEvent| {
        if ev.key() == "Escape" && dropdown_open.get_untracked() {
            ev.stop_propagation();
            dropdown_open.set(false);
        }
    };

    view! {
        <div class=sidebar_class>
            <div
                class=move || if state.status.is_mobile().get() { "sidebar-resize-handle hidden" } else { "sidebar-resize-handle" }
                on:mousedown=on_resize_start
            ></div>
            <div class="sidebar-tabs">
                <div
                    class="sidebar-tab-dropdown-wrap"
                    tabindex="-1"
                    on:focusout=on_dropdown_blur
                    on:keydown=on_dropdown_key
                >
                    <button class="sidebar-tab-dropdown" on:click=on_dropdown_toggle>
                        {move || state.panels.right_tab().get().label()}
                        <span class="dropdown-arrow">{move || if dropdown_open.get() { "\u{25B4}" } else { "\u{25BE}" }}</span>
                    </button>
                    {move || {
                        if dropdown_open.get() {
                            let items: Vec<_> = RightSidebarTab::ALL.iter().map(|&tab| {
                                let is_active = move || state.panels.right_tab().get() == tab;
                                let label = tab.label();
                                view! {
                                    <button
                                        class=move || if is_active() { "sidebar-tab-option active" } else { "sidebar-tab-option" }
                                        on:mousedown=move |ev: web_sys::MouseEvent| {
                                            ev.prevent_default();
                                            let callback = Closure::once_into_js(move || {
                                                state.panels.right_collapsed().set(false);
                                                state.panels.right_tab().set(tab);
                                                dropdown_open.set(false);
                                            });
                                            let _ = web_sys::window().unwrap().set_timeout_with_callback_and_timeout_and_arguments_0(
                                                callback.unchecked_ref(),
                                                0,
                                            );
                                        }
                                    >
                                        {label}
                                    </button>
                                }
                            }).collect();
                            view! {
                                <div class="sidebar-tab-menu">{items}</div>
                            }.into_any()
                        } else {
                            view! { <span></span> }.into_any()
                        }
                    }}
                </div>
            </div>
            {move || match state.panels.right_tab().get() {
                RightSidebarTab::Selection => view! { <SelectionPanel /> }.into_any(),
                RightSidebarTab::Export => view! { <ExportPanel /> }.into_any(),
                RightSidebarTab::Psd => view! { <PsdPanel /> }.into_any(),
                RightSidebarTab::Analysis => view! { <SidebarAnalysisPanel /> }.into_any(),
                RightSidebarTab::Harmonics => view! { <HarmonicsPanel /> }.into_any(),
                RightSidebarTab::CallDetection => view! { <CallDetectionPanel /> }.into_any(),
                RightSidebarTab::Metadata => view! { <MetadataPanel /> }.into_any(),
                RightSidebarTab::DebugLog => view! { <DebugPanel /> }.into_any(),
            }}
        </div>
    }
}
