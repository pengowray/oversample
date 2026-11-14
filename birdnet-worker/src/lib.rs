//! BirdNET, BattyBirdNET and Perch 2.0 for Oversample, run in a Web Worker.
//!
//! Trunk builds this crate as a worker (`data-type="worker"` in `index.html`),
//! so tract (about 15 MB of WASM) stays out of the app's own bundle and loads
//! only when the app starts the worker. The app's side is `src/classifier/`;
//! the messages are [`oversample_ipc::classifier::ToWorker`] and
//! [`oversample_ipc::classifier::FromWorker`].
//!
//! - [`resample`]: band-limited resampling to the model rate (rubato), with the
//!   resampler's delay removed so times line up.
//! - [`windows`]: cutting the resampled audio into the method's windows.
//! - [`denoise`]: the learned bat-call denoiser.
//! - [`engine`]: BirdNET's backbone and the regional heads in tract.
//! - [`perch`]: Perch's embedding model in tract and its classifier.
//! - [`session`]: the worker's state and message handling, browser-free so the
//!   native tests run it.

pub mod denoise;
pub mod engine;
pub mod geomodel;
pub mod perch;
pub mod resample;
pub mod session;
pub mod windows;

#[cfg(all(target_arch = "wasm32", target_os = "unknown", feature = "worker"))]
mod wasm {
    use crate::session::{Attached, Session};
    use oversample_ipc::classifier::{attach, FromWorker, ToWorker};
    use std::cell::RefCell;
    use wasm_bindgen::prelude::*;
    use web_sys::{DedicatedWorkerGlobalScope, MessageEvent};

    thread_local! {
        static SESSION: RefCell<Session> = RefCell::new(Session::new());
    }

    fn scope() -> DedicatedWorkerGlobalScope {
        js_sys::global().unchecked_into()
    }

    fn now_ms() -> f64 {
        scope().performance().map_or(0.0, |p| p.now())
    }

    fn post(msg: FromWorker, embedding: Option<&[f32]>) {
        let Ok(inner) = serde_wasm_bindgen::to_value(&msg) else {
            return;
        };
        let obj = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&obj, &attach::MSG.into(), &inner);
        let scope = scope();
        match embedding {
            Some(e) => {
                let arr = js_sys::Float32Array::from(e);
                let _ = js_sys::Reflect::set(&obj, &attach::EMBEDDING.into(), &arr);
                let transfer = js_sys::Array::of1(&arr.buffer());
                let _ = scope.post_message_with_transfer(&obj, &transfer);
            }
            None => {
                let _ = scope.post_message(&obj);
            }
        }
    }

    fn get(obj: &JsValue, name: &str) -> Option<JsValue> {
        js_sys::Reflect::get(obj, &JsValue::from_str(name))
            .ok()
            .filter(|v| !v.is_undefined() && !v.is_null())
    }

    fn on_message(ev: MessageEvent) {
        let data = ev.data();
        let inner = get(&data, attach::MSG).unwrap_or(JsValue::UNDEFINED);
        let msg: ToWorker = match serde_wasm_bindgen::from_value(inner) {
            Ok(m) => m,
            Err(e) => {
                web_sys::console::error_1(&format!("birdnet-worker: bad message: {e}").into());
                return;
            }
        };
        let att = Attached {
            bytes: get(&data, attach::BYTES).map(|b| js_sys::Uint8Array::new(&b).to_vec()),
            samples: get(&data, attach::SAMPLES)
                .map(|s| s.unchecked_into::<js_sys::Float32Array>().to_vec()),
            embedding: get(&data, attach::EMBEDDING)
                .map(|s| s.unchecked_into::<js_sys::Float32Array>().to_vec()),
        };
        SESSION.with(|s| {
            let mut out = post;
            s.borrow_mut().handle(msg, att, &mut out, &now_ms);
        });
    }

    #[wasm_bindgen(start)]
    pub fn start() {
        console_error_panic_hook::set_once();
        let scope = scope();
        let cb = Closure::<dyn Fn(MessageEvent)>::new(on_message);
        scope.set_onmessage(Some(cb.as_ref().unchecked_ref()));
        cb.forget();
        // Messages sent before the WASM has started are dropped, so the app
        // waits for this one.
        post(FromWorker::Ready, None);
    }
}
