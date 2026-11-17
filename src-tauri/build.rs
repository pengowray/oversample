// The inlined plugin command registry lives in `src/plugin_commands.rs` so it is
// a single source of truth shared with the lib's cross-check tests (which verify
// it against capabilities/default.json and the Kotlin @Command names). `include!`
// pulls in the `PLUGIN_COMMANDS` const; its `#[cfg(test)]` test module is stripped
// here (build scripts compile with cfg(test) = false).
include!("src/plugin_commands.rs");

/// The Google desktop client (see `src/google_auth.rs`), read at compile
/// time with `option_env!`.
const GOOGLE_CLIENT_VARS: [&str; 2] = [
    "OVERSAMPLE_GOOGLE_DESKTOP_CLIENT_ID",
    "OVERSAMPLE_GOOGLE_DESKTOP_CLIENT_SECRET",
];

/// Pass the Google desktop client to the compiler from the repo's
/// git-ignored `.env`, so that a plain `cargo tauri dev` or `cargo tauri
/// build` has it. A variable set in the real environment (CI) wins. Only
/// these two names are read, and their values are never printed.
fn google_client_from_dot_env() {
    let dot_env = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default())
        .join("..")
        .join(".env");
    println!("cargo:rerun-if-changed={}", dot_env.display());
    for name in GOOGLE_CLIENT_VARS {
        println!("cargo:rerun-if-env-changed={name}");
    }
    let Ok(text) = std::fs::read_to_string(&dot_env) else {
        return;
    };
    for line in text.lines() {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim();
        if !GOOGLE_CLIENT_VARS.contains(&name) || std::env::var_os(name).is_some() {
            continue;
        }
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
            .unwrap_or(value);
        if !value.is_empty() {
            println!("cargo:rustc-env={name}={value}");
        }
    }
}

fn main() {
    google_client_from_dot_env();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        println!("cargo:rustc-link-lib=c++_shared");
    }

    let mut attributes = tauri_build::Attributes::new();
    for &(name, commands) in PLUGIN_COMMANDS {
        attributes = attributes.plugin(name, tauri_build::InlinedPlugin::new().commands(commands));
    }

    tauri_build::try_build(attributes).expect("failed to run tauri-build");
}
