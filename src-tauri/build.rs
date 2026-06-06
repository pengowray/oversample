// The inlined plugin command registry lives in `src/plugin_commands.rs` so it is
// a single source of truth shared with the lib's cross-check tests (which verify
// it against capabilities/default.json and the Kotlin @Command names). `include!`
// pulls in the `PLUGIN_COMMANDS` const; its `#[cfg(test)]` test module is stripped
// here (build scripts compile with cfg(test) = false).
include!("src/plugin_commands.rs");

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        println!("cargo:rustc-link-lib=c++_shared");
    }

    let mut attributes = tauri_build::Attributes::new();
    for &(name, commands) in PLUGIN_COMMANDS {
        attributes = attributes.plugin(name, tauri_build::InlinedPlugin::new().commands(commands));
    }

    tauri_build::try_build(attributes).expect("failed to run tauri-build");
}
