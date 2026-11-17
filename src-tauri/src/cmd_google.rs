//! Tauri commands for Google sign-in (see `google_auth`). They exist on every
//! platform so that the frontend can ask; on Android and iOS, and in builds
//! with no client id baked in, `google_status` says the feature is
//! unavailable and the other commands fail with "unavailable".
//!
//! Errors reach the frontend as short codes: "unavailable", "cancelled",
//! "timed_out", "denied", "signed_out", "failed".

use serde::Serialize;

/// What the settings group needs to know.
#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleStatus {
    /// A client id is baked in and this is a desktop platform.
    pub available: bool,
    pub signed_in: bool,
    /// The sign-in is kept in the operating system's credential store. False
    /// when there is none: the sign-in then lasts until the app closes.
    pub persistent: bool,
}

/// A Drive access token for the webview.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessToken {
    pub access_token: String,
    /// Seconds until it expires.
    pub expires_in: u64,
}

#[tauri::command]
pub async fn google_status() -> GoogleStatus {
    #[cfg(desktop)]
    {
        imp::status().await
    }
    #[cfg(not(desktop))]
    {
        GoogleStatus::default()
    }
}

/// Open the browser and wait until sign-in finishes, fails, times out or is
/// cancelled.
#[tauri::command]
pub async fn google_sign_in() -> Result<GoogleStatus, String> {
    #[cfg(desktop)]
    {
        imp::sign_in().await
    }
    #[cfg(not(desktop))]
    {
        Err("unavailable".into())
    }
}

/// Stop a running `google_sign_in` (it then fails with "cancelled").
#[tauri::command]
pub fn google_sign_in_cancel() {
    #[cfg(desktop)]
    imp::cancel();
}

/// A valid access token, renewed with the refresh token when needed.
/// `force`: renew even if the cached one has not expired (Drive refused it).
#[tauri::command]
pub async fn google_access_token(force: Option<bool>) -> Result<AccessToken, String> {
    #[cfg(desktop)]
    {
        imp::access_token(force.unwrap_or(false)).await
    }
    #[cfg(not(desktop))]
    {
        let _ = force;
        Err("unavailable".into())
    }
}

/// Revoke the app's access and forget the stored refresh token.
#[tauri::command]
pub async fn google_sign_out() -> Result<(), String> {
    #[cfg(desktop)]
    {
        imp::sign_out().await;
    }
    Ok(())
}

#[cfg(desktop)]
mod imp {
    use super::{AccessToken, GoogleStatus};
    use crate::google_auth::{self, AuthError, Config, KeyringStore, TokenStore};
    use std::sync::{Arc, LazyLock, Mutex};
    use std::time::{Duration, Instant};
    use tokio::sync::Notify;

    /// Renew the access token when less than this is left.
    const RENEW_MARGIN: Duration = Duration::from_secs(60);

    #[derive(Default)]
    struct Session {
        /// The credential store has been read (once per run).
        loaded: bool,
        refresh: Option<String>,
        access: Option<(String, Instant)>,
        persistent: bool,
        cancel: Option<Arc<Notify>>,
    }

    static SESSION: LazyLock<Mutex<Session>> = LazyLock::new(Mutex::default);
    static HTTP: LazyLock<reqwest::Client> = LazyLock::new(xc_lib::http_client);

    fn with<R>(f: impl FnOnce(&mut Session) -> R) -> R {
        f(&mut SESSION.lock().unwrap_or_else(|p| p.into_inner()))
    }

    fn store() -> KeyringStore {
        KeyringStore
    }

    /// Run a credential-store call off the async runtime.
    async fn blocking<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> Option<R> {
        tokio::task::spawn_blocking(f).await.ok()
    }

    /// Read the stored refresh token once per run.
    async fn load_once() {
        if with(|s| s.loaded) {
            return;
        }
        let result = blocking(|| store().load()).await;
        with(|s| {
            if s.loaded {
                return;
            }
            s.loaded = true;
            match result {
                Some(Ok(token)) => {
                    s.persistent = true;
                    if s.refresh.is_none() {
                        s.refresh = token;
                    }
                }
                Some(Err(e)) => {
                    eprintln!("Google sign-in: no credential store ({e}); sign-in lasts until the app closes");
                    s.persistent = false;
                }
                None => s.persistent = false,
            }
        });
    }

    pub async fn status() -> GoogleStatus {
        if Config::from_build().is_none() {
            return GoogleStatus::default();
        }
        load_once().await;
        with(|s| GoogleStatus {
            available: true,
            signed_in: s.refresh.is_some(),
            persistent: s.persistent,
        })
    }

    pub async fn sign_in() -> Result<GoogleStatus, String> {
        let cfg = Config::from_build().ok_or("unavailable")?;
        load_once().await;
        let cancel = Arc::new(Notify::new());
        // A second press replaces a sign-in that is still waiting.
        if let Some(old) = with(|s| s.cancel.replace(cancel.clone())) {
            old.notify_one();
        }
        let result = google_auth::sign_in(
            &cfg,
            &HTTP,
            |url| open::that_detached(url).map_err(|e| e.to_string()),
            cancel.clone(),
            google_auth::SIGN_IN_TIMEOUT,
        )
        .await;
        with(|s| {
            if s.cancel.as_ref().is_some_and(|c| Arc::ptr_eq(c, &cancel)) {
                s.cancel = None;
            }
        });
        let tokens = result.map_err(|e| {
            if let AuthError::Failed(why) = &e {
                eprintln!("Google sign-in failed: {why}");
            }
            e.code().to_string()
        })?;
        let Some(refresh) = tokens.refresh_token.clone() else {
            eprintln!("Google sign-in failed: no refresh token in the reply");
            return Err("failed".into());
        };
        let saved = {
            let token = refresh.clone();
            blocking(move || store().save(&token)).await
        };
        let persistent = match saved {
            Some(Ok(())) => true,
            Some(Err(e)) => {
                eprintln!("Google sign-in: could not store the sign-in ({e}); it lasts until the app closes");
                false
            }
            None => false,
        };
        let expires = tokens.expires_in.unwrap_or(Duration::from_secs(3600));
        Ok(with(|s| {
            s.refresh = Some(refresh);
            s.access = Some((tokens.access_token, Instant::now() + expires));
            s.persistent = persistent;
            GoogleStatus {
                available: true,
                signed_in: true,
                persistent,
            }
        }))
    }

    pub fn cancel() {
        if let Some(c) = with(|s| s.cancel.take()) {
            c.notify_one();
        }
    }

    pub async fn access_token(force: bool) -> Result<AccessToken, String> {
        let cfg = Config::from_build().ok_or("unavailable")?;
        load_once().await;
        let now = Instant::now();
        if force {
            with(|s| s.access = None);
        }
        if let Some((token, until)) = with(|s| s.access.clone()) {
            if until > now + RENEW_MARGIN {
                return Ok(AccessToken {
                    access_token: token,
                    expires_in: (until - now).as_secs(),
                });
            }
        }
        let refresh = with(|s| s.refresh.clone()).ok_or("signed_out")?;
        match google_auth::refresh(&cfg, &HTTP, &refresh).await {
            Ok(tokens) => {
                let expires = tokens.expires_in.unwrap_or(Duration::from_secs(3600));
                if let Some(new_refresh) = tokens.refresh_token.clone() {
                    let token = new_refresh.clone();
                    let _ = blocking(move || store().save(&token)).await;
                    with(|s| s.refresh = Some(new_refresh));
                }
                with(|s| s.access = Some((tokens.access_token.clone(), Instant::now() + expires)));
                Ok(AccessToken {
                    access_token: tokens.access_token,
                    expires_in: expires.as_secs(),
                })
            }
            Err(AuthError::SignedOut) => {
                forget().await;
                Err("signed_out".into())
            }
            Err(e) => {
                if let AuthError::Failed(why) = &e {
                    eprintln!("Google token renewal failed: {why}");
                }
                Err(e.code().into())
            }
        }
    }

    /// Clear the session and the stored refresh token.
    async fn forget() {
        with(|s| {
            s.refresh = None;
            s.access = None;
        });
        if let Some(Err(e)) = blocking(|| store().delete()).await {
            eprintln!("Google sign-out: could not delete the stored sign-in ({e})");
        }
    }

    pub async fn sign_out() {
        cancel();
        let refresh = with(|s| s.refresh.clone());
        forget().await;
        if let (Some(cfg), Some(token)) = (Config::from_build(), refresh) {
            let revoke = google_auth::revoke(&cfg, &HTTP, &token);
            match tokio::time::timeout(Duration::from_secs(10), revoke).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => eprintln!("Google sign-out: revoke failed ({})", e.code()),
                Err(_) => eprintln!("Google sign-out: revoke timed out"),
            }
        }
    }
}
