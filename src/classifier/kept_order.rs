//! The order species were found in, kept for each file and method while the
//! app runs. A species' colour (its place among the first three) and its row
//! above the spectrogram come from this order, so neither changes as more
//! of a recording is analysed: a species found later goes after those
//! already there, even when it scores higher.
//!
//! A complete run seen for the first time (a seeded test, saved results)
//! gives the old order: best confidence first.

use super::{strips, ClassifierRun, Method};
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    /// For each file and method, the species (scientific names) in the order
    /// they were first found at or above the minimum confidence.
    static KEPT: RefCell<HashMap<(u64, Method), Vec<String>>> = RefCell::new(HashMap::new());
}

/// Add the `candidates` not yet in `kept` to its end, in their order.
/// Nothing already there moves or goes.
pub fn extend(kept: &mut Vec<String>, candidates: &[&str]) {
    for c in candidates {
        if !kept.iter().any(|k| k == c) {
            kept.push(c.to_string());
        }
    }
}

/// The kept order for `run` (file `file`'s results for `method`), brought up
/// to date, with the species that are some window's best match at
/// `min_confidence` or above now. Returns (kept order, those species), both
/// as classes of `run`.
fn update(
    file: u64,
    method: Method,
    run: &ClassifierRun,
    min_confidence: f32,
) -> (Vec<u16>, Vec<u16>) {
    let now = strips::auto_species(run, min_confidence);
    let names: Vec<&str> = now
        .iter()
        .map(|&c| run.labels[c as usize].scientific.as_str())
        .collect();
    let kept = KEPT.with(|k| {
        let mut k = k.borrow_mut();
        let kept = k.entry((file, method)).or_default();
        // No results (the file's were cleared): the order starts over.
        if run.windows.is_empty() {
            kept.clear();
        }
        extend(kept, &names);
        kept.iter()
            .filter_map(|name| strips::class_of(run, name))
            .collect()
    });
    (kept, now)
}

/// The order species' colours are given out in: the first three get the
/// three colours (`canvas::overlays::species_colour`), the rest share grey.
/// A species keeps its place once it has one, also when the minimum
/// confidence is raised above it.
pub fn colour_order(file: u64, method: Method, run: &ClassifierRun, min_confidence: f32) -> Vec<u16> {
    update(file, method, run, min_confidence).0
}

/// The species with a row above the spectrogram without a pin, in row order:
/// the first [`strips::AUTO_ROWS`] of the kept order that reach
/// `min_confidence` now.
pub fn auto_rows(file: u64, method: Method, run: &ClassifierRun, min_confidence: f32) -> Vec<u16> {
    let (kept, now) = update(file, method, run, min_confidence);
    kept.into_iter()
        .filter(|c| now.contains(c))
        .take(strips::AUTO_ROWS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_species_keeps_its_place() {
        let mut kept = Vec::new();
        extend(&mut kept, &["a", "b"]);
        assert_eq!(kept, ["a", "b"]);
        // A better species arrives: it goes after those already there.
        extend(&mut kept, &["c", "b", "a"]);
        assert_eq!(kept, ["a", "b", "c"]);
        // One no longer reaches the minimum: it keeps its place.
        extend(&mut kept, &["c", "d"]);
        assert_eq!(kept, ["a", "b", "c", "d"]);
    }
}
