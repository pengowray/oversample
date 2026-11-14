use crate::state::LoadedFile;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub struct TrackInfo {
    pub group_key: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SequenceInfo {
    /// Shared prefix identifying the sequence (e.g. "260212" or "site")
    pub sequence_key: String,
    /// Track label within the sequence (e.g. "5", "3-4", ""). Empty string for non-multitrack.
    pub track_label: String,
    /// Ordering number within the sequence
    pub sequence_number: u32,
    /// Gap in seconds since the previous file in the sequence ended.
    /// None for the first file in a sequence, or when timestamps are unavailable.
    pub gap_from_prev_secs: Option<f64>,
}

/// A file's links to the other half of an original + denoised copy pair.
/// Both fields are indices into the file list.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct DenoiseLinks {
    /// For a denoised copy: the loaded recording it was made from.
    pub original: Option<usize>,
    /// For an original: its loaded denoised copies, in file-list order.
    pub copies: Vec<usize>,
}

impl DenoiseLinks {
    /// The file the Denoised switch opens from this one: a copy's original,
    /// or an original's first copy. `None` when no partner is loaded.
    pub fn partner(&self) -> Option<usize> {
        self.original.or_else(|| self.copies.first().copied())
    }
}

/// Combined grouping info for a file: multitrack track info, sequence
/// membership, and its original + denoised copy links.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct FileGroupInfo {
    pub track: Option<TrackInfo>,
    pub sequence: Option<SequenceInfo>,
    pub denoise: DenoiseLinks,
}

/// What "Make Denoised Copy" adds to the end of a recording's stem:
/// "bat.flac" becomes "bat (denoised).wav".
pub const DENOISED_SUFFIX: &str = " (denoised)";

/// The file name without its extension.
fn file_stem(filename: &str) -> &str {
    filename
        .rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(filename)
}

/// For a denoised copy's name, the stem of the recording it was made from:
/// "bat (denoised).wav" gives `Some("bat")`. `None` for any other name.
pub fn denoised_original_stem(filename: &str) -> Option<&str> {
    file_stem(filename)
        .strip_suffix(DENOISED_SUFFIX)
        .filter(|stem| !stem.is_empty())
}

/// Find the original + denoised copy pairs among `names`.
///
/// A copy named "<stem> (denoised).<ext>" pairs with the first file in the
/// list whose stem is "<stem>", whatever its extension. The pairing uses only
/// the names, so a copy that was saved and opened again pairs up without a
/// project. Two copies of one recording both link to it. A copy of a copy
/// ("bat (denoised) (denoised).wav") links to the copy it was made from.
pub fn compute_denoise_links(names: &[String]) -> Vec<DenoiseLinks> {
    let mut first_with_stem: HashMap<&str, usize> = HashMap::new();
    for (i, name) in names.iter().enumerate() {
        first_with_stem.entry(file_stem(name)).or_insert(i);
    }
    let mut links = vec![DenoiseLinks::default(); names.len()];
    for (i, name) in names.iter().enumerate() {
        let Some(original) = denoised_original_stem(name)
            .and_then(|stem| first_with_stem.get(stem).copied())
            .filter(|&j| j != i)
        else {
            continue;
        };
        links[i].original = Some(original);
        links[original].copies.push(i);
    }
    links
}

/// Parse a filename to extract a track/channel suffix.
///
/// Recognises patterns like:
/// - `260305_0058_1-2.wav` → group_key="260305_0058", label="1-2"
/// - `recording_Ch1.flac` → group_key="recording", label="Ch1"
/// - `260227_0055_3 my recording.wav` → group_key="260227_0055", label="3"
/// - `site_004.wav` → group_key="site", label="004"
/// - `260305_0057_MIX.wav` → group_key="260305_0057", label="MIX"
pub fn parse_track_suffix(filename: &str) -> Option<TrackInfo> {
    // Strip extension
    let stem = filename
        .rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(filename);

    // Find last underscore — everything after it is the candidate segment
    let (prefix, segment) = stem.rsplit_once('_')?;

    // Don't match if prefix is empty
    if prefix.is_empty() {
        return None;
    }

    // Extract the leading "track" portion of the segment.
    // For renamed files like "3 my recording", take only the leading part.
    let track_part = segment.split_once(' ').map(|(t, _)| t).unwrap_or(segment);

    if track_part.is_empty() {
        return None;
    }

    // Pattern 1: channel range like "1-2", "3-4"
    if let Some((a, b)) = track_part.split_once('-') {
        if !a.is_empty()
            && a.chars().all(|c| c.is_ascii_digit())
            && !b.is_empty()
            && b.chars().all(|c| c.is_ascii_digit())
        {
            return Some(TrackInfo {
                group_key: prefix.to_string(),
                label: track_part.to_string(),
            });
        }
    }

    // Pattern 2: "Ch1", "ch2", "CH3" (case-insensitive)
    let lower = track_part.to_ascii_lowercase();
    if lower.starts_with("ch") && lower.len() > 2 && lower[2..].chars().all(|c| c.is_ascii_digit())
    {
        return Some(TrackInfo {
            group_key: prefix.to_string(),
            label: track_part.to_string(),
        });
    }

    // Pattern 3: "MIX" (Tascam mixdown track, case-insensitive)
    if lower == "mix" {
        return Some(TrackInfo {
            group_key: prefix.to_string(),
            label: track_part.to_string(),
        });
    }

    // Pattern 4: bare number like "3", "004"
    if track_part.chars().all(|c| c.is_ascii_digit()) {
        return Some(TrackInfo {
            group_key: prefix.to_string(),
            label: track_part.to_string(),
        });
    }

    None
}

/// Compute file groups from a list of filenames.
///
/// Returns a parallel Vec: `Some(TrackInfo)` for files that belong to a group
/// of 2+ files sharing the same `group_key`, `None` for singletons.
pub fn compute_file_groups(names: &[String]) -> Vec<Option<TrackInfo>> {
    // A denoised copy is never a channel of a multitrack recording. Its name
    // would otherwise parse like its original's ("rec_1 (denoised).wav" reads
    // as track "1"), because the parser ignores text after a space.
    let parsed: Vec<Option<TrackInfo>> = names
        .iter()
        .map(|n| {
            if denoised_original_stem(n).is_some() {
                None
            } else {
                parse_track_suffix(n)
            }
        })
        .collect();

    // Count occurrences per group_key
    let mut counts: HashMap<String, usize> = HashMap::new();
    for ti in parsed.iter().flatten() {
        *counts.entry(ti.group_key.clone()).or_insert(0) += 1;
    }

    // Only keep entries where group has 2+ members
    parsed
        .into_iter()
        .map(|opt| opt.filter(|ti| counts.get(&ti.group_key).copied().unwrap_or(0) >= 2))
        .collect()
}

/// Parse a filename stem (after stripping track suffix) to extract a sequence
/// prefix and number. Handles Tascam-style `YYMMDD_NNNN` and generic `prefix_NNN`.
///
/// Examples:
/// - `260212_0041` → Some(("260212", 41))
/// - `site_004`    → Some(("site", 4))
/// - `recording`   → None
fn parse_sequence_stem(stem: &str) -> Option<(String, u32)> {
    let (prefix, num_str) = stem.rsplit_once('_')?;
    if prefix.is_empty() || num_str.is_empty() {
        return None;
    }
    // The number part must be all digits
    if !num_str.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let num: u32 = num_str.parse().ok()?;
    Some((prefix.to_string(), num))
}

/// Get the effective stem for sequence detection: the group_key if the file
/// has a track suffix, otherwise the full filename stem.
fn sequence_stem(filename: &str) -> String {
    if let Some(ti) = parse_track_suffix(filename) {
        ti.group_key
    } else {
        // Use full stem (without extension)
        let stem = filename
            .rsplit_once('.')
            .map(|(s, _)| s)
            .unwrap_or(filename);
        // Also strip any trailing space+text (like "3 Metal lamp pole")
        stem.split_once(' ')
            .map(|(s, _)| s)
            .unwrap_or(stem)
            .to_string()
    }
}

/// Compute combined file group info (multitrack + sequence) for all loaded files.
///
/// Sequence detection:
/// 1. Strip track suffix to get the stem (e.g. "260212_0041")
/// 2. Parse stem into (prefix, number)
/// 3. Group files sharing the same prefix (and same track label, if multitrack)
/// 4. Require 2+ files to form a sequence
/// 5. Compute gaps between consecutive files using recording timestamps
pub fn compute_all_groups(names: &[String], files: &[LoadedFile]) -> Vec<FileGroupInfo> {
    let tracks = compute_file_groups(names);

    // Parse sequence stems for each file
    // A denoised copy is never part of a sequence. If it were, it would get
    // its original's sequence number and the two would share noise settings.
    let seq_parses: Vec<Option<(String, u32)>> = names
        .iter()
        .map(|name| {
            if denoised_original_stem(name).is_some() {
                return None;
            }
            let stem = sequence_stem(name);
            parse_sequence_stem(&stem)
        })
        .collect();

    // Build sequence key: (prefix, track_label_or_empty) → list of (file_index, seq_number)
    let mut seq_groups: HashMap<(String, String), Vec<(usize, u32)>> = HashMap::new();
    for (i, sp) in seq_parses.iter().enumerate() {
        if let Some((prefix, num)) = sp {
            let track_label = tracks[i]
                .as_ref()
                .map(|ti| ti.label.clone())
                .unwrap_or_default();
            seq_groups
                .entry((prefix.clone(), track_label))
                .or_default()
                .push((i, *num));
        }
    }

    // Sort each sequence group by sequence number and compute gaps
    let mut sequence_infos: Vec<Option<SequenceInfo>> = vec![None; names.len()];
    for ((prefix, track_label), mut members) in seq_groups {
        if members.len() < 2 {
            continue;
        }
        members.sort_by_key(|&(_, num)| num);

        for (pos, &(file_idx, seq_num)) in members.iter().enumerate() {
            let gap = if pos == 0 {
                None
            } else {
                let (prev_idx, _) = members[pos - 1];
                compute_gap(files, prev_idx, file_idx)
            };

            sequence_infos[file_idx] = Some(SequenceInfo {
                sequence_key: prefix.clone(),
                track_label: track_label.clone(),
                sequence_number: seq_num,
                gap_from_prev_secs: gap,
            });
        }
    }

    // Combine into FileGroupInfo
    tracks
        .into_iter()
        .zip(sequence_infos)
        .zip(compute_denoise_links(names))
        .map(|((track, sequence), denoise)| FileGroupInfo {
            track,
            sequence,
            denoise,
        })
        .collect()
}

/// Indices of every file linked to `idx` as an original or a denoised copy,
/// including `idx` itself: the original, all its copies, and copies of those
/// copies. Returns `[idx]` when the file has no loaded partner.
///
/// Per the cross-file state-scoping model (`scope.rs`): the pair shares the
/// view, the gain, the focus selections and the playhead, so the user can
/// compare the two. It does not share the noise settings.
pub fn denoise_pair_members(groups: &[FileGroupInfo], idx: usize) -> Vec<usize> {
    let mut members = vec![idx];
    let mut next = 0;
    while next < members.len() {
        if let Some(g) = groups.get(members[next]) {
            for j in g.denoise.original.iter().chain(g.denoise.copies.iter()) {
                if !members.contains(j) {
                    members.push(*j);
                }
            }
        }
        next += 1;
    }
    members.sort_unstable();
    members
}

/// The file the Denoised switch opens from `idx` (see `DenoiseLinks::partner`).
pub fn denoise_partner(groups: &[FileGroupInfo], idx: usize) -> Option<usize> {
    groups.get(idx)?.denoise.partner()
}

/// Move every denoised copy in `order` (a list of file indices) to directly
/// after its original, so the file list can show it indented under the
/// original. Copies of one original keep their relative order.
pub fn place_copies_after_originals(order: &[usize], groups: &[FileGroupInfo]) -> Vec<usize> {
    fn push_with_copies(i: usize, groups: &[FileGroupInfo], order: &[usize], out: &mut Vec<usize>) {
        if out.contains(&i) {
            return;
        }
        out.push(i);
        for &c in order {
            if groups.get(c).and_then(|g| g.denoise.original) == Some(i) {
                push_with_copies(c, groups, order, out);
            }
        }
    }
    let mut out = Vec::with_capacity(order.len());
    for &i in order {
        let has_original = groups.get(i).is_some_and(|g| g.denoise.original.is_some());
        if !has_original {
            push_with_copies(i, groups, order, &mut out);
        }
    }
    // Only reachable if links form a loop, which names cannot produce.
    for &i in order {
        if !out.contains(&i) {
            out.push(i);
        }
    }
    out
}

/// Indices of every file in `idx`'s MULTITRACK group — simultaneous channels of
/// one recording, sharing `track.group_key` — including `idx` itself. Returns
/// `[idx]` when the file isn't part of a multitrack group.
///
/// Per the cross-file state-scoping model: multitrack groups share VIEWPORT
/// settings (horizontal position + vertical frequency range).
pub fn multitrack_members(groups: &[FileGroupInfo], idx: usize) -> Vec<usize> {
    let Some(key) = groups
        .get(idx)
        .and_then(|g| g.track.as_ref())
        .map(|t| t.group_key.clone())
    else {
        return vec![idx];
    };
    (0..groups.len())
        .filter(|&i| groups[i].track.as_ref().map(|t| t.group_key.as_str()) == Some(key.as_str()))
        .collect()
}

/// Indices of every file in `idx`'s SEQUENTIAL group — consecutive recordings in
/// a series on the same channel, sharing `sequence.sequence_key` AND
/// `track_label` — including `idx` itself. Returns `[idx]` when the file isn't
/// part of a sequence.
///
/// Per the cross-file state-scoping model: sequential groups share AUDIO-CHARACTER
/// settings (gain + denoise) — same mic/session/environment.
pub fn sequential_members(groups: &[FileGroupInfo], idx: usize) -> Vec<usize> {
    let Some((seq_key, track_label)) = groups
        .get(idx)
        .and_then(|g| g.sequence.as_ref())
        .map(|s| (s.sequence_key.clone(), s.track_label.clone()))
    else {
        return vec![idx];
    };
    (0..groups.len())
        .filter(|&i| {
            groups[i]
                .sequence
                .as_ref()
                .map(|s| (s.sequence_key.as_str(), s.track_label.as_str()))
                == Some((seq_key.as_str(), track_label.as_str()))
        })
        .collect()
}

/// Compute the time gap between two files: how many seconds between the end of
/// `prev` and the start of `next`. Returns None if timestamps are unavailable.
fn compute_gap(files: &[LoadedFile], prev_idx: usize, next_idx: usize) -> Option<f64> {
    let prev = files.get(prev_idx)?;
    let next = files.get(next_idx)?;

    let prev_start_ms = prev.recording_start_epoch_ms()?;
    let prev_end_ms = prev_start_ms + prev.audio.duration_secs * 1000.0;
    let next_start_ms = next.recording_start_epoch_ms()?;

    Some((next_start_ms - prev_end_ms) / 1000.0)
}

#[cfg(test)]
mod scope_tests {
    use super::*;

    fn ti(key: &str, label: &str) -> Option<TrackInfo> {
        Some(TrackInfo {
            group_key: key.into(),
            label: label.into(),
        })
    }
    fn si(key: &str, label: &str, n: u32) -> Option<SequenceInfo> {
        Some(SequenceInfo {
            sequence_key: key.into(),
            track_label: label.into(),
            sequence_number: n,
            gap_from_prev_secs: None,
        })
    }

    #[test]
    fn multitrack_members_group_by_track_key() {
        // 0,1 are channels of recA; 2 is a lone file.
        let groups = vec![
            FileGroupInfo {
                track: ti("recA", "1"),
                sequence: None,
                ..Default::default()
            },
            FileGroupInfo {
                track: ti("recA", "2"),
                sequence: None,
                ..Default::default()
            },
            FileGroupInfo {
                track: None,
                sequence: None,
                ..Default::default()
            },
        ];
        assert_eq!(multitrack_members(&groups, 0), vec![0, 1]);
        assert_eq!(multitrack_members(&groups, 1), vec![0, 1]);
        assert_eq!(multitrack_members(&groups, 2), vec![2]); // singleton
    }

    #[test]
    fn sequential_members_group_by_seq_key_and_track_label() {
        // (site,"1") has takes 0 and 2; file 1 is track "2", alone in its sequence.
        let groups = vec![
            FileGroupInfo {
                track: ti("site", "1"),
                sequence: si("site", "1", 1),
                ..Default::default()
            },
            FileGroupInfo {
                track: ti("site", "2"),
                sequence: si("site", "2", 1),
                ..Default::default()
            },
            FileGroupInfo {
                track: ti("site", "1"),
                sequence: si("site", "1", 2),
                ..Default::default()
            },
        ];
        assert_eq!(sequential_members(&groups, 0), vec![0, 2]);
        assert_eq!(sequential_members(&groups, 2), vec![0, 2]);
        assert_eq!(sequential_members(&groups, 1), vec![1]);
    }

    #[test]
    fn multitrack_and_sequential_are_orthogonal() {
        // One multitrack recording (recA, channels 1 & 2), no sequence.
        let groups = vec![
            FileGroupInfo {
                track: ti("recA", "1"),
                sequence: None,
                ..Default::default()
            },
            FileGroupInfo {
                track: ti("recA", "2"),
                sequence: None,
                ..Default::default()
            },
        ];
        // Multitrack groups them; sequential does not (each is its own sequence).
        assert_eq!(multitrack_members(&groups, 0), vec![0, 1]);
        assert_eq!(sequential_members(&groups, 0), vec![0]);
        assert_eq!(sequential_members(&groups, 1), vec![1]);
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn denoised_copy_names_give_the_original_stem() {
        assert_eq!(denoised_original_stem("bat (denoised).wav"), Some("bat"));
        assert_eq!(denoised_original_stem("bat (denoised)"), Some("bat"));
        assert_eq!(
            denoised_original_stem("XC1 - Bat (denoised).wav"),
            Some("XC1 - Bat")
        );
        assert_eq!(denoised_original_stem("bat.wav"), None);
        assert_eq!(denoised_original_stem("bat (denoised) take 2.wav"), None);
        assert_eq!(denoised_original_stem(" (denoised).wav"), None);
        // The suffix is the one `denoised_name` writes.
        assert_eq!(
            denoised_original_stem(&crate::denoise::denoised_name("XC1 - Bat.flac")),
            Some("XC1 - Bat")
        );
    }

    #[test]
    fn a_copy_pairs_with_its_original_by_name() {
        // The extension of the original does not matter, nor the load order.
        let n = names(&["other.wav", "bat (denoised).wav", "bat.flac"]);
        let groups = compute_all_groups(&n, &[]);
        assert_eq!(groups[1].denoise.original, Some(2));
        assert_eq!(groups[2].denoise.copies, vec![1]);
        assert_eq!(denoise_pair_members(&groups, 1), vec![1, 2]);
        assert_eq!(denoise_pair_members(&groups, 2), vec![1, 2]);
        assert_eq!(denoise_pair_members(&groups, 0), vec![0]);
        assert_eq!(denoise_partner(&groups, 1), Some(2));
        assert_eq!(denoise_partner(&groups, 2), Some(1));
        assert_eq!(denoise_partner(&groups, 0), None);
    }

    #[test]
    fn a_copy_without_its_original_has_no_partner() {
        let groups = compute_all_groups(&names(&["bat (denoised).wav", "cat.wav"]), &[]);
        assert_eq!(denoise_partner(&groups, 0), None);
        assert_eq!(denoise_pair_members(&groups, 0), vec![0]);
    }

    #[test]
    fn two_copies_and_a_copy_of_a_copy() {
        let n = names(&[
            "bat.wav",
            "bat (denoised).wav",
            "bat (denoised).wav",
            "bat (denoised) (denoised).wav",
        ]);
        let groups = compute_all_groups(&n, &[]);
        // The original flips to its first copy; every copy flips to what it
        // was made from.
        assert_eq!(denoise_partner(&groups, 0), Some(1));
        assert_eq!(denoise_partner(&groups, 1), Some(0));
        assert_eq!(denoise_partner(&groups, 2), Some(0));
        assert_eq!(denoise_partner(&groups, 3), Some(1));
        for i in 0..4 {
            assert_eq!(denoise_pair_members(&groups, i), vec![0, 1, 2, 3]);
        }
    }

    #[test]
    fn same_stem_with_two_extensions_pairs_with_the_first() {
        let n = names(&["bat.wav", "bat.flac", "bat (denoised).wav"]);
        let groups = compute_all_groups(&n, &[]);
        assert_eq!(denoise_partner(&groups, 2), Some(0));
        assert_eq!(denoise_partner(&groups, 0), Some(2));
        assert_eq!(denoise_partner(&groups, 1), None);
    }

    #[test]
    fn a_copy_joins_no_multitrack_group_or_sequence() {
        // Recorder-style names: two takes, two channels each, and a copy of one.
        let n = names(&[
            "260212_0041_1.wav",
            "260212_0041_2.wav",
            "260212_0042_1.wav",
            "260212_0042_2.wav",
            "260212_0041_1 (denoised).wav",
        ]);
        let groups = compute_all_groups(&n, &[]);
        assert!(groups[4].track.is_none());
        assert!(groups[4].sequence.is_none());
        assert_eq!(groups[4].denoise.original, Some(0));
        // The originals group as they did before.
        assert_eq!(multitrack_members(&groups, 0), vec![0, 1]);
        assert_eq!(sequential_members(&groups, 0), vec![0, 2]);
        assert_eq!(sequential_members(&groups, 4), vec![4]);

        // A Tascam-style name: without the rule the copy would read as a
        // second track "3" and as a two-file sequence with its original.
        let n = names(&["260305_0058_3.wav", "260305_0058_3 (denoised).wav"]);
        let groups = compute_all_groups(&n, &[]);
        for g in &groups {
            assert!(g.track.is_none());
            assert!(g.sequence.is_none());
        }
        assert_eq!(denoise_pair_members(&groups, 0), vec![0, 1]);

        // An original with no track suffix and its copy are not two channels.
        let n = names(&["260212_0041.wav", "260212_0041 (denoised).wav"]);
        let groups = compute_all_groups(&n, &[]);
        assert!(groups[0].track.is_none());
        assert!(groups[1].track.is_none());
        assert!(groups[0].sequence.is_none());
    }

    #[test]
    fn copies_are_listed_after_their_originals() {
        let n = names(&[
            "bat (denoised).wav",
            "cat.wav",
            "bat.wav",
            "bat (denoised) (denoised).wav",
        ]);
        let groups = compute_all_groups(&n, &[]);
        assert_eq!(
            place_copies_after_originals(&[0, 1, 2, 3], &groups),
            vec![1, 2, 0, 3]
        );
        assert_eq!(
            place_copies_after_originals(&[3, 2, 1, 0], &groups),
            vec![2, 0, 3, 1]
        );
    }
}
