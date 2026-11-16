//! Bytes sent from the webview with `tauri_bridge::tauri_invoke_raw`.
//!
//! Most webviews send them as the raw request body. Android's cannot (Tauri
//! turns a `Uint8Array` there into a JSON array of numbers), so the frontend
//! sends `{"base64": "..."}` there instead. [`body`] accepts either.

use base64::Engine;
use std::borrow::Cow;
use tauri::ipc::{InvokeBody, Request};

/// The bytes a `tauri_invoke_raw` call sent.
pub fn body<'a>(request: &'a Request<'_>) -> Result<Cow<'a, [u8]>, String> {
    match request.body() {
        InvokeBody::Raw(bytes) => Ok(Cow::Borrowed(bytes)),
        InvokeBody::Json(v) => {
            let text = v
                .get("base64")
                .and_then(|t| t.as_str())
                .ok_or("expected raw bytes or base64 text")?;
            base64::engine::general_purpose::STANDARD
                .decode(text)
                .map(Cow::Owned)
                .map_err(|e| format!("bad base64: {e}"))
        }
    }
}

/// A request header as text.
pub fn header(request: &Request<'_>, name: &str) -> Result<String, String> {
    request
        .headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .ok_or_else(|| format!("missing header {name}"))
}
