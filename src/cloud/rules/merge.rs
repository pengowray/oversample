//! Merging two copies of an annotation set that both changed since a common
//! base: annotation by annotation.
//!
//! The base is not kept in full. [`MergeBase`] holds, per annotation id, a
//! hash of its content and a hash of its place in the list (`parent_id`,
//! `sort_order`), plus the set's own lowest bat frequency and a fingerprint
//! of its noise profile. That is all the rules need: whether each side
//! changed something since the base.
//!
//! Rules, per annotation id:
//! - Added on one side (not in the base): kept.
//! - Deleted on one side and unchanged on the other: deleted.
//! - Deleted on one side and changed on the other: kept, changed (an edit
//!   beats a deletion, as for whole files). Reported.
//! - Changed on one side only: that side's version.
//! - Changed on both sides: the content and the place are merged
//!   separately, so a move on one side and a label change on the other are
//!   both kept. When both changed the same part differently, the version
//!   with the later `modified_at` wins. Reported.
//!
//! Sets made by a program are handled as a whole: annotations tagged
//! `auto=<id>` (call detection, classifiers) and annotations imported from
//! Xeno-canto (`xc:set=<id>`). A new run makes new ids, so when both sides
//! added annotations of the same tag, only the later run's are kept.
//! Reported.
//!
//! An annotation whose group was deleted on the other side keeps its
//! `parent_id`; the app shows an annotation whose group is missing at the
//! top level (`build_annotation_tree`). Changing it here would make the
//! result depend on the inputs' history, and two devices would not agree.
//! The lowest bat
//! frequency and the noise profile follow the same rules as an annotation;
//! when both changed, the copy with the later `modified_at` wins.
//!
//! The two copies are equals (`a` and `b`): the result does not depend on
//! which is which, so two devices that merge the same copies upload the same
//! content and stop there. Ties are broken by comparing content.

use crate::annotations::{Annotation, AnnotationSet};
use crate::source_site::RecordingSource;
use oversample_core::annotation_tags::{self as tags, AUTO, XC_NS, XC_SET};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// What the merge needs of the common base. Kept per file key beside what
/// was last seen of its Drive copy.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MergeBase {
    /// `content_fingerprint` of the base set; the summary is used only for
    /// that base.
    pub fp: String,
    /// Per annotation id: `[content hash, place hash]`.
    pub annotations: BTreeMap<String, [String; 2]>,
    /// Ids of annotations made by a program, by their tag (`auto=…` or
    /// `xc:set=…`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub generated: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lowest_bat_hz: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noise: Option<String>,
}

impl MergeBase {
    /// The summary of `set` as a base.
    pub fn of(set: &AnnotationSet) -> Self {
        let mut generated: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for a in &set.annotations {
            if let Some(g) = generated_key(a) {
                generated
                    .entry(g)
                    .or_default()
                    .push(a.id.as_str().to_string());
            }
        }
        MergeBase {
            fp: super::content_fingerprint(set),
            annotations: set
                .annotations
                .iter()
                .map(|a| (a.id.as_str().to_string(), [content_hash(a), place_hash(a)]))
                .collect(),
            generated,
            lowest_bat_hz: set.lowest_bat_hz,
            noise: noise_fp(set),
        }
    }
}

/// Which of the two copies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    A,
    B,
}

impl Side {
    pub fn other(self) -> Side {
        match self {
            Side::A => Side::B,
            Side::B => Side::A,
        }
    }
}

/// A change that both copies made, where one copy's version was kept.
#[derive(Clone, Debug, PartialEq)]
pub enum Conflict {
    /// One annotation changed on both sides; `kept` side's version won.
    BothChanged {
        id: String,
        label: String,
        kept: Side,
    },
    /// Deleted on one side (`deleted_on`), changed on the other: kept.
    ChangedAndDeleted {
        id: String,
        label: String,
        deleted_on: Side,
    },
    /// Both sides made a new run of a program (`tag`, e.g. `auto=calls-2`);
    /// only `kept` side's run was kept (`dropped` annotations of the other).
    GeneratedTwice {
        tag: String,
        kept: Side,
        dropped: usize,
    },
    /// Both sides changed the file's lowest bat frequency.
    LowestBatHz { kept: Side },
    /// Both sides changed the noise profile.
    NoiseProfile { kept: Side },
}

/// What one side's changes did to the merged result.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub added: usize,
    pub removed: usize,
    pub changed: usize,
}

impl Changes {
    pub fn any(&self) -> bool {
        self.added + self.removed + self.changed > 0
    }
}

/// What the merge did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MergeReport {
    /// What `a`'s changes brought into the result.
    pub from_a: Changes,
    /// What `b`'s changes brought into the result.
    pub from_b: Changes,
    pub conflicts: Vec<Conflict>,
}

impl MergeReport {
    pub fn changes_by(&self, side: Side) -> Changes {
        match side {
            Side::A => self.from_a,
            Side::B => self.from_b,
        }
    }

    fn changes_by_mut(&mut self, side: Side) -> &mut Changes {
        match side {
            Side::A => &mut self.from_a,
            Side::B => &mut self.from_b,
        }
    }

    /// Whether the result lost anything `side` held: an annotation the other
    /// side deleted, or a version of `side` that lost a conflict.
    pub fn dropped_from(&self, side: Side) -> bool {
        self.changes_by(side.other()).removed > 0
            || self.conflicts.iter().any(|c| match c {
                Conflict::BothChanged { kept, .. }
                | Conflict::GeneratedTwice { kept, .. }
                | Conflict::LowestBatHz { kept }
                | Conflict::NoiseProfile { kept } => *kept != side,
                // The changed version was kept; the deletion is what lost.
                Conflict::ChangedAndDeleted { .. } => false,
            })
    }
}

/// Merge `a` and `b`, which both changed since `base`.
pub fn merge3(
    base: &MergeBase,
    a: &AnnotationSet,
    b: &AnnotationSet,
) -> (AnnotationSet, MergeReport) {
    let mut report = MergeReport::default();
    let by_id = |s: &AnnotationSet| -> HashMap<String, Annotation> {
        s.annotations
            .iter()
            .map(|x| (x.id.as_str().to_string(), x.clone()))
            .collect()
    };
    let (ma, mb) = (by_id(a), by_id(b));

    // Program-made runs added on both sides: the later run only.
    let mut skip: BTreeSet<String> = BTreeSet::new();
    let (runs_a, runs_b) = (added_runs(base, &ma), added_runs(base, &mb));
    for (tag, ra) in &runs_a {
        let Some(rb) = runs_b.get(tag) else {
            continue;
        };
        let ids_a: BTreeSet<&str> = ra.iter().map(|x| x.id.as_str()).collect();
        let ids_b: BTreeSet<&str> = rb.iter().map(|x| x.id.as_str()).collect();
        if ids_a == ids_b {
            continue;
        }
        let newest = |r: &[&Annotation]| {
            r.iter()
                .map(|x| (norm_time(&x.created_at), x.id.as_str().to_string()))
                .max()
        };
        let kept = if newest(ra) >= newest(rb) {
            Side::A
        } else {
            Side::B
        };
        let (keep, drop) = match kept {
            Side::A => (&ids_a, &ids_b),
            Side::B => (&ids_b, &ids_a),
        };
        let dropped: Vec<&str> = drop.difference(keep).copied().collect();
        report.conflicts.push(Conflict::GeneratedTwice {
            tag: tag.clone(),
            kept,
            dropped: dropped.len(),
        });
        skip.extend(dropped.into_iter().map(str::to_string));
    }

    let ids: BTreeSet<&String> = base
        .annotations
        .keys()
        .chain(ma.keys())
        .chain(mb.keys())
        .collect();
    let mut merged: BTreeMap<String, Annotation> = BTreeMap::new();
    for id in ids {
        if skip.contains(id) {
            continue;
        }
        let in_base = base.annotations.get(id);
        match (ma.get(id), mb.get(id)) {
            (None, None) => {}
            (Some(x), None) | (None, Some(x)) => {
                let side = if ma.contains_key(id) {
                    Side::A
                } else {
                    Side::B
                };
                match in_base {
                    None => {
                        report.changes_by_mut(side).added += 1;
                        merged.insert(id.clone(), x.clone());
                    }
                    Some(h) if *h == [content_hash(x), place_hash(x)] => {
                        // Deleted on the other side, untouched here.
                        report.changes_by_mut(side.other()).removed += 1;
                    }
                    Some(_) => {
                        report.conflicts.push(Conflict::ChangedAndDeleted {
                            id: id.clone(),
                            label: label(x),
                            deleted_on: side.other(),
                        });
                        report.changes_by_mut(side).changed += 1;
                        merged.insert(id.clone(), x.clone());
                    }
                }
            }
            (Some(x), Some(y)) => {
                let (out, changed_a, changed_b, conflict) = merge_one(in_base, x, y);
                if changed_a {
                    report.from_a.changed += 1;
                }
                if changed_b {
                    report.from_b.changed += 1;
                }
                if let Some(kept) = conflict {
                    report.conflicts.push(Conflict::BothChanged {
                        id: id.clone(),
                        label: label(&out),
                        kept,
                    });
                }
                merged.insert(id.clone(), out);
            }
        }
    }
    // The other side only learns of an addition it did not make, and of a
    // change it did not make. Additions made on both (same id) count for
    // neither.

    let mut out = set_fields(a, b);
    out.annotations = ordered(merged, a, b);

    // The file's own lowest bat frequency and noise profile.
    let time = |s: &AnnotationSet| s.modified_at.as_deref().map(norm_time);
    let bits = |v: Option<f32>| v.map(f32::to_bits);
    let (lo, lo_conflict) = scalar3(
        bits(base.lowest_bat_hz),
        bits(a.lowest_bat_hz),
        bits(b.lowest_bat_hz),
        time(a),
        time(b),
    );
    out.lowest_bat_hz = lo.map(f32::from_bits);
    // The saved time expansion factors have no copy in the base, so the
    // copy changed later gives them. On a tie the choice is by content, so
    // it does not depend on which copy is which.
    let te_key = |s: &AnnotationSet| {
        (
            time(s),
            serde_json::to_string(&s.time_expansion).unwrap_or_default(),
        )
    };
    let (later, earlier) = if te_key(a) >= te_key(b) {
        (a, b)
    } else {
        (b, a)
    };
    out.time_expansion = later.time_expansion.or(earlier.time_expansion);
    // The record of changes to the audio file goes with the same copy.
    out.file_edits = if later.file_edits.is_empty() {
        earlier.file_edits.clone()
    } else {
        later.file_edits.clone()
    };
    if let Some(kept) = lo_conflict {
        report.conflicts.push(Conflict::LowestBatHz { kept });
    }
    let (noise, noise_conflict) = scalar3(
        base.noise.clone(),
        noise_fp(a),
        noise_fp(b),
        time(a),
        time(b),
    );
    out.noise_profile = match (noise == noise_fp(a), noise == noise_fp(b)) {
        // The same profile on both: the one written later.
        (true, true) => std::cmp::max_by(&a.noise_profile, &b.noise_profile, |p, q| {
            serde_json::to_string(p)
                .unwrap_or_default()
                .cmp(&serde_json::to_string(q).unwrap_or_default())
        })
        .clone(),
        (true, false) => a.noise_profile.clone(),
        _ => b.noise_profile.clone(),
    };
    if let Some(kept) = noise_conflict {
        report.conflicts.push(Conflict::NoiseProfile { kept });
    }
    report.conflicts.sort_by_key(conflict_order);
    (out, report)
}

fn conflict_order(c: &Conflict) -> (u8, String) {
    match c {
        Conflict::BothChanged { id, .. } => (0, id.clone()),
        Conflict::ChangedAndDeleted { id, .. } => (1, id.clone()),
        Conflict::GeneratedTwice { tag, .. } => (2, tag.clone()),
        Conflict::LowestBatHz { .. } => (3, String::new()),
        Conflict::NoiseProfile { .. } => (4, String::new()),
    }
}

/// Annotations not in the base that a program made, by their tag.
fn added_runs<'m>(
    base: &MergeBase,
    m: &'m HashMap<String, Annotation>,
) -> BTreeMap<String, Vec<&'m Annotation>> {
    let mut runs: BTreeMap<String, Vec<&Annotation>> = BTreeMap::new();
    for x in m.values() {
        if base.annotations.contains_key(x.id.as_str()) {
            continue;
        }
        if let Some(g) = generated_key(x) {
            runs.entry(g).or_default().push(x);
        }
    }
    runs
}

/// One annotation present on both sides. Returns the result, whether each
/// side's change is in it, and which side won if both changed the same part.
fn merge_one(
    base: Option<&[String; 2]>,
    x: &Annotation,
    y: &Annotation,
) -> (Annotation, bool, bool, Option<Side>) {
    let (tx, ty) = (norm_time(&x.modified_at), norm_time(&y.modified_at));
    // Which side's version of one part; whether each side's change is in it;
    // the winner if both changed it differently. A tie in time goes to the
    // larger hash, so the answer is the same whichever side is `x`.
    let part = |b: Option<&String>, hx: &String, hy: &String| -> (Side, bool, bool, Option<Side>) {
        if hx == hy {
            return (Side::A, false, false, None);
        }
        match (b != Some(hx), b != Some(hy)) {
            (true, false) => (Side::A, true, false, None),
            (false, true) => (Side::B, false, true, None),
            _ => {
                let w = if (&tx, hx) >= (&ty, hy) {
                    Side::A
                } else {
                    Side::B
                };
                (w, w == Side::A, w == Side::B, Some(w))
            }
        }
    };
    let (content_from, ca, cb, c1) = part(base.map(|h| &h[0]), &content_hash(x), &content_hash(y));
    let (place_from, pa, pb, c2) = part(base.map(|h| &h[1]), &place_hash(x), &place_hash(y));
    let pick = |s: Side| if s == Side::A { x } else { y };
    let mut out = pick(content_from).clone();
    let place = pick(place_from);
    out.parent_id = place.parent_id.clone();
    out.sort_order = place.sort_order;
    // The later of the two times.
    out.modified_at = std::cmp::max_by(x.modified_at.clone(), y.modified_at.clone(), |p, q| {
        norm_time(p).cmp(&norm_time(q)).then_with(|| p.cmp(q))
    });
    (out, ca || pa, cb || pb, c1.or(c2))
}

/// Three-way choice of one value: the side that changed it; when both
/// changed it differently, the copy changed later (`ta`, `tb`: the copies'
/// `modified_at`), or on a tie the larger value.
fn scalar3<T: Ord + Clone>(
    base: T,
    a: T,
    b: T,
    ta: Option<String>,
    tb: Option<String>,
) -> (T, Option<Side>) {
    if a == b {
        return (a, None);
    }
    match (a != base, b != base) {
        (true, false) => (a, None),
        (false, true) => (b, None),
        _ => {
            if (ta, &a) >= (tb, &b) {
                (a, Some(Side::A))
            } else {
                (b, Some(Side::B))
            }
        }
    }
}

/// The set's own fields (not its annotations), taken symmetrically: the
/// recording's identity and metadata fields from whichever copy has them
/// (both have the same recording; a later hash fills a gap), the earliest
/// creation and latest change.
fn set_fields(a: &AnnotationSet, b: &AnnotationSet) -> AnnotationSet {
    let mut out = a.clone();
    out.annotations = Vec::new();
    out.version = a.version.max(b.version);
    out.id = std::cmp::min(&a.id, &b.id).clone();
    out.app_version = std::cmp::max(&a.app_version, &b.app_version).clone();
    out.created_at = match (&a.created_at, &b.created_at) {
        (Some(x), Some(y)) => {
            Some(std::cmp::min_by(x, y, |p, q| norm_time(p).cmp(&norm_time(q))).clone())
        }
        (x, y) => x.clone().or_else(|| y.clone()),
    };
    out.modified_at = match (&a.modified_at, &b.modified_at) {
        (Some(x), Some(y)) => {
            Some(std::cmp::max_by(x, y, |p, q| norm_time(p).cmp(&norm_time(q))).clone())
        }
        (x, y) => x.clone().or_else(|| y.clone()),
    };
    out.file_identity =
        merge_json(&a.file_identity, &b.file_identity).unwrap_or_else(|| a.file_identity.clone());
    out.audio_metadata = match (&a.audio_metadata, &b.audio_metadata) {
        (Some(x), Some(y)) => merge_json(x, y),
        (x, y) => x.clone().or_else(|| y.clone()),
    };
    out.source = later_source(a.source.as_ref(), b.source.as_ref()).cloned();
    out
}

/// Of two copies' recording sources: the one either has, or when both have
/// one, the one fetched later. Ties are broken by content, so the choice does
/// not depend on which copy is which. Never a conflict: the source is the
/// site's data, not the user's.
pub fn later_source<'s>(
    a: Option<&'s RecordingSource>,
    b: Option<&'s RecordingSource>,
) -> Option<&'s RecordingSource> {
    match (a, b) {
        (Some(x), Some(y)) => {
            let key = |s: &RecordingSource| {
                (
                    norm_time(&s.fetched_at),
                    serde_json::to_string(s).unwrap_or_default(),
                )
            };
            Some(if key(x) >= key(y) { x } else { y })
        }
        (x, y) => x.or(y),
    }
}

/// Two JSON objects merged key by key: a value present on one side only is
/// kept; when both have different values, the larger (as JSON text) is.
fn merge_json<T: Serialize + for<'de> Deserialize<'de>>(x: &T, y: &T) -> Option<T> {
    let (serde_json::Value::Object(mut ox), serde_json::Value::Object(oy)) =
        (serde_json::to_value(x).ok()?, serde_json::to_value(y).ok()?)
    else {
        return None;
    };
    for (k, vy) in oy {
        match ox.get(&k) {
            None | Some(serde_json::Value::Null) => {
                ox.insert(k, vy);
            }
            Some(vx) if vy.is_null() || *vx == vy => {}
            Some(vx) => {
                let (text_x, text_y) = (vx.to_string(), vy.to_string());
                if text_y > text_x {
                    ox.insert(k, vy);
                }
            }
        }
    }
    serde_json::from_value(serde_json::Value::Object(ox)).ok()
}

/// The merged annotations in order: one copy's order when the result holds
/// exactly that copy's annotations (so merging a copy with itself, or with a
/// copy that adds nothing, keeps its order), otherwise by creation time.
fn ordered(
    merged: BTreeMap<String, Annotation>,
    a: &AnnotationSet,
    b: &AnnotationSet,
) -> Vec<Annotation> {
    let ids = |s: &AnnotationSet| {
        s.annotations
            .iter()
            .map(|x| x.id.as_str().to_string())
            .collect::<Vec<_>>()
    };
    let (ia, ib) = (ids(a), ids(b));
    let same_ids = |order: &[String]| {
        order.len() == merged.len() && order.iter().all(|id| merged.contains_key(id))
    };
    let order = match (same_ids(&ia), same_ids(&ib)) {
        (true, true) if ia == ib => Some(ia),
        (true, false) => Some(ia),
        (false, true) => Some(ib),
        _ => None,
    };
    let mut merged = merged;
    match order {
        Some(order) => order.iter().filter_map(|id| merged.remove(id)).collect(),
        None => {
            let mut v: Vec<Annotation> = merged.into_values().collect();
            v.sort_by(|p, q| {
                norm_time(&p.created_at)
                    .cmp(&norm_time(&q.created_at))
                    .then_with(|| p.id.as_str().cmp(q.id.as_str()))
            });
            v
        }
    }
}

/// The tag of a program-made annotation: `auto=<id>`, or `xc:set=<id>` for
/// one imported from Xeno-canto.
fn generated_key(a: &Annotation) -> Option<String> {
    if let Some(v) = tags::value_of(&a.tags, None, AUTO) {
        return Some(format!("{AUTO}={v}"));
    }
    tags::value_of(&a.tags, Some(XC_NS), XC_SET).map(|v| format!("{XC_NS}:{XC_SET}={v}"))
}

/// The annotation as JSON, for hashing and for ties.
fn canonical(a: &Annotation) -> String {
    serde_json::to_string(a).unwrap_or_default()
}

fn short_hash(text: &str) -> String {
    blake3::hash(text.as_bytes()).to_hex()[..16].to_string()
}

/// Significant digits kept of each decimal number before hashing.
const HASH_DIGITS: usize = 8;

/// `value` as JSON text with every decimal number rounded to
/// [`HASH_DIGITS`] significant digits. A file read at a corrected rate
/// ("Correct for ×10 time expansion") is converted to the rate it is read at
/// and back before it is compared, and that changes the last digits of its
/// times and frequencies: without the rounding, every annotation would read
/// as changed. A real edit is far larger than the rounding.
fn rounded_json(value: &impl Serialize) -> String {
    fn walk(v: &mut serde_json::Value) {
        match v {
            serde_json::Value::Number(n) if n.is_f64() => {
                let x = n.as_f64().unwrap_or_default();
                let r: f64 = format!("{:.*e}", HASH_DIGITS - 1, x).parse().unwrap_or(x);
                if let Some(m) = serde_json::Number::from_f64(r) {
                    *n = m;
                }
            }
            serde_json::Value::Array(a) => a.iter_mut().for_each(walk),
            serde_json::Value::Object(o) => o.values_mut().for_each(walk),
            _ => {}
        }
    }
    let mut v = serde_json::to_value(value).unwrap_or_default();
    walk(&mut v);
    v.to_string()
}

/// Hash of what an annotation holds, without its time of change and its
/// place in the list.
fn content_hash(a: &Annotation) -> String {
    let mut c = a.clone();
    c.modified_at = String::new();
    c.parent_id = None;
    c.sort_order = None;
    short_hash(&rounded_json(&c))
}

/// Hash of an annotation's place in the list: its group and order.
fn place_hash(a: &Annotation) -> String {
    let place = (
        a.parent_id.as_ref().map(|p| p.as_str()),
        a.sort_order.map(f64::to_bits),
    );
    short_hash(&serde_json::to_string(&place).unwrap_or_default())
}

fn noise_fp(set: &AnnotationSet) -> Option<String> {
    set.noise_profile.as_ref().map(|p| {
        let mut p = p.clone();
        p.created = String::new();
        short_hash(&rounded_json(&p))
    })
}

/// The name shown for an annotation in a report.
fn label(a: &Annotation) -> String {
    crate::annotations::label_of(&a.kind)
        .map(str::to_string)
        .unwrap_or_else(|| crate::annotations::kind_name(&a.kind).to_string())
}

/// An ISO 8601 time with milliseconds, so that times written with and
/// without them compare in time order (`…00Z` and `…00.000Z`).
fn norm_time(t: &str) -> String {
    match t.strip_suffix('Z') {
        Some(head) if !head.contains('.') => format!("{head}.000Z"),
        _ => t.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const T0: &str = "2026-01-01T00:00:00.000Z";
    const T1: &str = "2026-01-02T00:00:00.000Z";
    const T2: &str = "2026-01-03T00:00:00.000Z";

    fn region(id: &str, start: f64, label: &str, at: &str) -> Annotation {
        serde_json::from_value(json!({
            "id": id, "type": "Region", "time_start": start, "time_end": start + 0.5,
            "label": label, "created_at": T0, "modified_at": at,
        }))
        .unwrap()
    }

    fn tagged(id: &str, tag: &str, created: &str) -> Annotation {
        serde_json::from_value(json!({
            "id": id, "type": "Region", "time_start": 1.0, "time_end": 1.1,
            "tags": ["call", tag], "created_at": created, "modified_at": created,
        }))
        .unwrap()
    }

    fn group(id: &str) -> Annotation {
        serde_json::from_value(json!({
            "id": id, "type": "Group", "label": "Pass", "created_at": T0, "modified_at": T0,
        }))
        .unwrap()
    }

    fn set(annotations: Vec<Annotation>, at: &str) -> AnnotationSet {
        let mut s: AnnotationSet = serde_json::from_value(json!({
            "version": 3, "id": "set", "app_version": "0.5.0", "modified_at": at,
            "file_identity": { "filename": "bat.wav", "file_size": 1000 },
        }))
        .unwrap();
        s.annotations = annotations;
        s
    }

    /// A Xeno-canto source fetched at `at`; `n` changes its quality field.
    fn source(at: &str, n: u64) -> RecordingSource {
        let meta = crate::source_site::SiteMetadata::new(
            crate::source_site::SourceSite::XenoCanto,
            vec![
                ("Recordist".into(), "Ann Bee".into()),
                ("Quality".into(), ["A", "B"][n as usize % 2].into()),
                ("URL".into(), "https://xeno-canto.org/928094".into()),
            ],
        );
        RecordingSource::from_site(&meta, at.into())
    }

    #[test]
    fn a_source_on_either_side_is_kept_and_the_later_fetch_wins() {
        let (b, base) = base_of(vec![region("r", 0.0, "a", T0)]);
        let mut a = b.clone();
        a.source = Some(source(T1, 0));
        let mut c = b.clone();
        c.lowest_bat_hz = Some(15_000.0);
        let (m, r) = merge3(&base, &a, &c);
        assert_eq!(m.source, a.source);
        assert!(r.conflicts.is_empty());
        let (m, _) = merge3(&base, &c, &a);
        assert_eq!(m.source, a.source);

        c.source = Some(source(T2, 1));
        for (x, y) in [(&a, &c), (&c, &a)] {
            let (m, r) = merge3(&base, x, y);
            assert_eq!(m.source, c.source, "fetched later");
            assert!(r.conflicts.is_empty());
        }
    }

    #[test]
    fn a_set_that_gains_a_source_has_new_content() {
        let (b, _) = base_of(vec![]);
        let mut a = b.clone();
        a.source = Some(source(T1, 0));
        assert_ne!(fp(&a), fp(&b));
        // The same site data fetched at another time is the same content.
        let mut later = b.clone();
        later.source = Some(source(T2, 0));
        assert_eq!(fp(&a), fp(&later));
    }

    fn ids(s: &AnnotationSet) -> Vec<&str> {
        s.annotations.iter().map(|a| a.id.as_str()).collect()
    }

    fn get<'s>(s: &'s AnnotationSet, id: &str) -> &'s Annotation {
        s.annotations.iter().find(|a| a.id.as_str() == id).unwrap()
    }

    fn label_in(s: &AnnotationSet, id: &str) -> String {
        label(get(s, id))
    }

    fn base_of(annotations: Vec<Annotation>) -> (AnnotationSet, MergeBase) {
        let b = set(annotations, T0);
        let m = MergeBase::of(&b);
        (b, m)
    }

    #[test]
    fn additions_on_either_side_are_kept() {
        let (b, base) = base_of(vec![region("r1", 1.0, "one", T0)]);
        let mut a = b.clone();
        a.annotations.push(region("ra", 2.0, "from a", T1));
        let mut c = b.clone();
        c.annotations.push(region("rb", 3.0, "from b", T1));
        let (m, r) = merge3(&base, &a, &c);
        let mut got = ids(&m);
        got.sort();
        assert_eq!(got, ["r1", "ra", "rb"]);
        assert_eq!(r.from_a.added, 1);
        assert_eq!(r.from_b.added, 1);
        assert!(r.conflicts.is_empty());
        assert!(!r.dropped_from(Side::A) && !r.dropped_from(Side::B));
    }

    #[test]
    fn a_deletion_on_one_side_of_an_untouched_annotation_is_kept() {
        let (b, base) = base_of(vec![
            region("r1", 1.0, "one", T0),
            region("r2", 2.0, "two", T0),
        ]);
        let mut a = b.clone();
        a.annotations.retain(|x| x.id.as_str() != "r2");
        let mut c = b.clone();
        c.annotations.push(region("rb", 3.0, "new", T1));
        let (m, r) = merge3(&base, &a, &c);
        assert!(!ids(&m).contains(&"r2"));
        assert_eq!(r.from_a.removed, 1);
        // B lost r2.
        assert!(r.dropped_from(Side::B));
        assert!(!r.dropped_from(Side::A));
    }

    #[test]
    fn a_change_beats_a_deletion_and_is_reported() {
        let (b, base) = base_of(vec![region("r1", 1.0, "one", T0)]);
        let mut a = b.clone();
        a.annotations.clear();
        a.annotations.push(region("ra", 2.0, "other", T1));
        let mut c = b.clone();
        c.annotations[0] = region("r1", 1.0, "renamed", T1);
        let (m, r) = merge3(&base, &a, &c);
        assert_eq!(label_in(&m, "r1"), "renamed");
        assert_eq!(
            r.conflicts,
            vec![Conflict::ChangedAndDeleted {
                id: "r1".into(),
                label: "renamed".into(),
                deleted_on: Side::A
            }]
        );
    }

    #[test]
    fn a_change_on_one_side_wins_over_an_unchanged_copy() {
        let (b, base) = base_of(vec![
            region("r1", 1.0, "one", T0),
            region("r2", 2.0, "two", T0),
        ]);
        let mut a = b.clone();
        a.annotations[0] = region("r1", 1.0, "changed on a", T1);
        let mut c = b.clone();
        c.annotations[1] = region("r2", 2.0, "changed on b", T1);
        let (m, r) = merge3(&base, &a, &c);
        assert_eq!(label_in(&m, "r1"), "changed on a");
        assert_eq!(label_in(&m, "r2"), "changed on b");
        assert_eq!(r.from_a.changed, 1);
        assert_eq!(r.from_b.changed, 1);
        assert!(r.conflicts.is_empty());
    }

    #[test]
    fn both_changed_one_annotation_the_later_wins_and_it_is_reported() {
        let (b, base) = base_of(vec![region("r1", 1.0, "one", T0)]);
        let mut a = b.clone();
        a.annotations[0] = region("r1", 1.0, "earlier", T1);
        let mut c = b.clone();
        c.annotations[0] = region("r1", 1.0, "later", T2);
        for (x, y, kept) in [(&a, &c, Side::B), (&c, &a, Side::A)] {
            let (m, r) = merge3(&base, x, y);
            assert_eq!(label_in(&m, "r1"), "later");
            assert_eq!(
                r.conflicts,
                vec![Conflict::BothChanged {
                    id: "r1".into(),
                    label: "later".into(),
                    kept
                }]
            );
            assert!(r.dropped_from(kept.other()));
        }
    }

    #[test]
    fn a_move_on_one_side_and_an_edit_on_the_other_are_both_kept() {
        let (b, base) = base_of(vec![group("g"), region("r1", 1.0, "one", T0)]);
        let mut a = b.clone();
        a.annotations[1].parent_id = Some(group("g").id);
        a.annotations[1].sort_order = Some(0.0);
        let mut c = b.clone();
        c.annotations[1] = region("r1", 1.0, "renamed", T1);
        let (m, r) = merge3(&base, &a, &c);
        let r1 = get(&m, "r1");
        assert_eq!(label(r1), "renamed");
        assert_eq!(r1.parent_id.as_ref().map(|p| p.as_str()), Some("g"));
        assert!(r.conflicts.is_empty());
    }

    #[test]
    fn an_annotation_added_to_a_group_deleted_on_the_other_side_is_kept_and_shown_at_the_top() {
        let (b, base) = base_of(vec![group("g")]);
        let mut a = b.clone();
        a.annotations.clear();
        let mut c = b.clone();
        let mut child = region("child", 1.0, "in group", T1);
        child.parent_id = Some(group("g").id);
        c.annotations.push(child);
        let (m, _) = merge3(&base, &a, &c);
        assert_eq!(ids(&m), ["child"]);
        let tree = crate::annotations::build_annotation_tree(&m.annotations);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].annotation.id.as_str(), "child");
    }

    #[test]
    fn detection_run_on_both_sides_keeps_only_the_later_run() {
        let old = tagged("old", "auto=calls-2", T0);
        let (b, base) = base_of(vec![old, region("mine", 2.0, "mine", T0)]);
        let mut a = b.clone();
        a.annotations.retain(|x| x.id.as_str() != "old");
        a.annotations.push(tagged("a1", "auto=calls-2", T1));
        a.annotations.push(tagged("a2", "auto=calls-2", T1));
        let mut c = b.clone();
        c.annotations.retain(|x| x.id.as_str() != "old");
        c.annotations.push(tagged("b1", "auto=calls-2", T2));
        let (m, r) = merge3(&base, &a, &c);
        let mut got = ids(&m);
        got.sort();
        assert_eq!(got, ["b1", "mine"]);
        assert!(r.conflicts.contains(&Conflict::GeneratedTwice {
            tag: "auto=calls-2".into(),
            kept: Side::B,
            dropped: 2
        }));
    }

    #[test]
    fn detection_run_again_on_one_side_replaces_the_old_run() {
        let (b, base) = base_of(vec![tagged("old", "auto=calls-2", T0)]);
        let mut a = b.clone();
        a.annotations = vec![tagged("new", "auto=calls-2", T1)];
        let mut c = b.clone();
        c.annotations.push(region("rb", 2.0, "mine", T1));
        let (m, r) = merge3(&base, &a, &c);
        let mut got = ids(&m);
        got.sort();
        assert_eq!(got, ["new", "rb"]);
        assert!(r.conflicts.is_empty());
    }

    #[test]
    fn xeno_canto_imports_made_on_both_sides_are_not_doubled() {
        let (b, base) = base_of(vec![]);
        let mut a = b.clone();
        a.annotations = vec![tagged("xa", "xc:set=77", T1)];
        let mut c = b.clone();
        c.annotations = vec![tagged("xb", "xc:set=77", T2), region("rb", 2.0, "mine", T2)];
        let (m, _) = merge3(&base, &a, &c);
        let mut got = ids(&m);
        got.sort();
        assert_eq!(got, ["rb", "xb"]);
    }

    #[test]
    fn time_expansion_factors_come_from_the_copy_changed_later() {
        use crate::annotations::SavedTimeExpansion;
        let user = |f: f64| {
            Some(SavedTimeExpansion {
                from_source: None,
                set_by_user: Some(f),
            })
        };
        let (b, base) = base_of(vec![region("r1", 1.0, "one", T0)]);
        // Only one copy has a factor: it is kept, whichever copy is later.
        let mut a = b.clone();
        a.time_expansion = user(10.0);
        a.modified_at = Some(T1.into());
        let mut c = b.clone();
        c.modified_at = Some(T2.into());
        assert_eq!(merge3(&base, &a, &c).0.time_expansion, user(10.0));
        assert_eq!(merge3(&base, &c, &a).0.time_expansion, user(10.0));
        // Two factors: the later copy's.
        c.time_expansion = user(20.0);
        assert_eq!(merge3(&base, &a, &c).0.time_expansion, user(20.0));
        assert_eq!(merge3(&base, &c, &a).0.time_expansion, user(20.0));
        // A factor the user took off is saved as 1, and wins when later.
        c.time_expansion = user(1.0);
        assert_eq!(merge3(&base, &a, &c).0.time_expansion, user(1.0));
    }

    #[test]
    fn lowest_bat_frequency_follows_the_side_that_changed_it() {
        let (b, base) = base_of(vec![region("r1", 1.0, "one", T0)]);
        let mut a = b.clone();
        a.lowest_bat_hz = Some(18_000.0);
        let mut c = b.clone();
        c.annotations.push(region("rb", 2.0, "b", T1));
        let (m, r) = merge3(&base, &a, &c);
        assert_eq!(m.lowest_bat_hz, Some(18_000.0));
        assert!(r.conflicts.is_empty());
        // Both changed it: the later copy's value, reported.
        let mut a2 = a.clone();
        a2.modified_at = Some(T1.into());
        let mut c2 = b.clone();
        c2.lowest_bat_hz = Some(20_000.0);
        c2.modified_at = Some(T2.into());
        let (m, r) = merge3(&base, &a2, &c2);
        assert_eq!(m.lowest_bat_hz, Some(20_000.0));
        assert_eq!(r.conflicts, vec![Conflict::LowestBatHz { kept: Side::B }]);
    }

    #[test]
    fn the_noise_profile_follows_the_side_that_changed_it() {
        let (b, base) = base_of(vec![]);
        let mut a = b.clone();
        a.noise_profile = Some(crate::dsp::notch::NoiseProfile {
            name: "wind".into(),
            bands: Vec::new(),
            source_sample_rate: 384_000,
            created: T1.into(),
            noise_floor: None,
        });
        let c = b.clone();
        let (m, r) = merge3(&base, &a, &c);
        assert_eq!(m.noise_profile.map(|p| p.name), Some("wind".to_string()));
        assert!(r.conflicts.is_empty());
    }

    #[test]
    fn times_with_and_without_milliseconds_compare_in_time_order() {
        assert!(norm_time("2026-01-01T00:00:01Z") > norm_time("2026-01-01T00:00:00.500Z"));
        assert!(norm_time("2026-01-01T00:00:00Z") < norm_time("2026-01-01T00:00:00.001Z"));
    }

    // ── Properties over random edits ────────────────────────────────────

    /// A small seeded generator (no crate needed).
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    const TIMES: [&str; 4] = [T0, T1, T2, "2026-01-03T00:00:00Z"];

    /// A random copy edited from `base`: deletions, label changes, moves into
    /// a group, additions, a detection run, and the lowest bat frequency.
    fn edit(rng: &mut Rng, base: &AnnotationSet, side: &str) -> AnnotationSet {
        let mut s = base.clone();
        s.annotations.retain(|_| rng.below(5) != 0);
        for x in s.annotations.iter_mut() {
            match rng.below(5) {
                0 => {
                    let at = TIMES[rng.below(4) as usize];
                    let new = region(x.id.as_str(), 1.0, &format!("{side}{}", rng.below(3)), at);
                    let (p, o) = (x.parent_id.clone(), x.sort_order);
                    *x = new;
                    x.parent_id = p;
                    x.sort_order = o;
                }
                1 => {
                    x.parent_id = Some(group("g").id);
                    x.sort_order = Some(rng.below(3) as f64);
                }
                _ => {}
            }
        }
        for i in 0..rng.below(3) {
            s.annotations.push(region(
                &format!("{side}-new{i}-{}", rng.below(1000)),
                2.0,
                "n",
                T1,
            ));
        }
        if rng.below(3) == 0 {
            s.annotations.retain(|x| generated_key(x).is_none());
            s.annotations.push(tagged(
                &format!("{side}-auto{}", rng.below(1000)),
                "auto=calls-2",
                TIMES[rng.below(4) as usize],
            ));
        }
        if rng.below(4) == 0 {
            s.lowest_bat_hz = Some(10_000.0 + rng.below(3) as f32 * 1000.0);
        }
        if rng.below(3) == 0 {
            s.source = Some(source(TIMES[rng.below(4) as usize], rng.below(2)));
        }
        s.modified_at = Some(TIMES[rng.below(4) as usize].into());
        s
    }

    fn random_base(rng: &mut Rng) -> AnnotationSet {
        let mut v = vec![group("g")];
        for i in 0..rng.below(6) {
            v.push(region(&format!("r{i}"), i as f64, "base", T0));
        }
        if rng.below(2) == 0 {
            v.push(tagged("auto-old", "auto=calls-2", T0));
        }
        set(v, T0)
    }

    fn fp(s: &AnnotationSet) -> String {
        super::super::content_fingerprint(s)
    }

    #[test]
    fn merging_is_symmetric_idempotent_and_a_copy_with_itself_is_unchanged() {
        let mut rng = Rng(12345);
        for _ in 0..2000 {
            let b = random_base(&mut rng);
            let base = MergeBase::of(&b);
            let x = edit(&mut rng, &b, "x");
            let y = edit(&mut rng, &b, "y");

            let (xy, rxy) = merge3(&base, &x, &y);
            let (yx, ryx) = merge3(&base, &y, &x);
            // The same result whichever side is called `a`.
            assert_eq!(fp(&xy), fp(&yx));
            assert_eq!(rxy.from_a, ryx.from_b);
            assert_eq!(rxy.dropped_from(Side::A), ryx.dropped_from(Side::B));

            // A copy merged with itself is unchanged.
            let (xx, rxx) = merge3(&base, &x, &x);
            assert_eq!(fp(&xx), fp(&x));
            assert_eq!(ids(&xx), ids(&x));
            assert!(rxx.conflicts.is_empty());

            // Merging again with either input changes nothing (the two
            // devices stop at the same content).
            let (again, _) = merge3(&base, &x, &xy);
            assert_eq!(fp(&again), fp(&xy));
            let (again, _) = merge3(&base, &xy, &y);
            assert_eq!(fp(&again), fp(&xy));
            // And with the merge as the new base, nothing changes either.
            let (same, r) = merge3(&MergeBase::of(&xy), &xy, &xy);
            assert_eq!(fp(&same), fp(&xy));
            assert!(r.conflicts.is_empty() && !r.from_a.any() && !r.from_b.any());
        }
    }
}
