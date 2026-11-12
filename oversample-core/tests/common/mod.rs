//! Shared by the corpus tests: where the recordings they read are kept.
//!
//! Each corpus is a folder outside this repository. Its location comes from an
//! environment variable, then the same key in a `.env` file at the workspace
//! root, then a default sibling folder under `../batchi-extras`. A test whose
//! folder is missing skips with a notice instead of failing.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// Workspace root (one level above this crate).
pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// `key` from the environment, else from the workspace's `.env` file.
fn env_or_dotenv(key: &str) -> Option<String> {
    fn clean(v: &str) -> Option<String> {
        let v = v.trim().trim_matches('"').trim_matches('\'').trim();
        (!v.is_empty()).then(|| v.to_string())
    }
    std::env::var(key).ok().and_then(|v| clean(&v)).or_else(|| {
        let contents = std::fs::read_to_string(workspace_root().join(".env")).ok()?;
        contents.lines().find_map(|line| {
            let line = line.trim();
            if line.starts_with('#') {
                return None;
            }
            let rest = line.strip_prefix(key)?.trim_start().strip_prefix('=')?;
            clean(rest)
        })
    })
}

/// A corpus repository's root: `key` (relative paths are taken from the
/// workspace root), else `default` relative to the workspace root.
fn corpus_root(key: &str, default: &str) -> PathBuf {
    match env_or_dotenv(key).map(PathBuf::from) {
        Some(p) if p.is_absolute() => p,
        Some(p) => workspace_root().join(p),
        None => workspace_root().join(default),
    }
}

/// The bat-demo-sounds `sounds/` folder (`BAT_DEMO_SOUNDS_DIR`, default
/// `../batchi-extras/bat-demo-sounds`).
pub fn bat_demo_sounds_dir() -> PathBuf {
    corpus_root("BAT_DEMO_SOUNDS_DIR", "../batchi-extras/bat-demo-sounds").join("sounds")
}

/// Recordings with no bats (other animals from xeno-canto, and sound effects),
/// for counting false bat calls (`NO_BAT_SOUNDS_DIR`, default
/// `../batchi-extras/no-bat-sounds`, its own git repository; see its README).
pub fn no_bat_sounds_dir() -> PathBuf {
    corpus_root("NO_BAT_SOUNDS_DIR", "../batchi-extras/no-bat-sounds").join("sounds")
}

/// FNV-1a over a byte stream, for snapshot hashes.
pub struct Fnv(pub u64);

impl Fnv {
    pub fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }

    pub fn bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}
