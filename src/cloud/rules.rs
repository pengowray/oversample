//! The decisions of the Google Drive copy, as plain functions of plain facts,
//! so that `cargo test` can check them on the host.
//!
//! Local storage (OPFS) is the source of truth. For each file key the app
//! remembers what the Drive copy held when this device last uploaded,
//! downloaded or compared it (its fingerprint and revision). That memory is
//! the common base of a three-way comparison:
//!
//! - Equal content on both sides: nothing to do.
//! - Only the local copy changed since the base: upload.
//! - Only the Drive copy changed since the base: download.
//! - Both changed: annotation sets are merged annotation by annotation
//!   against the base (`merge::merge3`; the base is kept as a summary of
//!   hashes, `merge::MergeBase`), and the user is told. Projects: the copy
//!   with the later `modified_at` (the time stamped inside the file) wins.
//! - No base (the first comparison of this key since sign-in on this device):
//!   annotation sets are merged, so that nothing is lost: the Drive copy is
//!   kept and the local annotations it does not have are added to it.
//!   Projects use the later `modified_at`.
//!
//! The merge is what keeps a second device from overwriting Drive with an
//! empty or untouched set: a set with no annotations of its own adds nothing,
//! so the result equals the Drive copy and nothing is uploaded.
//!
//! The fingerprint used for these decisions (`content_fingerprint`) covers
//! the whole set except what changes on every save or differs between
//! devices for the same recording (see its notes).

use crate::annotations::{Annotation, AnnotationSet};
use crate::project::BatProject;
use serde::Serialize;

pub mod merge;

/// What the app does with one file key after comparing the two copies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Both copies hold the same content.
    Nothing,
    /// Write the local copy to Drive.
    Upload,
    /// Replace the local copy with the Drive copy.
    Download,
    /// Annotation sets only. Both copies changed since the base: merge them
    /// annotation by annotation (`merge::merge3`). With no base (the first
    /// comparison on this device): keep the Drive copy and add the local
    /// annotations it does not have ([`merge_annotation_sets`]).
    Merge,
    /// The Drive file was deleted, and the copy on this device is the one
    /// last synced: remove it from the account's cache on this device.
    RemoveLocal,
}

/// What Drive holds for a key.
#[derive(Clone, Copy, Debug)]
pub enum DriveSide<'a> {
    /// A file with the key.
    Present(FileCopy<'a>),
    /// The file last seen for this key was deleted or trashed: Drive said so
    /// for its id (a missing entry in a listing is never taken as this).
    Gone,
    /// No file, and none was seen before.
    Absent,
}

/// The kind of file, which sets the rule for a first comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Annotations,
    Project,
}

/// One copy of a file, as far as the decision needs it.
#[derive(Clone, Copy, Debug)]
pub struct FileCopy<'a> {
    pub fingerprint: &'a str,
    /// `modified_at` from inside the file (ISO 8601, UTC).
    pub modified_at: Option<&'a str>,
}

/// Compare the local copy and the Drive copy of one key.
///
/// `base` is the fingerprint of the Drive copy as this device last saw it,
/// or `None` when this device has not compared this key since sign-in.
pub fn decide(
    kind: Kind,
    local: Option<FileCopy<'_>>,
    drive: Option<FileCopy<'_>>,
    base: Option<&str>,
) -> Action {
    let side = match drive {
        Some(d) => DriveSide::Present(d),
        None => DriveSide::Absent,
    };
    decide_with(kind, local, side, base)
}

/// `decide`, with the Drive side able to say Gone. Drive is the authority:
/// a Drive file that was deleted takes the copy on this device with it,
/// unless the copy changed since it was last synced (an edit beats a
/// delete, and the file is uploaded again).
pub fn decide_with(
    kind: Kind,
    local: Option<FileCopy<'_>>,
    drive: DriveSide<'_>,
    base: Option<&str>,
) -> Action {
    let drive = match drive {
        DriveSide::Present(d) => Some(d),
        DriveSide::Absent => None,
        DriveSide::Gone => {
            return match (local, base) {
                (None, _) => Action::Nothing,
                (Some(l), Some(b)) if l.fingerprint == b => Action::RemoveLocal,
                (Some(_), _) => Action::Upload,
            }
        }
    };
    let (local, drive) = match (local, drive) {
        (None, None) => return Action::Nothing,
        (Some(_), None) => return Action::Upload,
        (None, Some(_)) => return Action::Download,
        (Some(l), Some(d)) => (l, d),
    };
    if local.fingerprint == drive.fingerprint {
        return Action::Nothing;
    }
    let Some(base) = base else {
        return match kind {
            Kind::Annotations => Action::Merge,
            Kind::Project => later_wins(local, drive),
        };
    };
    let local_changed = local.fingerprint != base;
    let drive_changed = drive.fingerprint != base;
    match (local_changed, drive_changed) {
        (true, false) => Action::Upload,
        (false, _) => Action::Download,
        (true, true) => match kind {
            Kind::Annotations => Action::Merge,
            Kind::Project => later_wins(local, drive),
        },
    }
}

/// Both copies changed: the later `modified_at` wins. On a tie, or when the
/// local copy has no time, the Drive copy wins.
fn later_wins(local: FileCopy<'_>, drive: FileCopy<'_>) -> Action {
    match (local.modified_at, drive.modified_at) {
        (Some(l), Some(d)) if l > d => Action::Upload,
        (Some(_), None) => Action::Upload,
        _ => Action::Download,
    }
}

/// Whether an upload may replace the Drive file whose revision is now
/// `revision_now`: only if it is still the revision this device last saw.
/// Otherwise the Drive copy changed since, and the two copies are compared
/// again first.
pub fn may_overwrite(revision_now: Option<&str>, last_seen: Option<&str>) -> bool {
    match (revision_now, last_seen) {
        (Some(now), Some(seen)) => now == seen,
        _ => false,
    }
}

/// Of several Drive files with the same key (two devices each created one),
/// the one with the latest Drive `modifiedTime`. The others are left alone.
/// Drive writes `modifiedTime` as RFC 3339 in UTC with milliseconds, so the
/// strings sort in time order.
pub fn newest<T>(files: &[T], modified_time: impl Fn(&T) -> &str) -> Option<&T> {
    files
        .iter()
        .max_by(|a, b| modified_time(a).cmp(modified_time(b)))
}

/// The Drive copy, with the annotations of the local copy that it does not
/// have (by annotation id) added. Annotations that came from Xeno-canto are
/// not added: the app adds them again when the recording opens, and the
/// Drive copy may hold them under other ids. The file's own lowest bat
/// frequency and noise profile come from the Drive copy, or from the local
/// copy when the Drive copy has none. The recording's source is the one
/// fetched later (`merge::later_source`).
///
/// An annotation deleted on one device and still present on the other comes
/// back. This is the price of not losing any annotation when there is no
/// common base to tell a deletion from an addition.
pub fn merge_annotation_sets(drive: AnnotationSet, local: &AnnotationSet) -> AnnotationSet {
    let mut merged = drive;
    crate::annota::carry_over_unsaved(&mut merged.annotations, local.annotations.clone());
    if merged.lowest_bat_hz.is_none() {
        merged.lowest_bat_hz = local.lowest_bat_hz;
    }
    if merged.time_expansion.is_none() {
        merged.time_expansion = local.time_expansion;
    }
    if merged.noise_profile.is_none() {
        merged.noise_profile = local.noise_profile.clone();
    }
    merged.source = merge::later_source(merged.source.as_ref(), local.source.as_ref()).cloned();
    merged
}

/// Fingerprint of a whole annotation set, for every sync decision: whether
/// the copies match, whether an upload is needed, and whether a copy may be
/// removed. Everything counts except what changes on every save or differs
/// between devices for the same file: the set's id, its times, the app
/// version, when the noise profile was last written or the source fetched, and the recording's
/// file time and path on this device. So a hash computed later, or other
/// identity and metadata changes, reach Drive, and a copy is never removed
/// while anything in it has not reached Drive. 32 hex digits.
pub fn content_fingerprint(set: &AnnotationSet) -> String {
    let mut s = set.clone();
    s.id = String::new();
    s.app_version = String::new();
    s.created_at = None;
    s.modified_at = None;
    if let Some(p) = &mut s.noise_profile {
        p.created = String::new();
    }
    if let Some(src) = &mut s.source {
        src.fetched_at = String::new();
    }
    s.file_identity.last_modified = None;
    s.file_identity.file_path = None;
    hash_json(&s)
}

/// Whether a copy in the account's cache may be removed now (its Drive file
/// was deleted): only if every copy of it on this device, the file in the
/// cache (`on_disk`) and an open file's set (`open`), still has the
/// fingerprint of the content last synced (`base`). Anything else is an
/// edit that has not reached Drive, and is uploaded instead. Checked under
/// the file's lock, right before the removal.
pub fn may_remove(base: &str, on_disk: Option<&str>, open: Option<&str>) -> bool {
    !base.is_empty() && on_disk.is_none_or(|f| f == base) && open.is_none_or(|f| f == base)
}

/// Whether the removals found in one comparison of all files may be made:
/// not when the "Oversample" folder itself is no longer visible, and not
/// when most of the files that were synced before came back as deleted (a
/// client also gets "not found" for files it can no longer see). A few
/// deletions out of many go ahead.
pub fn removals_allowed(folder_visible: bool, synced_before: usize, gone: usize) -> bool {
    folder_visible && (gone < 3 || gone * 2 <= synced_before)
}

/// The keys (`kind:key`) of the old per-browser record that move into an
/// account's cache: all of them when the record was made by the account now
/// signed in (compared by email, which is all the old record kept), none
/// otherwise.
pub fn keys_to_migrate<'a>(
    record_account: &str,
    signed_in_email: &str,
    keys: impl Iterator<Item = &'a str>,
) -> Vec<String> {
    if record_account.is_empty() || !record_account.eq_ignore_ascii_case(signed_in_email) {
        return Vec::new();
    }
    keys.map(str::to_string).collect()
}

/// Fingerprint of what a user changes in an annotation set (see the module
/// notes). 32 hex digits. Used for when a save stamps `modified_at`, not for
/// sync decisions (`content_fingerprint`).
pub fn annotation_fingerprint(set: &AnnotationSet) -> String {
    #[derive(Serialize)]
    struct Content<'a> {
        annotations: &'a [Annotation],
        lowest_bat_hz: Option<f32>,
        /// Only the factor the user set: the source's factor is the site's
        /// data, and saving it is not an edit.
        #[serde(skip_serializing_if = "Option::is_none")]
        user_time_expansion: Option<f64>,
        noise_profile: Option<crate::dsp::notch::NoiseProfile>,
    }
    let noise_profile = set.noise_profile.clone().map(|mut p| {
        p.created = String::new();
        p
    });
    let content = Content {
        annotations: &set.annotations,
        lowest_bat_hz: set.lowest_bat_hz,
        user_time_expansion: set.time_expansion.and_then(|t| t.set_by_user),
        noise_profile,
    };
    hash_json(&content)
}

/// Fingerprint of a project: all of it except `modified_at`,
/// `app_version` and the recordings' paths (which only the desktop app's
/// copy holds, and Drive never gets). 32 hex digits.
pub fn project_fingerprint(project: &BatProject) -> String {
    let mut p = without_device_fields(project);
    p.modified_at = None;
    p.app_version = String::new();
    hash_json(&p)
}

fn without_paths(project: &BatProject) -> BatProject {
    let mut p = project.clone();
    for f in &mut p.files {
        f.identity.file_path = None;
    }
    p
}

/// `without_paths`, also without the recordings' file times (which differ
/// between devices), for the fingerprint.
fn without_device_fields(project: &BatProject) -> BatProject {
    let mut p = without_paths(project);
    for f in &mut p.files {
        f.identity.last_modified = None;
    }
    p
}

/// The text uploaded to Drive for a project: its YAML without the
/// recordings' paths (`files[].identity.file_path`, desktop only). Every
/// project upload goes through this.
pub fn project_drive_text(project: &BatProject) -> Option<String> {
    yaml_serde::to_string(&without_paths(project)).ok()
}

/// A project downloaded from Drive, with the recordings' paths on this
/// device put back from the local copy (matched as `BatProject::find_file`
/// matches files).
pub fn with_local_paths(mut from_drive: BatProject, local: Option<&BatProject>) -> BatProject {
    let Some(local) = local else {
        return from_drive;
    };
    for f in &mut from_drive.files {
        if let Some(i) = local.find_file(&f.identity) {
            f.identity.file_path = local.files[i].identity.file_path.clone();
        }
    }
    from_drive
}

fn hash_json(value: &impl Serialize) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    let hex = blake3::hash(&bytes).to_hex();
    hex[..32].to_string()
}

/// The text uploaded to Drive for an annotation set: its YAML without
/// `file_identity.file_path` (the desktop app's central copies hold the
/// recording's full path, which must not leave the device). Every upload
/// goes through this.
pub fn drive_text(set: &AnnotationSet) -> Option<String> {
    let mut set = set.clone();
    set.file_identity.file_path = None;
    yaml_serde::to_string(&set).ok()
}

/// Drive limits an `appProperties` key and value to 124 bytes together.
const APP_PROPERTY_BYTES: usize = 124;

/// The value stored in the Drive file's `osKey` property for a local key:
/// the key itself, or `h_` and a hash of it when the key is too long (only
/// name-and-size keys of very long file names are).
pub fn drive_key(local_key: &str) -> String {
    if KEY_PROPERTY.len() + local_key.len() <= APP_PROPERTY_BYTES {
        local_key.to_string()
    } else {
        let hex = blake3::hash(local_key.as_bytes()).to_hex();
        format!("h_{}", &hex[..40])
    }
}

/// Name of the Drive property that holds the key.
pub const KEY_PROPERTY: &str = "osKey";

/// Whether `key` may be used as a file name in local storage: a Drive file
/// could carry any text in its key property. Only the characters the app's
/// own keys use are allowed (letters, digits, `.`, `-`, `_`), so no path
/// separator, drive letter (`C:`) or `..` can reach a file path.
pub fn local_key_is_safe(key: &str, extension: &str) -> bool {
    !key.is_empty()
        && key.len() <= 255
        && key.ends_with(extension)
        && !key.starts_with('.')
        && !key.contains("..")
        && key
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// The local key of a Drive annotation file: its key property, or for a
/// hashed property, the key worked out again from the file identity inside
/// the set (`candidates`) that hashes to the same value.
pub fn local_annotation_key(drive_property: &str, candidates: &[String]) -> Option<String> {
    if !drive_property.starts_with("h_") {
        return local_key_is_safe(drive_property, ".batm").then(|| drive_property.to_string());
    }
    candidates
        .iter()
        .find(|c| drive_key(c) == drive_property && local_key_is_safe(c, ".batm"))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_yaml(modified_at: &str, regions: &[(&str, f64)]) -> String {
        let mut yaml = format!(
            "version: 3\nid: set-1\napp_version: 0.5.0\ncreated_at: 2026-01-01T00:00:00.000Z\n\
             modified_at: {modified_at}\nfile_identity:\n  filename: bat.wav\n  file_size: 1000\n\
             annotations:\n"
        );
        if regions.is_empty() {
            yaml.push_str("  []\n");
        }
        for (id, start) in regions {
            yaml.push_str(&format!(
                "- id: {id}\n  type: Region\n  time_start: {start}\n  time_end: {}\n  \
                 created_at: 2026-01-01T00:00:00.000Z\n  modified_at: 2026-01-01T00:00:00.000Z\n",
                start + 0.5
            ));
        }
        yaml
    }

    fn set(modified_at: &str, regions: &[(&str, f64)]) -> AnnotationSet {
        yaml_serde::from_str(&set_yaml(modified_at, regions)).expect("test YAML parses")
    }

    fn copy<'a>(fp: &'a str, at: &'a str) -> FileCopy<'a> {
        FileCopy {
            fingerprint: fp,
            modified_at: Some(at),
        }
    }

    const T1: &str = "2026-11-01T10:00:00.000Z";
    const T2: &str = "2026-11-02T10:00:00.000Z";

    #[test]
    fn the_upload_text_never_holds_the_recordings_path() {
        let mut set = set(T1, &[("a", 1.0)]);
        set.file_identity.file_path = Some("/home/someone/bats/night 3/pond.wav".into());
        let text = drive_text(&set).unwrap();
        assert!(!text.contains("file_path"));
        assert!(!text.contains("/home/someone"));
        assert!(text.contains("time_start"));
        // The set itself is left as it was.
        assert!(set.file_identity.file_path.is_some());
    }

    #[test]
    fn a_hash_computed_later_counts_as_a_change_but_device_fields_do_not() {
        let a = set(T1, &[("a", 1.0)]);
        // Later hash: a different content fingerprint, so it uploads.
        let mut hashed = a.clone();
        hashed.file_identity.content_hash = Some("abc".into());
        assert_ne!(content_fingerprint(&a), content_fingerprint(&hashed));
        // The edit fingerprint (for modified_at) does not see it.
        assert_eq!(annotation_fingerprint(&a), annotation_fingerprint(&hashed));
        // Per-device and per-save fields do not count.
        let mut other_device = a.clone();
        other_device.file_identity.last_modified = Some("1700000000000".into());
        other_device.file_identity.file_path = Some("/home/x/a.wav".into());
        other_device.modified_at = Some(T2.into());
        other_device.app_version = "9.9.9".into();
        other_device.id = "another".into();
        assert_eq!(content_fingerprint(&a), content_fingerprint(&other_device));
    }

    #[test]
    fn a_file_deleted_on_drive_is_removed_here_only_if_unchanged() {
        let base = "base-fp";
        let unchanged = copy(base, T1);
        let edited = copy("edited-fp", T2);
        let k = Kind::Annotations;
        assert_eq!(
            decide_with(k, Some(unchanged), DriveSide::Gone, Some(base)),
            Action::RemoveLocal
        );
        // An edit beats the delete: upload again.
        assert_eq!(
            decide_with(k, Some(edited), DriveSide::Gone, Some(base)),
            Action::Upload
        );
        assert_eq!(
            decide_with(k, None, DriveSide::Gone, Some(base)),
            Action::Nothing
        );
        // Never seen on Drive: a new file, uploaded.
        assert_eq!(
            decide_with(k, Some(edited), DriveSide::Absent, None),
            Action::Upload
        );
        assert_eq!(
            decide_with(Kind::Project, Some(unchanged), DriveSide::Gone, Some(base)),
            Action::RemoveLocal
        );
    }

    #[test]
    fn a_copy_with_any_change_since_the_last_sync_is_never_removed() {
        assert!(may_remove("b", Some("b"), None));
        assert!(may_remove("b", Some("b"), Some("b")));
        assert!(may_remove("b", None, None), "already gone here");
        assert!(
            !may_remove("b", Some("edited"), None),
            "edit saved in the cache"
        );
        assert!(
            !may_remove("b", Some("b"), Some("edited")),
            "edit not saved yet"
        );
        assert!(!may_remove("", Some(""), None), "no known base");
    }

    #[test]
    fn most_files_gone_at_once_removes_nothing() {
        assert!(!removals_allowed(false, 16, 1), "folder not visible");
        assert!(!removals_allowed(true, 16, 9), "most of them");
        assert!(!removals_allowed(true, 3, 3), "all of a few");
        assert!(removals_allowed(true, 16, 2));
        assert!(removals_allowed(true, 1, 1), "the only file, deleted");
        assert!(removals_allowed(true, 16, 8), "half is not most");
    }

    #[test]
    fn only_the_signed_in_accounts_record_migrates() {
        let keys = ["batm:a.batm", "batproj:p1"];
        assert_eq!(
            keys_to_migrate(
                "Bat.Person@example.com",
                "bat.person@example.com",
                keys.iter().copied()
            ),
            vec!["batm:a.batm".to_string(), "batproj:p1".to_string()]
        );
        assert!(keys_to_migrate(
            "someone@example.com",
            "bat.person@example.com",
            keys.iter().copied()
        )
        .is_empty());
        assert!(keys_to_migrate("", "bat.person@example.com", keys.iter().copied()).is_empty());
    }

    #[test]
    fn no_drive_copy_uploads() {
        let local = set(T1, &[("a", 1.0)]);
        let fp = annotation_fingerprint(&local);
        assert_eq!(
            decide(Kind::Annotations, Some(copy(&fp, T1)), None, None),
            Action::Upload
        );
    }

    #[test]
    fn no_local_copy_downloads() {
        assert_eq!(
            decide(Kind::Annotations, None, Some(copy("d", T1)), None),
            Action::Download
        );
        assert_eq!(decide(Kind::Annotations, None, None, None), Action::Nothing);
    }

    /// A second device opens a file: it has an empty set, stamped now. The
    /// Drive copy has real annotations and an older time. The empty set must
    /// not replace them, and nothing is uploaded.
    #[test]
    fn empty_untouched_local_does_not_win_over_drive() {
        let drive = set(T1, &[("a", 1.0), ("b", 2.0)]);
        let local = set(T2, &[]);
        let (dfp, lfp) = (
            annotation_fingerprint(&drive),
            annotation_fingerprint(&local),
        );
        let action = decide(
            Kind::Annotations,
            Some(copy(&lfp, T2)),
            Some(copy(&dfp, T1)),
            None,
        );
        assert_eq!(action, Action::Merge);
        let merged = merge_annotation_sets(drive.clone(), &local);
        assert_eq!(annotation_fingerprint(&merged), dfp, "nothing to upload");
        assert_eq!(merged.annotations.len(), 2);
    }

    /// A first comparison keeps the annotations of both sides.
    #[test]
    fn first_comparison_merges_both_sides() {
        let drive = set(T1, &[("a", 1.0)]);
        let local = set(T2, &[("a", 1.0), ("new", 3.0)]);
        let merged = merge_annotation_sets(drive, &local);
        let ids: Vec<&str> = merged.annotations.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["a", "new"]);
    }

    #[test]
    fn equal_content_with_different_times_is_nothing() {
        let a = set(T1, &[("a", 1.0)]);
        let mut b = set(T2, &[("a", 1.0)]);
        b.app_version = "9.9.9".into();
        b.id = "another-set-id".into();
        b.file_identity.spot_hash_b3 = Some("abc".into());
        let (fa, fb) = (annotation_fingerprint(&a), annotation_fingerprint(&b));
        assert_eq!(fa, fb);
        assert_eq!(
            decide(
                Kind::Annotations,
                Some(copy(&fa, T2)),
                Some(copy(&fb, T1)),
                None
            ),
            Action::Nothing
        );
    }

    #[test]
    fn noise_profile_time_is_not_content() {
        let mut a = set(T1, &[]);
        let mut b = set(T1, &[]);
        let profile = |created: &str| crate::dsp::notch::NoiseProfile {
            name: "n".into(),
            bands: Vec::new(),
            source_sample_rate: 96_000,
            created: created.into(),
            noise_floor: None,
        };
        a.noise_profile = Some(profile(T1));
        b.noise_profile = Some(profile(T2));
        assert_eq!(annotation_fingerprint(&a), annotation_fingerprint(&b));
    }

    #[test]
    fn local_edited_since_last_sync_uploads() {
        let base = annotation_fingerprint(&set(T1, &[("a", 1.0)]));
        let local = annotation_fingerprint(&set(T2, &[("a", 1.0), ("b", 2.0)]));
        assert_eq!(
            decide(
                Kind::Annotations,
                Some(copy(&local, T2)),
                Some(copy(&base, T1)),
                Some(&base)
            ),
            Action::Upload
        );
    }

    /// Undoing or deleting a file's last annotation leaves an empty set. It
    /// must replace the Drive copy it was last synced with; if the local set
    /// were removed instead, the Drive copy would be downloaded and the
    /// annotation would come back.
    #[test]
    fn an_emptied_local_set_uploads_and_a_missing_one_downloads() {
        let synced = set(T1, &[("a", 1.0)]);
        let base = annotation_fingerprint(&synced);
        let emptied = annotation_fingerprint(&set(T2, &[]));
        assert_eq!(
            decide(
                Kind::Annotations,
                Some(copy(&emptied, T2)),
                Some(copy(&base, T1)),
                Some(&base)
            ),
            Action::Upload
        );
        assert_eq!(
            decide(Kind::Annotations, None, Some(copy(&base, T1)), Some(&base)),
            Action::Download
        );
    }

    #[test]
    fn drive_edited_since_last_sync_downloads() {
        let base = annotation_fingerprint(&set(T1, &[("a", 1.0)]));
        let drive = annotation_fingerprint(&set(T2, &[("b", 2.0)]));
        // The local copy is unchanged, even if a later save stamped a later
        // time on it: it still downloads.
        assert_eq!(
            decide(
                Kind::Annotations,
                Some(copy(&base, "2026-12-01T00:00:00.000Z")),
                Some(copy(&drive, T2)),
                Some(&base)
            ),
            Action::Download
        );
    }

    #[test]
    fn both_edited_annotation_sets_are_merged_and_projects_take_the_later() {
        let base = "base";
        for (l, d) in [(T2, T1), (T1, T2), (T1, T1)] {
            assert_eq!(
                decide(
                    Kind::Annotations,
                    Some(copy("l", l)),
                    Some(copy("d", d)),
                    Some(base)
                ),
                Action::Merge
            );
        }
        assert_eq!(
            decide(
                Kind::Project,
                Some(copy("l", T2)),
                Some(copy("d", T1)),
                Some(base)
            ),
            Action::Upload
        );
        // A tie keeps the Drive copy.
        assert_eq!(
            decide(
                Kind::Project,
                Some(copy("l", T1)),
                Some(copy("d", T1)),
                Some(base)
            ),
            Action::Download
        );
    }

    #[test]
    fn projects_first_comparison_later_time_wins() {
        assert_eq!(
            decide(
                Kind::Project,
                Some(copy("l", T2)),
                Some(copy("d", T1)),
                None
            ),
            Action::Upload
        );
        assert_eq!(
            decide(
                Kind::Project,
                Some(copy("l", T1)),
                Some(copy("d", T2)),
                None
            ),
            Action::Download
        );
    }

    #[test]
    fn drive_revision_changed_since_last_seen_blocks_overwrite() {
        assert!(may_overwrite(Some("r1"), Some("r1")));
        assert!(!may_overwrite(Some("r2"), Some("r1")));
        assert!(!may_overwrite(Some("r1"), None));
        assert!(!may_overwrite(None, Some("r1")));
    }

    #[test]
    fn duplicate_keys_on_drive_use_the_newest() {
        let files = [
            ("old", "2026-11-01T10:00:00.000Z"),
            ("newest", "2026-11-03T09:00:00.000Z"),
            ("mid", "2026-11-02T23:59:59.999Z"),
        ];
        assert_eq!(newest(&files, |f| f.1).map(|f| f.0), Some("newest"));
        let none: [(&str, &str); 0] = [];
        assert!(newest(&none, |f| f.1).is_none());
    }

    #[test]
    fn long_keys_are_hashed_and_found_again() {
        let short = "b3_0123456789abcdef.batm";
        assert_eq!(drive_key(short), short);
        let long = format!("{}_1000.batm", "x".repeat(200));
        let hashed = drive_key(&long);
        assert!(hashed.starts_with("h_"));
        assert!(KEY_PROPERTY.len() + hashed.len() <= APP_PROPERTY_BYTES);
        assert_eq!(
            local_annotation_key(&hashed, &[short.to_string(), long.clone()]),
            Some(long)
        );
        assert_eq!(local_annotation_key(short, &[]), Some(short.to_string()));
        assert_eq!(local_annotation_key("../evil.batm", &[]), None);
        assert_eq!(local_annotation_key("a/b.batm", &[]), None);
        assert_eq!(local_annotation_key("notes.txt", &[]), None);
        assert_eq!(local_annotation_key("C:x.batm", &[]), None);
        assert_eq!(local_annotation_key("a..b.batm", &[]), None);
        assert_eq!(
            local_annotation_key("Fledermäuse_teich.wav_100.batm", &[]),
            Some("Fledermäuse_teich.wav_100.batm".to_string())
        );
    }

    #[test]
    fn projects_go_to_drive_without_paths_and_get_them_back_on_download() {
        let yaml = "version: 1\nid: p1\nname: Survey\nfiles:\n\
                    - identity:\n    filename: pond.wav\n    file_size: 100\n    spot_hash_b3: abc\n    \
                    file_path: /home/someone/bats/pond.wav\n\
                    - identity:\n    filename: wood.wav\n    file_size: 200\n";
        let local: BatProject = yaml_serde::from_str(yaml).unwrap();
        let text = project_drive_text(&local).unwrap();
        assert!(!text.contains("file_path") && !text.contains("/home/someone"));
        let from_drive: BatProject = yaml_serde::from_str(&text).unwrap();
        // The copy without paths counts as the same content.
        assert_eq!(
            project_fingerprint(&from_drive),
            project_fingerprint(&local)
        );
        let restored = with_local_paths(from_drive, Some(&local));
        assert_eq!(
            restored.files[0].identity.file_path.as_deref(),
            Some("/home/someone/bats/pond.wav")
        );
        assert_eq!(restored.files[1].identity.file_path, None);
    }

    #[test]
    fn project_fingerprint_ignores_time_and_version() {
        let yaml = |at: &str, ver: &str| {
            format!("version: 1\nid: p1\napp_version: {ver}\nmodified_at: {at}\nname: Survey\n")
        };
        let a: BatProject = yaml_serde::from_str(&yaml(T1, "0.5.0")).unwrap();
        let b: BatProject = yaml_serde::from_str(&yaml(T2, "0.6.0")).unwrap();
        assert_eq!(project_fingerprint(&a), project_fingerprint(&b));
        let mut c = b.clone();
        c.name = Some("Survey 2".into());
        assert_ne!(project_fingerprint(&a), project_fingerprint(&c));
    }
}
