use crate::native_playback::{self, NativePlayParams};
use crate::PlaybackMutex;

#[tauri::command]
pub fn native_play(
    app: tauri::AppHandle,
    state: tauri::State<PlaybackMutex>,
    params: NativePlayParams,
) -> Result<(), String> {
    let mut pb = state.lock().map_err(|e| e.to_string())?;
    // Stop existing playback
    native_playback::stop(&mut pb);
    // Start new playback
    let new_state = native_playback::start(params, app)?;
    *pb = Some(new_state);
    Ok(())
}

#[tauri::command]
pub fn native_stop(state: tauri::State<PlaybackMutex>) -> Result<(), String> {
    let mut pb = state.lock().map_err(|e| e.to_string())?;
    native_playback::stop(&mut pb);
    Ok(())
}
