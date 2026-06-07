// Mouse panning for a horizontally-overflowing toolbar (the hearing / view
// bars in the narrow "mobile" layout). Touch already pans via
// `touch-action: pan-x`; this adds the desktop-mouse equivalents that users
// find more discoverable than Shift+wheel:
//   • spin the wheel over the bar to scroll it sideways (no Shift), and
//   • drag with the left button to pan. A real drag (>6px) is swallowed by a
//     capture-phase click guard so it doesn't also click the button it ends on.
//
// Everything is gated on the bar actually overflowing, so it is a no-op in the
// normal (wide) desktop layout.

use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

const DRAG_THRESHOLD_PX: f64 = 6.0;

#[derive(Clone, Copy)]
pub struct BarPan {
    node: NodeRef<html::Div>,
    down: StoredValue<bool>,
    moved: StoredValue<bool>,
    start_x: StoredValue<f64>,
    start_scroll: StoredValue<f64>,
}

impl BarPan {
    /// Create panning state for `node` and install the capture-phase click
    /// guard. Bind `node` to the scroll container's `node_ref` and wire the
    /// `on_*` methods as its pointer/wheel handlers.
    pub fn new(node: NodeRef<html::Div>) -> Self {
        let me = Self {
            node,
            down: StoredValue::new(false),
            moved: StoredValue::new(false),
            start_x: StoredValue::new(0.0),
            start_scroll: StoredValue::new(0.0),
        };
        me.install_click_guard();
        me
    }

    fn el(&self) -> Option<web_sys::HtmlElement> {
        self.node.get_untracked().map(|d| d.unchecked_into())
    }

    fn overflowing(el: &web_sys::HtmlElement) -> bool {
        el.scroll_width() > el.client_width()
    }

    pub fn on_pointerdown(&self, ev: web_sys::PointerEvent) {
        if ev.button() != 0 {
            return; // left button only
        }
        let Some(el) = self.el() else { return };
        if !Self::overflowing(&el) {
            return;
        }
        self.down.set_value(true);
        self.moved.set_value(false);
        self.start_x.set_value(ev.client_x() as f64);
        self.start_scroll.set_value(el.scroll_left() as f64);
    }

    pub fn on_pointermove(&self, ev: web_sys::PointerEvent) {
        if !self.down.get_value() {
            return;
        }
        let Some(el) = self.el() else { return };
        let dx = ev.client_x() as f64 - self.start_x.get_value();
        if !self.moved.get_value() && dx.abs() > DRAG_THRESHOLD_PX {
            // Promote to a drag: grab the pointer so the pan continues even if
            // the cursor leaves the bar, and show the grabbing cursor.
            self.moved.set_value(true);
            let _ = el.style().set_property("cursor", "grabbing");
            let _ = el.set_pointer_capture(ev.pointer_id());
        }
        if self.moved.get_value() {
            el.set_scroll_left((self.start_scroll.get_value() - dx) as i32);
            ev.prevent_default();
        }
    }

    pub fn on_pointerup(&self, _ev: web_sys::PointerEvent) {
        self.down.set_value(false);
        if let Some(el) = self.el() {
            let _ = el.style().remove_property("cursor");
        }
        // `moved` stays set until the trailing click is swallowed by the guard;
        // a fresh pointerdown resets it if no click follows.
    }

    pub fn on_wheel(&self, ev: web_sys::WheelEvent) {
        let Some(el) = self.el() else { return };
        if !Self::overflowing(&el) {
            return;
        }
        // Use whichever wheel axis dominates so trackpads and mice both pan.
        let delta = if ev.delta_x().abs() > ev.delta_y().abs() {
            ev.delta_x()
        } else {
            ev.delta_y()
        };
        if delta != 0.0 {
            ev.prevent_default();
            el.set_scroll_left((el.scroll_left() as f64 + delta) as i32);
        }
    }

    fn install_click_guard(&self) {
        let node = self.node;
        let moved = self.moved;
        Effect::new(move |_| {
            let Some(el) = node.get() else { return };
            let el: web_sys::HtmlElement = el.unchecked_into();
            let cb = Closure::wrap(Box::new(move |ev: web_sys::Event| {
                if moved.get_value() {
                    moved.set_value(false);
                    // Capture phase: stopping propagation here keeps the click
                    // from ever reaching the button the drag ended on.
                    ev.stop_propagation();
                    ev.prevent_default();
                }
            }) as Box<dyn FnMut(web_sys::Event)>);
            let _ = el.add_event_listener_with_callback_and_bool(
                "click",
                cb.as_ref().unchecked_ref(),
                true, // capture
            );
            // The bar lives for the whole session; leak the closure rather than
            // track it for a teardown that never comes.
            cb.forget();
        });
    }
}
