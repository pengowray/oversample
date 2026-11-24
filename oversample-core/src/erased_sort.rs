//! Sorting that shares one copy of the sort code between every list type.
//!
//! `slice::sort_by` is compiled again for each element type and each
//! comparison, at 2 to 13 KB a copy. The app had 224 copies (378 KB of wasm,
//! 2026-11-24), and a browser compiles all of them at start-up. These methods
//! sort a list of positions with one shared, non-generic sort, then move the
//! elements into that order, so each use adds only a small loop.
//!
//! They are stable, like `sort_by`. They are slower than `sort_by` (one
//! allocation, and each comparison is an indirect call), so lists sorted in
//! a hot loop should keep the standard sort.

use std::cmp::Ordering;

/// Slice sorts that share one copy of the sort code. See the module docs.
pub trait SortErased<T> {
    /// Like `sort_by`.
    fn sort_erased_by(&mut self, cmp: impl FnMut(&T, &T) -> Ordering);
    /// Like `sort_by_key`.
    fn sort_erased_by_key<K: Ord>(&mut self, key: impl FnMut(&T) -> K);
    /// Like `sort`.
    fn sort_erased(&mut self)
    where
        T: Ord;
}

impl<T> SortErased<T> for [T] {
    fn sort_erased_by(&mut self, mut cmp: impl FnMut(&T, &T) -> Ordering) {
        if self.len() < 2 {
            return;
        }
        if self.len() > u32::MAX as usize {
            self.sort_by(cmp);
            return;
        }
        let mut order: Vec<u32> = (0..self.len() as u32).collect();
        sort_positions(&mut order, &mut |a, b| {
            cmp(&self[a as usize], &self[b as usize])
        });
        // `order[i]` is where the element that belongs at `i` is now. Each
        // pass of the outer loop moves one cycle of elements into place.
        for start in 0..self.len() {
            let mut at = start;
            loop {
                let from = order[at] as usize;
                order[at] = at as u32;
                if from == start {
                    break;
                }
                self.swap(at, from);
                at = from;
            }
        }
    }

    fn sort_erased_by_key<K: Ord>(&mut self, mut key: impl FnMut(&T) -> K) {
        self.sort_erased_by(|a, b| key(a).cmp(&key(b)));
    }

    fn sort_erased(&mut self)
    where
        T: Ord,
    {
        self.sort_erased_by(T::cmp);
    }
}

/// The one copy of the sort. Not generic and never inlined, so every caller
/// shares it.
#[inline(never)]
fn sort_positions(order: &mut [u32], cmp: &mut dyn FnMut(u32, u32) -> Ordering) {
    order.sort_by(|&a, &b| cmp(a, b));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Small fixed pseudo-random sequence, so the tests need no extra crate.
    fn numbers(n: usize, seed: u64) -> Vec<u32> {
        let mut x = seed;
        (0..n)
            .map(|_| {
                x = x
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (x >> 40) as u32 % 50
            })
            .collect()
    }

    #[test]
    fn matches_the_standard_stable_sort() {
        for n in [0, 1, 2, 3, 7, 20, 21, 100, 1000] {
            for seed in 1..6 {
                // The second field shows whether equal keys keep their order.
                let items: Vec<(u32, usize)> = numbers(n, seed)
                    .into_iter()
                    .enumerate()
                    .map(|(i, k)| (k, i))
                    .collect();
                let mut want = items.clone();
                want.sort_by(|a, b| a.0.cmp(&b.0));

                let mut by = items.clone();
                by.sort_erased_by(|a, b| a.0.cmp(&b.0));
                assert_eq!(by, want, "sort_erased_by, n={n} seed={seed}");

                let mut by_key = items.clone();
                by_key.sort_erased_by_key(|a| a.0);
                assert_eq!(by_key, want, "sort_erased_by_key, n={n} seed={seed}");

                let mut plain = items.clone();
                plain.sort_erased();
                let mut want_plain = items;
                want_plain.sort();
                assert_eq!(plain, want_plain, "sort_erased, n={n} seed={seed}");
            }
        }
    }

    #[test]
    fn sorts_elements_that_are_not_copy() {
        let mut names: Vec<String> = ["pear", "apple", "fig", "apple", "kiwi"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        names.sort_erased_by(|a, b| b.cmp(a));
        assert_eq!(names, ["pear", "kiwi", "fig", "apple", "apple"]);
    }

    #[test]
    fn already_sorted_and_reversed_lists() {
        let mut up: Vec<u32> = (0..500).collect();
        up.sort_erased();
        assert!(up.iter().copied().eq(0..500));
        let mut down: Vec<u32> = (0..500).rev().collect();
        down.sort_erased();
        assert!(down.iter().copied().eq(0..500));
    }
}
