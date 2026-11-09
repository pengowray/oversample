//! Converting between Oversample annotations and Annota-JSON, Xeno-canto's
//! annotation format (wire types in `oversample_ipc::annota`).
//!
//! Import: each XC annotation becomes a locked region inside a group for its
//! set, with XC's fields kept as `#xc:` tags so nothing is lost and a second
//! import of the same set adds nothing. XC annotations can't be edited on XC
//! (only the whole set deleted and uploaded again), hence locked.
//!
//! Export: regions become XC annotations. Tags with an Annota-JSON field
//! (`#species`, `#xc:sound_type`, `#xc:sex`, `#xc:life_stage`,
//! `#xc:annotator`) fill that field; the rest (such as `#call` or `#train`)
//! go at the end of `annotation_remarks` as hashtags, so they come back as tags
//! when the set is imported again. Regions imported from XC are left out.
//!
//! Nothing here touches the DOM or `js_sys`: ids and timestamps are passed in,
//! so the conversions are tested on the host.

use std::collections::HashMap;

use oversample_core::annotation_tags::{self as tags, Tag};
use oversample_ipc::annota::{AnnotaAnnotation, AnnotaSet, AnnotaSetMeta};

use crate::annotations::{Annotation, AnnotationId, AnnotationKind, Group, Region};

/// XC's sound types for bats, as `group_properties` lists them.
pub const BAT_SOUND_TYPES: &[&str] = &[
    "echolocation",
    "feeding buzz",
    "social call",
    "distress call",
    "song",
    "uncertain",
];

/// An echolocation annotation longer than this holds more than one call (no
/// bat call lasts this long), so it is tagged `#train` on import.
const TRAIN_MIN_SECS: f64 = 0.2;

/// The result of importing a set.
#[derive(Debug, Default)]
pub struct ImportOutcome {
    /// New annotations: groups first, then regions.
    pub added: Vec<Annotation>,
    /// Annotations already present (same `#xc:annotation_id`).
    pub already_present: usize,
    /// Annotations with no start or end time.
    pub unusable: usize,
}

impl ImportOutcome {
    /// Number of new regions (not counting groups).
    pub fn regions_added(&self) -> usize {
        self.added
            .iter()
            .filter(|a| matches!(a.kind, AnnotationKind::Region(_)))
            .count()
    }
}

/// Key that identifies the set an annotation came from.
fn set_key(meta: &AnnotaSetMeta) -> String {
    meta.set_uri
        .clone()
        .or_else(|| meta.set_name.clone())
        .unwrap_or_else(|| "xeno-canto".to_string())
}

/// Credit for a set, one line per known field: who made it, its licence and
/// where it is.
fn set_credit(meta: &AnnotaSetMeta) -> String {
    let mut lines = vec!["From Xeno-canto".to_string()];
    for (label, value) in [
        ("Creator", &meta.set_creator),
        ("Owner", &meta.set_owner),
        ("Licence", &meta.set_license),
        ("Date made", &meta.set_creation_date),
        ("Link", &meta.set_uri),
    ] {
        if let Some(v) = value {
            lines.push(format!("{label}: {v}"));
        }
    }
    lines.join("\n")
}

/// Split remarks into hashtags and the remaining text. Hashtags that can't be
/// read stay in the text.
fn split_hashtags(remarks: &str) -> (Vec<Tag>, String) {
    let mut found = Vec::new();
    let mut text = Vec::new();
    for word in remarks.split_whitespace() {
        if word.starts_with('#') && word.len() > 1 {
            if let Ok(mut t) = tags::parse_tag_input(word) {
                if t.len() == 1 && !t[0].is_plain() {
                    found.push(t.remove(0));
                    continue;
                }
            }
        }
        text.push(word);
    }
    (found, text.join(" "))
}

/// Import an Annota-JSON set into a file's annotations.
///
/// `existing` is the file's current annotations, used to skip annotations
/// already imported and to reuse a group made for the same set before.
pub fn import_set(
    set: &AnnotaSet,
    existing: &[Annotation],
    mut new_id: impl FnMut() -> AnnotationId,
    now: &str,
) -> ImportOutcome {
    let mut out = ImportOutcome::default();

    let mut known_xc_ids: Vec<String> = existing
        .iter()
        .filter_map(|a| tags::value_of(&a.tags, Some(tags::XC_NS), tags::XC_ANNOTATION_ID))
        .collect();

    // Groups already made for a set, by set key.
    let mut groups: HashMap<String, AnnotationId> = existing
        .iter()
        .filter(|a| matches!(a.kind, AnnotationKind::Group(_)))
        .filter_map(|a| {
            tags::value_of(&a.tags, Some(tags::XC_NS), tags::XC_SET).map(|k| (k, a.id.clone()))
        })
        .collect();

    let next_order = existing
        .iter()
        .filter_map(|a| a.sort_order)
        .fold(0.0_f64, f64::max)
        + 1.0;
    let mut order = next_order;

    for ann in &set.annotations {
        if let Some(xc_id) = &ann.annotation_xc_id {
            if known_xc_ids.contains(xc_id) {
                out.already_present += 1;
                continue;
            }
        }
        let (Some(t0), Some(t1)) = (ann.start_time, ann.end_time) else {
            out.unusable += 1;
            continue;
        };
        let (t0, t1) = if t0 <= t1 { (t0, t1) } else { (t1, t0) };

        let meta = ann.source_set(&set.meta);
        let key = set_key(meta);
        let group_id = match groups.get(&key) {
            Some(id) => id.clone(),
            None => {
                let id = new_id();
                let name = meta
                    .set_name
                    .clone()
                    .unwrap_or_else(|| "Annotations".to_string());
                let credit = set_credit(meta);
                out.added.push(Annotation {
                    id: id.clone(),
                    kind: AnnotationKind::Group(Group {
                        label: Some(format!("XC: {name}")),
                        color: None,
                        collapsed: Some(false),
                    }),
                    created_at: now.to_string(),
                    modified_at: now.to_string(),
                    notes: Some(credit),
                    parent_id: None,
                    sort_order: Some(order),
                    tags: vec![Tag::namespaced(tags::XC_NS, tags::XC_SET, key.clone()).to_stored()],
                    label_default: None,
                });
                order += 1.0;
                groups.insert(key.clone(), id.clone());
                id
            }
        };

        let mut t: Vec<String> = Vec::new();
        let (remark_tags, remark_text) = ann
            .annotation_remarks
            .as_deref()
            .map(split_hashtags)
            .unwrap_or_default();
        for rt in &remark_tags {
            t.push(rt.to_stored());
        }
        if let Some(sp) = &ann.scientific_name {
            tags::set_value(&mut t, Tag::with_value(tags::SPECIES, sp.clone()));
        }
        let sound_type = ann.sound_type.as_deref().map(str::to_lowercase);
        if let Some(st) = &sound_type {
            tags::set_value(
                &mut t,
                Tag::namespaced(tags::XC_NS, tags::XC_SOUND_TYPE, st.clone()),
            );
            let flag = match st.as_str() {
                "feeding buzz" => Some(tags::BUZZ),
                "social call" => Some(tags::SOCIAL),
                _ => None,
            };
            if let Some(f) = flag {
                if !tags::has_flag(&t, f) {
                    t.push(Tag::flag(f).to_stored());
                }
            }
            let marked = tags::has_flag(&t, tags::CALL) || tags::has_flag(&t, tags::TRAIN);
            if st == "echolocation" && !marked && t1 - t0 > TRAIN_MIN_SECS {
                t.push(Tag::flag(tags::TRAIN).to_stored());
            }
        }
        for (name, value) in [
            (tags::XC_ANNOTATOR, &ann.annotator),
            (tags::XC_SEX, &ann.sex),
            (tags::XC_LIFE_STAGE, &ann.life_stage),
        ] {
            if let Some(v) = value {
                tags::set_value(&mut t, Tag::namespaced(tags::XC_NS, name, v.clone()));
            }
        }
        if let Some(xc_id) = &ann.annotation_xc_id {
            tags::set_value(
                &mut t,
                Tag::namespaced(tags::XC_NS, tags::XC_ANNOTATION_ID, xc_id.clone()),
            );
            known_xc_ids.push(xc_id.clone());
        }
        tags::set_value(&mut t, Tag::namespaced(tags::XC_NS, tags::XC_SET, key));

        let label = match (&ann.scientific_name, &sound_type) {
            (Some(sp), Some(st)) => format!("{sp} ({st})"),
            (Some(sp), None) => sp.clone(),
            (None, Some(st)) => st.clone(),
            (None, None) => "Xeno-canto annotation".to_string(),
        };

        let (freq_low, freq_high) = match (ann.frequency_low, ann.frequency_high) {
            (Some(lo), Some(hi)) => (Some(lo.min(hi)), Some(lo.max(hi))),
            _ => (None, None),
        };

        out.added.push(Annotation {
            id: new_id(),
            kind: AnnotationKind::Region(Region {
                time_start: t0,
                time_end: t1,
                freq_low,
                freq_high,
                label: Some(label),
                color: None,
                locked: Some(true),
            }),
            created_at: now.to_string(),
            modified_at: now.to_string(),
            notes: (!remark_text.is_empty()).then_some(remark_text),
            parent_id: Some(group_id),
            sort_order: Some(order),
            tags: t,
            label_default: None,
        });
        order += 1.0;
    }
    out
}

/// What an export needs to know beyond the annotations.
#[derive(Clone, Debug, Default)]
pub struct ExportContext {
    /// XC number of the recording, without `XC`. Required by XC.
    pub xc_nr: Option<String>,
    /// File name of the recording.
    pub sound_file: Option<String>,
    /// Species for regions with no `#species` tag (from the XC metadata).
    pub default_species: Option<String>,
    /// Annotator and set creator.
    pub annotator: Option<String>,
    pub set_name: Option<String>,
    /// One of `oversample_ipc::annota::LICENSES`.
    pub license: Option<String>,
    /// Program name and version.
    pub software: String,
}

/// Something XC would reject or warn about.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportIssue {
    /// True when XC would reject the upload.
    pub blocking: bool,
    pub message: String,
}

/// The result of an export.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportOutcome {
    pub set: AnnotaSet,
    pub issues: Vec<ExportIssue>,
    /// Regions left out because they came from XC.
    pub from_xc_skipped: usize,
}

/// Round to `places` decimal places.
fn round_to(v: f64, places: i32) -> f64 {
    let m = 10f64.powi(places);
    (v * m).round() / m
}

/// Tags that fill an Annota-JSON field of their own, so they don't go in the
/// remarks.
fn has_own_field(t: &Tag) -> bool {
    t.is(None, tags::SPECIES)
        || t.is(Some(tags::XC_NS), tags::XC_SOUND_TYPE)
        || t.is(Some(tags::XC_NS), tags::XC_SEX)
        || t.is(Some(tags::XC_NS), tags::XC_LIFE_STAGE)
        || t.is(Some(tags::XC_NS), tags::XC_ANNOTATOR)
}

/// XC sound type for a region's tags.
fn sound_type_for(t: &[String]) -> Option<String> {
    if let Some(st) = tags::value_of(t, Some(tags::XC_NS), tags::XC_SOUND_TYPE) {
        return Some(st);
    }
    if tags::has_flag(t, tags::BUZZ) {
        Some("feeding buzz".into())
    } else if tags::has_flag(t, tags::SOCIAL) {
        Some("social call".into())
    } else if tags::has_flag(t, tags::CALL) || tags::has_flag(t, tags::TRAIN) {
        Some("echolocation".into())
    } else {
        None
    }
}

/// Export regions as an Annota-JSON set. `only` limits the export to these
/// annotations (and regions inside them); `None` exports every region.
pub fn export_set(
    annotations: &[Annotation],
    only: Option<&[AnnotationId]>,
    ctx: &ExportContext,
) -> ExportOutcome {
    let mut issues = Vec::new();
    let mut from_xc_skipped = 0;

    let in_scope = |a: &Annotation| -> bool {
        let Some(ids) = only else { return true };
        let mut cur = Some(a);
        while let Some(x) = cur {
            if ids.contains(&x.id) {
                return true;
            }
            cur = x
                .parent_id
                .as_ref()
                .and_then(|p| annotations.iter().find(|y| &y.id == p));
        }
        false
    };

    let mut regions: Vec<(&Annotation, &Region)> = annotations
        .iter()
        .filter(|a| in_scope(a))
        .filter_map(|a| match &a.kind {
            AnnotationKind::Region(r) => Some((a, r)),
            _ => None,
        })
        .filter(|(a, _)| {
            let from_xc =
                tags::value_of(&a.tags, Some(tags::XC_NS), tags::XC_ANNOTATION_ID).is_some();
            if from_xc {
                from_xc_skipped += 1;
            }
            !from_xc
        })
        .collect();
    regions.sort_by(|a, b| a.1.time_start.total_cmp(&b.1.time_start));

    let mut no_species = 0;
    let mut no_freq = 0;
    let mut no_type = 0;
    let mut out = Vec::new();
    for (a, r) in &regions {
        let parsed = tags::parse_stored(&a.tags);
        let species = tags::value_of(&a.tags, None, tags::SPECIES).or(ctx.default_species.clone());
        if species.is_none() {
            no_species += 1;
        }
        let sound_type = sound_type_for(&a.tags);
        if sound_type.is_none() {
            no_type += 1;
        }
        if r.freq_low.is_none() || r.freq_high.is_none() {
            no_freq += 1;
        }

        let mut remarks: Vec<String> = Vec::new();
        let label = r.label.as_deref().filter(|_| a.label_default != Some(true));
        if let Some(l) = label {
            remarks.push(l.trim().to_string());
        }
        if let Some(n) = a.notes.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
            remarks.push(n.to_string());
        }
        let extra: Vec<String> = parsed
            .iter()
            .filter(|t| !t.is_plain() && !has_own_field(t))
            .map(|t| t.display())
            .collect();
        let mut remark_text = remarks.join(". ");
        if !extra.is_empty() {
            if !remark_text.is_empty() {
                remark_text.push(' ');
            }
            remark_text.push_str(&extra.join(" "));
        }

        out.push(AnnotaAnnotation {
            annotation_source_id: Some(a.id.to_string()),
            sound_file: ctx.sound_file.clone(),
            xc_nr: ctx.xc_nr.clone(),
            annotator: tags::value_of(&a.tags, Some(tags::XC_NS), tags::XC_ANNOTATOR)
                .or(ctx.annotator.clone()),
            frequency_high: r.freq_high.map(|f| f.round()),
            frequency_low: r.freq_low.map(|f| f.round()),
            start_time: Some(round_to(r.time_start.max(0.0), 6)),
            end_time: Some(round_to(r.time_end.max(0.0), 6)),
            scientific_name: species,
            sound_type,
            date_identified: a.created_at.get(..10).map(str::to_string),
            sex: tags::value_of(&a.tags, Some(tags::XC_NS), tags::XC_SEX),
            life_stage: tags::value_of(&a.tags, Some(tags::XC_NS), tags::XC_LIFE_STAGE),
            annotation_remarks: (!remark_text.is_empty()).then_some(remark_text),
            ..Default::default()
        });
    }

    let n = out.len();
    let blocking = |message: String| ExportIssue {
        blocking: true,
        message,
    };
    let warning = |message: String| ExportIssue {
        blocking: false,
        message,
    };
    if n == 0 {
        issues.push(blocking("No annotations to export".into()));
    }
    if ctx.xc_nr.is_none() {
        issues.push(blocking("No XC number".into()));
    }
    if no_species > 0 {
        issues.push(blocking(format!(
            "No species: {no_species} of {n} annotations. Add #species=Genus_species to them, or fill in Species."
        )));
    }
    if ctx.annotator.is_none() {
        issues.push(warning("No annotator name".into()));
    }
    if ctx.license.is_none() {
        issues.push(warning("No licence".into()));
    }
    if no_type > 0 {
        issues.push(warning(format!(
            "No sound type: {no_type} of {n} annotations. Tag them #call, #train, #buzz or #social."
        )));
    }
    if no_freq > 0 {
        issues.push(warning(format!(
            "No frequency range: {no_freq} of {n} annotations are segments (time only)."
        )));
    }

    let set = AnnotaSet {
        meta: AnnotaSetMeta {
            set_name: ctx.set_name.clone(),
            annotation_software_name_and_version: Some(ctx.software.clone()),
            set_creator: ctx.annotator.clone(),
            set_owner: ctx.annotator.clone(),
            set_license: ctx.license.clone(),
            ..Default::default()
        },
        annotations: out,
    };
    ExportOutcome {
        set,
        issues,
        from_xc_skipped,
    }
}

// ---------------------------------------------------------------------------
// App glue: annotation sets that arrive with Xeno-canto recordings.
// ---------------------------------------------------------------------------

thread_local! {
    /// XC annotation sets by file id, kept so they can be merged again after
    /// the file's saved annotations finish loading (which replaces the file's
    /// annotation set in the store).
    static XC_SETS: std::cell::RefCell<HashMap<u64, AnnotaSet>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Remember the XC annotation set of a newly loaded file and add its
/// annotations to the file.
pub fn attach_xc_set(state: crate::state::AppState, file_id: u64, set: AnnotaSet) {
    XC_SETS.with(|m| m.borrow_mut().insert(file_id, set));
    merge_xc_set(state, file_id, true);
}

/// Add any XC annotations of `file_id` that aren't in its annotation set yet.
/// Safe to call more than once: annotations already present are skipped.
pub fn merge_xc_set(state: crate::state::AppState, file_id: u64, announce: bool) {
    use crate::state::store_fields::*;
    use leptos::prelude::*;

    let Some(set) = XC_SETS.with(|m| m.borrow().get(&file_id).cloned()) else {
        return;
    };
    let Some(file_idx) = state.file_idx_for_id(file_id) else {
        return;
    };
    let new_set = state.library.files().with_untracked(|files| {
        files.get(file_idx).map(|f| {
            let id = f.identity.clone().unwrap_or_else(|| {
                crate::file_identity::identity_layer1(&f.name, f.audio.metadata.file_size as u64)
            });
            crate::annotations::AnnotationSet::new_with_metadata(
                id,
                &f.audio,
                f.cached_peak_db,
                f.cached_full_peak_db,
            )
        })
    });
    let Some(new_set) = new_set else {
        return;
    };
    let existing: Vec<Annotation> = state.annotations.store().with_untracked(|store| {
        store
            .get(file_id)
            .map(|s| s.annotations.clone())
            .unwrap_or_default()
    });
    let out = import_set(
        &set,
        &existing,
        AnnotationId::new,
        &crate::annotations::now_iso8601(),
    );
    if out.added.is_empty() {
        return;
    }
    let n = out.regions_added();
    state.annotations.store().update(|store| {
        let s = store.entry_or_insert_with(file_id, || new_set);
        s.annotations.extend(out.added);
    });
    state.annotations.dirty().set(true);
    if announce {
        state.show_info_toast(if n == 1 {
            "Added 1 annotation from XC".to_string()
        } else {
            format!("Added {n} annotations from XC")
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> impl FnMut() -> AnnotationId {
        let mut n = 0;
        move || {
            n += 1;
            AnnotationId::from(format!("id{n}"))
        }
    }

    fn sample_set() -> AnnotaSet {
        serde_json::from_str(
            r#"{
            "set_name": "Annotation set for XC1",
            "annotations": [
                {"annotation_xc_id": 1, "xc_nr": "1", "scientific_name": "Nyctalus noctula",
                 "annotator": "A. Recordist", "start_time": 0.2, "end_time": 4.6,
                 "frequency_high": 23063, "frequency_low": 15563, "sound_type": "echolocation",
                 "original_set_metadata": {"set_name": "Set A", "set_license": "CC-BY-4.0",
                    "set_creator": "A. Recordist", "set_uri": "https://xeno-canto.org/annotation/set/174"}},
                {"annotation_xc_id": 2, "xc_nr": "1", "scientific_name": "Nyctalus noctula",
                 "start_time": 9.72, "end_time": 10.05, "frequency_high": 46688,
                 "frequency_low": 0, "sound_type": "feeding buzz",
                 "annotation_remarks": "close pass #call",
                 "original_set_metadata": {"set_name": "Set A",
                    "set_uri": "https://xeno-canto.org/annotation/set/174"}},
                {"annotation_xc_id": 3, "start_time": null, "end_time": 3}
            ]}"#,
        )
        .unwrap()
    }

    fn region(a: &Annotation) -> &Region {
        match &a.kind {
            AnnotationKind::Region(r) => r,
            _ => panic!("not a region"),
        }
    }

    #[test]
    fn import_makes_one_group_and_locked_regions() {
        let out = import_set(&sample_set(), &[], ids(), "2026-11-09T00:00:00Z");
        assert_eq!(out.unusable, 1);
        assert_eq!(out.regions_added(), 2);
        let group = &out.added[0];
        assert!(matches!(group.kind, AnnotationKind::Group(_)));
        assert!(group
            .notes
            .as_deref()
            .unwrap()
            .contains("\nLicence: CC-BY-4.0\n"));

        let first = &out.added[1];
        let r = region(first);
        assert_eq!(r.locked, Some(true));
        assert_eq!(r.label.as_deref(), Some("Nyctalus noctula (echolocation)"));
        assert_eq!(first.parent_id.as_ref(), Some(&group.id));
        // A 4.4 s echolocation box is a train.
        assert!(tags::has_flag(&first.tags, tags::TRAIN));
        assert_eq!(
            tags::value_of(&first.tags, None, tags::SPECIES).as_deref(),
            Some("Nyctalus noctula")
        );

        let buzz = &out.added[2];
        assert_eq!(region(buzz).freq_low, Some(0.0));
        assert!(tags::has_flag(&buzz.tags, tags::BUZZ));
        // The hashtag in the remarks became a tag and left the notes.
        assert!(tags::has_flag(&buzz.tags, tags::CALL));
        assert!(!tags::has_flag(&buzz.tags, tags::TRAIN));
        assert_eq!(buzz.notes.as_deref(), Some("close pass"));
    }

    #[test]
    fn second_import_adds_nothing() {
        let set = sample_set();
        let first = import_set(&set, &[], ids(), "t");
        let again = import_set(&set, &first.added, ids(), "t");
        assert!(again.added.is_empty());
        assert_eq!(again.already_present, 2);
    }

    fn user_region(id: &str, tags_in: &[&str], label: Option<&str>) -> Annotation {
        Annotation {
            id: AnnotationId::from(id),
            kind: AnnotationKind::Region(Region {
                time_start: 1.0,
                time_end: 1.006,
                freq_low: Some(44_812.4),
                freq_high: Some(71_000.0),
                label: label.map(str::to_string),
                color: None,
                locked: None,
            }),
            created_at: "2026-11-09T10:00:00Z".into(),
            modified_at: "2026-11-09T10:00:00Z".into(),
            notes: None,
            parent_id: None,
            sort_order: None,
            tags: tags_in.iter().map(|s| s.to_string()).collect(),
            label_default: None,
        }
    }

    #[test]
    fn export_maps_tags_to_fields_and_remarks() {
        let a = user_region(
            "u1",
            &["call", "auto=calls-1", "species=Pipistrellus pipistrellus"],
            Some("First call"),
        );
        let ctx = ExportContext {
            xc_nr: Some("123".into()),
            annotator: Some("Me".into()),
            license: Some("CC-BY-4.0".into()),
            software: "Oversample 0.5".into(),
            ..Default::default()
        };
        let out = export_set(&[a], None, &ctx);
        assert!(out.issues.iter().all(|i| !i.blocking), "{:?}", out.issues);
        let x = &out.set.annotations[0];
        assert_eq!(
            x.scientific_name.as_deref(),
            Some("Pipistrellus pipistrellus")
        );
        assert_eq!(x.sound_type.as_deref(), Some("echolocation"));
        assert_eq!(x.frequency_low, Some(44_812.0));
        assert_eq!(
            x.annotation_remarks.as_deref(),
            Some("First call #call #auto=calls-1")
        );
        assert_eq!(x.date_identified.as_deref(), Some("2026-11-09"));
    }

    #[test]
    fn export_round_trips_call_and_train_tags() {
        let train = user_region("t1", &["train", "species=Myotis daubentonii"], None);
        let ctx = ExportContext {
            xc_nr: Some("5".into()),
            ..Default::default()
        };
        let mut set = export_set(&[train], None, &ctx).set;
        set.annotations[0].annotation_xc_id = Some("99".into());
        let back = import_set(&set, &[], ids(), "t");
        let r = back
            .added
            .iter()
            .find(|a| matches!(a.kind, AnnotationKind::Region(_)))
            .unwrap();
        assert!(tags::has_flag(&r.tags, tags::TRAIN));
        assert!(!tags::has_flag(&r.tags, tags::CALL));
    }

    #[test]
    fn export_blocks_without_xc_number_or_species_and_skips_xc_regions() {
        let imported = import_set(&sample_set(), &[], ids(), "t").added;
        let mut all = imported.clone();
        all.push(user_region("u1", &["call"], None));
        let out = export_set(&all, None, &ExportContext::default());
        assert_eq!(out.from_xc_skipped, 2);
        assert_eq!(out.set.annotations.len(), 1);
        let blocking: Vec<_> = out.issues.iter().filter(|i| i.blocking).collect();
        assert_eq!(blocking.len(), 2, "{:?}", out.issues);
    }
}
