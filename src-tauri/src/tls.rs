//! Set-up for HTTPS certificate checks.
//!
//! reqwest checks certificates with `rustls-platform-verifier`, which uses the
//! operating system's trust store. On Android that means calling Android's
//! certificate verifier through JNI, so the verifier needs the JVM and the
//! app's context before the first request. The Kotlin side comes from the
//! `org.rustls:rustls-platform-verifier` Gradle dependency.

/// Give `rustls-platform-verifier` the JVM and application context.
/// Does nothing on other platforms.
#[cfg(target_os = "android")]
pub fn init_platform_verifier() {
    // tao stores the application context in ndk-context during the
    // activity's onCreate, before Tauri's setup hook runs.
    let ctx = ndk_context::android_context();
    // SAFETY: both pointers come from tao's ndk-context set-up: a valid JavaVM
    // and a global reference to the application context that lives for the
    // whole process.
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) };
    let result = vm.attach_current_thread(|env| -> Result<(), jni::errors::Error> {
        let context = unsafe { jni::objects::JObject::from_raw(env, ctx.context().cast()) };
        rustls_platform_verifier::android::init_with_env(env, context)
    });
    if let Err(e) = result {
        eprintln!("TLS: failed to initialise the Android certificate verifier: {e}");
    }
}

#[cfg(not(target_os = "android"))]
pub fn init_platform_verifier() {}
