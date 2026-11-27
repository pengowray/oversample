//! The folder new recordings are saved in on the desktop, and opening a
//! file's folder in the system file manager. The folder is chosen and kept
//! by native code (`src-tauri/src/save_folder.rs`): it is needed at start-up
//! for crash recovery, before the webview runs, and the webview never names a
//! folder to write to, it only asks for the system's folder picker.

use crate::state::AppState;
use crate::tauri_bridge::{tauri_invoke_args, tauri_invoke_typed_no_args};
use leptos::prelude::*;
pub use oversample_ipc::mic::RecordingFolder;

thread_local! {
    // An `Arc` signal, made on first use and kept for the whole session.
    static FOLDER: ArcRwSignal<Option<RecordingFolder>> = ArcRwSignal::new(None);
}

/// The folder for new recordings, once native code has said.
pub fn folder() -> ArcRwSignal<Option<RecordingFolder>> {
    FOLDER.with(Clone::clone)
}

/// The desktop app (Linux, Windows, macOS): the only build with a folder
/// setting and paths on disk. Android saves recordings to shared storage.
pub fn is_desktop(state: AppState) -> bool {
    state.is_tauri && !crate::tauri_bridge::is_android()
}

/// Ask native code where the next recording goes.
pub fn refresh() {
    wasm_bindgen_futures::spawn_local(async {
        match tauri_invoke_typed_no_args::<RecordingFolder>("recording_folder_get").await {
            Ok(f) => folder().set(Some(f)),
            Err(e) => log::warn!("recording_folder_get: {e}"),
        }
    });
}

/// Open the system folder picker and keep the folder picked.
pub fn choose(state: AppState) {
    wasm_bindgen_futures::spawn_local(async move {
        match tauri_invoke_typed_no_args::<Option<RecordingFolder>>("recording_folder_choose").await
        {
            Ok(Some(f)) => folder().set(Some(f)),
            Ok(None) => {}
            Err(e) => state.show_error_toast(format!("Couldn't change folder: {e}")),
        }
    });
}

/// Go back to the default folder.
pub fn reset(state: AppState) {
    wasm_bindgen_futures::spawn_local(async move {
        match tauri_invoke_typed_no_args::<RecordingFolder>("recording_folder_reset").await {
            Ok(f) => folder().set(Some(f)),
            Err(e) => state.show_error_toast(format!("Couldn't change folder: {e}")),
        }
    });
}

/// Open the folder that the file at `path` is in.
pub fn open_containing(state: AppState, path: String) {
    #[derive(serde::Serialize)]
    struct Path {
        path: String,
    }
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = tauri_invoke_args("open_containing_folder", &Path { path }).await {
            state.show_error_toast(format!("Couldn't open folder: {e}"));
        }
    });
}

/// The last two parts of `path` ("Music/Oversample"), with the path's own
/// separator.
pub fn last_two_parts(path: &str) -> String {
    let sep = if path.contains('\\') && !path.contains('/') {
        '\\'
    } else {
        '/'
    };
    let parts: Vec<&str> = path.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
    let n = parts.len();
    parts[n.saturating_sub(2)..].join(&sep.to_string())
}

/// The label of the button that goes back to the default folder.
pub fn reset_label(f: &RecordingFolder) -> String {
    if f.music_missing {
        "Reset to default folder".to_string()
    } else {
        format!("Reset to {}", last_two_parts(&f.default_path))
    }
}

/// `path` split after each separator, so it can wrap between folder names:
/// the text of each piece, the last one being the file or folder name.
pub fn path_pieces(path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut piece = String::new();
    for c in path.chars() {
        piece.push(c);
        if c == '/' || c == '\\' {
            out.push(std::mem::take(&mut piece));
        }
    }
    if !piece.is_empty() {
        out.push(piece);
    }
    out
}

/// `path` with a place to wrap after each separator. Long folder names can
/// still wrap anywhere (`.path-text` in style.css).
pub fn path_view(path: &str) -> impl IntoView {
    path_pieces(path)
        .into_iter()
        .map(|p| view! { {p}<wbr/> })
        .collect::<Vec<_>>()
}

/// `path` split for one line shortened in the middle: everything up to the
/// last separator, which may be cut with "...", and the last name, which is
/// always shown.
pub fn head_and_tail(path: &str) -> (String, String) {
    let trimmed = path.trim_end_matches(['/', '\\']);
    match trimmed.rfind(['/', '\\']) {
        Some(i) => (trimmed[..=i].to_string(), trimmed[i + 1..].to_string()),
        None => (String::new(), trimmed.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_button_names_the_default_folder() {
        let mut f = RecordingFolder {
            default_path: "/home/a/Music/Oversample".into(),
            ..Default::default()
        };
        assert_eq!(reset_label(&f), "Reset to Music/Oversample");
        f.default_path = r"C:\Users\Ann\Music\Oversample".into();
        assert_eq!(reset_label(&f), r"Reset to Music\Oversample");
        f.music_missing = true;
        assert_eq!(reset_label(&f), "Reset to default folder");
    }

    #[test]
    fn a_path_wraps_after_each_separator() {
        assert_eq!(
            path_pieces("/home/a/rec.wav"),
            ["/", "home/", "a/", "rec.wav"]
        );
        assert_eq!(path_pieces(r"C:\x\y.wav"), [r"C:\", r"x\", "y.wav"]);
        assert_eq!(path_pieces("name"), ["name"]);
    }

    #[test]
    fn the_last_name_is_kept_whole() {
        assert_eq!(
            head_and_tail("~/Music/Oversample"),
            ("~/Music/".to_string(), "Oversample".to_string())
        );
        assert_eq!(
            head_and_tail("/media/usb/bats/"),
            ("/media/usb/".to_string(), "bats".to_string())
        );
        assert_eq!(head_and_tail("x"), (String::new(), "x".to_string()));
    }
}
