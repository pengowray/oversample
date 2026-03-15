use std::collections::HashMap;
use crate::state::LoadedFile;

#[derive(Clone, Debug, PartialEq)]
pub struct TrackInfo {
    pub group_key: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SequenceInfo {
    /// Shared prefix identifying the sequence (e.g. "260212" or "site")
    pub sequence_key: String,
    /// Ordering number within the sequence
    pub sequence_number: u32,
    /// Gap in seconds since the previous file in the sequence ended.
    /// None for the first file in a sequence, or when timestamps are unavailable.
    pub gap_from_prev_secs: Option<f64>,
}

/// Combined grouping info for a file: multitrack track info and/or sequence membership.
#[derive(Clone, Debug, PartialEq)]
pub struct FileGroupInfo {
    pub track: Option<TrackInfo>,
    pub sequence: Option<SequenceInfo>,
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
    let stem = filename.rsplit_once('.').map(|(s, _)| s).unwrap_or(filename);

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
        if !a.is_empty() && a.chars().all(|c| c.is_ascii_digit())
            && !b.is_empty() && b.chars().all(|c| c.is_ascii_digit())
        {
            return Some(TrackInfo {
                group_key: prefix.to_string(),
                label: track_part.to_string(),
            });
        }
    }

    // Pattern 2: "Ch1", "ch2", "CH3" (case-insensitive)
    let lower = track_part.to_ascii_lowercase();
    if lower.starts_with("ch") && lower.len() > 2 && lower[2..].chars().all(|c| c.is_ascii_digit()) {
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
    let parsed: Vec<Option<TrackInfo>> = names.iter().map(|n| parse_track_suffix(n)).collect();

    // Count occurrences per group_key
    let mut counts: HashMap<String, usize> = HashMap::new();
    for info in &parsed {
        if let Some(ti) = info {
            *counts.entry(ti.group_key.clone()).or_insert(0) += 1;
        }
    }

    // Only keep entries where group has 2+ members
    parsed
        .into_iter()
        .map(|opt| {
            opt.filter(|ti| counts.get(&ti.group_key).copied().unwrap_or(0) >= 2)
        })
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
        let stem = filename.rsplit_once('.').map(|(s, _)| s).unwrap_or(filename);
        // Also strip any trailing space+text (like "3 Metal lamp pole")
        stem.split_once(' ').map(|(s, _)| s).unwrap_or(stem).to_string()
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
pub fn compute_all_groups(
    names: &[String],
    files: &[LoadedFile],
) -> Vec<FileGroupInfo> {
    let tracks = compute_file_groups(names);

    // Parse sequence stems for each file
    let seq_parses: Vec<Option<(String, u32)>> = names.iter().map(|name| {
        let stem = sequence_stem(name);
        parse_sequence_stem(&stem)
    }).collect();

    // Build sequence key: (prefix, track_label_or_empty) → list of (file_index, seq_number)
    let mut seq_groups: HashMap<(String, String), Vec<(usize, u32)>> = HashMap::new();
    for (i, sp) in seq_parses.iter().enumerate() {
        if let Some((prefix, num)) = sp {
            let track_label = tracks[i].as_ref()
                .map(|ti| ti.label.clone())
                .unwrap_or_default();
            seq_groups.entry((prefix.clone(), track_label))
                .or_default()
                .push((i, *num));
        }
    }

    // Sort each sequence group by sequence number and compute gaps
    let mut sequence_infos: Vec<Option<SequenceInfo>> = vec![None; names.len()];
    for ((prefix, _track_label), mut members) in seq_groups {
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
                sequence_number: seq_num,
                gap_from_prev_secs: gap,
            });
        }
    }

    // Combine into FileGroupInfo
    tracks.into_iter().zip(sequence_infos).map(|(track, sequence)| {
        FileGroupInfo { track, sequence }
    }).collect()
}

/// Compute the time gap between two files: how many seconds between the end of
/// `prev` and the start of `next`. Returns None if timestamps are unavailable.
fn compute_gap(files: &[LoadedFile], prev_idx: usize, next_idx: usize) -> Option<f64> {
    let prev = &files[prev_idx];
    let next = &files[next_idx];

    let prev_start_ms = prev.recording_start_epoch_ms()?;
    let prev_end_ms = prev_start_ms + prev.audio.duration_secs * 1000.0;
    let next_start_ms = next.recording_start_epoch_ms()?;

    Some((next_start_ms - prev_end_ms) / 1000.0)
}
