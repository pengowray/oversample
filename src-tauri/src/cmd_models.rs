//! Model files for BirdNET, BattyBirdNET, Perch and the denoiser, kept in the
//! app data folder's `models/` (WebKitGTK, the Linux webview, has no OPFS).
//!
//! [`models_download`] downloads a file in native code: it streams the
//! catalog URL to `<id>.part`, hashes the bytes as they arrive, and renames
//! the part to `<id>` once the size and SHA-256 match the catalog. While it
//! runs it sends `models-download-progress` events (`{file, fraction}`). The
//! webview never handles the downloaded bytes; it reads the finished file with
//! [`models_read`].
//!
//! Only file names from the catalog (`oversample_ipc::classifier::catalog`) are
//! accepted, so no command can touch a path outside `models/`. Deleting also
//! accepts the catalog's retired files, which earlier versions stored.

use oversample_ipc::classifier::{catalog, ClassifierError};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{Emitter, Manager};
use tokio::io::AsyncWriteExt;

/// The event [`models_download`] sends progress with.
const PROGRESS_EVENT: &str = "models-download-progress";
/// The most progress updates per file (plus one at the start and the end).
const PROGRESS_STEPS: f64 = 200.0;
/// A download fails when the server sends nothing for this long.
const STALL_TIMEOUT: Duration = Duration::from_secs(60);

fn models_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("models");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn checked(file: &str) -> Result<&'static catalog::ModelFile, String> {
    catalog::file_by_id(file).ok_or_else(|| format!("unknown model file '{file}'"))
}

fn part_name(id: &str) -> String {
    format!("{id}.part")
}

/// Stored catalog files and their sizes in bytes.
#[tauri::command]
pub fn models_sizes(app: tauri::AppHandle) -> Result<Vec<(String, u64)>, String> {
    let dir = models_dir(&app)?;
    Ok(catalog::all_files()
        .filter_map(|f| {
            let meta = std::fs::metadata(dir.join(f.id)).ok()?;
            Some((f.id.to_string(), meta.len()))
        })
        .collect())
}

#[derive(Clone, serde::Serialize)]
struct DownloadProgress<'a> {
    file: &'a str,
    fraction: f64,
}

/// Files being downloaded now, so two downloads of one file never write to
/// the same part file.
static DOWNLOADING: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);

/// Holds a file's place in [`DOWNLOADING`] until its download ends, however
/// it ends.
struct DownloadingGuard(&'static str);

impl DownloadingGuard {
    fn start(id: &'static str) -> Option<Self> {
        let mut set = DOWNLOADING.lock().unwrap_or_else(|e| e.into_inner());
        set.get_or_insert_with(HashSet::new)
            .insert(id)
            .then_some(Self(id))
    }
}

impl Drop for DownloadingGuard {
    fn drop(&mut self) {
        let mut set = DOWNLOADING.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(set) = set.as_mut() {
            set.remove(self.0);
        }
    }
}

/// Download a catalog file into `models/` and check its size and SHA-256.
/// Fails with `ClassifierError::Download` (network, HTTP status, or the file
/// is already downloading), `Corrupt` (wrong size or hash) or `Storage`
/// (writing the file).
#[tauri::command]
pub async fn models_download(
    app: tauri::AppHandle,
    xc: tauri::State<'_, Mutex<crate::xc::XcState>>,
    file: String,
) -> Result<(), ClassifierError> {
    let model = checked(&file).map_err(|message| ClassifierError::Download {
        file: file.clone(),
        message,
    })?;
    let dir = models_dir(&app).map_err(|message| ClassifierError::Storage {
        file: file.clone(),
        message,
    })?;
    // The XC client: reqwest with the OS certificate checks set up in
    // `tls.rs`, which Android needs.
    let client = xc
        .lock()
        .map_err(|e| ClassifierError::Download {
            file: file.clone(),
            message: e.to_string(),
        })?
        .client
        .clone();
    let Some(_guard) = DownloadingGuard::start(model.id) else {
        return Err(ClassifierError::Download {
            file,
            message: "this file is already downloading".into(),
        });
    };
    download_to(&client, model, &dir, |fraction| {
        let _ = app.emit(
            PROGRESS_EVENT,
            DownloadProgress {
                file: model.id,
                fraction,
            },
        );
    })
    .await
}

/// Stream `file` from its URL to `dir/<id>.part`, check it, and rename it to
/// `dir/<id>`. `on_progress` gets the fraction downloaded, about
/// [`PROGRESS_STEPS`] times at most. The part file is deleted if anything
/// fails.
async fn download_to(
    client: &reqwest::Client,
    file: &catalog::ModelFile,
    dir: &Path,
    mut on_progress: impl FnMut(f64),
) -> Result<(), ClassifierError> {
    let part = dir.join(part_name(file.id));
    if let Err(e) = download_part(client, file, &part, &mut on_progress).await {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(e);
    }
    tokio::fs::rename(&part, dir.join(file.id))
        .await
        .map_err(|e| ClassifierError::Storage {
            file: file.id.to_string(),
            message: e.to_string(),
        })?;
    on_progress(1.0);
    Ok(())
}

async fn download_part(
    client: &reqwest::Client,
    file: &catalog::ModelFile,
    part: &Path,
    on_progress: &mut impl FnMut(f64),
) -> Result<(), ClassifierError> {
    let dl_err = |message: String| ClassifierError::Download {
        file: file.id.to_string(),
        message,
    };
    let stalled = |_| dl_err("the server stopped sending".into());
    let storage_err = |e: std::io::Error| ClassifierError::Storage {
        file: file.id.to_string(),
        message: e.to_string(),
    };
    let corrupt = || ClassifierError::Corrupt {
        file: file.id.to_string(),
    };

    let mut resp = tokio::time::timeout(STALL_TIMEOUT, client.get(file.url).send())
        .await
        .map_err(stalled)?
        .map_err(|e| dl_err(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(dl_err(format!("HTTP {}", resp.status().as_u16())));
    }
    let mut out = tokio::fs::File::create(part).await.map_err(storage_err)?;
    let mut hasher = Sha256::new();
    let mut at: u64 = 0;
    let mut next_step = 0.0;
    loop {
        let chunk = tokio::time::timeout(STALL_TIMEOUT, resp.chunk())
            .await
            .map_err(stalled)?
            .map_err(|e| dl_err(e.to_string()))?;
        let Some(chunk) = chunk else { break };
        at += chunk.len() as u64;
        if at > file.size {
            return Err(corrupt());
        }
        hasher.update(&chunk);
        out.write_all(&chunk).await.map_err(storage_err)?;
        let fraction = at as f64 / file.size as f64;
        if fraction >= next_step {
            on_progress(fraction);
            next_step = fraction + 1.0 / PROGRESS_STEPS;
        }
    }
    out.flush().await.map_err(storage_err)?;
    out.sync_all().await.map_err(storage_err)?;
    drop(out);
    let hex: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if at != file.size || hex != file.sha256 {
        return Err(corrupt());
    }
    Ok(())
}

/// Read a stored file, as raw bytes.
#[tauri::command]
pub fn models_read(app: tauri::AppHandle, file: String) -> Result<tauri::ipc::Response, String> {
    let file = checked(&file)?;
    let bytes = std::fs::read(models_dir(&app)?.join(file.id)).map_err(|e| e.to_string())?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Delete a stored file (and any unfinished part), from the catalog or its
/// retired files. Not stored is not an error.
#[tauri::command]
pub fn models_delete(app: tauri::AppHandle, file: String) -> Result<(), String> {
    let file = catalog::retired_file_by_id(&file).map_or_else(|| checked(&file), Ok)?;
    let dir = models_dir(&app)?;
    let _ = std::fs::remove_file(dir.join(part_name(file.id)));
    match std::fs::remove_file(dir.join(file.id)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "oversample-models-test-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn smallest() -> &'static catalog::ModelFile {
        catalog::all_files().min_by_key(|f| f.size).unwrap()
    }

    /// Downloads the smallest catalog file from its real URL, then the same
    /// file with a wrong SHA-256. Needs the network:
    /// `cargo test -p oversample-desktop -- --ignored models_download`
    #[test]
    #[ignore]
    fn models_download_real_file() {
        let client = xc_lib::http_client();
        let file = smallest();

        let dir = scratch_dir("ok");
        let mut calls = 0;
        tauri::async_runtime::block_on(download_to(&client, file, &dir, |_| calls += 1))
            .expect("download");
        let bytes = std::fs::read(dir.join(file.id)).unwrap();
        assert_eq!(bytes.len() as u64, file.size);
        let hex: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(hex, file.sha256);
        assert!(!dir.join(part_name(file.id)).exists());
        assert!(
            calls <= PROGRESS_STEPS as usize + 2,
            "{calls} progress calls"
        );
        eprintln!("{}: {} bytes, {calls} progress calls", file.id, file.size);
        let _ = std::fs::remove_dir_all(&dir);

        let dir = scratch_dir("bad-hash");
        let wrong = catalog::ModelFile {
            sha256: "0000000000000000000000000000000000000000000000000000000000000000",
            ..*file
        };
        let err =
            tauri::async_runtime::block_on(download_to(&client, &wrong, &dir, |_| {})).unwrap_err();
        assert!(matches!(err, ClassifierError::Corrupt { .. }), "{err:?}");
        assert!(!dir.join(file.id).exists());
        assert!(!dir.join(part_name(file.id)).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Downloads a multi-megabyte file and counts the progress calls. Needs the
    /// network: `cargo test -p oversample-desktop -- --ignored models_download`
    #[test]
    #[ignore]
    fn models_download_progress_count() {
        let client = xc_lib::http_client();
        let file = &catalog::GEOMODEL;
        let dir = scratch_dir("geomodel");
        let mut calls = 0;
        tauri::async_runtime::block_on(download_to(&client, file, &dir, |_| calls += 1))
            .expect("download");
        assert!(
            calls <= PROGRESS_STEPS as usize + 2,
            "{calls} progress calls"
        );
        eprintln!("{}: {} bytes, {calls} progress calls", file.id, file.size);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
