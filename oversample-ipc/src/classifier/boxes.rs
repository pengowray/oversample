//! BirdBox "Just-Bird" boxes: from the network's output to boxes in seconds and
//! Hz (NMS), and the window fusion that merges the boxes of overlapping windows.
//!
//! Just-Bird (BirdBox v2.1.0, a YOLO11s detector with one class, `bird`) runs on
//! 3 s windows of 32 kHz audio that start every 1.5 s. The worker
//! (`birdnet-worker/src/birdbox/`) makes each window's image, runs the network
//! and keeps the boxes [`decode`] gives at [`KEEP_CONF`] or above. The app keeps
//! those window boxes and runs [`fuse`] itself, so a new minimum confidence or
//! box size merges again at once without running the model.
//!
//! Ported from BirdBox v2.1.0 (MIT, BirdNET-Team,
//! <https://github.com/birdnet-team/BirdBox>): `BirdDetectionGraph.postprocess`
//! and `pixels_to_hz` in `src/inference/onnx_export.py` (which mirror
//! Ultralytics' `non_max_suppression` and BirdBox's `_parse_box_detections`),
//! `_limit_raw_to_content`, and `fuse_window_detections` with its helpers in
//! `src/inference/detect_birds.py` (window ownership, weighted box fusion,
//! nested-box removal, link join). Checked against BirdBox's own CLI in
//! `research/birdbox-bench`.

use serde::{Deserialize, Serialize};

/// Length of one window, in seconds.
pub const CLIP_S: f64 = 3.0;
/// Time between window starts, in seconds.
pub const CLIP_HOP_S: f64 = 1.5;
/// The network's input image is this many pixels square.
pub const IMAGE: usize = 256;
/// Anchors in the network's output: 32 x 32, 16 x 16 and 8 x 8 grid cells.
pub const ANCHORS: usize = 1344;
/// Values in the network's output: centre x, centre y, width, height and score
/// for each anchor, row by row.
pub const HEAD: usize = 5 * ANCHORS;
/// The image's frequency axis: HTK mel from 50 Hz (bottom) to 15 kHz (top).
pub const FMIN: f64 = 50.0;
pub const FMAX: f64 = 15_000.0;
pub const NMS_IOU: f32 = 0.7;
pub const MAX_DET: usize = 300;
const FUSION_WBF_IOU: f64 = 0.4;
const FUSION_JOIN_GAP: f64 = -0.05;
const FUSION_JOIN_FREQ_IOU: f64 = 0.8;
const FUSION_NESTED_RATIO: f64 = 0.8;
/// The lowest score the worker keeps, as BirdBox's demo does, so that any
/// minimum from here up can be applied afterwards. NMS keeps the same boxes
/// above a minimum whether it ran at this score or at the minimum (a box is
/// only ever removed by a box with a higher score).
pub const KEEP_CONF: f32 = 0.05;
/// Just-Bird's default minimum confidence (`detect_birds.resolve_conf`).
pub const DEFAULT_CONF: f32 = 0.15;
/// Just-Bird's default phrase gap in seconds (`resolve_phrase_gap`): boxes in
/// one band closer than this become one box. 0 keeps the boxes of single calls.
pub const DEFAULT_PHRASE_GAP: f64 = 0.05;

/// One box of one window, after NMS.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RawBox {
    /// Start of the window, in seconds.
    pub clip_start: f64,
    /// Start and end, in seconds in the recording.
    pub t0: f64,
    pub t1: f64,
    /// Lowest and highest frequency, in Hz.
    pub f0: f64,
    pub f1: f64,
    /// The network's score, 0 to 1.
    pub conf: f64,
}

/// A box after window fusion (BirdBox's "song" layout).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MergedBox {
    pub t0: f64,
    pub t1: f64,
    pub f0: f64,
    pub f1: f64,
    /// Mean confidence of the boxes joined into it, weighted by how many
    /// window boxes each holds.
    pub avg_conf: f64,
    /// Highest confidence of the boxes joined into it.
    pub max_conf: f64,
    /// How many window boxes were merged into it.
    pub merged: f64,
}

fn hz_to_mel(f: f64) -> f64 {
    2595.0 * (1.0 + f / 700.0).log10()
}

/// Image row to Hz: the HTK mel axis from 50 Hz (row 256) to 15 kHz (row 0),
/// rounded to whole Hz.
pub fn pixels_to_hz(y: f64) -> f64 {
    let (lo, hi) = (hz_to_mel(FMIN), hz_to_mel(FMAX));
    let mel = (1.0 - y / IMAGE as f64) * (hi - lo) + lo;
    let hz = 700.0 * (10f64.powf(mel / 2595.0) - 1.0);
    hz.clamp(FMIN, FMAX).round()
}

/// The network's output for the window starting at `clip_start` (seconds) to
/// boxes: scores above `conf`, greedy NMS at IoU above 0.7 from the highest
/// score down (ONNX NonMaxSuppression and torchvision's nms suppress the same
/// way), at most 300, clamped to the image. `head` holds [`HEAD`] values.
pub fn decode(head: &[f32], clip_start: f64, conf: f32) -> Vec<RawBox> {
    if head.len() < HEAD {
        return Vec::new();
    }
    let at = |r: usize, a: usize| head[r * ANCHORS + a];
    let mut cand: Vec<usize> = (0..ANCHORS).filter(|&a| at(4, a) > conf).collect();
    cand.sort_by(|&a, &b| at(4, b).total_cmp(&at(4, a)));
    let corners = |a: usize| {
        let (cx, cy, w, h) = (at(0, a), at(1, a), at(2, a), at(3, a));
        (cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0)
    };
    let iou = |a: usize, b: usize| {
        let (ax0, ay0, ax1, ay1) = corners(a);
        let (bx0, by0, bx1, by1) = corners(b);
        let iw = (ax1.min(bx1) - ax0.max(bx0)).max(0.0);
        let ih = (ay1.min(by1) - ay0.max(by0)).max(0.0);
        let inter = iw * ih;
        let union = (ax1 - ax0) * (ay1 - ay0) + (bx1 - bx0) * (by1 - by0) - inter;
        if union > 0.0 {
            inter / union
        } else {
            0.0
        }
    };
    let mut kept: Vec<usize> = vec![];
    for &a in &cand {
        if kept.len() >= MAX_DET {
            break;
        }
        if kept.iter().all(|&k| iou(k, a) <= NMS_IOU) {
            kept.push(a);
        }
    }
    let size = IMAGE as f32;
    kept.into_iter()
        .map(|a| {
            let (x0, y0, x1, y1) = corners(a);
            let (x0, x1) = (x0.clamp(0.0, size) as f64, x1.clamp(0.0, size) as f64);
            let (y0, y1) = (y0.clamp(0.0, size) as f64, y1.clamp(0.0, size) as f64);
            RawBox {
                clip_start,
                t0: clip_start + x0 / IMAGE as f64 * CLIP_S,
                t1: clip_start + x1 / IMAGE as f64 * CLIP_S,
                f0: pixels_to_hz(y1),
                f1: pixels_to_hz(y0),
                conf: at(4, a) as f64,
            }
        })
        .collect()
}

/// Drop boxes that start in the silence added after the end of the audio
/// (`content_end`, seconds) to complete the last window (which ends at
/// `file_end`), and cut the rest at the end of the audio.
pub fn limit_to_content(raw: Vec<RawBox>, content_end: f64, file_end: f64) -> Vec<RawBox> {
    if file_end <= content_end + 0.5 / 32000.0 {
        return raw;
    }
    raw.into_iter()
        .filter_map(|mut r| {
            if r.t0 >= content_end {
                return None;
            }
            r.t1 = r.t1.min(content_end);
            (r.t1 > r.t0).then_some(r)
        })
        .collect()
}

/// The end of the last window for `samples` samples of 32 kHz audio: the
/// first window on the 1.5 s grid that holds the last sample, and at least
/// one window. In seconds.
pub fn padded_end_s(samples: u64) -> f64 {
    let clip = (CLIP_S * 32_000.0) as u64;
    let hop = (CLIP_HOP_S * 32_000.0) as u64;
    let extra = samples.saturating_sub(clip);
    (clip + extra.div_ceil(hop) * hop) as f64 / 32_000.0
}

fn iou_2d(a: &MergedBox, b: &MergedBox) -> f64 {
    let t = a.t1.min(b.t1) - a.t0.max(b.t0);
    let f = a.f1.min(b.f1) - a.f0.max(b.f0);
    if t <= 0.0 || f <= 0.0 {
        return 0.0;
    }
    let inter = t * f;
    let union = (a.t1 - a.t0) * (a.f1 - a.f0) + (b.t1 - b.t0) * (b.f1 - b.f0) - inter;
    if union > 0.0 {
        inter / union
    } else {
        0.0
    }
}

fn freq_iou(a: &MergedBox, b: &MergedBox) -> f64 {
    let inter = a.f1.min(b.f1) - a.f0.max(b.f0);
    let union = (a.f1 - a.f0) + (b.f1 - b.f0) - inter;
    if inter > 0.0 && union > 0.0 {
        inter / union
    } else {
        0.0
    }
}

/// Window fusion (`fuse_window_detections`) of one file's window boxes.
/// `file_end` is the end of the last window: give `f64::INFINITY` while windows
/// are still arriving, so the newest window does not yet own the end of its
/// audio. `phrase_gap` above 0 joins boxes in one band that are closer than
/// that; 0 keeps the boxes of single calls.
pub fn fuse(raw: &[RawBox], file_end: f64, phrase_gap: f64) -> Vec<MergedBox> {
    // 1. Ownership: a box counts only in the window that owns its centre (the
    //    window's middle half; the first window also owns the start of the
    //    file, the last one its end).
    let owned: Vec<&RawBox> = raw
        .iter()
        .filter(|d| {
            let clip_end = d.clip_start + CLIP_S;
            let quarter = CLIP_S / 4.0;
            let centre = (d.t0 + d.t1) / 2.0;
            let lo = if d.clip_start > 0.0 {
                d.clip_start + quarter
            } else {
                f64::NEG_INFINITY
            };
            let hi = if clip_end < file_end {
                clip_end - quarter
            } else {
                f64::INFINITY
            };
            lo <= centre && centre < hi
        })
        .collect();
    if owned.is_empty() {
        return vec![];
    }
    // 2. Weighted box fusion: from the most confident box down, boxes at 2D IoU
    //    0.4 or more to it are averaged in, weighted by confidence; the first
    //    box's confidence is kept.
    let mut order: Vec<MergedBox> = owned
        .iter()
        .map(|d| MergedBox {
            t0: d.t0,
            t1: d.t1,
            f0: d.f0,
            f1: d.f1,
            avg_conf: d.conf,
            max_conf: d.conf,
            merged: 1.0,
        })
        .collect();
    order.sort_by(|a, b| b.avg_conf.total_cmp(&a.avg_conf)); // stable, as Python's sorted
    let mut by_start: Vec<usize> = (0..order.len()).collect();
    by_start.sort_by(|&a, &b| order[a].t0.total_cmp(&order[b].t0));
    let mut assigned = vec![false; order.len()];
    let mut fused = vec![];
    for i in 0..order.len() {
        if assigned[i] {
            continue;
        }
        assigned[i] = true;
        let mut members = vec![i];
        for &j in &by_start {
            if !assigned[j] && iou_2d(&order[i], &order[j]) >= FUSION_WBF_IOU {
                assigned[j] = true;
                members.push(j);
            }
        }
        let wsum: f64 = members.iter().map(|&m| order[m].avg_conf).sum();
        let avg = |f: &dyn Fn(&MergedBox) -> f64| -> f64 {
            if wsum > 0.0 {
                members
                    .iter()
                    .map(|&m| order[m].avg_conf * f(&order[m]))
                    .sum::<f64>()
                    / wsum
            } else {
                members.iter().map(|&m| f(&order[m])).sum::<f64>() / members.len() as f64
            }
        };
        let h = &order[i];
        fused.push(MergedBox {
            t0: avg(&|m| m.t0),
            t1: avg(&|m| m.t1),
            f0: avg(&|m| m.f0),
            f1: avg(&|m| m.f1),
            avg_conf: h.avg_conf,
            max_conf: h.avg_conf,
            merged: members.len() as f64,
        });
    }
    // 3. Drop a box that lies at least 80% inside a larger one (equal areas:
    //    the less confident, then the later one, is the inner box).
    let area: Vec<f64> = fused
        .iter()
        .map(|s| (s.t1 - s.t0) * (s.f1 - s.f0))
        .collect();
    let mut kept: Vec<MergedBox> = vec![];
    for i in 0..fused.len() {
        let inside = (0..fused.len()).any(|j| {
            if j == i {
                return false;
            }
            let (a, b) = (&fused[i], &fused[j]);
            let inter = (a.t1.min(b.t1) - a.t0.max(b.t0)).max(0.0)
                * (a.f1.min(b.f1) - a.f0.max(b.f0)).max(0.0);
            let larger = area[j] > area[i]
                || (area[j] == area[i]
                    && (b.max_conf > a.max_conf || (b.max_conf == a.max_conf && j < i)));
            inter >= FUSION_NESTED_RATIO * area[i] && larger
        });
        if !inside {
            kept.push(fused[i]);
        }
    }
    kept.sort_by(|a, b| a.t0.total_cmp(&b.t0));
    // 4. Link join, repeated until nothing joins: boxes in one band (frequency
    //    IoU 0.8 or more) whose time gap is at most the join gap become one.
    let gap = if phrase_gap > 0.0 {
        phrase_gap
    } else {
        FUSION_JOIN_GAP
    };
    let mut linked = kept;
    linked.sort_by(|a, b| a.t0.total_cmp(&b.t0).then(a.f0.total_cmp(&b.f0)));
    loop {
        let again = link_pass(&linked, gap);
        if again.len() == linked.len() {
            break;
        }
        linked = again;
    }
    linked.sort_by(|a, b| a.t0.total_cmp(&b.t0));
    linked
}

fn link_pass(songs: &[MergedBox], gap: f64) -> Vec<MergedBox> {
    let n = songs.len();
    if n < 2 {
        return songs.to_vec();
    }
    let mut parent: Vec<usize> = (0..n).collect();
    fn root(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    for i in 0..n {
        for j in i + 1..n {
            let (a, b) = (&songs[i], &songs[j]);
            let tgap = a.t0.max(b.t0) - a.t1.min(b.t1);
            if tgap <= gap && freq_iou(a, b) >= FUSION_JOIN_FREQ_IOU {
                let (ri, rj) = (root(&mut parent, i), root(&mut parent, j));
                if ri != rj {
                    parent[ri.max(rj)] = ri.min(rj);
                }
            }
        }
    }
    let mut groups: Vec<(usize, Vec<usize>)> = vec![];
    for i in 0..n {
        let r = root(&mut parent, i);
        match groups.iter_mut().find(|g| g.0 == r) {
            Some(g) => g.1.push(i),
            None => groups.push((r, vec![i])),
        }
    }
    groups
        .into_iter()
        .map(|(_, m)| {
            let count: f64 = m.iter().map(|&i| songs[i].merged).sum();
            MergedBox {
                t0: m.iter().map(|&i| songs[i].t0).fold(f64::INFINITY, f64::min),
                t1: m
                    .iter()
                    .map(|&i| songs[i].t1)
                    .fold(f64::NEG_INFINITY, f64::max),
                f0: m.iter().map(|&i| songs[i].f0).fold(f64::INFINITY, f64::min),
                f1: m
                    .iter()
                    .map(|&i| songs[i].f1)
                    .fold(f64::NEG_INFINITY, f64::max),
                max_conf: m
                    .iter()
                    .map(|&i| songs[i].max_conf)
                    .fold(f64::NEG_INFINITY, f64::max),
                avg_conf: m
                    .iter()
                    .map(|&i| songs[i].avg_conf * songs[i].merged)
                    .sum::<f64>()
                    / count,
                merged: count,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_tsv(text: &str) -> Vec<Vec<f64>> {
        text.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.split('\t').map(|x| x.trim().parse().unwrap()).collect())
            .collect()
    }

    /// BirdBox's CLI on the Gentoo Penguin demo (47.024 s at 32 kHz): its
    /// window boxes in, its merged boxes out (`research/birdbox-bench`).
    #[test]
    fn fusion_matches_birdbox_on_the_gentoo_penguins() {
        let raw: Vec<RawBox> = read_tsv(include_str!("boxes_fixtures/gentoo.cli.raw.tsv"))
            .into_iter()
            .map(|v| RawBox {
                clip_start: v[0],
                t0: v[1],
                t1: v[2],
                f0: v[3],
                f1: v[4],
                conf: v[5],
            })
            .collect();
        let want = read_tsv(include_str!("boxes_fixtures/gentoo.cli.boxes.tsv"));
        assert_eq!(raw.len(), 76);
        let file_end = padded_end_s(1_504_768);
        assert_eq!(file_end, 48.0);
        let got = fuse(&raw, file_end, DEFAULT_PHRASE_GAP);
        assert_eq!(got.len(), want.len());
        assert_eq!(got.len(), 27);
        for (g, w) in got.iter().zip(&want) {
            let have = [g.t0, g.t1, g.f0, g.f1, g.avg_conf, g.max_conf, g.merged];
            for (a, b) in have.iter().zip(w) {
                // The fixture has 6 decimals.
                assert!((a - b).abs() < 1e-4 * b.abs().max(1.0), "{have:?} vs {w:?}");
            }
        }
        // Call-sized boxes (no phrase join) are at least as many.
        assert!(fuse(&raw, file_end, 0.0).len() >= got.len());
    }

    #[test]
    fn a_window_still_arriving_does_not_own_its_end() {
        let b = |clip_start: f64, t0: f64, t1: f64| RawBox {
            clip_start,
            t0,
            t1,
            f0: 1000.0,
            f1: 2000.0,
            conf: 0.5,
        };
        // Centre 4.0 s, in the last quarter of the window at 1.5 s.
        let raw = [b(1.5, 3.8, 4.2)];
        assert!(fuse(&raw, f64::INFINITY, 0.05).is_empty());
        assert_eq!(fuse(&raw, 4.5, 0.05).len(), 1);
        // The window at 3 s owns it once it has come.
        let raw = [b(1.5, 3.8, 4.2), b(3.0, 3.81, 4.19)];
        assert_eq!(fuse(&raw, f64::INFINITY, 0.05).len(), 1);
    }

    /// A head of `n` made-up boxes: (centre x, centre y, width, height, score)
    /// in the first anchors, 0 elsewhere.
    fn head(boxes: &[(f32, f32, f32, f32, f32)]) -> Vec<f32> {
        let mut h = vec![0.0; HEAD];
        for (a, b) in boxes.iter().enumerate() {
            let v = [b.0, b.1, b.2, b.3, b.4];
            for (r, x) in v.iter().enumerate() {
                h[r * ANCHORS + a] = *x;
            }
        }
        h
    }

    #[test]
    fn nms_keeps_the_same_boxes_above_a_minimum_whatever_it_ran_at() {
        // Overlapping pairs and singles at many scores.
        let mut boxes = Vec::new();
        for i in 0..40 {
            let x = 10.0 + (i % 10) as f32 * 22.0;
            let y = 40.0 + (i / 10) as f32 * 50.0;
            let score = 0.06 + 0.023 * ((i * 7) % 40) as f32;
            boxes.push((x, y, 30.0, 20.0, score));
            boxes.push((x + 2.0, y + 1.0, 30.0, 20.0, score * 0.9));
        }
        let h = head(&boxes);
        let low = decode(&h, 6.0, KEEP_CONF);
        for min in [0.15f32, 0.3, 0.5] {
            let direct = decode(&h, 6.0, min);
            let filtered: Vec<RawBox> = low
                .iter()
                .filter(|b| b.conf > min as f64)
                .copied()
                .collect();
            assert_eq!(direct, filtered, "minimum {min}");
            assert!(!direct.is_empty());
        }
    }

    #[test]
    fn decode_gives_seconds_and_hz() {
        let h = head(&[(128.0, 128.0, 64.0, 256.0, 0.9)]);
        let b = decode(&h, 1.5, 0.15);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].t0, 1.5 + 96.0 / 256.0 * 3.0);
        assert_eq!(b[0].t1, 1.5 + 160.0 / 256.0 * 3.0);
        assert_eq!(b[0].f0, FMIN);
        assert_eq!(b[0].f1, FMAX);
        assert!((b[0].conf - 0.9).abs() < 1e-6);
        assert!(decode(&h[..10], 0.0, 0.1).is_empty());
    }

    #[test]
    fn padding() {
        assert_eq!(padded_end_s(10), 3.0);
        assert_eq!(padded_end_s(96_000), 3.0);
        assert_eq!(padded_end_s(96_001), 4.5);
        assert_eq!(padded_end_s(144_000), 4.5);
        let raw = vec![
            RawBox {
                clip_start: 0.0,
                t0: 0.5,
                t1: 2.9,
                f0: 1.0,
                f1: 2.0,
                conf: 0.5,
            },
            RawBox {
                clip_start: 0.0,
                t0: 2.6,
                t1: 2.9,
                f0: 1.0,
                f1: 2.0,
                conf: 0.5,
            },
        ];
        let cut = limit_to_content(raw, 2.5, 3.0);
        assert_eq!(cut.len(), 1);
        assert_eq!(cut[0].t1, 2.5);
    }
}
