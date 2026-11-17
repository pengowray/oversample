//! Google sign-in through the Oversample API.
//!
//! The API keeps the Google sign-in (a cookie on its own site) and hands out
//! short-lived Drive access tokens. The app keeps the current token in memory
//! only, never in localStorage, and asks for a new one shortly before it
//! expires or when Drive answers 401.

use super::http::{self, Request};
use std::cell::RefCell;

/// Header the API asks for on its POST routes, so that a plain form on
/// another site cannot call them.
const REQUEST_HEADER: &str = "X-Oversample-Request";

/// Ask for a new token this long before the current one expires.
const RENEW_MARGIN_MS: f64 = 60_000.0;

thread_local! {
    static TOKEN: RefCell<Option<Token>> = const { RefCell::new(None) };
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

/// Whether the server offers Google sign-in, and whether this browser is
/// signed in. `None` means the feature is unavailable: the route is missing,
/// the server has no Google set-up, or it cannot be reached.
pub async fn status() -> Option<bool> {
    #[derive(serde::Deserialize)]
    struct Status {
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
    let status: Status = serde_json::from_str(&reply.body).ok()?;
    status.configured.then_some(status.signed_in)
}

/// Why no access token could be had.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenError {
    /// The server says this browser is not signed in (or Google access was
    /// withdrawn).
    SignedOut,
    /// No answer, or an answer that is not a token.
    Unreachable,
}

/// A Drive access token: the cached one, or a new one from the server.
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
    let lifetime_ms = if parsed.expires_in > 0.0 {
        parsed.expires_in * 1000.0
    } else {
        // No lifetime given: assume Google's usual hour.
        3_600_000.0
    };
    TOKEN.with(|t| {
        *t.borrow_mut() = Some(Token {
            value: parsed.access_token.clone(),
            expires_at_ms: now_ms() + lifetime_ms,
        })
    });
    Ok(parsed.access_token)
}

/// Drop the cached token (after Drive answered 401, or at sign-out).
pub fn forget_token() {
    TOKEN.with(|t| *t.borrow_mut() = None);
}

/// Ask the server to end the sign-in. Returns whether it answered.
pub async fn logout() -> bool {
    forget_token();
    let url = format!("{}/google/logout", base());
    let mut req = Request::new("POST", &url).header(REQUEST_HEADER, "1");
    req.with_cookies = true;
    http::send(req).await.is_ok()
}

/// The page that starts the Google sign-in, for a pop-up window.
pub fn login_url() -> String {
    let origin = web_sys::window()
        .and_then(|w| w.location().origin().ok())
        .unwrap_or_default();
    format!("{}/google/login?origin={}", base(), http::encode(&origin))
}
