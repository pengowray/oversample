//! Google sign-in, from one of two providers:
//!
//! - Web: the Oversample API keeps the Google sign-in (a cookie on its own
//!   site) and hands out short-lived Drive access tokens.
//! - Desktop: the app's own Rust side signs in through the system browser
//!   (`src-tauri/src/google_auth.rs`) and hands out access tokens through
//!   Tauri commands. Android and builds with no client id report the
//!   feature as unavailable.
//!
//! The app keeps the current access token in memory only, never in
//! localStorage, and asks for a new one shortly before it expires or when
//! Drive answers 401.

use super::http::{self, Request};
use std::cell::{Cell, RefCell};
use wasm_bindgen::JsValue;

/// Header the API asks for on its POST routes, so that a plain form on
/// another site cannot call them.
const REQUEST_HEADER: &str = "X-Oversample-Request";

/// Ask for a new token this long before the current one expires.
const RENEW_MARGIN_MS: f64 = 60_000.0;

/// Where sign-in and tokens come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Web,
    Desktop,
}

thread_local! {
    static TOKEN: RefCell<Option<Token>> = const { RefCell::new(None) };
    static PROVIDER: Cell<Provider> = const { Cell::new(Provider::Web) };
    /// Drive refused the last token: the desktop side must renew, not
    /// hand back its cached one.
    static FORCE_RENEW: Cell<bool> = const { Cell::new(false) };
}

/// Choose the provider (once, at start).
pub fn set_provider(p: Provider) {
    PROVIDER.with(|c| c.set(p));
}

pub fn provider() -> Provider {
    PROVIDER.with(|c| c.get())
}

struct Token {
    value: String,
    expires_at_ms: f64,
}

fn base() -> String {
    crate::xc_client::proxy_base()
}

fn now_ms() -> f64 {
    js_sys::Date::now()
}

/// The sign-in state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub signed_in: bool,
    /// The sign-in is kept after the app closes. Always true on the web;
    /// false on a desktop with no credential store.
    pub persistent: bool,
}

/// Call a desktop command; `Err` holds its error code ("signed_out",
/// "cancelled", ...), or "unavailable" when the command does not exist.
async fn desktop<T: serde::de::DeserializeOwned>(cmd: &str, args: JsValue) -> Result<T, String> {
    let v = crate::tauri_bridge::tauri_invoke_js(cmd, &args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "unavailable".into()))?;
    serde_wasm_bindgen::from_value(v).map_err(|_| "failed".into())
}

fn no_args() -> JsValue {
    js_sys::Object::new().into()
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DesktopStatus {
    available: bool,
    signed_in: bool,
    persistent: bool,
}

/// Whether Google sign-in is offered, and the sign-in state. `None` means
/// the feature is unavailable: on the web the route is missing, the server
/// has no Google set-up, or it cannot be reached; on the desktop no client
/// id is baked in, or the platform is Android.
pub async fn status() -> Option<Status> {
    if provider() == Provider::Desktop {
        let s: DesktopStatus = desktop("google_status", no_args()).await.ok()?;
        return s.available.then_some(Status {
            signed_in: s.signed_in,
            persistent: s.persistent,
        });
    }
    #[derive(serde::Deserialize)]
    struct WebStatus {
        #[serde(default)]
        configured: bool,
        #[serde(default)]
        signed_in: bool,
    }
    let url = format!("{}/google/status", base());
    let mut req = Request::new("GET", &url);
    req.with_cookies = true;
    let reply = http::send(req).await.ok()?;
    if !reply.ok() {
        return None;
    }
    let status: WebStatus = serde_json::from_str(&reply.body).ok()?;
    status.configured.then_some(Status {
        signed_in: status.signed_in,
        persistent: true,
    })
}

/// Why no access token could be had.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenError {
    /// Not signed in (or Google access was withdrawn).
    SignedOut,
    /// No answer, or an answer that is not a token.
    Unreachable,
}

/// A Drive access token: the cached one, or a new one.
pub async fn access_token() -> Result<String, TokenError> {
    let cached = TOKEN.with(|t| {
        t.borrow()
            .as_ref()
            .filter(|t| t.expires_at_ms - now_ms() > RENEW_MARGIN_MS)
            .map(|t| t.value.clone())
    });
    if let Some(token) = cached {
        return Ok(token);
    }
    let (token, expires_in) = match provider() {
        Provider::Desktop => desktop_token().await?,
        Provider::Web => web_token().await?,
    };
    let lifetime_ms = if expires_in > 0.0 {
        expires_in * 1000.0
    } else {
        // No lifetime given: assume Google's usual hour.
        3_600_000.0
    };
    TOKEN.with(|t| {
        *t.borrow_mut() = Some(Token {
            value: token.clone(),
            expires_at_ms: now_ms() + lifetime_ms,
        })
    });
    Ok(token)
}

async fn desktop_token() -> Result<(String, f64), TokenError> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Reply {
        access_token: String,
        expires_in: f64,
    }
    let args = js_sys::Object::new();
    let force = FORCE_RENEW.with(|c| c.replace(false));
    let _ = js_sys::Reflect::set(&args, &"force".into(), &JsValue::from_bool(force));
    match desktop::<Reply>("google_access_token", args.into()).await {
        Ok(r) => Ok((r.access_token, r.expires_in)),
        Err(code) if code == "signed_out" || code == "unavailable" => Err(TokenError::SignedOut),
        Err(_) => Err(TokenError::Unreachable),
    }
}

async fn web_token() -> Result<(String, f64), TokenError> {
    #[derive(serde::Deserialize)]
    struct Reply {
        access_token: String,
        #[serde(default)]
        expires_in: f64,
    }
    let url = format!("{}/google/token", base());
    let mut req = Request::new("POST", &url).header(REQUEST_HEADER, "1");
    req.with_cookies = true;
    let reply = http::send(req).await.map_err(|_| TokenError::Unreachable)?;
    if reply.status == 401 {
        forget_token();
        return Err(TokenError::SignedOut);
    }
    if !reply.ok() {
        return Err(TokenError::Unreachable);
    }
    let parsed: Reply = serde_json::from_str(&reply.body).map_err(|_| TokenError::Unreachable)?;
    Ok((parsed.access_token, parsed.expires_in))
}

/// Drop the cached token (after Drive answered 401, or at sign-out). On the
/// desktop the next request also renews the token on the Rust side.
pub fn forget_token() {
    TOKEN.with(|t| *t.borrow_mut() = None);
    FORCE_RENEW.with(|c| c.set(true));
}

/// End the sign-in: on the web the server forgets it; on the desktop the
/// app revokes its access and deletes the stored sign-in. Returns whether
/// that was done.
pub async fn logout() -> bool {
    TOKEN.with(|t| *t.borrow_mut() = None);
    if provider() == Provider::Desktop {
        return desktop::<()>("google_sign_out", no_args()).await.is_ok();
    }
    let url = format!("{}/google/logout", base());
    let mut req = Request::new("POST", &url).header(REQUEST_HEADER, "1");
    req.with_cookies = true;
    http::send(req).await.is_ok()
}

/// The page that starts the Google sign-in, for a pop-up window (web).
pub fn login_url() -> String {
    let origin = web_sys::window()
        .and_then(|w| w.location().origin().ok())
        .unwrap_or_default();
    format!("{}/google/login?origin={}", base(), http::encode(&origin))
}

/// How a desktop sign-in ended without signing in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignInEnd {
    /// The user pressed Cancel in the app.
    Cancelled,
    /// The user declined on Google's page.
    Denied,
    /// The browser did not come back within the time limit.
    TimedOut,
    /// Anything else (no network, a refused request).
    Failed,
}

/// Desktop: open the system browser and wait until sign-in ends.
pub async fn desktop_sign_in() -> Result<Status, SignInEnd> {
    match desktop::<DesktopStatus>("google_sign_in", no_args()).await {
        Ok(s) => Ok(Status {
            signed_in: s.signed_in,
            persistent: s.persistent,
        }),
        Err(code) => Err(match code.as_str() {
            "cancelled" => SignInEnd::Cancelled,
            "denied" => SignInEnd::Denied,
            "timed_out" => SignInEnd::TimedOut,
            _ => SignInEnd::Failed,
        }),
    }
}

/// Desktop: stop a sign-in that waits for the browser.
pub fn desktop_cancel_sign_in() {
    wasm_bindgen_futures::spawn_local(async {
        let _ = crate::tauri_bridge::tauri_invoke_js("google_sign_in_cancel", &no_args()).await;
    });
}
