//! Google sign-in for the desktop app (Windows, macOS, Linux), for the Drive
//! copies of annotations and projects. Android and iOS do not compile this.
//!
//! Google's flow for desktop apps
//! (developers.google.com/identity/protocols/oauth2/native-app): open the
//! system browser on Google's sign-in page, with an authorization code
//! request (PKCE S256 and a `state` value) whose redirect address is a
//! listener on a free port of 127.0.0.1. Google sends the browser back to
//! that address with the code, and the app exchanges the code for an access
//! token and a refresh token. The scope is `drive.file`: the app sees only
//! the Drive files it created.
//!
//! The client id (and the client secret, which Google issues to desktop
//! clients but does not treat as secret) are baked in at build time from
//! `OVERSAMPLE_GOOGLE_DESKTOP_CLIENT_ID` and
//! `OVERSAMPLE_GOOGLE_DESKTOP_CLIENT_SECRET`. With no client id the feature
//! is unavailable and the settings group is not shown.
//!
//! The refresh token is kept in the operating system's credential store
//! (`TokenStore`, `KeyringStore`): Windows Credential Manager, the macOS
//! Keychain, or the Secret Service (GNOME Keyring, KWallet) on Linux. When
//! there is none, it is kept in memory until the app closes. It is never
//! written to a file, the log, or the webview.

use oauth2::basic::{BasicClient, BasicErrorResponseType};
use oauth2::{
    AuthType, AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointNotSet,
    EndpointSet, HttpRequest, HttpResponse, PkceCodeChallenge, RedirectUrl, RefreshToken,
    RequestTokenError, Scope, TokenResponse, TokenUrl,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;

/// The Drive scope: files this app created or opened.
pub const DRIVE_FILE_SCOPE: &str = "https://www.googleapis.com/auth/drive.file";

const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";

/// How long the app waits for the browser to come back.
pub const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// The only path the loopback listener answers.
const CALLBACK_PATH: &str = "/";

/// The client and the endpoints.
#[derive(Clone, Debug)]
pub struct Config {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub auth_url: String,
    pub token_url: String,
    pub revoke_url: String,
}

impl Config {
    /// The client baked in at build time, or `None` (feature unavailable).
    /// Debug builds read `OVERSAMPLE_GOOGLE_AUTH_URL`, `..._TOKEN_URL` and
    /// `..._REVOKE_URL` at run time, to point the flow at a local mock.
    pub fn from_build() -> Option<Self> {
        let nonempty = |v: Option<&'static str>| {
            v.map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let client_id = nonempty(option_env!("OVERSAMPLE_GOOGLE_DESKTOP_CLIENT_ID"))?;
        let client_secret = nonempty(option_env!("OVERSAMPLE_GOOGLE_DESKTOP_CLIENT_SECRET"));
        let url = |name: &str, default: &str| {
            if cfg!(debug_assertions) {
                if let Ok(v) = std::env::var(name) {
                    if !v.trim().is_empty() {
                        return v;
                    }
                }
            }
            default.to_string()
        };
        Some(Self {
            client_id,
            client_secret,
            auth_url: url("OVERSAMPLE_GOOGLE_AUTH_URL", GOOGLE_AUTH_URL),
            token_url: url("OVERSAMPLE_GOOGLE_TOKEN_URL", GOOGLE_TOKEN_URL),
            revoke_url: url("OVERSAMPLE_GOOGLE_REVOKE_URL", GOOGLE_REVOKE_URL),
        })
    }
}

/// Why sign-in or a token request failed. `code()` is what the frontend
/// receives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthError {
    /// The user pressed Cancel in the app.
    Cancelled,
    /// The browser did not come back in time.
    TimedOut,
    /// The user declined on Google's page.
    Denied,
    /// The refresh token is no longer valid (revoked or expired).
    SignedOut,
    /// Anything else: no network, a refused request, a bad answer. The text
    /// is for the log and never holds a token.
    Failed(String),
}

impl AuthError {
    pub fn code(&self) -> &'static str {
        match self {
            AuthError::Cancelled => "cancelled",
            AuthError::TimedOut => "timed_out",
            AuthError::Denied => "denied",
            AuthError::SignedOut => "signed_out",
            AuthError::Failed(_) => "failed",
        }
    }
}

/// Tokens from Google.
#[derive(Clone, Debug)]
pub struct Tokens {
    pub access_token: String,
    pub expires_in: Option<Duration>,
    /// Sent at sign-in; on a refresh Google usually sends none.
    pub refresh_token: Option<String>,
}

type GoogleClient =
    BasicClient<EndpointSet, EndpointNotSet, EndpointNotSet, EndpointNotSet, EndpointSet>;

fn client(cfg: &Config, redirect: Option<&str>) -> Result<GoogleClient, AuthError> {
    let bad = |what: &str| AuthError::Failed(format!("bad {what} URL"));
    let mut c = BasicClient::new(ClientId::new(cfg.client_id.clone()))
        .set_auth_uri(AuthUrl::new(cfg.auth_url.clone()).map_err(|_| bad("authorization"))?)
        .set_token_uri(TokenUrl::new(cfg.token_url.clone()).map_err(|_| bad("token"))?)
        // Google's examples send the client id (and secret) in the body.
        .set_auth_type(AuthType::RequestBody);
    if let Some(secret) = &cfg.client_secret {
        c = c.set_client_secret(ClientSecret::new(secret.clone()));
    }
    if let Some(r) = redirect {
        c = c.set_redirect_uri(RedirectUrl::new(r.to_string()).map_err(|_| bad("redirect"))?);
    }
    Ok(c)
}

/// The error type of `http_call`.
#[derive(Debug)]
pub struct HttpError(String);

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for HttpError {}

/// The app's reqwest client as oauth2's HTTP client. (A closure borrowing
/// the client does not satisfy the `Send` bounds of async Tauri commands.)
struct Http(reqwest::Client);

impl<'c> oauth2::AsyncHttpClient<'c> for Http {
    type Error = HttpError;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<HttpResponse, HttpError>> + Send + 'c>,
    >;

    fn call(&'c self, request: HttpRequest) -> Self::Future {
        Box::pin(http_call(self.0.clone(), request))
    }
}

/// Send an oauth2 request with the app's reqwest client.
async fn http_call(http: reqwest::Client, request: HttpRequest) -> Result<HttpResponse, HttpError> {
    let request: reqwest::Request = request
        .try_into()
        .map_err(|e: reqwest::Error| HttpError(e.to_string()))?;
    let response = http
        .execute(request)
        .await
        .map_err(|e| HttpError(e.to_string()))?;
    let mut builder = oauth2::http::Response::builder().status(response.status());
    for (name, value) in response.headers() {
        builder = builder.header(name, value);
    }
    let body = response
        .bytes()
        .await
        .map_err(|e| HttpError(e.to_string()))?;
    builder
        .body(body.to_vec())
        .map_err(|e| HttpError(e.to_string()))
}

/// What a token request error means for the app.
fn token_error<RE: std::error::Error + 'static>(
    e: RequestTokenError<RE, oauth2::StandardErrorResponse<BasicErrorResponseType>>,
) -> AuthError {
    match e {
        RequestTokenError::ServerResponse(r)
            if *r.error() == BasicErrorResponseType::InvalidGrant =>
        {
            AuthError::SignedOut
        }
        RequestTokenError::ServerResponse(r) => AuthError::Failed(format!("Google: {}", r.error())),
        RequestTokenError::Request(e) => AuthError::Failed(format!("request: {e}")),
        // The body is not logged: it could hold a token.
        RequestTokenError::Parse(e, _) => AuthError::Failed(format!("reply not understood: {e}")),
        RequestTokenError::Other(e) => AuthError::Failed(e),
    }
}

fn tokens_from(r: &oauth2::basic::BasicTokenResponse) -> Tokens {
    Tokens {
        access_token: r.access_token().secret().clone(),
        expires_in: r.expires_in(),
        refresh_token: r.refresh_token().map(|t| t.secret().clone()),
    }
}

/// Run the browser sign-in. `open_browser` opens the given URL; the
/// function returns when Google's redirect has been handled and the code
/// exchanged, or on `cancel`, or after `timeout`.
pub async fn sign_in(
    cfg: &Config,
    http: &reqwest::Client,
    open_browser: impl FnOnce(&str) -> Result<(), String>,
    cancel: Arc<Notify>,
    timeout: Duration,
) -> Result<Tokens, AuthError> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|e| AuthError::Failed(format!("loopback listener: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| AuthError::Failed(format!("loopback port: {e}")))?
        .port();
    let redirect = format!("http://127.0.0.1:{port}");
    let oauth = client(cfg, Some(&redirect))?;
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let (url, state) = oauth
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new(DRIVE_FILE_SCOPE.to_string()))
        .set_pkce_challenge(challenge)
        .add_extra_param("access_type", "offline")
        .add_extra_param("prompt", "consent")
        .url();
    open_browser(url.as_str()).map_err(|e| AuthError::Failed(format!("open browser: {e}")))?;

    let (mut stream, reply) = tokio::select! {
        r = wait_for_redirect(&listener) => r?,
        _ = cancel.notified() => return Err(AuthError::Cancelled),
        _ = tokio::time::sleep(timeout) => return Err(AuthError::TimedOut),
    };
    // One request only: the listener closes here.
    drop(listener);

    let result = match reply {
        Redirect::Error(e) if e == "access_denied" => Err(AuthError::Denied),
        Redirect::Error(e) => Err(AuthError::Failed(format!("Google: {e}"))),
        Redirect::Code { state: s, .. } if s != *state.secret() => {
            Err(AuthError::Failed("state does not match".into()))
        }
        Redirect::Code { code, .. } => oauth
            .exchange_code(AuthorizationCode::new(code))
            .set_pkce_verifier(verifier)
            .request_async(&Http(http.clone()))
            .await
            .map(|r| tokens_from(&r))
            .map_err(token_error),
    };
    let page = match &result {
        Ok(_) => PAGE_SIGNED_IN,
        Err(AuthError::Denied) => PAGE_DENIED,
        Err(_) => PAGE_FAILED,
    };
    let _ = write_page(&mut stream, "200 OK", page).await;
    result
}

/// Ask for a new access token with the refresh token.
pub async fn refresh(
    cfg: &Config,
    http: &reqwest::Client,
    refresh_token: &str,
) -> Result<Tokens, AuthError> {
    let oauth = client(cfg, None)?;
    let rt = RefreshToken::new(refresh_token.to_string());
    oauth
        .exchange_refresh_token(&rt)
        .request_async(&Http(http.clone()))
        .await
        .map(|r| tokens_from(&r))
        .map_err(token_error)
}

/// Withdraw the app's access (the refresh token and its access tokens):
/// a form POST of `token` to Google's revocation endpoint. Sent directly:
/// oauth2's revocation request refuses a plain-HTTP endpoint, which the
/// local mock in the tests is.
pub async fn revoke(
    cfg: &Config,
    http: &reqwest::Client,
    refresh_token: &str,
) -> Result<(), AuthError> {
    let body = oauth2::url::form_urlencoded::Serializer::new(String::new())
        .append_pair("token", refresh_token)
        .finish();
    let reply = http
        .post(&cfg.revoke_url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|e| AuthError::Failed(format!("revoke: {e}")))?;
    match reply.status().as_u16() {
        200 => Ok(()),
        // Already revoked or expired: nothing left to withdraw.
        400 => Ok(()),
        s => Err(AuthError::Failed(format!("revoke: HTTP {s}"))),
    }
}

// ── Loopback listener ───────────────────────────────────────────────────

/// What Google sent back to the redirect address.
#[derive(Debug, PartialEq, Eq)]
enum Redirect {
    Code { code: String, state: String },
    Error(String),
}

macro_rules! page {
    ($text:literal) => {
        concat!(
            "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">",
            "<meta name=\"viewport\" content=\"width=device-width\">",
            "<title>Oversample</title></head>",
            "<body style=\"font-family: system-ui, sans-serif; margin: 3em; color: #222\">",
            "<p>",
            $text,
            "</p></body></html>"
        )
    };
}

/// Pages the browser shows after the redirect. Fixed text: nothing from the
/// request is put in them.
const PAGE_SIGNED_IN: &str = page!("Signed in to Google. You can close this window.");
const PAGE_DENIED: &str = page!("Google sign-in was cancelled. You can close this window.");
const PAGE_FAILED: &str =
    page!("Google sign-in did not finish. You can close this window and try again in Oversample.");
const PAGE_NOT_FOUND: &str = page!("Not found.");

async fn write_page(stream: &mut TcpStream, status: &str, page: &str) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n",
        page.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(page.as_bytes()).await?;
    stream.shutdown().await
}

/// Read one request head (up to 16 KB, 10 s).
async fn read_head(stream: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    let read = async {
        loop {
            let n = stream.read(&mut chunk).await.ok()?;
            if n == 0 {
                return None;
            }
            buf.extend_from_slice(&chunk[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                return Some(());
            }
            if buf.len() > 16 * 1024 {
                return None;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(10), read)
        .await
        .ok()??;
    String::from_utf8(buf).ok()
}

/// The parameters of a GET request to `CALLBACK_PATH`, or `None` for any
/// other request.
fn parse_callback(head: &str) -> Option<Redirect> {
    let line = head.lines().next()?;
    let mut parts = line.split(' ');
    let (method, target) = (parts.next()?, parts.next()?);
    if method != "GET" {
        return None;
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != CALLBACK_PATH {
        return None;
    }
    let mut code = None;
    let mut state = None;
    let mut error = None;
    for (k, v) in oauth2::url::form_urlencoded::parse(query.as_bytes()) {
        match &*k {
            "code" => code = Some(v.into_owned()),
            "state" => state = Some(v.into_owned()),
            "error" => error = Some(v.into_owned()),
            _ => {}
        }
    }
    if let Some(e) = error {
        return Some(Redirect::Error(e));
    }
    Some(Redirect::Code {
        code: code?,
        state: state.unwrap_or_default(),
    })
}

/// Accept connections until one is Google's redirect; answer anything else
/// (a favicon request, another path) with "Not found".
async fn wait_for_redirect(listener: &TcpListener) -> Result<(TcpStream, Redirect), AuthError> {
    loop {
        let (mut stream, _) = listener
            .accept()
            .await
            .map_err(|e| AuthError::Failed(format!("loopback accept: {e}")))?;
        match read_head(&mut stream)
            .await
            .as_deref()
            .and_then(parse_callback)
        {
            Some(r) => return Ok((stream, r)),
            None => {
                let _ = write_page(&mut stream, "404 Not Found", PAGE_NOT_FOUND).await;
            }
        }
    }
}

// ── Refresh-token storage ───────────────────────────────────────────────

/// Where the refresh token is kept between runs.
pub trait TokenStore: Send + Sync {
    fn load(&self) -> Result<Option<String>, String>;
    fn save(&self, token: &str) -> Result<(), String>;
    fn delete(&self) -> Result<(), String>;
}

/// The operating system's credential store.
pub struct KeyringStore;

const KEYRING_SERVICE: &str = "com.oversample.app";
const KEYRING_USER: &str = "google-drive-refresh-token";

impl KeyringStore {
    fn entry() -> Result<keyring::Entry, String> {
        keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|e| e.to_string())
    }
}

impl TokenStore for KeyringStore {
    fn load(&self) -> Result<Option<String>, String> {
        match Self::entry()?.get_password() {
            Ok(t) => Ok(Some(t)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
    fn save(&self, token: &str) -> Result<(), String> {
        Self::entry()?
            .set_password(token)
            .map_err(|e| e.to_string())
    }
    fn delete(&self) -> Result<(), String> {
        match Self::entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use sha2::Digest;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[test]
    fn only_get_on_the_callback_path_is_accepted() {
        let head = |line: &str| format!("{line}\r\nHost: x\r\n\r\n");
        assert_eq!(
            parse_callback(&head("GET /?code=abc&state=s1 HTTP/1.1")),
            Some(Redirect::Code {
                code: "abc".into(),
                state: "s1".into()
            })
        );
        assert_eq!(
            parse_callback(&head("GET /?error=access_denied&state=s1 HTTP/1.1")),
            Some(Redirect::Error("access_denied".into()))
        );
        assert_eq!(parse_callback(&head("GET /favicon.ico HTTP/1.1")), None);
        assert_eq!(parse_callback(&head("POST /?code=abc HTTP/1.1")), None);
        assert_eq!(parse_callback(&head("GET /other?code=abc HTTP/1.1")), None);
        assert_eq!(parse_callback(&head("GET / HTTP/1.1")), None);
    }

    #[test]
    fn pages_hold_no_request_data() {
        for p in [PAGE_SIGNED_IN, PAGE_DENIED, PAGE_FAILED, PAGE_NOT_FOUND] {
            assert!(!p.contains("code=") && !p.contains('{'));
        }
        assert!(PAGE_SIGNED_IN.contains("Signed in to Google. You can close this window."));
    }

    // ── A mock Google: token and revoke endpoints ──────────────────────

    #[derive(Default)]
    struct Mock {
        /// The PKCE challenge the "browser" saw in the authorization URL.
        challenge: Mutex<Option<String>>,
        /// Form bodies received, by path.
        requests: Mutex<Vec<(String, HashMap<String, String>)>>,
    }

    fn form(body: &str) -> HashMap<String, String> {
        oauth2::url::form_urlencoded::parse(body.as_bytes())
            .into_owned()
            .collect()
    }

    async fn serve_mock(mock: Arc<Mock>) -> String {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            loop {
                let Ok((mut s, _)) = listener.accept().await else {
                    return;
                };
                let mock = mock.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    let (head, body) = loop {
                        let n = s.read(&mut chunk).await.unwrap();
                        buf.extend_from_slice(&chunk[..n]);
                        let text = String::from_utf8_lossy(&buf).to_string();
                        if let Some(i) = text.find("\r\n\r\n") {
                            let head = text[..i].to_string();
                            let len = head
                                .lines()
                                .find_map(|l| {
                                    let (k, v) = l.split_once(':')?;
                                    k.eq_ignore_ascii_case("content-length")
                                        .then(|| v.trim().parse::<usize>().ok())?
                                })
                                .unwrap_or(0);
                            if buf.len() >= i + 4 + len {
                                break (
                                    head,
                                    String::from_utf8_lossy(&buf[i + 4..i + 4 + len]).to_string(),
                                );
                            }
                        }
                        if n == 0 {
                            return;
                        }
                    };
                    let path = head.split(' ').nth(1).unwrap_or("").to_string();
                    let f = form(&body);
                    mock.requests
                        .lock()
                        .unwrap()
                        .push((path.clone(), f.clone()));
                    let (status, json) = match (
                        path.as_str(),
                        f.get("grant_type").map(String::as_str),
                    ) {
                        ("/token", Some("authorization_code")) => {
                            let verifier = f.get("code_verifier").cloned().unwrap_or_default();
                            let expect = mock.challenge.lock().unwrap().clone().unwrap_or_default();
                            let got = base64::engine::general_purpose::URL_SAFE_NO_PAD
                                .encode(sha2::Sha256::digest(verifier.as_bytes()));
                            if got == expect
                                && f.get("code").map(String::as_str) == Some("good-code")
                            {
                                (
                                    "200 OK",
                                    r#"{"access_token":"at-1","token_type":"Bearer","expires_in":3599,"refresh_token":"rt-1","scope":"https://www.googleapis.com/auth/drive.file"}"#,
                                )
                            } else {
                                ("400 Bad Request", r#"{"error":"invalid_grant"}"#)
                            }
                        }
                        ("/token", Some("refresh_token")) => {
                            match f.get("refresh_token").map(String::as_str) {
                                Some("rt-1") => (
                                    "200 OK",
                                    r#"{"access_token":"at-2","token_type":"Bearer","expires_in":3599}"#,
                                ),
                                _ => (
                                    "400 Bad Request",
                                    r#"{"error":"invalid_grant","error_description":"Token has been expired or revoked."}"#,
                                ),
                            }
                        }
                        ("/revoke", _) => ("200 OK", "{}"),
                        _ => ("404 Not Found", "{}"),
                    };
                    let reply = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}",
                        json.len()
                    );
                    let _ = s.write_all(reply.as_bytes()).await;
                });
            }
        });
        base
    }

    fn config(base: &str) -> Config {
        Config {
            client_id: "desktop-client".into(),
            client_secret: Some("desktop-secret".into()),
            auth_url: format!("{base}/auth"),
            token_url: format!("{base}/token"),
            revoke_url: format!("{base}/revoke"),
        }
    }

    /// A "browser": checks the authorization URL, then calls the redirect
    /// address (first a stray favicon request), with `make_query` building
    /// the query from the request's `state`. Returns the page it was shown.
    fn browser(
        mock: Arc<Mock>,
        make_query: impl Fn(&str) -> String + Send + 'static,
        page: Arc<Mutex<Option<String>>>,
    ) -> impl FnOnce(&str) -> Result<(), String> {
        move |url: &str| {
            let url = oauth2::url::Url::parse(url).map_err(|e| e.to_string())?;
            let q: HashMap<String, String> = url.query_pairs().into_owned().collect();
            assert_eq!(q["response_type"], "code");
            assert_eq!(q["client_id"], "desktop-client");
            assert_eq!(q["scope"], DRIVE_FILE_SCOPE);
            assert_eq!(q["code_challenge_method"], "S256");
            assert_eq!(q["access_type"], "offline");
            assert_eq!(q["prompt"], "consent");
            assert!(q["redirect_uri"].starts_with("http://127.0.0.1:"));
            *mock.challenge.lock().unwrap() = Some(q["code_challenge"].clone());
            let redirect = q["redirect_uri"].clone();
            let query = make_query(&q["state"]);
            tokio::spawn(async move {
                let http = reqwest::Client::new();
                let stray = http
                    .get(format!("{redirect}/favicon.ico"))
                    .send()
                    .await
                    .unwrap();
                assert_eq!(stray.status(), 404);
                let r = http
                    .get(format!("{redirect}/?{query}"))
                    .send()
                    .await
                    .unwrap();
                *page.lock().unwrap() = Some(r.text().await.unwrap());
            });
            Ok(())
        }
    }

    fn http() -> reqwest::Client {
        xc_lib::http_client()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn full_loopback_flow_then_refresh_and_revoke() {
        let mock = Arc::new(Mock::default());
        let base = serve_mock(mock.clone()).await;
        let cfg = config(&base);
        let page = Arc::new(Mutex::new(None));
        let tokens = sign_in(
            &cfg,
            &http(),
            browser(
                mock.clone(),
                |state| format!("code=good-code&state={state}"),
                page.clone(),
            ),
            Arc::new(Notify::new()),
            Duration::from_secs(10),
        )
        .await
        .expect("sign-in works");
        assert_eq!(tokens.access_token, "at-1");
        assert_eq!(tokens.refresh_token.as_deref(), Some("rt-1"));
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(page
            .lock()
            .unwrap()
            .as_deref()
            .unwrap()
            .contains("Signed in to Google"));
        // The exchange sent the client secret and the redirect address.
        let exchange = mock.requests.lock().unwrap()[0].1.clone();
        assert_eq!(exchange["client_secret"], "desktop-secret");
        assert!(exchange["redirect_uri"].starts_with("http://127.0.0.1:"));

        let refreshed = refresh(&cfg, &http(), "rt-1").await.unwrap();
        assert_eq!(refreshed.access_token, "at-2");
        assert_eq!(
            refresh(&cfg, &http(), "rt-revoked").await.unwrap_err(),
            AuthError::SignedOut
        );
        revoke(&cfg, &http(), "rt-1").await.unwrap();
        let reqs = mock.requests.lock().unwrap();
        let (path, f) = reqs.last().unwrap();
        assert_eq!(path, "/revoke");
        assert_eq!(f["token"], "rt-1");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_wrong_state_is_refused_and_no_code_is_exchanged() {
        let mock = Arc::new(Mock::default());
        let base = serve_mock(mock.clone()).await;
        let page = Arc::new(Mutex::new(None));
        let err = sign_in(
            &config(&base),
            &http(),
            browser(
                mock.clone(),
                |_| "code=good-code&state=forged".into(),
                page.clone(),
            ),
            Arc::new(Notify::new()),
            Duration::from_secs(10),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AuthError::Failed(_)));
        assert!(mock.requests.lock().unwrap().is_empty());
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(page
            .lock()
            .unwrap()
            .as_deref()
            .unwrap()
            .contains("did not finish"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn declining_on_googles_page_is_denied() {
        let mock = Arc::new(Mock::default());
        let base = serve_mock(mock.clone()).await;
        let page = Arc::new(Mutex::new(None));
        let err = sign_in(
            &config(&base),
            &http(),
            browser(
                mock.clone(),
                |s| format!("error=access_denied&state={s}"),
                page.clone(),
            ),
            Arc::new(Notify::new()),
            Duration::from_secs(10),
        )
        .await
        .unwrap_err();
        assert_eq!(err, AuthError::Denied);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(page
            .lock()
            .unwrap()
            .as_deref()
            .unwrap()
            .contains("was cancelled"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cancel_and_timeout_stop_the_wait() {
        let base = "http://127.0.0.1:9";
        let cancel = Arc::new(Notify::new());
        let c2 = cancel.clone();
        let err = sign_in(
            &config(base),
            &http(),
            move |_| {
                c2.notify_one();
                Ok(())
            },
            cancel,
            Duration::from_secs(10),
        )
        .await
        .unwrap_err();
        assert_eq!(err, AuthError::Cancelled);
        let err = sign_in(
            &config(base),
            &http(),
            |_| Ok(()),
            Arc::new(Notify::new()),
            Duration::from_millis(50),
        )
        .await
        .unwrap_err();
        assert_eq!(err, AuthError::TimedOut);
    }
}
