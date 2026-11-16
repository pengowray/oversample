//! The Live explorer tab (experimental): views of BirdNET's readings of the
//! last 3 s, for exploring the embedding rather than reading off a species.
//! The readings come from `crate::live_embedding`; this file turns them into
//! what `live-explorer.js` draws:
//!
//! - the power spectrum of the last 3 s with its strongest peaks (works
//!   without BirdNET, and for bat recordings);
//! - the 15 highest class scores of the newest reading;
//! - every reading of the session as a dot, placed by PCA fitted on the
//!   session's own embeddings (UMAP is computed in the page on request);
//! - the cosine similarity between the readings of the last 15 s;
//! - the embedding over the last 15 s: the dimensions that vary most, one row
//!   each.

use crate::classifier::{self, Method};
use crate::live_embedding::{self, SessionPoint};
use crate::state::{AppState, ClassifierStateStoreFields};
use leptos::prelude::*;
use std::cell::RefCell;
use std::collections::VecDeque;
use wasm_bindgen::prelude::*;

/// Readings in the similarity matrix and the embedding image: 15 s at 4 a
/// second.
const RECENT: usize = 60;
/// Rows in the embedding image.
const DIMS: usize = 48;
/// Most dots drawn in the session map (the newest).
const MAP_POINTS: usize = 600;
/// Points in the spectrum line.
const SPECTRUM_POINTS: usize = 400;

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

thread_local! {
    static FIT: RefCell<Option<Fit>> = const { RefCell::new(None) };
    static UMAP_HOOK: RefCell<Option<Closure<dyn Fn() -> JsValue>>> = const { RefCell::new(None) };
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn normalize(v: &mut [f32]) -> f32 {
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

/// Cosine similarity, 0 when either vector is all zero.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let (na, nb) = (dot(a, a).sqrt(), dot(b, b).sqrt());
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot(a, b) / (na * nb)
    }
}

/// The `k` dimensions with the highest variance over `points`, highest first.
pub fn top_variance_dims(points: &[&[f32]], k: usize) -> Vec<usize> {
    let Some(d) = points.first().map(|p| p.len()) else {
        return Vec::new();
    };
    let n = points.len() as f32;
    let mut var: Vec<(usize, f32)> = (0..d)
        .map(|j| {
            let m = points.iter().map(|p| p[j]).sum::<f32>() / n;
            let v = points.iter().map(|p| (p[j] - m).powi(2)).sum::<f32>() / n;
            (j, v)
        })
        .collect();
    var.sort_by(|a, b| b.1.total_cmp(&a.1));
    var.into_iter().take(k).map(|(j, _)| j).collect()
}

fn r3(v: f32) -> f32 {
    (v * 1000.0).round() / 1000.0
}

fn refit(session: &VecDeque<SessionPoint>) -> Option<Fit> {
    let n = session.len();
    let prev = FIT.with(|f| f.borrow().clone());
    // Refit on the first 3 readings, then each time the session grows by a
    // tenth (at least 8 readings), so the dots do not jump at every reading.
    let due = match &prev {
        None => n >= 3,
        Some(p) => n < p.n || n >= p.n + (p.n / 10).max(8),
    };
    if !due {
        return prev;
    }
    let pts: Vec<&[f32]> = session.iter().map(|p| p.embedding.as_slice()).collect();
    let fit = fit_pca(&pts, prev.as_ref()).or(prev);
    FIT.with(|f| *f.borrow_mut() = fit.clone());
    fit
}

fn json(state: AppState) -> serde_json::Value {
    let min = classifier::min_confidence_untracked(state, Method::BirdNet);
    let status = state.classifier.live().with_untracked(|s| match s {
        live_embedding::LiveStatus::NeedsDownload => "download",
        live_embedding::LiveStatus::Loading => "loading",
        live_embedding::LiveStatus::Failed(_) => "failed",
        _ => "ready",
    });
    let spectrum = live_embedding::with_spectrum(|s| {
        let s = s?;
        let psd = &s.psd;
        let step = psd.power_db.len().div_ceil(SPECTRUM_POINTS).max(1);
        let pts: Vec<_> = psd
            .power_db
            .chunks(step)
            .enumerate()
            .map(|(i, c)| {
                let db = c.iter().copied().fold(f64::MIN, f64::max);
                let khz = (i * step) as f64 * psd.freq_resolution / 1000.0;
                ((khz * 100.0).round() / 100.0, (db * 10.0).round() / 10.0)
            })
            .collect();
        let peaks: Vec<_> = psd
            .peaks
            .iter()
            .take(8)
            .map(|p| {
                (
                    (p.freq_hz / 10.0).round() / 100.0,
                    (p.power_db * 10.0).round() / 10.0,
                )
            })
            .collect();
        Some(serde_json::json!({ "pts": pts, "peaks": peaks, "rate": s.rate }))
    });
    live_embedding::with_session(|session| {
        let now = session.back().map_or(0.0, |p| p.t);
        let ago = |p: &SessionPoint| r3(((p.t - now) / 1000.0) as f32);
        let top: Vec<_> = session
            .back()
            .and_then(|p| p.scores.as_ref())
            .map(|s| {
                live_embedding::top_classes(s, 15)
                    .into_iter()
                    .enumerate()
                    // Scores under 0.02 add rows and say nothing.
                    .filter(|&(i, (_, v))| i == 0 || v >= 0.02)
                    .map(|(_, t)| t)
                    .map(|(c, v)| (live_embedding::class_name(c), r3(v)))
                    .collect()
            })
            .unwrap_or_default();
        let fit = refit(session);
        let map = fit.as_ref().map(|f| {
            let pts: Vec<_> = session
                .iter()
                .skip(session.len().saturating_sub(MAP_POINTS))
                .map(|p| {
                    let c: Vec<f32> = p.embedding.iter().zip(&f.mean).map(|(x, m)| x - m).collect();
                    let (name, score) = p
                        .top
                        .first()
                        .map_or((String::new(), 0.0), |&(c, s)| (live_embedding::class_name(c), s));
                    (
                        r3(dot(&c, &f.comps[0])),
                        r3(dot(&c, &f.comps[1])),
                        ago(p),
                        name,
                        r3(score),
                    )
                })
                .collect();
            serde_json::json!({ "pts": pts, "share": [r3(f.share[0]), r3(f.share[1])], "n": session.len() })
        });
        let recent: Vec<&SessionPoint> = session
            .iter()
            .skip(session.len().saturating_sub(RECENT))
            .collect();
        let times: Vec<f32> = recent.iter().map(|p| ago(p)).collect();
        let mut sim = Vec::with_capacity(recent.len() * recent.len());
        for (i, a) in recent.iter().enumerate() {
            for (j, b) in recent.iter().enumerate() {
                sim.push((i, j, r3(cosine(&a.embedding, &b.embedding))));
            }
        }
        let rec: Vec<&[f32]> = recent.iter().map(|p| p.embedding.as_slice()).collect();
        let dims = top_variance_dims(&rec, DIMS);
        // Each row scaled to its own spread over the window (z-score), so a
        // quiet dimension that changes is as visible as a loud one.
        let mut emb = Vec::with_capacity(rec.len() * dims.len());
        for (row, &j) in dims.iter().enumerate() {
            let n = rec.len() as f32;
            let m = rec.iter().map(|p| p[j]).sum::<f32>() / n;
            let sd = (rec.iter().map(|p| (p[j] - m).powi(2)).sum::<f32>() / n).sqrt();
            for (i, p) in rec.iter().enumerate() {
                let z = if sd > 0.0 { (p[j] - m) / sd } else { 0.0 };
                emb.push((i, row, r3(z.clamp(-3.0, 3.0)), r3(p[j])));
            }
        }
        serde_json::json!({
            "status": status,
            "min": min,
            "spectrum": spectrum,
            "top": top,
            "map": map,
            "times": times,
            "sim": sim,
            "dims": dims,
            "emb": emb,
        })
    })
}

/// Give the page a way to fetch the session's embeddings for UMAP:
/// `window.__oversampleExplorerEmbeddings()` returns
/// `{ n, dim, data: Float32Array, ago: [s...], names: [...] }`.
fn install_umap_hook() {
    if UMAP_HOOK.with(|h| h.borrow().is_some()) {
        return;
    }
    let f = Closure::<dyn Fn() -> JsValue>::new(|| {
        live_embedding::with_session(|session| {
            let now = session.back().map_or(0.0, |p| p.t);
            let dim = session.front().map_or(0, |p| p.embedding.len());
            let mut data = Vec::with_capacity(session.len() * dim);
            let mut ago = Vec::new();
            let mut names = Vec::new();
            for p in session {
                data.extend_from_slice(&p.embedding);
                ago.push(r3(((p.t - now) / 1000.0) as f32));
                names.push(
                    p.top
                        .first()
                        .map_or(String::new(), |&(c, _)| live_embedding::class_name(c)),
                );
            }
            let obj = js_sys::Object::new();
            let set = |k: &str, v: JsValue| {
                let _ = js_sys::Reflect::set(&obj, &k.into(), &v);
            };
            set("n", JsValue::from(session.len() as u32));
            set("dim", JsValue::from(dim as u32));
            set("data", js_sys::Float32Array::from(data.as_slice()).into());
            set(
                "ago",
                serde_wasm_bindgen::to_value(&ago).unwrap_or(JsValue::NULL),
            );
            set(
                "names",
                serde_wasm_bindgen::to_value(&names).unwrap_or(JsValue::NULL),
            );
            obj.into()
        })
    });
    let _ = js_sys::Reflect::set(
        &js_sys::global(),
        &"__oversampleExplorerEmbeddings".into(),
        f.as_ref(),
    );
    UMAP_HOOK.with(|h| *h.borrow_mut() = Some(f));
}

/// Draw the explorer, if its tab is shown.
pub fn draw(state: AppState) {
    let Some(el) = live_embedding::explorer_el() else {
        return;
    };
    install_umap_hook();
    let j = json(state).to_string();
    call("update", &[&el, &JsValue::from_str(&j)]);
}

/// Forget the PCA fit (a new session).
pub fn reset() {
    FIT.with(|f| *f.borrow_mut() = None);
}

/// Call `oversampleLiveExplorer.<name>(args...)`.
pub fn call(name: &str, args: &[&JsValue]) -> Option<JsValue> {
    use wasm_bindgen::JsCast;
    let api = js_sys::Reflect::get(&js_sys::global(), &"oversampleLiveExplorer".into()).ok()?;
    let f: js_sys::Function = js_sys::Reflect::get(&api, &name.into())
        .ok()?
        .dyn_into()
        .ok()?;
    let a = js_sys::Array::new();
    for v in args {
        a.push(v);
    }
    f.apply(&api, &a).ok()
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
    fn top_dims_by_variance() {
        let pts = [vec![0.0, 5.0, 1.0], vec![0.0, -5.0, 2.0]];
        let refs: Vec<&[f32]> = pts.iter().map(|p| p.as_slice()).collect();
        assert_eq!(top_variance_dims(&refs, 2), vec![1, 2]);
    }
}
