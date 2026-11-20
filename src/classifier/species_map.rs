//! Positions for the species map: each model's classes from its map file in
//! `research/species-map` (`birdnet-map.bin`, `birdnet-v3-map.bin`,
//! `perch-map.bin`), and each window
//! of a run placed among the species it scored highest.

use super::summary::{window_kind, WindowKind};
use super::{ClassifierRun, Method};

/// One model's species map.
pub struct MapSpec {
    /// The map file, served next to the app (index.html copies it).
    pub file: &'static str,
    /// A class further than this from the window's top class on the map (in
    /// map units; the map is 2 wide) does not pull the window's dot, so a
    /// window is never drawn in the empty space between two unrelated groups.
    pub radius: f32,
    /// Shown in the legend beside the grey points.
    pub classes: &'static str,
    /// The model's name in "Species that X hears as alike".
    pub model: &'static str,
    /// Where the positions come from, and the licence.
    pub source: &'static str,
}

pub const BIRDNET_MAP: MapSpec = MapSpec {
    file: "birdnet-map.bin",
    radius: 0.15,
    classes: "BirdNET's 6,522 species and sounds",
    model: "BirdNET",
    source: "Positions from BirdNET V2.4's classifier weights. CC BY-NC-SA 4.0.",
};

pub const PERCH_MAP: MapSpec = MapSpec {
    file: "perch-map.bin",
    radius: PERCH_RADIUS,
    classes: "Perch's 14,795 species and sounds",
    model: "Perch",
    source: "Positions from Perch 2.0's classifier weights. Apache 2.0.",
};

/// See research/species-map/README.md: BirdNET's 0.15, scaled by how much
/// closer together classes sit on the Perch map.
const PERCH_RADIUS: f32 = 0.09;

pub const BIRDNET_V3_MAP: MapSpec = MapSpec {
    file: "birdnet-v3-map.bin",
    radius: BIRDNET_V3_RADIUS,
    classes: "BirdNET+ V3.0's 11,725 species and sounds",
    model: "BirdNET+ V3.0",
    source: "Positions from BirdNET+ V3.0's classifier weights. Apache 2.0.",
};

/// BirdNET's 0.15, scaled by how much closer together classes sit on the
/// V3.0 map: the median distance to a class's 3rd nearest class is 0.0104
/// map units, against 0.0149 on BirdNET's (research/species-map/README.md,
/// `v3-check.txt`).
const BIRDNET_V3_RADIUS: f32 = 0.105;

/// The species map for `method`, if it has one. BattyBirdNET's regional
/// classifiers give no usable map (research/species-map/README.md).
pub fn map_spec(method: Method) -> Option<&'static MapSpec> {
    match method {
        Method::BirdNet => Some(&BIRDNET_MAP),
        Method::BirdNetV3 => Some(&BIRDNET_V3_MAP),
        Method::Perch => Some(&PERCH_MAP),
        Method::Bats => None,
    }
}

/// Map positions, -1 to 1 on both axes, one per class in label order. A class
/// written as (-32768, -32768) has no position (Perch's classes that never
/// score above their bias).
pub fn parse_map(bytes: &[u8]) -> Vec<Option<(f32, f32)>> {
    bytes
        .chunks_exact(4)
        .map(|c| {
            let x = i16::from_le_bytes([c[0], c[1]]);
            let y = i16::from_le_bytes([c[2], c[3]]);
            (x != i16::MIN || y != i16::MIN).then(|| (x as f32 / 32767.0, y as f32 / 32767.0))
        })
        .collect()
}

/// How many of a window's top classes can pull its dot towards them.
const PLACE_TOP: usize = 3;
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
    map: &[Option<(f32, f32)>],
    radius: f32,
    min_confidence: f32,
) -> Vec<PlacedWindow> {
    let mut out: Vec<PlacedWindow> = Vec::new();
    let mut at_spot: std::collections::HashMap<(i32, i32), usize> = Default::default();
    for (i, w) in run.windows.iter().enumerate() {
        if window_kind(run, i, min_confidence) != WindowKind::Species {
            continue;
        }
        let Some(&Some((x1, y1))) = map.get(w.top[0].class as usize) else {
            continue;
        };
        let (mut sx, mut sy, mut sw) = (0.0f32, 0.0f32, 0.0f32);
        for s in w.top.iter().take(PLACE_TOP) {
            let Some(&Some((x, y))) = map.get(s.class as usize) else {
                continue;
            };
            if ((x - x1).powi(2) + (y - y1).powi(2)).sqrt() > radius {
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
            step_secs: 3.0,
            spacing_secs: 3.0,
            labels: Arc::new(parse_labels("A a_A\nB b_B\nC c_C\n")),
            windows: tops
                .iter()
                .enumerate()
                .map(|(i, t)| WindowScores {
                    start_s: i as f64 * 3.0,
                    end_s: (i + 1) as f64 * 3.0,
                    slot: i as u32,
                    live: false,
                    top: t
                        .iter()
                        .map(|&(class, confidence)| Score { class, confidence })
                        .collect(),
                })
                .collect(),
            whole_file: true,
            complete: true,
        }
    }

    /// The shipped V3.0 map has a finite place inside the map for every one
    /// of the model's classes, and its radius is set.
    #[test]
    fn birdnet_v3_map_covers_every_class() {
        let m = parse_map(include_bytes!(
            "../../research/species-map/birdnet-v3-map.bin"
        ));
        assert_eq!(m.len(), oversample_ipc::classifier::V3_CLASSES);
        assert!(m.iter().all(|p| p.is_some_and(|(x, y)| x.is_finite()
            && y.is_finite()
            && x.abs() <= 1.0
            && y.abs() <= 1.0)));
        let spec = map_spec(Method::BirdNetV3).expect("a map for V3.0");
        assert_eq!(spec.file, "birdnet-v3-map.bin");
        assert!(spec.radius > 0.0 && spec.radius < BIRDNET_MAP.radius);
        assert!(map_spec(Method::Bats).is_none());
    }

    #[test]
    fn map_file_parses() {
        let m = parse_map(&[0xff, 0x7f, 0x01, 0x80, 0x00, 0x80, 0x00, 0x80]);
        assert_eq!(m, vec![Some((1.0, -1.0)), None]);
    }

    #[test]
    fn near_classes_pull_far_ones_do_not() {
        let map = [Some((0.0, 0.0)), Some((0.1, 0.0)), Some((0.9, 0.9))];
        let r = run(&[&[(0, 0.8), (1, 0.8), (2, 0.8)], &[(0, 0.2)]]);
        let p = place_windows(&r, &map, 0.15, 0.5);
        assert_eq!(p.len(), 1);
        assert!((p[0].x - 0.05).abs() < 1e-6, "{:?}", p[0]);
        assert_eq!(p[0].y, 0.0);
    }

    #[test]
    fn same_spot_windows_are_spread() {
        let map = [Some((0.0, 0.0)), Some((0.5, 0.5)), Some((0.9, 0.9))];
        let r = run(&[&[(0, 0.9)], &[(0, 0.9)]]);
        let p = place_windows(&r, &map, 0.15, 0.5);
        assert_eq!(p.len(), 2);
        assert_ne!((p[0].x, p[0].y), (p[1].x, p[1].y));
    }
}
