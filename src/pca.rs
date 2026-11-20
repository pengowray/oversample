//! Principal component analysis of embeddings, by power iteration. Pure
//! functions, no dependencies.
//!
//! - [`fit_pca`]: the first two components with their variance shares, for the
//!   Live explorer's map (`crate::live_explorer`).
//! - [`project_top`]: each point's coordinates on the top few components, for
//!   grouping a recording's windows (`crate::classifier::sound_groups`).

/// A PCA fit of the session's embeddings.
#[derive(Clone)]
pub struct Fit {
    pub mean: Vec<f32>,
    pub comps: [Vec<f32>; 2],
    /// Share of the total variance on each component, 0 to 1.
    pub share: [f32; 2],
    /// Number of readings the fit used.
    pub n: usize,
}

pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Scale `v` to length 1 and return its old length. A zero vector stays zero.
pub fn normalize(v: &mut [f32]) -> f32 {
    let n = dot(v, v).sqrt();
    if n > 0.0 {
        v.iter_mut().for_each(|x| *x /= n);
    }
    n
}

/// The first two principal components of `points` by power iteration, with
/// signs matched to `prev` so the picture does not flip between fits.
pub fn fit_pca(points: &[&[f32]], prev: Option<&Fit>) -> Option<Fit> {
    let n = points.len();
    let d = points.first()?.len();
    if n < 3 || d == 0 {
        return None;
    }
    let mut mean = vec![0.0f32; d];
    for p in points {
        for (m, x) in mean.iter_mut().zip(*p) {
            *m += x;
        }
    }
    mean.iter_mut().for_each(|m| *m /= n as f32);
    let centred: Vec<Vec<f32>> = points
        .iter()
        .map(|p| p.iter().zip(&mean).map(|(x, m)| x - m).collect())
        .collect();
    let total: f32 = centred.iter().map(|c| dot(c, c)).sum::<f32>() / n as f32;
    if total <= 0.0 {
        return None;
    }
    let mut comps: Vec<Vec<f32>> = Vec::new();
    let mut share = [0.0f32; 2];
    for k in 0..2 {
        // Start from the previous fit's axis if there is one, else from the
        // reading furthest from the mean.
        let mut v = prev.map(|p| p.comps[k].clone()).unwrap_or_else(|| {
            centred
                .iter()
                .max_by(|a, b| dot(a, a).total_cmp(&dot(b, b)))
                .cloned()
                .unwrap_or_else(|| vec![1.0; d])
        });
        for _ in 0..60 {
            let mut w = vec![0.0f32; d];
            for c in &centred {
                let s = dot(c, &v);
                for (wi, ci) in w.iter_mut().zip(c) {
                    *wi += s * ci;
                }
            }
            for prev_c in &comps {
                let s = dot(&w, prev_c);
                for (wi, pi) in w.iter_mut().zip(prev_c) {
                    *wi -= s * pi;
                }
            }
            if normalize(&mut w) == 0.0 {
                break;
            }
            v = w;
        }
        normalize(&mut v);
        if let Some(p) = prev {
            if dot(&v, &p.comps[k]) < 0.0 {
                v.iter_mut().for_each(|x| *x = -*x);
            }
        }
        let var: f32 = centred.iter().map(|c| dot(c, &v).powi(2)).sum::<f32>() / n as f32;
        share[k] = var / total;
        comps.push(v);
    }
    let [a, b]: [Vec<f32>; 2] = comps.try_into().ok()?;
    Some(Fit {
        mean,
        comps: [a, b],
        share,
        n,
    })
}

/// Most rounds of [`project_top`]'s iteration.
const SUBSPACE_ROUNDS: usize = 25;
/// [`project_top`] stops once the basis moves less than this between rounds:
/// 1 minus the share of the new basis that lies in the old one's span.
const SUBSPACE_TOLERANCE: f32 = 1e-3;

/// [`dot`] with eight running sums, which the compiler can keep in vector
/// registers: several times faster on long vectors. The sum is added up in
/// a different order, so the last bits can differ from [`dot`].
pub fn dot_fast(a: &[f32], b: &[f32]) -> f32 {
    let len = a.len().min(b.len());
    let mut acc = [0.0f32; 8];
    let (ac, a_rest) = a[..len].as_chunks::<8>();
    let (bc, b_rest) = b[..len].as_chunks::<8>();
    let tail: f32 = a_rest.iter().zip(b_rest).map(|(x, y)| x * y).sum();
    for (x, y) in ac.iter().zip(bc) {
        for i in 0..8 {
            acc[i] += x[i] * y[i];
        }
    }
    acc.iter().sum::<f32>() + tail
}

/// A small, fixed pseudo-random generator (SplitMix64), so results that use
/// it are the same on every run and every platform.
#[derive(Clone)]
pub struct SplitMix64(u64);

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// 0 (included) to 1 (excluded).
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Make `basis` orthonormal in place (modified Gram-Schmidt). A vector that
/// lies in the span of the ones before it becomes all zero.
fn orthonormalise(basis: &mut [Vec<f32>]) {
    for j in 0..basis.len() {
        let (done, rest) = basis.split_at_mut(j);
        let v = &mut rest[0];
        for u in done.iter() {
            let s = dot(v, u);
            for (vi, ui) in v.iter_mut().zip(u) {
                *vi -= s * ui;
            }
        }
        if normalize(v) < 1e-6 {
            v.iter_mut().for_each(|x| *x = 0.0);
        }
    }
}

/// Each point's coordinates on the top `dims` principal components of the
/// points, worked out one round at a time so a caller on the page's thread
/// can let the page draw between rounds ([`TopProjection::step`]).
///
/// The components come from block power iteration (orthogonal iteration)
/// from a fixed start, so the result is the same each time. The basis is only
/// found up to a rotation within the top `dims` components; distances
/// between points, all that k-means uses, do not depend on that rotation.
///
/// Cost: about `2 * n * d * dims` multiply-adds per round, at most
/// [`SUBSPACE_ROUNDS`] rounds.
pub struct TopProjection {
    points: Vec<f32>,
    n: usize,
    d: usize,
    dims: usize,
    basis: Vec<Vec<f32>>,
    rounds: usize,
    settled: bool,
}

impl TopProjection {
    /// `points` holds `n` points of `d` values back to back, already centred
    /// (mean zero).
    pub fn new(points: Vec<f32>, n: usize, d: usize, dims: usize) -> Self {
        let usable = n > 0 && d > 0 && dims > 0 && points.len() >= n * d;
        let mut rng = SplitMix64::new(0x0b5e_55ed_ca11_5eed);
        let mut basis: Vec<Vec<f32>> = if usable {
            (0..dims)
                .map(|_| (0..d).map(|_| rng.next_f64() as f32 * 2.0 - 1.0).collect())
                .collect()
        } else {
            Vec::new()
        };
        orthonormalise(&mut basis);
        Self {
            points,
            n,
            d,
            dims,
            basis,
            rounds: 0,
            settled: !usable,
        }
    }

    fn rows(&self) -> impl Iterator<Item = &[f32]> {
        self.points.chunks_exact(self.d.max(1)).take(self.n)
    }

    /// Each point's coordinates on the current basis, `n` rows of `dims`.
    fn coords(&self) -> Vec<f32> {
        let mut coords = vec![0.0f32; self.n * self.dims];
        if self.basis.is_empty() {
            return coords;
        }
        for (row, out) in self.rows().zip(coords.chunks_exact_mut(self.dims)) {
            for (o, v) in out.iter_mut().zip(&self.basis) {
                *o = dot_fast(row, v);
            }
        }
        coords
    }

    /// One round. Returns true once the basis has settled or the rounds are
    /// used up; further calls do nothing.
    pub fn step(&mut self) -> bool {
        if self.settled || self.rounds >= SUBSPACE_ROUNDS {
            return true;
        }
        // next = X^T * (X * basis), made orthonormal.
        let coords = self.coords();
        let mut next = vec![vec![0.0f32; self.d]; self.dims];
        for (row, c) in self.rows().zip(coords.chunks_exact(self.dims)) {
            for (w, &s) in next.iter_mut().zip(c) {
                for (wi, xi) in w.iter_mut().zip(row) {
                    *wi += s * xi;
                }
            }
        }
        orthonormalise(&mut next);
        // Share of the new basis inside the old one's span: 1 when the span
        // has stopped moving, whatever the rotation within it.
        let kept: f32 = next
            .iter()
            .map(|w| self.basis.iter().map(|v| dot_fast(w, v).powi(2)).sum::<f32>())
            .sum::<f32>()
            / self.dims as f32;
        self.settled = 1.0 - kept < SUBSPACE_TOLERANCE;
        self.basis = next;
        self.rounds += 1;
        self.settled || self.rounds >= SUBSPACE_ROUNDS
    }

    /// Rounds run so far.
    pub fn rounds(&self) -> usize {
        self.rounds
    }

    /// The coordinates on the basis found: `n` rows of `dims` values.
    pub fn finish(self) -> Vec<f32> {
        self.coords()
    }
}

/// [`TopProjection`] run to the end in one go.
pub fn project_top(points: &[f32], n: usize, d: usize, dims: usize) -> Vec<f32> {
    let mut p = TopProjection::new(points.to_vec(), n, d, dims);
    while !p.step() {}
    p.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pca_finds_the_long_axis() {
        // Points along (1, 1, 0) with a little spread along (0, 0, 1).
        let pts: Vec<Vec<f32>> = (0..20)
            .map(|i| {
                let t = i as f32 - 10.0;
                vec![t, t, if i % 2 == 0 { 0.5 } else { -0.5 }]
            })
            .collect();
        let refs: Vec<&[f32]> = pts.iter().map(|p| p.as_slice()).collect();
        let f = fit_pca(&refs, None).unwrap();
        let c = &f.comps[0];
        assert!((c[0].abs() - 0.7071).abs() < 1e-3 && (c[1].abs() - 0.7071).abs() < 1e-3);
        // The spread along z is slightly correlated with t, so not exactly 0.
        assert!(c[2].abs() < 0.02, "{c:?}");
        assert!(f.share[0] > 0.99 && f.share[1] > 0.0);
        // A refit keeps the signs.
        let g = fit_pca(&refs, Some(&f)).unwrap();
        assert!(dot(&g.comps[0], &f.comps[0]) > 0.99);
    }

    #[test]
    fn projection_keeps_the_top_components() {
        // 4-D points spread widely along x and y, a little along z, and not
        // at all along w. Two dimensions keep x and y: distances between
        // points on the plane are kept.
        let mut pts = Vec::new();
        for i in 0..12 {
            let (a, b) = ((i % 4) as f32 - 1.5, (i / 4) as f32 - 1.0);
            pts.extend([a * 10.0, b * 6.0, if i % 2 == 0 { 0.1 } else { -0.1 }, 0.0]);
        }
        let p = project_top(&pts, 12, 4, 2);
        assert_eq!(p.len(), 24);
        let dist = |v: &[f32], i: usize, j: usize, d: usize| -> f32 {
            (0..d).map(|k| (v[i * d + k] - v[j * d + k]).powi(2)).sum::<f32>().sqrt()
        };
        for (i, j) in [(0, 1), (0, 5), (3, 8), (2, 11)] {
            let full = dist(&pts, i, j, 4);
            let kept = dist(&p, i, j, 2);
            assert!((full - kept).abs() < 0.25, "{i} {j}: {full} vs {kept}");
        }
        // The same input gives the same output.
        assert_eq!(p, project_top(&pts, 12, 4, 2));
    }

    #[test]
    fn projection_of_too_few_points() {
        // Asking for more components than the points span gives zeros for
        // the extra ones, not NaN.
        let pts = [1.0, 0.0, -1.0, 0.0];
        let p = project_top(&pts, 2, 2, 2);
        assert!(p.iter().all(|x| x.is_finite()));
        assert!((p[0].abs() - 1.0).abs() < 1e-4 || (p[1].abs() - 1.0).abs() < 1e-4);
        assert_eq!(project_top(&[], 0, 4, 2), Vec::<f32>::new());
    }
}
