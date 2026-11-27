use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

/// Get the Tauri internals object, if running in Tauri.
pub fn get_tauri_internals() -> Option<JsValue> {
    let window = web_sys::window()?;
    let tauri = js_sys::Reflect::get(&window, &JsValue::from_str("__TAURI_INTERNALS__")).ok()?;
    if tauri.is_undefined() {
        None
    } else {
        Some(tauri)
    }
}

/// Invoke a Tauri command and return the result as a JsValue.
pub async fn tauri_invoke(cmd: &str, args: &JsValue) -> Result<JsValue, String> {
    tauri_invoke_js(cmd, args)
        .await
        .map_err(|e| format!("Command '{}' failed: {:?}", cmd, e))
}

/// The message a native command failed with, from an error that
/// [`tauri_invoke`] built (`Command 'x' failed: JsValue("message")`), for
/// showing to the user. Any other text is returned as it is.
pub fn command_error_message(e: &str) -> String {
    let Some(start) = e.find("JsValue(\"") else {
        return e.to_string();
    };
    let inner = &e[start + "JsValue(\"".len()..];
    let Some(inner) = inner.strip_suffix("\")") else {
        return e.to_string();
    };
    // `{:?}` of a JS string escapes quotes and backslashes.
    inner.replace("\\\"", "\"").replace("\\\\", "\\")
}

/// Like [`tauri_invoke`], but an error is the value the command rejected
/// with, so a command's serialized error type can be read back with
/// `serde_wasm_bindgen`.
pub async fn tauri_invoke_js(cmd: &str, args: &JsValue) -> Result<JsValue, JsValue> {
    let tauri = get_tauri_internals().ok_or("Not running in Tauri")?;
    let invoke = js_sys::Reflect::get(&tauri, &JsValue::from_str("invoke"))
        .map_err(|_| "No invoke function")?;
    let invoke_fn = js_sys::Function::from(invoke);

    let promise_val = invoke_fn.call2(&tauri, &JsValue::from_str(cmd), args)?;

    let promise: js_sys::Promise = promise_val
        .dyn_into()
        .map_err(|_| "Result is not a Promise")?;

    JsFuture::from(promise).await
}

/// True in Android's webview, which cannot send raw IPC bodies: Tauri turns a
/// `Uint8Array` there into a JSON array of numbers.
pub fn is_android() -> bool {
    web_sys::window()
        .and_then(|w| w.navigator().user_agent().ok())
        .is_some_and(|ua| ua.to_ascii_lowercase().contains("android"))
}

/// The most bytes to send in one [`tauri_invoke_raw`] call. On Android each
/// call carries the bytes as base64 text (about 1.4 MB for this limit).
pub fn raw_part_limit() -> u32 {
    if is_android() {
        1024 * 1024
    } else {
        8 * 1024 * 1024
    }
}

/// Invoke a Tauri command with `body` as its bytes and `headers` as request
/// headers. The command reads them with `crate::ipc_bytes::body` and
/// `crate::ipc_bytes::header` in `src-tauri`. Elsewhere the bytes go as the
/// raw request body; on Android, as base64 text in a `{"base64": ...}` JSON
/// body. Keep each call within [`raw_part_limit`].
pub async fn tauri_invoke_raw(
    cmd: &str,
    body: &js_sys::Uint8Array,
    headers: &[(&str, String)],
) -> Result<JsValue, String> {
    let tauri = get_tauri_internals().ok_or("Not running in Tauri")?;
    let invoke = js_sys::Reflect::get(&tauri, &JsValue::from_str("invoke"))
        .map_err(|_| "No invoke function")?;
    let invoke_fn = js_sys::Function::from(invoke);
    let hdrs = js_sys::Object::new();
    for (k, v) in headers {
        let _ = js_sys::Reflect::set(&hdrs, &JsValue::from_str(k), &JsValue::from_str(v));
    }
    let options = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&options, &JsValue::from_str("headers"), &hdrs);
    let payload: JsValue = if is_android() {
        use base64::Engine;
        let text = base64::engine::general_purpose::STANDARD.encode(body.to_vec());
        let obj = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&obj, &JsValue::from_str("base64"), &text.into());
        obj.into()
    } else {
        body.clone().into()
    };
    let promise_val = invoke_fn
        .call3(&tauri, &JsValue::from_str(cmd), &payload, &options)
        .map_err(|e| format!("Invoke call failed: {:?}", e))?;
    let promise: js_sys::Promise = promise_val
        .dyn_into()
        .map_err(|_| "Result is not a Promise")?;
    JsFuture::from(promise)
        .await
        .map_err(|e| format!("Command '{}' failed: {:?}", cmd, e))
}

/// Invoke a Tauri command with no arguments.
pub async fn tauri_invoke_no_args(cmd: &str) -> Result<JsValue, String> {
    tauri_invoke(cmd, &js_sys::Object::new().into()).await
}

/// Invoke a Tauri command and deserialize its result into a typed value via
/// `serde_wasm_bindgen`. Prefer this with the shared `oversample-ipc` DTOs over
/// parsing the returned `JsValue` field-by-field with `Reflect::get`.
pub async fn tauri_invoke_typed<R: serde::de::DeserializeOwned>(
    cmd: &str,
    args: &JsValue,
) -> Result<R, String> {
    let val = tauri_invoke(cmd, args).await?;
    serde_wasm_bindgen::from_value(val)
        .map_err(|e| format!("Failed to deserialize '{}' result: {:?}", cmd, e))
}

/// Like [`tauri_invoke_typed`] but for a command that takes no arguments.
pub async fn tauri_invoke_typed_no_args<R: serde::de::DeserializeOwned>(
    cmd: &str,
) -> Result<R, String> {
    tauri_invoke_typed(cmd, &js_sys::Object::new().into()).await
}

/// Serialize a command's arguments with `serde_wasm_bindgen`.
fn to_args<A: serde::Serialize>(cmd: &str, args: &A) -> Result<JsValue, String> {
    serde_wasm_bindgen::to_value(args)
        .map_err(|e| format!("Failed to serialize args for '{}': {:?}", cmd, e))
}

/// Invoke a command with serde-serialized `args` and deserialize the result into
/// `R`. Replaces hand-built `Object` + `Reflect::set` arg plumbing.
pub async fn tauri_invoke_typed_args<A: serde::Serialize, R: serde::de::DeserializeOwned>(
    cmd: &str,
    args: &A,
) -> Result<R, String> {
    tauri_invoke_typed(cmd, &to_args(cmd, args)?).await
}

/// Invoke a command with serde-serialized `args`, ignoring the (typed) result —
/// only success/failure matters.
pub async fn tauri_invoke_args<A: serde::Serialize>(cmd: &str, args: &A) -> Result<(), String> {
    tauri_invoke(cmd, &to_args(cmd, args)?).await.map(|_| ())
}

/// Invoke a command with serde-serialized `args`, returning the raw `JsValue`
/// result for callers that parse it themselves.
pub async fn tauri_invoke_with_args<A: serde::Serialize>(
    cmd: &str,
    args: &A,
) -> Result<JsValue, String> {
    tauri_invoke(cmd, &to_args(cmd, args)?).await
}

/// Like [`tauri_invoke_with_args`], but an error is the value the command
/// rejected with (see [`tauri_invoke_js`]).
pub async fn tauri_invoke_js_args<A: serde::Serialize>(
    cmd: &str,
    args: &A,
) -> Result<JsValue, JsValue> {
    tauri_invoke_js(cmd, &to_args(cmd, args)?).await
}

/// Invoke a command that returns raw bytes (`tauri::ipc::Response`), with
/// serde-serialized `args`.
pub async fn tauri_invoke_bytes<A: serde::Serialize>(
    cmd: &str,
    args: &A,
) -> Result<js_sys::ArrayBuffer, String> {
    tauri_invoke_with_args(cmd, args)
        .await?
        .dyn_into()
        .map_err(|_| format!("Expected bytes from '{}'", cmd))
}

#[derive(serde::Serialize)]
struct RangeArgs<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fd: Option<i32>,
    offset: u64,
    length: u64,
}

/// The most bytes to ask the native side for in one read. Android copies each
/// response through Java, and a single 92 MB read ran its heap out of memory.
pub const NATIVE_READ_PART: u64 = 8 * 1024 * 1024;

/// Read a whole native file, in parts of [`NATIVE_READ_PART`] bytes.
pub async fn read_whole_file(path: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let part = read_file_range(path, out.len() as u64, NATIVE_READ_PART).await?;
        let done = (part.len() as u64) < NATIVE_READ_PART;
        out.extend_from_slice(&part);
        if done {
            return Ok(out);
        }
    }
}

/// Read a byte range from a native file via Tauri IPC.
///
/// Returns the raw bytes for the range `[offset, offset + length)`, or fewer at
/// the end of the file.
pub async fn read_file_range(path: &str, offset: u64, length: u64) -> Result<Vec<u8>, String> {
    let args = RangeArgs {
        path: Some(path),
        fd: None,
        offset,
        length,
    };
    let buf = tauri_invoke_bytes("read_file_range", &args).await?;
    Ok(js_sys::Uint8Array::new(&buf).to_vec())
}

/// Read a byte range from a file descriptor the media-store plugin opened, as
/// raw bytes (the `read_fd_range` command). Fewer bytes at the end of the file.
pub async fn read_fd_range(fd: i32, offset: u64, length: u64) -> Result<Vec<u8>, String> {
    let args = RangeArgs {
        path: None,
        fd: Some(fd),
        offset,
        length,
    };
    let buf = tauri_invoke_bytes("read_fd_range", &args).await?;
    Ok(js_sys::Uint8Array::new(&buf).to_vec())
}

/// Register `callback` for `event_name` with Tauri's event plugin. Returns the
/// `plugin:event|listen` promise, which resolves to the event id.
fn listen_raw(event_name: &str, callback: &Closure<dyn FnMut(JsValue)>) -> Option<js_sys::Promise> {
    let tauri = get_tauri_internals()?;
    let transform_fn =
        js_sys::Reflect::get(&tauri, &JsValue::from_str("transformCallback")).ok()?;
    let handler_id = js_sys::Function::from(transform_fn)
        .call1(&tauri, callback.as_ref().unchecked_ref())
        .ok()?;
    #[derive(serde::Serialize)]
    struct Target {
        kind: &'static str,
    }
    #[derive(serde::Serialize)]
    struct ListenArgs<'a> {
        event: &'a str,
        target: Target,
    }
    let args = to_args(
        "plugin:event|listen",
        &ListenArgs {
            event: event_name,
            target: Target { kind: "Any" },
        },
    )
    .ok()?;
    js_sys::Reflect::set(&args, &"handler".into(), &handler_id).ok()?;
    let invoke_fn = js_sys::Reflect::get(&tauri, &JsValue::from_str("invoke")).ok()?;
    js_sys::Function::from(invoke_fn)
        .call2(&tauri, &JsValue::from_str("plugin:event|listen"), &args)
        .ok()?
        .dyn_into()
        .ok()
}

/// Subscribe to a Tauri event for the lifetime of the app. The closure is
/// leaked (via `forget`). Returns `true` if the listener was registered.
/// For a subscription that ends, use [`tauri_listen_scoped`].
pub fn tauri_listen(event_name: &str, callback: Closure<dyn FnMut(JsValue)>) -> bool {
    if listen_raw(event_name, &callback).is_none() {
        return false;
    }
    callback.forget();
    true
}

#[derive(Default)]
struct ListenerState {
    /// The id `plugin:event|listen` resolved to.
    event_id: Option<f64>,
    /// The [`TauriListener`] was dropped before the id arrived.
    dropped: bool,
}

/// A Tauri event subscription from [`tauri_listen_scoped`]. Dropping it
/// unsubscribes and frees the callback.
pub struct TauriListener {
    event: String,
    state: std::rc::Rc<std::cell::RefCell<ListenerState>>,
    _callback: Closure<dyn FnMut(JsValue)>,
}

/// Subscribe to a Tauri event until the returned [`TauriListener`] is dropped.
pub fn tauri_listen_scoped(
    event_name: &str,
    callback: Closure<dyn FnMut(JsValue)>,
) -> Option<TauriListener> {
    let promise = listen_raw(event_name, &callback)?;
    let state = std::rc::Rc::new(std::cell::RefCell::new(ListenerState::default()));
    let (event, st) = (event_name.to_string(), state.clone());
    wasm_bindgen_futures::spawn_local(async move {
        let Some(id) = JsFuture::from(promise).await.ok().and_then(|v| v.as_f64()) else {
            return;
        };
        let dropped = st.borrow().dropped;
        if dropped {
            unlisten(&event, id);
        } else {
            st.borrow_mut().event_id = Some(id);
        }
    });
    Some(TauriListener {
        event: event_name.to_string(),
        state,
        _callback: callback,
    })
}

impl Drop for TauriListener {
    fn drop(&mut self) {
        let mut st = self.state.borrow_mut();
        st.dropped = true;
        if let Some(id) = st.event_id.take() {
            unlisten(&self.event, id);
        }
    }
}

/// Unsubscribe the way `@tauri-apps/api`'s `unlisten` does: forget the
/// webview's callback first, so it is never called again, then tell the
/// native side.
fn unlisten(event: &str, event_id: f64) {
    let internals = web_sys::window()
        .and_then(|w| js_sys::Reflect::get(&w, &"__TAURI_EVENT_PLUGIN_INTERNALS__".into()).ok());
    if let Some(internals) = internals {
        if let Ok(f) = js_sys::Reflect::get(&internals, &"unregisterListener".into())
            .and_then(|f| f.dyn_into::<js_sys::Function>())
        {
            let _ = f.call2(
                &internals,
                &JsValue::from_str(event),
                &JsValue::from_f64(event_id),
            );
        }
    }
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct UnlistenArgs {
        event: String,
        event_id: f64,
    }
    let args = UnlistenArgs {
        event: event.to_string(),
        event_id,
    };
    wasm_bindgen_futures::spawn_local(async move {
        let _ = tauri_invoke_args("plugin:event|unlisten", &args).await;
    });
}

#[cfg(test)]
mod tests {
    use super::command_error_message;

    #[test]
    fn command_error_message_is_the_native_text() {
        assert_eq!(
            command_error_message(
                r#"Command 'save_open_recording' failed: JsValue("Couldn't save a.wav: /x/\"y\": Permission denied")"#
            ),
            r#"Couldn't save a.wav: /x/"y": Permission denied"#
        );
        assert_eq!(
            command_error_message("Not running in Tauri"),
            "Not running in Tauri"
        );
    }
}
