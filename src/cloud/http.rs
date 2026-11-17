//! One `fetch` helper for the cloud module.

use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

/// A request to send.
pub struct Request<'a> {
    pub method: &'a str,
    pub url: &'a str,
    pub headers: Vec<(&'a str, String)>,
    pub body: Option<String>,
    /// Send the browser's cookies for the URL's site (the Oversample API's
    /// sign-in session).
    pub with_cookies: bool,
    /// Let the request finish after the page is hidden or closed. Browsers
    /// refuse bodies over 64 KB with this set, so it is only used for small
    /// ones.
    pub keepalive: bool,
}

impl<'a> Request<'a> {
    pub fn new(method: &'a str, url: &'a str) -> Self {
        Self {
            method,
            url,
            headers: Vec::new(),
            body: None,
            with_cookies: false,
            keepalive: false,
        }
    }

    pub fn header(mut self, name: &'a str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }

    pub fn body(mut self, body: String) -> Self {
        self.body = Some(body);
        self
    }
}

/// The status and text of a response.
pub struct Reply {
    pub status: u16,
    pub body: String,
}

impl Reply {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// Largest body sent with `keepalive` (browsers allow 64 KB in all).
const KEEPALIVE_MAX_BYTES: usize = 60_000;

/// Send `req`. `Err` means there was no response at all: offline, the
/// server could not be reached, or the browser blocked the response.
pub async fn send(req: Request<'_>) -> Result<Reply, ()> {
    let window = web_sys::window().ok_or(())?;
    let init = web_sys::RequestInit::new();
    init.set_method(req.method);
    let headers = web_sys::Headers::new().map_err(|_| ())?;
    for (name, value) in &req.headers {
        headers.set(name, value).map_err(|_| ())?;
    }
    init.set_headers(&headers);
    if req.with_cookies {
        init.set_credentials(web_sys::RequestCredentials::Include);
    }
    let small = req
        .body
        .as_ref()
        .is_none_or(|b| b.len() < KEEPALIVE_MAX_BYTES);
    if req.keepalive && small {
        let _ = js_sys::Reflect::set(&init, &"keepalive".into(), &JsValue::TRUE);
    }
    if let Some(body) = &req.body {
        init.set_body(&JsValue::from_str(body));
    }
    let resp: web_sys::Response = JsFuture::from(window.fetch_with_str_and_init(req.url, &init))
        .await
        .map_err(|_| ())?
        .dyn_into()
        .map_err(|_| ())?;
    let body = match resp.text() {
        Ok(p) => JsFuture::from(p)
            .await
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_default(),
        Err(_) => String::new(),
    };
    Ok(Reply {
        status: resp.status(),
        body,
    })
}

/// `encodeURIComponent`.
pub fn encode(s: &str) -> String {
    js_sys::encode_uri_component(s).into()
}
