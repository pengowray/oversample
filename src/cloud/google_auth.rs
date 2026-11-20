//! Google sign-in, from one of three providers:
//!
//! - Web: the Oversample API keeps the Google sign-in (a cookie on its own
//!   site) and hands out short-lived Drive access tokens.
//! - Desktop: the app's own Rust side signs in through the system browser
//!   (`src-tauri/src/google_auth.rs`) and hands out access tokens through
//!   Tauri commands. Builds with no client id report the feature as
//!   unavailable.
//! - Android: Google Play services signs in (`GoogleAuthPlugin.kt`, the
//!   `google-auth` plugin) and hands out access tokens. Phones without Play
//!   services report the feature as unavailable.
//!
//! Desktop and Android ("native") answer with the same shapes, so the code
//! below treats them alike apart from the command names.
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
    Android,
}

impl Provider {
    /// Signs in inside the app (desktop or Android), not through the API.
    pub fn is_native(self) -> bool {
        self != Provider::Web
    }
}

thread_local! {
    static TOKEN: RefCell<Option<Token>> = const { RefCell::new(None) };
    static PROVIDER: Cell<Provider> = const { Cell::new(Provider::Web) };
    /// Drive refused the last token: the native side must renew, not
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

/// The native commands, by their desktop names.
#[derive(Clone, Copy)]
enum Cmd {
    Status,
    SignIn,
    CancelSignIn,
    AccessToken,
    SignOut,
}

impl Cmd {
    fn name(self, provider: Provider) -> &'static str {
        match (provider, self) {
            (Provider::Android, Cmd::Status) => "plugin:google-auth|status",
            (Provider::Android, Cmd::SignIn) => "plugin:google-auth|signIn",
            (Provider::Android, Cmd::CancelSignIn) => "plugin:google-auth|cancelSignIn",
            (Provider::Android, Cmd::AccessToken) => "plugin:google-auth|accessToken",
            (Provider::Android, Cmd::SignOut) => "plugin:google-auth|signOut",
            (_, Cmd::Status) => "google_status",
            (_, Cmd::SignIn) => "google_sign_in",
            (_, Cmd::CancelSignIn) => "google_sign_in_cancel",
            (_, Cmd::AccessToken) => "google_access_token",
            (_, Cmd::SignOut) => "google_sign_out",
        }
    }
}

/// Call a native command; `Err` holds its error code ("signed_out",
/// "cancelled", ...), or "unavailable" when the command does not exist.
/// Desktop commands fail with the code; the Android plugin answers
/// `{ "error": code }`.
async fn native<T: serde::de::DeserializeOwned>(cmd: Cmd, args: JsValue) -> Result<T, String> {
    let v = crate::tauri_bridge::tauri_invoke_js(cmd.name(provider()), &args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "unavailable".into()))?;
    if let Some(code) = js_sys::Reflect::get(&v, &"error".into())
        .ok()
        .and_then(|e| e.as_string())
    {
        return Err(code);
    }
    serde_wasm_bindgen::from_value(v).map_err(|_| "failed".into())
}

fn no_args() -> JsValue {
    js_sys::Object::new().into()
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeStatus {
    available: bool,
    signed_in: bool,
    persistent: bool,
}

/// Whether Google sign-in is offered, and the sign-in state. `None` means
/// the feature is unavailable: on the web the route is missing, the server
/// has no Google set-up, or it cannot be reached; on the desktop no client
/// id is baked in; on Android there is no Google Play services.
pub async fn status() -> Option<Status> {
    if provider().is_native() {
        let s: NativeStatus = native(Cmd::Status, no_args()).await.ok()?;
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
        Provider::Desktop | Provider::Android => native_token().await?,
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

async fn native_token() -> Result<(String, f64), TokenError> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Reply {
        access_token: String,
        expires_in: f64,
    }
    let args = js_sys::Object::new();
    let force = FORCE_RENEW.with(|c| c.replace(false));
    let _ = js_sys::Reflect::set(&args, &"force".into(), &JsValue::from_bool(force));
    match native::<Reply>(Cmd::AccessToken, args.into()).await {
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
/// desktop and Android the next request also renews the native side's token.
pub fn forget_token() {
    TOKEN.with(|t| *t.borrow_mut() = None);
    FORCE_RENEW.with(|c| c.set(true));
}

/// End the sign-in: on the web the server forgets it; on the desktop the
/// app revokes its access and deletes the stored sign-in (Android: through
/// Play services). Returns whether that was done.
pub async fn logout() -> bool {
    TOKEN.with(|t| *t.borrow_mut() = None);
    if provider().is_native() {
        // Android's reply carries no fields, so it is read as a unit only
        // when it is not an error.
        return native::<serde::de::IgnoredAny>(Cmd::SignOut, no_args())
            .await
            .is_ok();
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

/// How a native sign-in ended without signing in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignInEnd {
    /// The user pressed Cancel in the app.
    Cancelled,
    /// The user declined on Google's page (Android: closed Google's screen).
    Denied,
    /// The browser did not come back within the time limit.
    TimedOut,
    /// Anything else (no network, a refused request).
    Failed,
}

/// Native: open Google's sign-in (desktop: the system browser; Android:
/// Play services' account and consent screen) and wait until it ends.
pub async fn native_sign_in() -> Result<Status, SignInEnd> {
    match native::<NativeStatus>(Cmd::SignIn, no_args()).await {
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

/// Android: tell Play services which Google account signed in (the email
/// Drive reports), so that token renewal and sign-out ask that account on a
/// phone with several. Google's own reply does not name the account when
/// only `drive.file` is asked for. Does nothing elsewhere.
pub fn remember_account(email: &str) {
    if provider() != Provider::Android {
        return;
    }
    let args = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&args, &"email".into(), &JsValue::from_str(email));
    wasm_bindgen_futures::spawn_local(async move {
        let _ = crate::tauri_bridge::tauri_invoke_js("plugin:google-auth|setAccount", &args.into())
            .await;
    });
}

/// Native: stop a sign-in that is still waiting.
pub fn native_cancel_sign_in() {
    let cmd = Cmd::CancelSignIn.name(provider());
    wasm_bindgen_futures::spawn_local(async move {
        let _ = crate::tauri_bridge::tauri_invoke_js(cmd, &no_args()).await;
    });
}
