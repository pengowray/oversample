//! Positions for the species map: BirdNET's classes from
//! `research/species-map/birdnet-map.bin`, and each window of a run placed
//! among the species it scored highest.

use super::summary::{window_kind, WindowKind};
use super::ClassifierRun;

/// Map positions, -1 to 1 on both axes, one per BirdNET class in label order.
pub fn parse_map(bytes: &[u8]) -> Vec<(f32, f32)> {
    bytes
        .chunks_exact(4)
        .map(|c| {
            let x = i16::from_le_bytes([c[0], c[1]]) as f32 / 32767.0;
            let y = i16::from_le_bytes([c[2], c[3]]) as f32 / 32767.0;
            (x, y)
        })
        .collect()
}

/// How many of a window's top classes can pull its dot towards them.
const PLACE_TOP: usize = 3;
/// A class further than this from the top class on the map (in map units; the
/// map is 2 wide) is left out, so a window is never drawn in the empty space
/// between two unrelated groups.
const PLACE_RADIUS: f32 = 0.15;
/// Spacing of the spiral that keeps windows with the same position apart.
const SPREAD: f32 = 0.006;

/// One window's dot.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedWindow {
    pub index: usize,
    pub x: f32,
    pub y: f32,
}

/// Place each window at or above `min_confidence` whose top class is a
/// species: at the confidence-weighted mean of its top 3 classes' positions,
/// counting only classes near the top class. Windows that land on the same
/// spot are spread on a small spiral so each can be hovered.
pub fn place_windows(
    run: &ClassifierRun,
    map: &[(f32, f32)],
    min_confidence: f32,
) -> Vec<PlacedWindow> {
    let mut out: Vec<PlacedWindow> = Vec::new();
    let mut at_spot: std::collections::HashMap<(i32, i32), usize> = Default::default();
    for (i, w) in run.windows.iter().enumerate() {
        if window_kind(run, i, min_confidence) != WindowKind::Species {
            continue;
        }
        let Some(&(x1, y1)) = map.get(w.top[0].class as usize) else {
            continue;
        };
        let (mut sx, mut sy, mut sw) = (0.0f32, 0.0f32, 0.0f32);
        for s in w.top.iter().take(PLACE_TOP) {
            let Some(&(x, y)) = map.get(s.class as usize) else {
                continue;
            };
            if ((x - x1).powi(2) + (y - y1).powi(2)).sqrt() > PLACE_RADIUS {
                continue;
            }
            sx += x * s.confidence;
            sy += y * s.confidence;
            sw += s.confidence;
        }
        let (mut x, mut y) = if sw > 0.0 {
            (sx / sw, sy / sw)
        } else {
            (x1, y1)
        };
        let spot = ((x / SPREAD).round() as i32, (y / SPREAD).round() as i32);
        let n = at_spot.entry(spot).or_insert(0);
        if *n > 0 {
            let k = *n as f32;
            let angle = k * 2.4;
            let r = SPREAD * k.sqrt();
            x += r * angle.cos();
            y += r * angle.sin();
        }
        *n += 1;
        out.push(PlacedWindow { index: i, x, y });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classifier::types::{parse_labels, WindowScores};
    use crate::classifier::{catalog, Method, Score};
    use std::sync::Arc;

    fn run(tops: &[&[(u16, f32)]]) -> ClassifierRun {
        ClassifierRun {
            method: Method::BirdNet,
            region: None,
            backbone: &catalog::BACKBONE,
            head: None,
            source_rate: 48_000,
            model_rate: 48_000,
            window_secs: 3.0,
            labels: Arc::new(parse_labels("A a_A\nB b_B\nC c_C\n")),
            windows: tops
                .iter()
                .enumerate()
                .map(|(i, t)| WindowScores {
                    start_s: i as f64 * 3.0,
                    end_s: (i + 1) as f64 * 3.0,
                    top: t
                        .iter()
                        .map(|&(class, confidence)| Score { class, confidence })
                        .collect(),
                })
                .collect(),
            complete: true,
        }
    }

    #[test]
    fn map_file_parses() {
        let m = parse_map(&[0xff, 0x7f, 0x01, 0x80]);
        assert_eq!(m, vec![(1.0, -1.0)]);
    }

    #[test]
    fn near_classes_pull_far_ones_do_not() {
        let map = [(0.0, 0.0), (0.1, 0.0), (0.9, 0.9)];
        let r = run(&[&[(0, 0.8), (1, 0.8), (2, 0.8)], &[(0, 0.2)]]);
        let p = place_windows(&r, &map, 0.5);
        assert_eq!(p.len(), 1);
        assert!((p[0].x - 0.05).abs() < 1e-6, "{:?}", p[0]);
        assert_eq!(p[0].y, 0.0);
    }

    #[test]
    fn same_spot_windows_are_spread() {
        let map = [(0.0, 0.0), (0.5, 0.5), (0.9, 0.9)];
        let r = run(&[&[(0, 0.9)], &[(0, 0.9)]]);
        let p = place_windows(&r, &map, 0.5);
        assert_eq!(p.len(), 2);
        assert_ne!((p[0].x, p[0].y), (p[1].x, p[1].y));
    }
}
