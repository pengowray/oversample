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
//! - Both changed: the copy with the later `modified_at` (the time stamped
//!   inside the file) wins.
//! - No base (the first comparison of this key since sign-in on this device):
//!   annotation sets are merged, so that nothing is lost: the Drive copy is
//!   kept and the local annotations it does not have are added to it.
//!   Projects use the later `modified_at`.
//!
//! The merge is what keeps a second device from overwriting Drive with an
//! empty or untouched set: a set with no annotations of its own adds nothing,
//! so the result equals the Drive copy and nothing is uploaded.
//!
//! The fingerprint covers only what a user changes: the annotations, the
//! file's own lowest bat frequency and its noise profile (without the time
//! the profile was last written). Timestamps, the app version, the file
//! identity and the audio metadata are left out, because they change on
//! saves that are not edits.

use crate::annotations::{Annotation, AnnotationSet};
use crate::project::BatProject;
use serde::Serialize;

/// What the app does with one file key after comparing the two copies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Both copies hold the same content.
    Nothing,
    /// Write the local copy to Drive.
    Upload,
    /// Replace the local copy with the Drive copy.
    Download,
    /// Keep the Drive copy and add the local annotations it does not have
    /// (see [`merge_annotation_sets`]). Annotation sets only.
    Merge,
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
        (true, true) => later_wins(local, drive),
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
/// copy when the Drive copy has none.
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
    if merged.noise_profile.is_none() {
        merged.noise_profile = local.noise_profile.clone();
    }
    merged
}

/// Fingerprint of what a user changes in an annotation set (see the module
/// notes). 32 hex digits.
pub fn annotation_fingerprint(set: &AnnotationSet) -> String {
    #[derive(Serialize)]
    struct Content<'a> {
        annotations: &'a [Annotation],
        lowest_bat_hz: Option<f32>,
        noise_profile: Option<crate::dsp::notch::NoiseProfile>,
    }
    let noise_profile = set.noise_profile.clone().map(|mut p| {
        p.created = String::new();
        p
    });
    let content = Content {
        annotations: &set.annotations,
        lowest_bat_hz: set.lowest_bat_hz,
        noise_profile,
    };
    hash_json(&content)
}

/// Fingerprint of a project: all of it except `modified_at` and
/// `app_version`. 32 hex digits.
pub fn project_fingerprint(project: &BatProject) -> String {
    let mut p = project.clone();
    p.modified_at = None;
    p.app_version = String::new();
    hash_json(&p)
}

fn hash_json(value: &impl Serialize) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    let hex = blake3::hash(&bytes).to_hex();
    hex[..32].to_string()
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
/// could carry any text in its key property.
pub fn local_key_is_safe(key: &str, extension: &str) -> bool {
    !key.is_empty()
        && key.len() <= 255
        && key.ends_with(extension)
        && !key.starts_with('.')
        && !key.contains(['/', '\\', '\0'])
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
    fn both_edited_later_time_wins() {
        let base = "base";
        assert_eq!(
            decide(
                Kind::Annotations,
                Some(copy("l", T2)),
                Some(copy("d", T1)),
                Some(base)
            ),
            Action::Upload
        );
        assert_eq!(
            decide(
                Kind::Annotations,
                Some(copy("l", T1)),
                Some(copy("d", T2)),
                Some(base)
            ),
            Action::Download
        );
        // A tie keeps the Drive copy.
        assert_eq!(
            decide(
                Kind::Annotations,
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
