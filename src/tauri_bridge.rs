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

/// Subscribe to a Tauri event. The closure is leaked (via `forget`) so the
/// listener lives for the lifetime of the app.  Returns `true` if the
/// listener was registered successfully.
pub fn tauri_listen(event_name: &str, callback: Closure<dyn FnMut(JsValue)>) -> bool {
    let Some(tauri) = get_tauri_internals() else {
        return false;
    };

    let Ok(transform_fn) = js_sys::Reflect::get(&tauri, &JsValue::from_str("transformCallback"))
    else {
        return false;
    };
    let transform_fn = js_sys::Function::from(transform_fn);
    let Ok(handler_id) = transform_fn.call1(&tauri, callback.as_ref().unchecked_ref()) else {
        return false;
    };

    let Ok(invoke_fn) = js_sys::Reflect::get(&tauri, &JsValue::from_str("invoke")) else {
        return false;
    };
    let invoke_fn = js_sys::Function::from(invoke_fn);

    let args = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&args, &"event".into(), &JsValue::from_str(event_name));
    let target = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&target, &"kind".into(), &JsValue::from_str("Any"));
    let _ = js_sys::Reflect::set(&args, &"target".into(), &target);
    let _ = js_sys::Reflect::set(&args, &"handler".into(), &handler_id);

    let _ = invoke_fn.call2(&tauri, &JsValue::from_str("plugin:event|listen"), &args);

    // Leak the closure so it lives forever (event listener for app lifetime)
    callback.forget();

    true
}
