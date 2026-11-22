//! Setting the time expansion factor in a WAV file's GUANO metadata, as a
//! small change to the file's bytes that can be taken back exactly.
//!
//! [`set_te`] works out the change as a [`FilePatch`]: the runs of bytes to
//! replace. Nothing here reads or writes a file. The caller applies the patch
//! ([`FilePatch::apply`]), writes the result, and keeps the patch's
//! [`FilePatch::inverse`] with the file's original hashes, so the change can
//! be undone and the result checked against those hashes.
//!
//! Only the `guan` chunk and the RIFF size change. The audio in the `data`
//! chunk is never part of a patch.

use super::wav::{is_riff_wave, riff_chunks};

/// One run of bytes replaced by another.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Splice {
    /// Where the run starts, in the file before the change.
    pub offset: u64,
    /// The bytes there before the change. Empty when bytes are only added.
    pub old: Vec<u8>,
    /// The bytes that take their place. Empty when bytes are only removed.
    pub new: Vec<u8>,
}

/// A change to a file: runs of bytes to replace, in ascending order of
/// offset, none overlapping another.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct FilePatch {
    pub splices: Vec<Splice>,
}

impl FilePatch {
    /// The file after the change. `Err` when `bytes` is not the file the
    /// patch was made for: a run's old bytes are not at its offset.
    pub fn apply(&self, bytes: &[u8]) -> Result<Vec<u8>, String> {
        let mut out = Vec::with_capacity(bytes.len() + self.grown_by().max(0) as usize);
        let mut pos = 0usize;
        for s in &self.splices {
            let start = usize::try_from(s.offset).map_err(|_| "patch offset too large")?;
            let end = start
                .checked_add(s.old.len())
                .ok_or("patch offset too large")?;
            if start < pos || end > bytes.len() || bytes[start..end] != s.old[..] {
                return Err(format!(
                    "the file does not hold the bytes the change expects at offset {start}"
                ));
            }
            out.extend_from_slice(&bytes[pos..start]);
            out.extend_from_slice(&s.new);
            pos = end;
        }
        out.extend_from_slice(&bytes[pos..]);
        Ok(out)
    }

    /// The patch that takes the changed file back to the file before the
    /// change. Its offsets are in the changed file.
    pub fn inverse(&self) -> FilePatch {
        let mut shift: i64 = 0;
        let splices = self
            .splices
            .iter()
            .map(|s| {
                let offset = (s.offset as i64 + shift) as u64;
                shift += s.new.len() as i64 - s.old.len() as i64;
                Splice {
                    offset,
                    old: s.new.clone(),
                    new: s.old.clone(),
                }
            })
            .collect();
        FilePatch { splices }
    }

    /// How many bytes longer the changed file is. Negative when shorter.
    pub fn grown_by(&self) -> i64 {
        self.splices
            .iter()
            .map(|s| s.new.len() as i64 - s.old.len() as i64)
            .sum()
    }

    /// The number of bytes the patch holds, old and new together.
    pub fn stored_len(&self) -> usize {
        self.splices.iter().map(|s| s.old.len() + s.new.len()).sum()
    }
}

/// Why [`set_te`] can't change a file. Each is shown to the user.
pub mod msg {
    pub const NOT_WAV: &str = "Only a WAV file's GUANO metadata can be changed.";
    pub const RF64: &str = "This WAV file is in the RF64 layout, which can't be changed yet.";
    pub const BAD_SIZES: &str =
        "This WAV file's chunk sizes don't match its length, so it is left as it is.";
    pub const GUANO_NOT_TEXT: &str =
        "This file's GUANO metadata is not valid UTF-8 text, so it is left as it is.";
    pub const TOO_LARGE: &str = "The file would be over 4 GB, which a WAV file can't hold.";
}

/// The factor as GUANO's `TE` field holds it: "10", "2.5".
pub fn te_value(factor: f64) -> String {
    if (factor - factor.round()).abs() < 1e-9 {
        format!("{}", factor.round() as i64)
    } else {
        format!("{factor}")
    }
}

/// GUANO text with its `TE` field set to `factor`. An existing `TE` line is
/// replaced where it is, keeping its line ending. Otherwise a line is added
/// at the end. Every other byte of the text is kept. Text with no GUANO
/// header (an empty chunk) gets "GUANO|Version: 1.0" first.
pub fn set_te_in_text(text: &str, factor: f64) -> String {
    // Some writers fill the end of the chunk with NUL bytes.
    let text = text.trim_end_matches('\0');
    let line = format!("TE: {}", te_value(factor));
    let mut out = String::with_capacity(text.len() + line.len() + 24);
    let mut replaced = false;
    for raw in text.split_inclusive('\n') {
        let body = raw.trim_end_matches(['\n', '\r']);
        let is_te = body
            .split_once(':')
            .is_some_and(|(key, _)| key.trim() == "TE");
        if is_te && !replaced {
            out.push_str(&line);
            out.push_str(&raw[body.len()..]);
            replaced = true;
        } else {
            out.push_str(raw);
        }
    }
    if replaced {
        return out;
    }
    if out.trim().is_empty() {
        out = "GUANO|Version: 1.0\n".to_string();
    } else if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&line);
    out.push('\n');
    out
}

/// The change that sets GUANO's `TE` field of the WAV file `wav` to
/// `factor`: the file's `guan` chunk is rewritten where it is, or one is
/// added at the end of a file that has none, and the RIFF size is updated.
///
/// `Err` (one of [`msg`]) for a file that is not a plain RIFF WAV whose
/// chunk sizes account for every byte. Such a file is not changed, since a
/// patch could not be checked against it.
pub fn set_te(wav: &[u8], factor: f64) -> Result<FilePatch, &'static str> {
    if !is_riff_wave(wav) {
        return Err(msg::NOT_WAV);
    }
    if &wav[0..4] == b"RF64" {
        return Err(msg::RF64);
    }
    let len = wav.len() as u64;
    let riff_size = u64::from(u32::from_le_bytes([wav[4], wav[5], wav[6], wav[7]]));
    let mut end = 12u64;
    let mut guan = None;
    for chunk in riff_chunks(wav, 12) {
        if !chunk.complete() {
            return Err(msg::BAD_SIZES);
        }
        end = chunk.body_offset + chunk.size + (chunk.size & 1);
        if chunk.id == b"guan" && guan.is_none() {
            guan = Some(chunk);
        }
    }
    // The last chunk may lack its pad byte: some writers leave it out.
    let missing_pad = end == len + 1;
    if (end != len && !missing_pad) || riff_size != len - 8 {
        return Err(msg::BAD_SIZES);
    }

    let chunk_bytes = |text: &str| {
        let mut b = Vec::with_capacity(text.len() + 5);
        b.extend_from_slice(&(text.len() as u32).to_le_bytes());
        b.extend_from_slice(text.as_bytes());
        if text.len() % 2 == 1 {
            b.push(0);
        }
        b
    };
    let change = match guan {
        Some(chunk) => {
            let text = std::str::from_utf8(chunk.body).map_err(|_| msg::GUANO_NOT_TEXT)?;
            let start = chunk.body_offset - 4;
            let stop = (chunk.body_offset + chunk.size + (chunk.size & 1)).min(len);
            Splice {
                offset: start,
                old: wav[start as usize..stop as usize].to_vec(),
                new: chunk_bytes(&set_te_in_text(text, factor)),
            }
        }
        None => {
            let mut new = Vec::new();
            if missing_pad {
                new.push(0);
            }
            new.extend_from_slice(b"guan");
            new.extend_from_slice(&chunk_bytes(&set_te_in_text("", factor)));
            Splice {
                offset: len,
                old: Vec::new(),
                new,
            }
        }
    };
    let new_len = len as i64 + change.new.len() as i64 - change.old.len() as i64;
    let new_riff = u32::try_from(new_len - 8).map_err(|_| msg::TOO_LARGE)?;
    Ok(FilePatch {
        splices: vec![
            Splice {
                offset: 4,
                old: wav[4..8].to_vec(),
                new: new_riff.to_le_bytes().to_vec(),
            },
            change,
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::guano::parse_guano;

    /// A mono 16-bit WAV with `data_len` bytes of audio. `before` and
    /// `after` are whole chunks placed before and after `data`.
    fn wav(data_len: usize, before: &[u8], after: &[u8]) -> Vec<u8> {
        let mut b = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&[1, 0, 1, 0]);
        b.extend_from_slice(&38_400u32.to_le_bytes());
        b.extend_from_slice(&76_800u32.to_le_bytes());
        b.extend_from_slice(&[2, 0, 16, 0]);
        b.extend_from_slice(before);
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(data_len as u32).to_le_bytes());
        b.extend((0..data_len).map(|i| (i * 7) as u8));
        if data_len % 2 == 1 {
            b.push(0);
        }
        b.extend_from_slice(after);
        let size = (b.len() - 8) as u32;
        b[4..8].copy_from_slice(&size.to_le_bytes());
        b
    }

    fn guan(text: &str) -> Vec<u8> {
        let mut b = b"guan".to_vec();
        b.extend_from_slice(&(text.len() as u32).to_le_bytes());
        b.extend_from_slice(text.as_bytes());
        if text.len() % 2 == 1 {
            b.push(0);
        }
        b
    }

    fn te_of(bytes: &[u8]) -> Option<String> {
        parse_guano(bytes)?
            .fields
            .into_iter()
            .find(|(k, _)| k == "TE")
            .map(|(_, v)| v)
    }

    /// Applies the change, checks the result is a well-formed WAV with the
    /// factor, and that the inverse gives the original bytes back.
    fn check(original: &[u8], factor: f64, expect: &str) -> Vec<u8> {
        let patch = set_te(original, factor).unwrap();
        let changed = patch.apply(original).unwrap();
        assert_eq!(te_of(&changed).as_deref(), Some(expect));
        assert_eq!(changed.len() as i64, original.len() as i64 + patch.grown_by());
        // Still a file this module accepts: sizes account for every byte.
        assert!(set_te(&changed, factor).is_ok());
        assert_eq!(patch.inverse().apply(&changed).unwrap(), original);
        // The audio is not in the patch.
        assert!(patch.stored_len() < 200, "{}", patch.stored_len());
        changed
    }

    #[test]
    fn adds_a_guano_chunk_to_a_file_without_one() {
        for data_len in [100, 101] {
            let original = wav(data_len, &[], &[]);
            let changed = check(&original, 10.0, "10");
            assert!(changed.ends_with(b"GUANO|Version: 1.0\nTE: 10\n"));
        }
    }

    #[test]
    fn adds_the_pad_byte_a_writer_left_out() {
        let mut original = wav(101, &[], &[]);
        original.pop();
        let size = (original.len() - 8) as u32;
        original[4..8].copy_from_slice(&size.to_le_bytes());
        check(&original, 8.0, "8");
    }

    #[test]
    fn replaces_the_te_line_and_keeps_every_other_byte_of_the_text() {
        let text = "GUANO|Version: 1.0\r\nMake: Pettersson\r\nTE: \r\nNote: a: b\r\n";
        for original in [wav(64, &[], &guan(text)), wav(64, &guan(text), &[])] {
            let changed = check(&original, 2.5, "2.5");
            let body = parse_guano(&changed).unwrap();
            assert_eq!(body.fields.len(), 4);
            let at = changed.windows(8).position(|w| w == b"TE: 2.5\r").unwrap();
            assert!(changed[at..].starts_with(b"TE: 2.5\r\nNote: a: b\r\n"));
        }
    }

    #[test]
    fn adds_a_te_line_to_guano_text_that_has_none() {
        assert_eq!(
            set_te_in_text("GUANO|Version: 1.0\nMake: X", 20.0),
            "GUANO|Version: 1.0\nMake: X\nTE: 20\n"
        );
        assert_eq!(
            set_te_in_text("GUANO|Version: 1.0\nMake: X\n\0\0", 20.0),
            "GUANO|Version: 1.0\nMake: X\nTE: 20\n"
        );
        // Only the first TE line is the field.
        assert_eq!(
            set_te_in_text("TE: 1\nTE: 3\n", 10.0),
            "TE: 10\nTE: 3\n"
        );
        check(&wav(10, &[], &guan("GUANO|Version: 1.0\nMake: X\n")), 256.0, "256");
    }

    #[test]
    fn leaves_files_it_cannot_check() {
        assert_eq!(set_te(b"OggS and more bytes", 10.0), Err(msg::NOT_WAV));
        let good = wav(10, &[], &[]);
        let mut rf64 = good.clone();
        rf64[0..4].copy_from_slice(b"RF64");
        assert_eq!(set_te(&rf64, 10.0), Err(msg::RF64));
        let mut wrong_riff = good.clone();
        wrong_riff[4] ^= 1;
        assert_eq!(set_te(&wrong_riff, 10.0), Err(msg::BAD_SIZES));
        let mut trailing = good.clone();
        trailing.extend_from_slice(b"xyz");
        let size = (trailing.len() - 8) as u32;
        trailing[4..8].copy_from_slice(&size.to_le_bytes());
        assert_eq!(set_te(&trailing, 10.0), Err(msg::BAD_SIZES));
        let bad_text = wav(10, &[], &{
            let mut g = b"guan".to_vec();
            g.extend_from_slice(&2u32.to_le_bytes());
            g.extend_from_slice(&[0xff, 0xfe]);
            g
        });
        assert_eq!(set_te(&bad_text, 10.0), Err(msg::GUANO_NOT_TEXT));
    }

    #[test]
    fn a_patch_is_refused_on_another_file() {
        let original = wav(100, &[], &[]);
        let patch = set_te(&original, 10.0).unwrap();
        let other = wav(102, &[], &[]);
        assert!(patch.apply(&other).is_err());
        // And the undo is refused on a file changed since.
        let mut changed = patch.apply(&original).unwrap();
        let last = changed.len() - 2;
        changed[last] = b'9';
        assert!(patch.inverse().apply(&changed).is_err());
    }

    #[test]
    fn the_loader_reads_the_factor_written() {
        let changed = check(&wav(100, &[], &[]), 10.0, "10");
        let guano = parse_guano(&changed);
        assert_eq!(
            crate::types::declared_time_expansion(
                crate::types::AudioFormat::Wav,
                guano.as_ref(),
                None
            ),
            Some(10.0)
        );
    }
}
