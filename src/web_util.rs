//! Small browser helpers used from many places in the frontend.
//!
//! Both helpers wrap the same `setTimeout`-via-`Promise::new` dance that
//! pre-existed in dozens of inline copies across the crate. Centralising
//! them removes the boilerplate, keeps the `web_sys::window().unwrap()` /
//! `set_timeout(...).unwrap()` panic surface in one place, and gives a
//! single seam if we ever want to switch to e.g. `requestAnimationFrame`
//! or `queueMicrotask`.

use wasm_bindgen_futures::JsFuture;

/// Sleep for approximately `ms` milliseconds, yielding to the browser event
/// loop. Resolves on the next tick the JS host picks up the timer; like
/// `setTimeout` it is best-effort, not precise.
///
/// Panics only if `web_sys::window()` is unavailable (e.g. running off the
/// main thread), which never happens in our app.
pub async fn sleep_ms(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        let window = web_sys::window().expect("no window in current context");
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
    });
    let _ = JsFuture::from(promise).await;
}

/// Yield to the browser so it can paint / run other tasks. Same as
/// `sleep_ms(0)` but reads more clearly at call sites that just want a
/// cooperative break.
pub async fn yield_now() {
    sleep_ms(0).await;
}

/// Percent-encode for use as a query-string value. Everything outside the
/// unreserved set is escaped, which is stricter than `encodeURIComponent` but
/// always valid.
pub fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::encode_component;

    #[test]
    fn encodes_utf8_as_bytes() {
        assert_eq!(encode_component("Côte"), "C%C3%B4te");
    }

    #[test]
    fn encodes_quotes_and_apostrophe() {
        assert_eq!(
            encode_component("cnt:\"Côte d'Ivoire\""),
            "cnt%3A%22C%C3%B4te%20d%27Ivoire%22"
        );
    }

    #[test]
    fn encodes_query_delimiters() {
        assert_eq!(encode_component("a&b=c+d"), "a%26b%3Dc%2Bd");
    }

    #[test]
    fn leaves_unreserved_alone() {
        assert_eq!(encode_component("-_.~AZaz09"), "-_.~AZaz09");
    }
}
