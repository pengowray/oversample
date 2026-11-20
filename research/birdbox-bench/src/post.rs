//! From the network's head to boxes in seconds and Hz, then window fusion.
//!
//! Ported from BirdBox v2.1.0 (MIT, BirdNET-Team, https://github.com/birdnet-team/BirdBox):
//! `BirdDetectionGraph.postprocess` and `pixels_to_hz` in `src/inference/onnx_export.py`
//! (which mirror Ultralytics' `non_max_suppression` and BirdBox's `_parse_box_detections`),
//! `_limit_raw_to_content`, and `fuse_window_detections` with its helpers in
//! `src/inference/detect_birds.py` (window ownership, weighted box fusion, nested-box
//! removal, link join).

use crate::frontend::{FMAX, FMIN, IMAGE};
use crate::network::ANCHORS;

pub const CLIP_S: f64 = 3.0;
pub const NMS_IOU: f32 = 0.7;
pub const MAX_DET: usize = 300;
pub const FUSION_WBF_IOU: f64 = 0.4;
pub const FUSION_JOIN_GAP: f64 = -0.05;
pub const FUSION_JOIN_FREQ_IOU: f64 = 0.8;
pub const FUSION_NESTED_RATIO: f64 = 0.8;
/// Just-Bird's defaults (detect_birds.resolve_conf, resolve_phrase_gap).
pub const DEFAULT_CONF: f32 = 0.15;
pub const DEFAULT_PHRASE_GAP: f64 = 0.05;

/// One box of one window, after NMS.
#[derive(Clone, Debug)]
pub struct RawBox {
    pub clip_start: f64,
    pub t0: f64,
    pub t1: f64,
    pub f0: f64,
    pub f1: f64,
    pub conf: f64,
}

/// A merged box (BirdBox's "song" layout).
#[derive(Clone, Debug)]
pub struct Merged {
    pub t0: f64,
    pub t1: f64,
    pub f0: f64,
    pub f1: f64,
    pub avg_conf: f64,
    pub max_conf: f64,
    pub merged: f64,
}

fn hz_to_mel(f: f64) -> f64 {
    2595.0 * (1.0 + f / 700.0).log10()
}

/// Image row to Hz: the HTK mel axis from 50 Hz (row 256) to 15 kHz (row 0), rounded.
pub fn pixels_to_hz(y: f64) -> f64 {
    let (lo, hi) = (hz_to_mel(FMIN), hz_to_mel(FMAX));
    let mel = (1.0 - y / IMAGE as f64) * (hi - lo) + lo;
    let hz = 700.0 * (10f64.powf(mel / 2595.0) - 1.0);
    hz.clamp(FMIN, FMAX).round()
}

/// Head (5 x 1344: cx, cy, w, h, score) of window `clip_start` -> boxes. Scores above
/// `conf`, greedy NMS at IoU above 0.7 from the highest score down (ONNX
/// NonMaxSuppression and torchvision's nms suppress the same way), at most 300.
pub fn decode(head: &[f32], clip_start: f64, conf: f32) -> Vec<RawBox> {
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

/// Drop boxes that start in the end padding and cut the rest at the real end.
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

fn iou_2d(a: &Merged, b: &Merged) -> f64 {
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

fn freq_iou(a: &Merged, b: &Merged) -> f64 {
    let inter = a.f1.min(b.f1) - a.f0.max(b.f0);
    let union = (a.f1 - a.f0) + (b.f1 - b.f0) - inter;
    if inter > 0.0 && union > 0.0 {
        inter / union
    } else {
        0.0
    }
}

/// Window fusion (`fuse_window_detections`), for one class and one file. `file_end` is
/// the end of the last window. `phrase_gap` above 0 joins same-band boxes closer than
/// that; 0 keeps call-level boxes.
pub fn fuse(raw: &[RawBox], file_end: f64, phrase_gap: f64) -> Vec<Merged> {
    // 1. Ownership: a box counts only in the window that owns its centre (the window's
    //    middle half; the first window also owns the file start, the last one the end).
    let owned: Vec<&RawBox> = raw
        .iter()
        .filter(|d| {
            let clip_end = d.clip_start + CLIP_S;
            let quarter = CLIP_S / 4.0;
            let centre = (d.t0 + d.t1) / 2.0;
            let lo = if d.clip_start > 0.0 { d.clip_start + quarter } else { f64::NEG_INFINITY };
            let hi = if clip_end < file_end { clip_end - quarter } else { f64::INFINITY };
            lo <= centre && centre < hi
        })
        .collect();
    if owned.is_empty() {
        return vec![];
    }
    // 2. Weighted box fusion: from the most confident box down, boxes at 2D IoU >= 0.4
    //    to it are averaged in, weighted by confidence; the head's confidence is kept.
    let mut order: Vec<Merged> = owned
        .iter()
        .map(|d| Merged { t0: d.t0, t1: d.t1, f0: d.f0, f1: d.f1, avg_conf: d.conf, max_conf: d.conf, merged: 1.0 })
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
        let avg = |f: &dyn Fn(&Merged) -> f64| -> f64 {
            if wsum > 0.0 {
                members.iter().map(|&m| order[m].avg_conf * f(&order[m])).sum::<f64>() / wsum
            } else {
                members.iter().map(|&m| f(&order[m])).sum::<f64>() / members.len() as f64
            }
        };
        let h = &order[i];
        fused.push(Merged {
            t0: avg(&|m| m.t0),
            t1: avg(&|m| m.t1),
            f0: avg(&|m| m.f0),
            f1: avg(&|m| m.f1),
            avg_conf: h.avg_conf,
            max_conf: h.avg_conf,
            merged: members.len() as f64,
        });
    }
    // 3. Drop a box that lies at least 80% inside a larger one (equal areas: the less
    //    confident, then the later one, is the inner box).
    let area: Vec<f64> = fused.iter().map(|s| (s.t1 - s.t0) * (s.f1 - s.f0)).collect();
    let mut kept: Vec<Merged> = vec![];
    for i in 0..fused.len() {
        let inside = (0..fused.len()).any(|j| {
            if j == i {
                return false;
            }
            let (a, b) = (&fused[i], &fused[j]);
            let inter = (a.t1.min(b.t1) - a.t0.max(b.t0)).max(0.0) * (a.f1.min(b.f1) - a.f0.max(b.f0)).max(0.0);
            let larger = area[j] > area[i]
                || (area[j] == area[i] && (b.max_conf > a.max_conf || (b.max_conf == a.max_conf && j < i)));
            inter >= FUSION_NESTED_RATIO * area[i] && larger
        });
        if !inside {
            kept.push(fused[i].clone());
        }
    }
    kept.sort_by(|a, b| a.t0.total_cmp(&b.t0));
    // 4. Link join, repeated until nothing joins: boxes in one band (frequency IoU >= 0.8)
    //    whose time gap is at most the join gap become one box.
    let gap = if phrase_gap > 0.0 { phrase_gap } else { FUSION_JOIN_GAP };
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

fn link_pass(songs: &[Merged], gap: f64) -> Vec<Merged> {
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
            Merged {
                t0: m.iter().map(|&i| songs[i].t0).fold(f64::INFINITY, f64::min),
                t1: m.iter().map(|&i| songs[i].t1).fold(f64::NEG_INFINITY, f64::max),
                f0: m.iter().map(|&i| songs[i].f0).fold(f64::INFINITY, f64::min),
                f1: m.iter().map(|&i| songs[i].f1).fold(f64::NEG_INFINITY, f64::max),
                max_conf: m.iter().map(|&i| songs[i].max_conf).fold(f64::NEG_INFINITY, f64::max),
                avg_conf: m.iter().map(|&i| songs[i].avg_conf * songs[i].merged).sum::<f64>() / count,
                merged: count,
            }
        })
        .collect()
}
