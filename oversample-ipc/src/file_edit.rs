//! A change the app made to an audio file's bytes, and what is needed to
//! take it back. Returned by the desktop commands `guano_set_te` and
//! `file_edit_undo`, and saved in the recording's .batm file.

use serde::{Deserialize, Serialize};

/// One run of bytes replaced by another, with the bytes as lowercase hex.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSplice {
    /// Where the run starts in the file the patch applies to.
    pub offset: u64,
    /// The bytes there now.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub old: String,
    /// The bytes that take their place.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub new: String,
}

/// A file's size and hashes at one point in time.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileHashes {
    pub file_size: u64,
    /// SHA-256 of the whole file, lowercase hex.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// BLAKE3 of the whole file, when the app had computed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blake3: Option<String>,
}

/// What a change to a file did: the file before and after, and the patch
/// that takes the changed file back to the original bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEditOutcome {
    pub original: FileHashes,
    pub changed: FileHashes,
    /// Applied to the changed file, gives the original file.
    pub undo: Vec<SavedSplice>,
}

/// Lowercase hex of `bytes`.
pub fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// The bytes of a hex string, or `None` when it is not hex.
pub fn from_hex(hex: &str) -> Option<Vec<u8>> {
    if hex.len() % 2 != 0 || !hex.is_ascii() {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips() {
        let bytes = [0u8, 1, 0x7f, 0x80, 0xff];
        assert_eq!(to_hex(&bytes), "00017f80ff");
        assert_eq!(from_hex("00017f80ff"), Some(bytes.to_vec()));
        assert_eq!(from_hex("0"), None);
        assert_eq!(from_hex("zz"), None);
        assert_eq!(from_hex(""), Some(Vec::new()));
    }
}
