//! Asks before the desktop window closes while a recording is not saved.
//!
//! A "To memory" recording lives only in the webview's memory until the user
//! presses Save, so closing the window loses it. The frontend sends the names
//! of the finished recordings that are not saved (`set_unsaved_recordings`)
//! whenever that list changes. When the window is asked to close and the list
//! is not empty, the close is stopped and a system message box asks whether
//! to close anyway. The webview is never asked at close time, so a busy or
//! broken page cannot keep the window open.
//!
//! Android has no window to close; the command is kept there so the frontend
//! needs no platform check, but nothing reads the list.

use std::sync::atomic::AtomicBool;
#[cfg(desktop)]
use std::sync::atomic::Ordering;
use std::sync::Mutex;

/// The unsaved recordings the frontend last reported, and whether the close
/// message box is showing.
#[derive(Default)]
pub struct CloseWarning {
    /// The newest list's number, and the list.
    names: Mutex<(u64, Unsaved)>,
    #[cfg_attr(not(desktop), allow(dead_code))]
    dialog_open: AtomicBool,
}

/// The recordings that closing the window would lose.
#[derive(Default, Clone)]
struct Unsaved {
    /// Finished recordings that are not saved.
    finished: Vec<String>,
    /// The "To memory" recording still running, if any.
    running: Option<String>,
}

/// The frontend's list of open recordings that are not saved (file names).
/// `running` is the "To memory" recording still in progress, if any. With
/// neither, the window closes without asking. `seq` counts up with
/// each list; a list older than the one kept is dropped.
#[tauri::command]
pub fn set_unsaved_recordings(
    state: tauri::State<CloseWarning>,
    names: Vec<String>,
    running: Option<String>,
    seq: u64,
) {
    if let Ok(mut kept) = state.names.lock() {
        if seq >= kept.0 {
            let unsaved = Unsaved {
                finished: names,
                running,
            };
            *kept = (seq, unsaved);
        }
    }
}

/// Button labels. If a system can only show OK and Cancel, the message's
/// question reads with that pair too.
#[cfg_attr(not(desktop), allow(dead_code))]
const CLOSE_LABEL: &str = "Close without saving";
#[cfg_attr(not(desktop), allow(dead_code))]
const CANCEL_LABEL: &str = "Cancel";

/// Names listed in the message before it says how many more there are.
#[cfg_attr(not(desktop), allow(dead_code))]
const MAX_LISTED: usize = 6;
/// Names listed when there are more than `MAX_LISTED`.
#[cfg_attr(not(desktop), allow(dead_code))]
const LISTED_WHEN_CUT: usize = 5;

/// The message box's title and text for these unsaved recordings.
#[cfg_attr(not(desktop), allow(dead_code))]
fn message_text(unsaved: &Unsaved) -> (String, String) {
    let names: Vec<&String> = unsaved.running.iter().chain(&unsaved.finished).collect();
    let stop = if unsaved.running.is_some() {
        "stop the recording, then "
    } else {
        ""
    };
    let n = names.len();
    let title = if n == 1 {
        "Recording not saved".to_string()
    } else {
        format!("{n} recordings not saved")
    };
    let mut lines = Vec::new();
    lines.push(if n == 1 {
        "Close Oversample and lose this recording?".to_string()
    } else {
        format!("Close Oversample and lose these {n} recordings?")
    });
    let listed = if n > MAX_LISTED { LISTED_WHEN_CUT } else { n };
    lines.extend(names.iter().take(listed).map(|s| s.to_string()));
    if n > listed {
        lines.push(format!("and {} more recordings", n - listed));
    }
    lines.push(if n == 1 {
        format!("To keep it, choose Cancel, then {stop}press Save.")
    } else {
        format!("To keep them, choose Cancel, then {stop}select each recording and press Save.")
    });
    (title, lines.join("\n"))
}

/// Whether the message box's answer closes the window. `failed_is_cancel`:
/// on Linux, `rfd` shows the box with zenity and answers a plain `Cancel`
/// only when zenity could not be run (a press of either button gives that
/// button's label). The window then closes, as it did before this check,
/// because a window that will not close and says nothing is worse.
#[cfg_attr(not(desktop), allow(dead_code))]
fn closes_window(answer: &rfd_answer::Answer, failed_is_cancel: bool) -> bool {
    use rfd_answer::Answer;
    match answer {
        Answer::Ok | Answer::Yes => true,
        Answer::Custom(label) => label == CLOSE_LABEL,
        Answer::Cancel => failed_is_cancel,
        Answer::No => false,
    }
}

/// The message box answers, without `rfd` (which Android does not build), so
/// `closes_window` and its tests build everywhere.
#[cfg_attr(not(desktop), allow(dead_code))]
mod rfd_answer {
    #[derive(Debug, PartialEq)]
    pub enum Answer {
        Yes,
        No,
        Ok,
        Cancel,
        Custom(String),
    }

    #[cfg(desktop)]
    impl From<rfd::MessageDialogResult> for Answer {
        fn from(r: rfd::MessageDialogResult) -> Self {
            match r {
                rfd::MessageDialogResult::Yes => Answer::Yes,
                rfd::MessageDialogResult::No => Answer::No,
                rfd::MessageDialogResult::Ok => Answer::Ok,
                rfd::MessageDialogResult::Cancel => Answer::Cancel,
                rfd::MessageDialogResult::Custom(s) => Answer::Custom(s),
            }
        }
    }
}

/// zenity reads its text as Pango markup, so a file name with `&` or `<`
/// would break the message.
#[cfg(target_os = "linux")]
fn for_zenity(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// `on_window_event` hook for `CloseRequested`: stops the close and asks
/// when a recording is not saved. Closes with `destroy`, which does not ask
/// again.
#[cfg(desktop)]
pub fn on_close_requested<R: tauri::Runtime>(
    window: &tauri::Window<R>,
    api: &tauri::CloseRequestApi,
) {
    use tauri::Manager;
    let state = window.state::<CloseWarning>();
    let names = state.names.lock().map(|n| n.1.clone()).unwrap_or_default();
    if names.finished.is_empty() && names.running.is_none() {
        return;
    }
    api.prevent_close();
    // A second close while the box is showing does nothing more.
    if state.dialog_open.swap(true, Ordering::SeqCst) {
        return;
    }
    let (title, body) = message_text(&names);
    #[cfg(target_os = "linux")]
    let body = for_zenity(&body);
    // Built here, on the event loop's thread, where the window handle for
    // the parent can be read.
    let dialog = rfd::AsyncMessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title(title)
        .set_description(body)
        .set_buttons(rfd::MessageButtons::OkCancelCustom(
            CLOSE_LABEL.into(),
            CANCEL_LABEL.into(),
        ))
        .set_parent(window);
    let window = window.clone();
    tauri::async_runtime::spawn(async move {
        let answer = rfd_answer::Answer::from(dialog.show().await);
        window
            .state::<CloseWarning>()
            .dialog_open
            .store(false, Ordering::SeqCst);
        let failed_is_cancel = cfg!(target_os = "linux");
        if answer == rfd_answer::Answer::Cancel && failed_is_cancel {
            eprintln!("close warning: the message box could not be shown; closing");
        }
        if closes_window(&answer, failed_is_cancel) {
            let _ = window.destroy();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::rfd_answer::Answer;
    use super::*;

    fn names(n: usize) -> Unsaved {
        Unsaved {
            finished: (1..=n).map(|i| format!("batcap_{i}.wav")).collect(),
            running: None,
        }
    }

    #[test]
    fn running_recording_is_listed_first() {
        let mut unsaved = names(1);
        unsaved.running = Some("batcap_now.wav".into());
        let (title, body) = message_text(&unsaved);
        assert_eq!(title, "2 recordings not saved");
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines[1], "batcap_now.wav");
        assert_eq!(
            lines[3],
            "To keep them, choose Cancel, then stop the recording, then select each recording and press Save."
        );
        unsaved.finished.clear();
        let (title, body) = message_text(&unsaved);
        assert_eq!(title, "Recording not saved");
        assert!(
            body.ends_with("To keep it, choose Cancel, then stop the recording, then press Save.")
        );
    }

    #[test]
    fn one_recording() {
        let (title, body) = message_text(&names(1));
        assert_eq!(title, "Recording not saved");
        assert_eq!(
            body,
            "Close Oversample and lose this recording?\nbatcap_1.wav\n\
             To keep it, choose Cancel, then press Save."
        );
    }

    #[test]
    fn several_recordings_all_listed() {
        let (title, body) = message_text(&names(6));
        assert_eq!(title, "6 recordings not saved");
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines[0], "Close Oversample and lose these 6 recordings?");
        assert_eq!(lines[6], "batcap_6.wav");
        assert_eq!(
            lines[7],
            "To keep them, choose Cancel, then select each recording and press Save."
        );
        assert_eq!(lines.len(), 8);
    }

    #[test]
    fn many_recordings_cut_after_five() {
        let (_, body) = message_text(&names(12));
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines[5], "batcap_5.wav");
        assert_eq!(lines[6], "and 7 more recordings");
        assert_eq!(lines.len(), 8);
    }

    #[test]
    fn answers() {
        // Linux and macOS: the pressed button's label.
        assert!(closes_window(&Answer::Custom(CLOSE_LABEL.into()), true));
        assert!(!closes_window(&Answer::Custom(CANCEL_LABEL.into()), true));
        // Windows: OK and Cancel.
        assert!(closes_window(&Answer::Ok, false));
        assert!(!closes_window(&Answer::Cancel, false));
        // Linux: zenity could not be run.
        assert!(closes_window(&Answer::Cancel, true));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn zenity_markup_escaped() {
        assert_eq!(for_zenity("a & b <c>.wav"), "a &amp; b &lt;c&gt;.wav");
    }
}
