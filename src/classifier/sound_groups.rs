//! Sound groups: the windows of a classifier run sorted into 2 to 4 groups by
//! how alike their embeddings are, with no use of the species scores. Shown
//! as the "Sound groups" row above the spectrogram, so the parts of a
//! recording that sound alike, and the parts that differ from the rest, can
//! be seen at a glance.
//!
//! Method: each window's embedding is scaled to length 1, the set is reduced
//! to its top [`MAX_DIMS`] principal components (`crate::pca::project_top`),
//! and k-means (k-means++ seeding from a fixed seed, [`RESTARTS`] restarts,
//! the lowest total squared distance kept) splits it. A test on recordings
//! found that 2 groups split calls from background as well as anything else
//! tried, and held up with few windows.
//!
//! Groups are numbered by size, largest first, so Group 1 is usually the
//! background and the numbers stay put when the grouping is redone.
//!
//! Pure functions; the caching and scheduling are in
//! `crate::components::sound_groups`.

use super::ClassifierRun;
use crate::pca::{SplitMix64, TopProjection};

/// The group counts offered.
pub const GROUP_CHOICES: [usize; 3] = [2, 3, 4];
/// The group count used until one is picked.
pub const DEFAULT_GROUPS: usize = 2;
/// Most principal components kept before grouping.
pub const MAX_DIMS: usize = 10;
/// k-means runs from different seeds; the best is kept.
const RESTARTS: u64 = 8;
/// Most k-means rounds per run.
const MAX_ROUNDS: usize = 100;
const SEED: u64 = 0x50_0d_67_00_95;

/// Fill colours of the groups on the row and in the legend, Group 1 first.
///
/// Group 1 (the largest, usually background) is a dark grey, so the smaller
/// groups stand out against it. The others are kept apart from the species
/// rows above them, which are pink, sky blue, violet and light grey
/// (`SPECIES_COLOURS` in `canvas::overlays`): green, orange and yellow.
///
/// Checked with the dataviz `validate_palette.js --mode dark --surface
/// "#0a0a0a" --pairs all` on these four with the four species colours: the
/// closest pair for normal vision is yellow and green (15.9, passes), and no
/// pair with a group colour is closer for colour-blind vision than the
/// species' own sky blue and violet (7.1). The orange is close to the top
/// fifth of a component row (`PC_FIFTH_COLOURS`); those rows draw lines, not
/// filled cells, and every row is named. Gates traded on purpose: the grey
/// has no chroma, is darker than the band and is 2.3:1 against the row
/// background (it is meant to be quiet).
pub const GROUP_COLOURS: [&str; 4] = ["#4a4c56", "#50dc96", "#e8743c", "#e0c040"];

/// The windows' embeddings made ready for grouping: scaled to length 1,
/// centred and reduced to the top principal components.
#[derive(Clone, Debug, PartialEq)]
pub struct Reduced {
    /// Number of windows.
    pub n: usize,
    /// Values per window in `points`. 0 when every embedding is the same.
    pub dims: usize,
    /// `n` rows of `dims` values.
    pub points: Vec<f32>,
}

/// Which group each window is in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grouping {
    /// Per window (in window order), its group: 0 is Group 1, the largest.
    pub of_window: Vec<u8>,
    /// Windows in each group, largest first. Every group has at least one.
    pub sizes: Vec<usize>,
}

/// Reduce the first `embeddings.len() / dim` windows' embeddings
/// (`ClassifierState::embeddings`, `dim` floats per window), in one go.
pub fn reduce(embeddings: &[f32], dim: usize) -> Reduced {
    let mut r = Reducer::new(embeddings, dim);
    while !r.step() {}
    r.finish()
}

/// [`reduce`] one round of the principal component search at a time, so a
/// caller on the page's thread can let the page draw between rounds.
pub struct Reducer {
    n: usize,
    dims: usize,
    projection: Option<TopProjection>,
}

impl Reducer {
    /// Copies the embeddings, so `embeddings` can be let go before stepping.
    pub fn new(embeddings: &[f32], dim: usize) -> Self {
        let n = embeddings.len().checked_div(dim).unwrap_or(0);
        match centred(embeddings, dim, n) {
            Some(x) => {
                let dims = MAX_DIMS.min(n - 1).min(dim);
                Self {
                    n,
                    dims,
                    projection: Some(TopProjection::new(x, n, dim, dims)),
                }
            }
            None => Self {
                n,
                dims: 0,
                projection: None,
            },
        }
    }

    /// One round; true when done.
    pub fn step(&mut self) -> bool {
        self.projection.as_mut().is_none_or(TopProjection::step)
    }

    pub fn finish(self) -> Reduced {
        Reduced {
            n: self.n,
            dims: self.dims,
            points: self
                .projection
                .map_or_else(Vec::new, TopProjection::finish),
        }
    }
}

/// The first `n` windows scaled to length 1 and centred, or `None` when
/// there is nothing to separate them by (fewer than 2 windows, or all the
/// same).
fn centred(embeddings: &[f32], dim: usize, n: usize) -> Option<Vec<f32>> {
    let mut x: Vec<f32> = embeddings[..n * dim].to_vec();
    // Scale each window to length 1. A window of zeros stays zeros.
    for row in x.chunks_exact_mut(dim.max(1)) {
        crate::pca::normalize(row);
    }
    let mut mean = vec![0.0f32; dim];
    for row in x.chunks_exact(dim.max(1)) {
        for (m, v) in mean.iter_mut().zip(row) {
            *m += v;
        }
    }
    if n > 0 {
        mean.iter_mut().for_each(|m| *m /= n as f32);
    }
    let mut spread = 0.0f32;
    for row in x.chunks_exact_mut(dim.max(1)) {
        for (v, m) in row.iter_mut().zip(&mean) {
            *v -= m;
            spread = spread.max(v.abs());
        }
    }
    (n >= 2 && spread >= 1e-6).then_some(x)
}

fn dist2(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum()
}

/// One k-means run from k-means++ seeding: each window's cluster and the
/// total squared distance to the cluster centres.
fn kmeans_once(r: &Reduced, k: usize, rng: &mut SplitMix64) -> (Vec<usize>, f64) {
    let (n, d) = (r.n, r.dims);
    let point = |i: usize| &r.points[i * d..(i + 1) * d];
    let mut centres: Vec<Vec<f32>> = Vec::with_capacity(k);
    centres.push(point((rng.next_u64() % n as u64) as usize).to_vec());
    let mut nearest: Vec<f32> = (0..n).map(|i| dist2(point(i), &centres[0])).collect();
    while centres.len() < k {
        let total: f64 = nearest.iter().map(|&v| v as f64).sum();
        // Fewer distinct points than groups: stop with fewer centres.
        if total <= 1e-12 {
            break;
        }
        let target = rng.next_f64() * total;
        let mut acc = 0.0f64;
        let mut pick = n - 1;
        for (i, &v) in nearest.iter().enumerate() {
            acc += v as f64;
            if acc > target {
                pick = i;
                break;
            }
        }
        let c = point(pick).to_vec();
        for (i, m) in nearest.iter_mut().enumerate() {
            *m = m.min(dist2(point(i), &c));
        }
        centres.push(c);
    }
    let k = centres.len();
    let mut assign = vec![usize::MAX; n];
    for _ in 0..MAX_ROUNDS {
        let mut changed = false;
        for (i, a) in assign.iter_mut().enumerate() {
            let p = point(i);
            // Ties go to the lower-numbered centre.
            let best = (0..k)
                .map(|c| (dist2(p, &centres[c]), c))
                .min_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)))
                .map_or(0, |(_, c)| c);
            if *a != best {
                *a = best;
                changed = true;
            }
        }
        if !changed {
            break;
        }
        let mut sums = vec![vec![0.0f32; d]; k];
        let mut counts = vec![0usize; k];
        for (i, &a) in assign.iter().enumerate() {
            counts[a] += 1;
            for (s, v) in sums[a].iter_mut().zip(point(i)) {
                *s += v;
            }
        }
        // A centre that lost all its windows stays where it was.
        for c in 0..k {
            if counts[c] > 0 {
                centres[c] = sums[c].iter().map(|s| s / counts[c] as f32).collect();
            }
        }
    }
    let cost = assign
        .iter()
        .enumerate()
        .map(|(i, &a)| dist2(point(i), &centres[a]) as f64)
        .sum();
    (assign, cost)
}

/// Sort the windows of `r` into `k` groups. `None` when there are fewer
/// than `k + 1` windows. Identical windows give one group; fewer distinct
/// windows than `k` give fewer groups. The same input always gives the same
/// result.
pub fn group(r: &Reduced, k: usize) -> Option<Grouping> {
    if k == 0 || r.n < k + 1 {
        return None;
    }
    if r.dims == 0 {
        return Some(Grouping {
            of_window: vec![0; r.n],
            sizes: vec![r.n],
        });
    }
    let mut best: Option<(Vec<usize>, f64)> = None;
    for restart in 0..RESTARTS {
        let mut rng = SplitMix64::new(SEED.wrapping_add(restart));
        let (assign, cost) = kmeans_once(r, k, &mut rng);
        if best.as_ref().is_none_or(|(_, c)| cost < *c) {
            best = Some((assign, cost));
        }
    }
    let (assign, _) = best?;
    Some(number_by_size(&assign))
}

/// Renumber clusters by size, largest first; equal sizes by the earliest
/// window. Empty clusters are dropped.
fn number_by_size(assign: &[usize]) -> Grouping {
    let k = assign.iter().copied().max().map_or(0, |m| m + 1);
    let mut size = vec![0usize; k];
    let mut first = vec![usize::MAX; k];
    for (i, &a) in assign.iter().enumerate() {
        size[a] += 1;
        first[a] = first[a].min(i);
    }
    let mut order: Vec<usize> = (0..k).filter(|&c| size[c] > 0).collect();
    order.sort_by(|&a, &b| size[b].cmp(&size[a]).then(first[a].cmp(&first[b])));
    let mut new_number = vec![0u8; k];
    for (n, &c) in order.iter().enumerate() {
        new_number[c] = n as u8;
    }
    Grouping {
        of_window: assign.iter().map(|&a| new_number[a]).collect(),
        sizes: order.iter().map(|&c| size[c]).collect(),
    }
}

/// What the model most often heard in one group.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GroupTop {
    /// Windows in the group.
    pub size: usize,
    /// The class that is the top class of the most windows in the group,
    /// whatever its score, and how many windows it tops. `None` when no
    /// window in the group has a score.
    pub top: Option<(u16, usize)>,
    /// That class's average confidence over the windows it tops.
    pub mean: f32,
    /// The class most often in second place in the group's windows, other
    /// than `top`'s class.
    pub second: Option<u16>,
}

/// The class with the highest count; equal counts go to the lower class
/// number.
fn most_frequent(counts: &std::collections::HashMap<u16, usize>) -> Option<(u16, usize)> {
    counts
        .iter()
        .map(|(&c, &n)| (c, n))
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
}

/// For each group of `g`, its most frequent top class in `run`. Windows
/// without scores count in the group's size. Equal counts go to the lower
/// class number.
pub fn group_tops(g: &Grouping, run: &ClassifierRun) -> Vec<GroupTop> {
    use std::collections::HashMap;
    let n = g.sizes.len();
    let mut firsts: Vec<HashMap<u16, usize>> = vec![HashMap::new(); n];
    for (w, &grp) in run.windows.iter().zip(&g.of_window) {
        if let Some(s) = w.top.first() {
            *firsts[grp as usize].entry(s.class).or_default() += 1;
        }
    }
    let tops: Vec<Option<(u16, usize)>> = firsts.iter().map(most_frequent).collect();
    let mut sums = vec![0.0f32; n];
    let mut seconds: Vec<HashMap<u16, usize>> = vec![HashMap::new(); n];
    for (w, &grp) in run.windows.iter().zip(&g.of_window) {
        let grp = grp as usize;
        let Some((class, _)) = tops[grp] else { continue };
        if w.top.first().is_some_and(|s| s.class == class) {
            sums[grp] += w.top[0].confidence;
        }
        if let Some(s) = w.top.get(1).filter(|s| s.class != class) {
            *seconds[grp].entry(s.class).or_default() += 1;
        }
    }
    (0..n)
        .map(|i| GroupTop {
            size: g.sizes[i],
            top: tops[i],
            mean: tops[i].map_or(0.0, |(_, count)| sums[i] / count as f32),
            second: most_frequent(&seconds[i]).map(|(c, _)| c),
        })
        .collect()
}

/// The class a group is named after: its top class when that is the top
/// class in at least half the group's windows.
fn named_class(top: &GroupTop) -> Option<u16> {
    top.top
        .filter(|&(_, count)| count * 2 >= top.size)
        .map(|(class, _)| class)
}

/// The group's hint: "mostly Gentoo Penguin" when that class is the top
/// class in at least half the group's windows, else "mixed". `name` gives a
/// class's name as shown.
pub fn hint(top: &GroupTop, name: impl Fn(u16) -> String) -> String {
    match named_class(top) {
        Some(class) => format!("mostly {}", name(class)),
        None => "mixed".to_string(),
    }
}

/// Each group's hint. Groups named after the same class also get that
/// class's average confidence in the group, which is what tells them apart:
/// "mostly Gentoo Penguin, average confidence 0.97".
pub fn hints(tops: &[GroupTop], name: impl Fn(u16) -> String) -> Vec<String> {
    tops.iter()
        .enumerate()
        .map(|(i, top)| {
            let base = hint(top, &name);
            let class = named_class(top);
            let shared = class.is_some()
                && tops
                    .iter()
                    .enumerate()
                    .any(|(j, other)| j != i && named_class(other) == class);
            if shared {
                format!("{base}, average confidence {:.2}", top.mean)
            } else {
                base
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classifier::{catalog, Method, Score, SpeciesLabel, WindowScores};
    use std::sync::Arc;

    const DIM: usize = 16;

    /// `n` windows near `centre` (a unit axis), with a little spread.
    fn blob(axis: usize, n: usize, rng: &mut SplitMix64) -> Vec<f32> {
        let mut out = Vec::new();
        for _ in 0..n {
            let mut v = vec![0.0f32; DIM];
            v[axis] = 1.0;
            for x in &mut v {
                *x += (rng.next_f64() as f32 - 0.5) * 0.1;
            }
            out.extend(v);
        }
        out
    }

    /// Interleave the windows of two blobs so the groups are not just the
    /// first and second half: windows `i % 4 == 1` are from `b`.
    fn mixed(a_axis: usize, b_axis: usize, n: usize) -> (Vec<f32>, Vec<bool>) {
        let mut rng = SplitMix64::new(7);
        let mut e = Vec::new();
        let mut is_b = Vec::new();
        for i in 0..n {
            let b = i % 4 == 1;
            e.extend(blob(if b { b_axis } else { a_axis }, 1, &mut rng));
            is_b.push(b);
        }
        (e, is_b)
    }

    #[test]
    fn two_blobs_are_split() {
        let (e, is_b) = mixed(0, 5, 40);
        let g = group(&reduce(&e, DIM), 2).unwrap();
        assert_eq!(g.sizes, vec![30, 10]);
        for (i, &b) in is_b.iter().enumerate() {
            // The larger blob is Group 1.
            assert_eq!(g.of_window[i], u8::from(b), "window {i}");
        }
    }

    #[test]
    fn same_input_same_groups() {
        let mut rng = SplitMix64::new(3);
        let mut e = blob(0, 12, &mut rng);
        e.extend(blob(3, 9, &mut rng));
        e.extend(blob(9, 6, &mut rng));
        let r = reduce(&e, DIM);
        for k in GROUP_CHOICES {
            let a = group(&r, k).unwrap();
            assert_eq!(Some(a.clone()), group(&reduce(&e, DIM), k));
            assert_eq!(a.sizes.len(), k);
        }
        let g = group(&r, 3).unwrap();
        assert_eq!(g.sizes, vec![12, 9, 6]);
        assert!(g.of_window[..12].iter().all(|&x| x == 0));
        assert!(g.of_window[12..21].iter().all(|&x| x == 1));
        assert!(g.of_window[21..].iter().all(|&x| x == 2));
    }

    #[test]
    fn numbered_by_size_then_first_window() {
        let g = number_by_size(&[2, 2, 0, 1, 1, 1, 0, 2, 2]);
        assert_eq!(g.sizes, vec![4, 3, 2]);
        assert_eq!(g.of_window, vec![0, 0, 2, 1, 1, 1, 2, 0, 0]);
        // Equal sizes: the group seen first comes first. A cluster number
        // with no windows is dropped.
        let g = number_by_size(&[3, 3, 1, 1]);
        assert_eq!(g.sizes, vec![2, 2]);
        assert_eq!(g.of_window, vec![0, 0, 1, 1]);
    }

    #[test]
    fn too_few_windows() {
        let (e, _) = mixed(0, 5, 3);
        let r = reduce(&e, DIM);
        assert_eq!(r.n, 3);
        assert!(group(&r, 2).is_some());
        assert!(group(&r, 3).is_none());
        assert!(group(&r, 4).is_none());
        assert!(group(&reduce(&[], DIM), 2).is_none());
        // A partial window at the end is ignored.
        assert_eq!(reduce(&e[..2 * DIM + 3], DIM).n, 2);
    }

    #[test]
    fn identical_windows_are_one_group() {
        let mut e = Vec::new();
        for _ in 0..6 {
            e.extend((0..DIM).map(|i| i as f32));
        }
        let g = group(&reduce(&e, DIM), 3).unwrap();
        assert_eq!(g.sizes, vec![6]);
        assert_eq!(g.of_window, vec![0; 6]);
        // Same direction, different lengths: alike once scaled to length 1.
        let mut e2 = Vec::new();
        for s in 1..=5 {
            e2.extend((0..DIM).map(|i| (i + 1) as f32 * s as f32));
        }
        assert_eq!(group(&reduce(&e2, DIM), 2).unwrap().sizes, vec![5]);
        // Only two distinct windows, four groups asked for: two groups.
        let mut e3 = Vec::new();
        for i in 0..6 {
            let mut v = vec![0.0f32; DIM];
            v[if i < 4 { 0 } else { 1 }] = 1.0;
            e3.extend(v);
        }
        let g = group(&reduce(&e3, DIM), 4).unwrap();
        assert_eq!(g.sizes, vec![4, 2]);
    }

    #[test]
    fn zero_windows_do_not_break_grouping() {
        let (mut e, _) = mixed(0, 5, 12);
        e[..DIM].iter_mut().for_each(|x| *x = 0.0);
        let g = group(&reduce(&e, DIM), 2).unwrap();
        assert_eq!(g.of_window.len(), 12);
        assert!(g.sizes.iter().sum::<usize>() == 12);
    }

    fn run(tops: &[Option<u16>]) -> ClassifierRun {
        let labels = ["A a_Alpha", "B b_Bravo", "C c_Charlie"]
            .iter()
            .map(|l| SpeciesLabel::parse(l))
            .collect();
        ClassifierRun {
            method: Method::BirdNet,
            region: None,
            backbone: &catalog::BACKBONE,
            head: None,
            source_rate: 48_000,
            model_rate: 48_000,
            window_secs: 3.0,
            labels: Arc::new(labels),
            windows: tops
                .iter()
                .enumerate()
                .map(|(i, t)| WindowScores {
                    start_s: i as f64 * 3.0,
                    end_s: (i + 1) as f64 * 3.0,
                    top: t
                        .iter()
                        .map(|&class| Score {
                            class,
                            confidence: 0.01,
                        })
                        .collect(),
                })
                .collect(),
            complete: true,
        }
    }

    #[test]
    fn groups_with_the_same_name_get_their_average_confidence() {
        let g = Grouping {
            of_window: vec![0, 0, 1, 1, 2],
            sizes: vec![2, 2, 1],
        };
        let mut r = run(&[Some(1), Some(1), Some(1), Some(1), Some(2)]);
        for (w, c) in r.windows.iter_mut().zip([0.9, 1.0, 0.2, 0.1, 0.5]) {
            w.top[0].confidence = c;
        }
        r.windows[2].top.push(Score {
            class: 0,
            confidence: 0.05,
        });
        let tops = group_tops(&g, &r);
        assert!((tops[0].mean - 0.95).abs() < 1e-6);
        assert_eq!(tops[0].second, None);
        assert_eq!(tops[1].second, Some(0));
        let name = |c: u16| crate::classifier::strips::display_name(&r.labels[c as usize]);
        assert_eq!(
            hints(&tops, name),
            [
                "mostly Bravo, average confidence 0.95",
                "mostly Bravo, average confidence 0.15",
                "mostly Charlie",
            ]
        );
    }

    #[test]
    fn hints_name_the_most_frequent_top_class() {
        let g = Grouping {
            of_window: vec![0, 0, 0, 0, 1, 1, 1, 0],
            sizes: vec![5, 3],
        };
        let r = run(&[Some(1), Some(1), Some(0), None, Some(2), Some(0), Some(1), Some(1)]);
        let tops = group_tops(&g, &r);
        assert_eq!((tops[0].size, tops[0].top), (5, Some((1, 3))));
        // A three-way tie goes to the lowest class number.
        assert_eq!((tops[1].size, tops[1].top), (3, Some((0, 1))));
        let name = |c: u16| crate::classifier::strips::display_name(&r.labels[c as usize]);
        assert_eq!(hint(&tops[0], name), "mostly Bravo");
        assert_eq!(hint(&tops[1], name), "mixed");
        // Exactly half is enough.
        assert_eq!(hint(&GroupTop { size: 4, top: Some((2, 2)), ..Default::default() }, name), "mostly Charlie");
        assert_eq!(hint(&GroupTop { size: 4, top: None, ..Default::default() }, name), "mixed");
    }

    #[test]
    #[ignore = "timing; run with --release -- --ignored --nocapture"]
    fn time_an_hour_of_birdnet() {
        // 1,200 windows of 1,024 floats: an hour of BirdNET at 3 s a window.
        let dim = 1024;
        let mut rng = SplitMix64::new(11);
        // Pure noise: no strong components, so the search runs all its rounds
        // (the slowest case).
        let noise: Vec<f32> = (0..1200 * dim).map(|_| rng.next_f64() as f32).collect();
        // Closer to real embeddings: a few sound types (directions) with
        // noise on top.
        let types: Vec<Vec<f32>> = (0..6)
            .map(|_| (0..dim).map(|_| rng.next_f64() as f32).collect())
            .collect();
        let mut sounds = Vec::with_capacity(1200 * dim);
        for i in 0..1200 {
            let t = &types[(i * 7 / 5) % 6];
            sounds.extend(t.iter().map(|v| v + rng.next_f64() as f32 * 0.5));
        }
        for (name, e) in [("noise", &noise), ("6 sound types", &sounds)] {
            let t = std::time::Instant::now();
            let mut r = Reducer::new(e, dim);
            let mut longest = std::time::Duration::ZERO;
            loop {
                let s = std::time::Instant::now();
                let done = r.step();
                longest = longest.max(s.elapsed());
                if done {
                    break;
                }
            }
            let rounds = r.projection.as_ref().map_or(0, TopProjection::rounds);
            let r = r.finish();
            let reduced = t.elapsed();
            let t = std::time::Instant::now();
            for k in GROUP_CHOICES {
                group(&r, k).unwrap();
            }
            let grouped = t.elapsed() / 3;
            println!(
                "{name}: reduce {reduced:?} ({rounds} rounds, longest {longest:?}), group (mean of k = 2, 3, 4) {grouped:?}"
            );
        }
    }
}
