//! A one-shot fix of this device's position, for choosing a region. The
//! position stays on the device: callers turn it into a country offline and
//! drop it.
//!
//! In the Tauri app on Android the native geolocation plugin answers; elsewhere
//! (desktop Tauri, browsers) `navigator.geolocation` does. Coarse accuracy is
//! enough for a country, so a cached fix up to 10 minutes old is accepted.

use crate::tauri_bridge::tauri_invoke_typed_no_args;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

/// Why no position came back. `Unavailable` carries the system's own message
/// (or the plugin's error code) for the UI to show.
#[derive(Clone, Debug, PartialEq)]
pub enum LocationError {
    Denied,
    Unavailable(String),
}

/// This device's position, also kept for this session as the classifier's
/// place for recordings without one (`ClassifierState::device_position`).
pub async fn ask(state: crate::state::AppState) -> Result<(f64, f64), LocationError> {
    use crate::state::ClassifierStateStoreFields;
    use leptos::prelude::*;
    let pos = current(state.is_tauri).await?;
    state
        .classifier
        .device_position()
        .set(Some((pos.0 as f32, pos.1 as f32)));
    Ok(pos)
}

/// This device's latitude and longitude.
pub async fn current(is_tauri: bool) -> Result<(f64, f64), LocationError> {
    if is_tauri {
        if let Ok(r) = tauri_invoke_typed_no_args::<oversample_ipc::plugins::GeolocationResult>(
            "plugin:geolocation|getCurrentLocation",
        )
        .await
        {
            return match (r.error, r.latitude, r.longitude) {
                (Some(e), _, _) if e == "permission_denied" => Err(LocationError::Denied),
                (Some(e), _, _) => Err(LocationError::Unavailable(e.replace('_', " "))),
                (None, Some(lat), Some(lon)) => Ok((lat, lon)),
                _ => Err(LocationError::Unavailable("no position".into())),
            };
        }
        // No native plugin (desktop Tauri): fall through to the webview's API.
    }
    browser().await
}

async fn browser() -> Result<(f64, f64), LocationError> {
    let geo = web_sys::window()
        .and_then(|w| js_sys::Reflect::get(&w.navigator(), &"geolocation".into()).ok())
        .filter(|g| !g.is_undefined() && !g.is_null())
        .ok_or_else(|| LocationError::Unavailable("not supported here".into()))?;
    let get: js_sys::Function = js_sys::Reflect::get(&geo, &"getCurrentPosition".into())
        .ok()
        .and_then(|f| f.dyn_into().ok())
        .ok_or_else(|| LocationError::Unavailable("not supported here".into()))?;

    let promise = js_sys::Promise::new(&mut |resolve, reject| {
        let ok = Closure::once_into_js(move |pos: JsValue| {
            let _ = resolve.call1(&JsValue::NULL, &pos);
        });
        let reject_async = reject.clone();
        let err = Closure::once_into_js(move |e: JsValue| {
            let _ = reject_async.call1(&JsValue::NULL, &e);
        });
        let opts = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&opts, &"maximumAge".into(), &600_000.into());
        let _ = js_sys::Reflect::set(&opts, &"timeout".into(), &20_000.into());
        if let Err(e) = get.call3(&geo, &ok, &err, &opts) {
            let _ = reject.call1(&JsValue::NULL, &e);
        }
    });

    match wasm_bindgen_futures::JsFuture::from(promise).await {
        Ok(pos) => {
            let coords = js_sys::Reflect::get(&pos, &"coords".into()).unwrap_or_default();
            let num = |k: &str| {
                js_sys::Reflect::get(&coords, &k.into())
                    .ok()
                    .and_then(|v| v.as_f64())
            };
            match (num("latitude"), num("longitude")) {
                (Some(lat), Some(lon)) => Ok((lat, lon)),
                _ => Err(LocationError::Unavailable("no position".into())),
            }
        }
        Err(e) => {
            // GeolocationPositionError: code 1 = PERMISSION_DENIED.
            let code = js_sys::Reflect::get(&e, &"code".into())
                .ok()
                .and_then(|v| v.as_f64());
            if code == Some(1.0) {
                return Err(LocationError::Denied);
            }
            let msg = js_sys::Reflect::get(&e, &"message".into())
                .ok()
                .and_then(|v| v.as_string())
                .filter(|m| !m.is_empty())
                .unwrap_or_else(|| "no position".into());
            Err(LocationError::Unavailable(msg))
        }
    }
}
