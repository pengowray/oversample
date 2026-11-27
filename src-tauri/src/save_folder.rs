//! The folder new recordings are saved in, and moving or writing a finished
//! recording there.
//!
//! On the desktop the default is the system Music folder's `Oversample`
//! (`~/Music/Oversample`; on Linux the XDG `MUSIC` user folder). The user can
//! choose another folder with the system's folder picker; that choice is kept
//! in `recording-folder.json` in the app's config folder, so native code knows
//! it at start-up (crash recovery runs before the webview asks for anything)
//! and the webview never sends a folder path to write to.
//!
//! Each save tries the folders in [`Places::candidates`] order: the chosen
//! folder, the default folder, then the app data folder's `recordings/`, the
//! only folder used before (and still the only one on Android, where finished
//! recordings normally go to shared storage instead). Recordings already in
//! the app data folder stay there.
//!
//! Crash-recovery files stay in the app data folder's `recordings/.recovery`
//! (`crate::recovery`). The save folder can be on another file system, where
//! a rename fails, so [`move_file`] copies to a hidden temporary name in the
//! target folder, syncs it, renames it and only then deletes the source. If
//! the app stops part way, the source is still in `.recovery` and is
//! recovered on the next start.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use oversample_ipc::mic::RecordingFolder;
use serde::{Deserialize, Serialize};

/// The subfolder of the Music folder that recordings go in by default.
pub const SUBFOLDER: &str = "Oversample";
/// The file in the app config folder that keeps the user's chosen folder.
const CONFIG_FILE: &str = "recording-folder.json";

/// The folders a recording can be saved in.
#[derive(Clone, Debug, PartialEq)]
pub struct Places {
    /// The folder the user chose, if any.
    pub custom: Option<PathBuf>,
    /// The system Music folder, if it is known (never on Android).
    pub music: Option<PathBuf>,
    /// The app data folder.
    pub app_data: PathBuf,
}

/// Which of the [`Places`] a folder is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Custom,
    Music,
    AppData,
}

impl Places {
    /// The app data folder's `recordings/`: the last folder to try.
    pub fn app_data_recordings(&self) -> PathBuf {
        crate::recovery::recordings_dir(&self.app_data)
    }

    /// The folder used when the user has chosen none, if there is a Music folder.
    pub fn music_folder(&self) -> Option<PathBuf> {
        self.music.as_ref().map(|m| m.join(SUBFOLDER))
    }

    /// The folders to save in, in the order to try them, each once.
    pub fn candidates(&self) -> Vec<(PathBuf, Kind)> {
        let mut out: Vec<(PathBuf, Kind)> = Vec::new();
        let all = [
            self.custom.clone().map(|c| (c, Kind::Custom)),
            self.music_folder().map(|m| (m, Kind::Music)),
            Some((self.app_data_recordings(), Kind::AppData)),
        ];
        for (path, kind) in all.into_iter().flatten() {
            if !out.iter().any(|(p, _)| *p == path) {
                out.push((path, kind));
            }
        }
        out
    }

    /// The first candidate that `usable` accepts; the app data folder when
    /// none is.
    pub fn pick(&self, usable: impl Fn(&Path) -> bool) -> (PathBuf, Kind) {
        self.candidates()
            .into_iter()
            .find(|(p, _)| usable(p))
            .unwrap_or_else(|| (self.app_data_recordings(), Kind::AppData))
    }

    /// What the settings show: the folder the next recording goes to, as far
    /// as can be told without writing to it.
    pub fn describe(
        &self,
        usable: impl Fn(&Path) -> bool,
        home: Option<&Path>,
        tilde: bool,
    ) -> RecordingFolder {
        let (path, _) = self.pick(&usable);
        let default = Places {
            custom: None,
            ..self.clone()
        };
        let (default_path, default_kind) = default.pick(&usable);
        let unusable_custom = self
            .custom
            .as_ref()
            .filter(|c| !usable(c))
            .map(|c| c.to_string_lossy().to_string());
        RecordingFolder {
            short_path: short_path(&path, home, tilde),
            path: path.to_string_lossy().to_string(),
            default_path: default_path.to_string_lossy().to_string(),
            is_custom: self.custom.is_some(),
            unusable_custom,
            music_missing: default_kind == Kind::AppData,
        }
    }
}

/// `path` with the home folder written as `~` when `tilde` is set.
pub fn short_path(path: &Path, home: Option<&Path>, tilde: bool) -> String {
    if tilde {
        if let Some(rest) = home.and_then(|h| path.strip_prefix(h).ok()) {
            return if rest.as_os_str().is_empty() {
                "~".to_string()
            } else {
                format!("~/{}", rest.to_string_lossy())
            };
        }
    }
    path.to_string_lossy().to_string()
}

/// Whether a recording could probably be saved in `dir` without creating
/// anything: `dir` is a folder that is not read-only, or the nearest folder
/// above it that exists is. A save checks for real by writing.
pub fn looks_usable(dir: &Path) -> bool {
    let mut at = Some(dir);
    while let Some(d) = at {
        match std::fs::metadata(d) {
            Ok(m) => return m.is_dir() && !m.permissions().readonly(),
            Err(_) => at = d.parent(),
        }
    }
    false
}

impl Places {
    /// The same places without the chosen folder when it is not an existing
    /// folder. A chosen folder is never created: when it is on a drive that
    /// is not plugged in, creating it would put recordings on the wrong disk
    /// (and the mount folder above it often looks writable, so
    /// [`looks_usable`] can't tell).
    pub fn without_missing_custom(&self, is_dir: impl Fn(&Path) -> bool) -> Places {
        Places {
            custom: self.custom.clone().filter(|c| is_dir(c)),
            ..self.clone()
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
struct Config {
    folder: Option<PathBuf>,
}

/// The folder the user chose, from the config folder `config_dir`.
pub fn load_custom(config_dir: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(config_dir.join(CONFIG_FILE)).ok()?;
    serde_json::from_str::<Config>(&text)
        .ok()?
        .folder
        .filter(|p| p.is_absolute())
}

/// Keep `folder` as the user's choice, or forget the choice when `None`.
pub fn save_custom(config_dir: &Path, folder: Option<&Path>) -> std::io::Result<()> {
    let file = config_dir.join(CONFIG_FILE);
    let Some(folder) = folder else {
        return match std::fs::remove_file(&file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    };
    std::fs::create_dir_all(config_dir)?;
    let json = serde_json::to_string_pretty(&Config {
        folder: Some(folder.to_path_buf()),
    })
    .map_err(std::io::Error::other)?;
    let tmp = config_dir.join(format!("{CONFIG_FILE}.tmp"));
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, &file)
}

/// `dir/name`, or `dir/<stem> (2).<ext>` and so on when that is taken, so a
/// save never replaces a file that is already there.
pub fn free_name(dir: &Path, name: &str, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let first = dir.join(name);
    if !exists(&first) {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (2u32..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !exists(p))
        .expect("some numbered name is free")
}

/// Move `src` to `dst`, also across file systems. A rename where it works;
/// otherwise a copy to a hidden name beside `dst`, synced and renamed, then
/// `src` is deleted. On failure `src` is left as it was and nothing is left at
/// `dst`.
pub fn move_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    if std::fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    let name = dst
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let tmp = dst.with_file_name(format!(".{name}.copying"));
    let copied = (|| {
        std::fs::copy(src, &tmp)?;
        File::open(&tmp)?.sync_all()?;
        std::fs::rename(&tmp, dst)
    })();
    match copied {
        Ok(()) => {
            let _ = std::fs::remove_file(src);
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Move the finished recording `src` into the first of `dirs` that takes it,
/// named `name` (numbered if taken). Returns its new path.
pub fn place_in(dirs: &[PathBuf], src: &Path, name: &str) -> Result<PathBuf, String> {
    let mut errors = Vec::new();
    for dir in dirs {
        let placed = std::fs::create_dir_all(dir).and_then(|()| {
            let dst = free_name(dir, name, |p| p.exists());
            move_file(src, &dst).map(|()| dst)
        });
        match placed {
            Ok(dst) => return Ok(dst),
            Err(e) => errors.push(format!("{}: {e}", dir.display())),
        }
    }
    Err(format!("Couldn't save {name}: {}", errors.join("; ")))
}

/// Create a new file named `name` (numbered if taken) in the first of `dirs`
/// where that works. Returns the open file and its path.
pub fn create_in(dirs: &[PathBuf], name: &str) -> Result<(File, PathBuf), String> {
    let mut errors = Vec::new();
    for dir in dirs {
        let created = std::fs::create_dir_all(dir).and_then(|()| {
            let dst = free_name(dir, name, |p| p.exists());
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&dst)
                .map(|f| (f, dst))
        });
        match created {
            Ok(c) => return Ok(c),
            Err(e) => errors.push(format!("{}: {e}", dir.display())),
        }
    }
    Err(format!("Couldn't save {name}: {}", errors.join("; ")))
}

/// Write `bytes` as a new file named `name` in the first of `dirs` that
/// takes it. Returns its path.
pub fn write_in(dirs: &[PathBuf], name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let mut errors = Vec::new();
    for dir in dirs {
        let written = std::fs::create_dir_all(dir).and_then(|()| {
            let dst = free_name(dir, name, |p| p.exists());
            let mut f = OpenOptions::new().write(true).create_new(true).open(&dst)?;
            match f.write_all(bytes).and_then(|()| f.sync_data()) {
                Ok(()) => Ok(dst),
                Err(e) => {
                    drop(f);
                    let _ = std::fs::remove_file(&dst);
                    Err(e)
                }
            }
        });
        match written {
            Ok(p) => return Ok(p),
            Err(e) => errors.push(format!("{}: {e}", dir.display())),
        }
    }
    Err(format!("Couldn't save {name}: {}", errors.join("; ")))
}

/// The file name of `path` (which may be numbered by [`free_name`]), or
/// `fallback`.
pub fn file_name_of(path: &Path, fallback: String) -> String {
    path.file_name()
        .map_or(fallback, |n| n.to_string_lossy().to_string())
}

/// The folders for this app, from Tauri's path resolver.
pub fn places(app: &tauri::AppHandle) -> Result<Places, String> {
    use tauri::Manager;
    let path = app.path();
    let app_data = path.app_data_dir().map_err(|e| e.to_string())?;
    if cfg!(target_os = "android") {
        return Ok(Places {
            custom: None,
            music: None,
            app_data,
        });
    }
    Ok(Places {
        custom: path.app_config_dir().ok().and_then(|d| load_custom(&d)),
        music: path.audio_dir().ok(),
        app_data,
    })
}

/// The folders to save a recording in, in the order to try them.
pub fn dirs(app: &tauri::AppHandle) -> Result<Vec<PathBuf>, String> {
    Ok(places(app)?
        .without_missing_custom(Path::is_dir)
        .candidates()
        .into_iter()
        .map(|(p, _)| p)
        .collect())
}

/// Only the app data folder's `recordings/`, for the WAV a "To memory"
/// recording leaves on disk at stop (the app reads it back for its hashes).
/// The user did not ask for a file there, so it stays out of the visible
/// folder, where it was before. Also used when a "To file" recording could
/// not stream to disk, which is rare.
pub fn memory_dirs(app: &tauri::AppHandle) -> Result<Vec<PathBuf>, String> {
    Ok(vec![places(app)?.app_data_recordings()])
}

/// What the settings show, for this app.
pub fn describe(app: &tauri::AppHandle) -> Result<RecordingFolder, String> {
    use tauri::Manager;
    let home = app.path().home_dir().ok();
    let places = places(app)?;
    Ok(places.describe(
        shown_usable(places.custom.clone()),
        home.as_deref(),
        !cfg!(windows),
    ))
}

/// Whether the settings show a folder as usable: the chosen folder
/// `custom` must exist (it is never created, see
/// [`Places::without_missing_custom`]); the others need only
/// [`looks_usable`].
pub fn shown_usable(custom: Option<PathBuf>) -> impl Fn(&Path) -> bool {
    move |p: &Path| (custom.as_deref() != Some(p) || p.is_dir()) && looks_usable(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "oversample-save-folder-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn places(custom: Option<&str>, music: Option<&str>) -> Places {
        Places {
            custom: custom.map(PathBuf::from),
            music: music.map(PathBuf::from),
            app_data: PathBuf::from("/data/app"),
        }
    }

    #[test]
    fn default_is_the_music_folders_oversample() {
        let p = places(None, Some("/home/a/Music"));
        assert_eq!(
            p.pick(|_| true),
            (PathBuf::from("/home/a/Music/Oversample"), Kind::Music)
        );
        assert_eq!(
            p.candidates().last().unwrap().0,
            PathBuf::from("/data/app/recordings")
        );
    }

    #[test]
    fn a_chosen_folder_comes_first() {
        let p = places(Some("/media/usb/bats"), Some("/home/a/Music"));
        assert_eq!(
            p.pick(|_| true),
            (PathBuf::from("/media/usb/bats"), Kind::Custom)
        );
        let order: Vec<_> = p.candidates().into_iter().map(|(_, k)| k).collect();
        assert_eq!(order, [Kind::Custom, Kind::Music, Kind::AppData]);
    }

    #[test]
    fn an_unusable_chosen_folder_falls_back_to_the_default() {
        let p = places(Some("/media/usb/bats"), Some("/home/a/Music"));
        let usable = |d: &Path| !d.starts_with("/media");
        assert_eq!(
            p.pick(usable),
            (PathBuf::from("/home/a/Music/Oversample"), Kind::Music)
        );
        let shown = p.describe(usable, Some(Path::new("/home/a")), true);
        assert_eq!(shown.path, "/home/a/Music/Oversample");
        assert_eq!(shown.short_path, "~/Music/Oversample");
        assert!(shown.is_custom);
        assert_eq!(shown.unusable_custom.as_deref(), Some("/media/usb/bats"));
        assert!(!shown.music_missing);
    }

    #[test]
    fn no_music_folder_falls_back_to_app_data() {
        let p = places(None, None);
        assert_eq!(
            p.pick(|_| true),
            (PathBuf::from("/data/app/recordings"), Kind::AppData)
        );
        let shown = p.describe(|_| true, None, true);
        assert!(shown.music_missing);
        assert_eq!(shown.default_path, "/data/app/recordings");

        // A Music folder that can't be written is the same.
        let p = places(None, Some("/ro/Music"));
        let shown = p.describe(|d: &Path| !d.starts_with("/ro"), None, true);
        assert_eq!(shown.path, "/data/app/recordings");
        assert!(shown.music_missing);
    }

    #[test]
    fn android_uses_only_the_app_data_folder() {
        assert_eq!(
            places(None, None).candidates(),
            [(PathBuf::from("/data/app/recordings"), Kind::AppData)]
        );
    }

    #[test]
    fn short_path_uses_a_tilde_for_home() {
        let home = Some(Path::new("/home/a"));
        assert_eq!(
            short_path(Path::new("/home/a/Music/Oversample"), home, true),
            "~/Music/Oversample"
        );
        assert_eq!(short_path(Path::new("/home/a"), home, true), "~");
        assert_eq!(
            short_path(Path::new("/home/ab/x"), home, true),
            "/home/ab/x"
        );
        assert_eq!(
            short_path(Path::new("/home/a/Music"), home, false),
            "/home/a/Music"
        );
    }

    #[test]
    fn the_chosen_folder_is_kept_and_forgotten() {
        let dir = temp("config");
        assert_eq!(load_custom(&dir), None);
        let chosen = dir.join("bats");
        save_custom(&dir, Some(&chosen)).unwrap();
        assert_eq!(load_custom(&dir), Some(chosen));
        save_custom(&dir, None).unwrap();
        assert_eq!(load_custom(&dir), None);
        save_custom(&dir, None).unwrap();
        // A relative path is never used.
        std::fs::write(dir.join(CONFIG_FILE), r#"{"folder":"bats"}"#).unwrap();
        assert_eq!(load_custom(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_taken_name_gets_a_number() {
        let taken = [PathBuf::from("/d/a.wav"), PathBuf::from("/d/a (2).wav")];
        let exists = |p: &Path| taken.iter().any(|t| t == p);
        assert_eq!(
            free_name(Path::new("/d"), "b.wav", exists),
            PathBuf::from("/d/b.wav")
        );
        assert_eq!(
            free_name(Path::new("/d"), "a.wav", exists),
            PathBuf::from("/d/a (3).wav")
        );
        let exists = |p: &Path| p == Path::new("/d/noext");
        assert_eq!(
            free_name(Path::new("/d"), "noext", exists),
            PathBuf::from("/d/noext (2)")
        );
    }

    /// A chosen folder on a drive that is not plugged in: its parent exists
    /// and is writable, but the folder is not made and not used.
    #[test]
    fn a_missing_chosen_folder_is_shown_unusable_and_skipped() {
        let root = temp("missing-custom");
        let chosen = root.join("gone/bats");
        let p = Places {
            custom: Some(chosen.clone()),
            music: Some(root.join("Music")),
            app_data: root.join("app"),
        };
        let shown = p.describe(shown_usable(p.custom.clone()), None, true);
        assert_eq!(
            shown.unusable_custom.as_deref(),
            Some(chosen.to_string_lossy().as_ref())
        );
        assert_eq!(shown.path, root.join("Music/Oversample").to_string_lossy());
        let dirs: Vec<_> = p
            .without_missing_custom(Path::is_dir)
            .candidates()
            .into_iter()
            .map(|(d, _)| d)
            .collect();
        assert_eq!(dirs[0], root.join("Music/Oversample"));
        assert!(!dirs.contains(&chosen));

        // Once it is there again, it is used.
        std::fs::create_dir_all(&chosen).unwrap();
        let shown = p.describe(shown_usable(p.custom.clone()), None, true);
        assert_eq!(shown.unusable_custom, None);
        assert_eq!(shown.path, chosen.to_string_lossy());
        assert_eq!(
            p.without_missing_custom(Path::is_dir).candidates()[0].0,
            chosen
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn looks_usable_checks_the_nearest_existing_folder() {
        let dir = temp("usable");
        assert!(looks_usable(&dir));
        assert!(looks_usable(&dir.join("not/yet/made")));
        let file = dir.join("file");
        std::fs::write(&file, b"x").unwrap();
        assert!(!looks_usable(&file));
        assert!(!looks_usable(&file.join("below")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_recording_is_moved_and_keeps_existing_files() {
        let root = temp("place");
        let src_dir = root.join("app/recordings/.recovery");
        std::fs::create_dir_all(&src_dir).unwrap();
        let target = root.join("Music/Oversample");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("rec.wav"), b"older").unwrap();
        let src = src_dir.join("rec.wav.part");
        std::fs::write(&src, b"newer").unwrap();

        let placed = place_in(&[target.clone()], &src, "rec.wav").unwrap();
        assert_eq!(placed, target.join("rec (2).wav"));
        assert_eq!(std::fs::read(&placed).unwrap(), b"newer");
        assert_eq!(std::fs::read(target.join("rec.wav")).unwrap(), b"older");
        assert!(!src.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_that_cant_be_made_falls_back_to_the_next() {
        let root = temp("fallback");
        // A file where the folder should be: create_dir_all fails.
        let blocked = root.join("blocked");
        std::fs::write(&blocked, b"not a folder").unwrap();
        let fallback = root.join("app/recordings");
        let src = root.join("x.part");
        std::fs::write(&src, b"audio").unwrap();

        let placed = place_in(
            &[blocked.join("Oversample"), fallback.clone()],
            &src,
            "x.wav",
        )
        .unwrap();
        assert_eq!(placed, fallback.join("x.wav"));
        let written = write_in(
            &[blocked.join("Oversample"), fallback.clone()],
            "y.wav",
            b"y",
        )
        .unwrap();
        assert_eq!(written, fallback.join("y.wav"));
        let (_, created) =
            create_in(&[blocked.join("Oversample"), fallback.clone()], "y.wav").unwrap();
        assert_eq!(created, fallback.join("y (2).wav"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A move between two file systems, where a rename fails: /dev/shm is a
    /// separate tmpfs on most Linux systems. Skipped where it is not.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_move_across_file_systems_copies_then_deletes() {
        use std::os::unix::fs::MetadataExt;
        let root = temp("cross-fs");
        let shm = PathBuf::from("/dev/shm");
        let (Ok(a), Ok(b)) = (std::fs::metadata(&root), std::fs::metadata(&shm)) else {
            return;
        };
        if a.dev() == b.dev() {
            return;
        }
        let other = shm.join(format!("oversample-save-folder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&other);
        std::fs::create_dir_all(&other).unwrap();
        let src = other.join("rec.wav.part");
        std::fs::write(&src, b"audio across").unwrap();
        let placed = place_in(&[root.join("Music/Oversample")], &src, "rec.wav").unwrap();
        assert_eq!(std::fs::read(&placed).unwrap(), b"audio across");
        assert!(!src.exists());
        let left: Vec<_> = std::fs::read_dir(root.join("Music/Oversample"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(left, ["rec.wav"], "no hidden copy is left behind");
        let _ = std::fs::remove_dir_all(&other);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn move_file_leaves_the_source_when_the_target_cant_be_written() {
        let root = temp("move-fail");
        let src = root.join("a.part");
        std::fs::write(&src, b"audio").unwrap();
        let dst = root.join("missing-folder/a.wav");
        assert!(move_file(&src, &dst).is_err());
        assert!(src.exists());
        assert!(!dst.exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
