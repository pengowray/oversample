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
