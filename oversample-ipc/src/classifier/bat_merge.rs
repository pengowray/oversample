//! BattyBirdNET's "All regions": the 11 regional heads scored as one list of
//! species. [`merge_bat_labels`] builds the list from the heads' label files;
//! [`merge_bat_logits`] merges their scores for one window.

use std::collections::HashMap;

/// The label lists of several heads as one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergedBatLabels {
    /// One label-file line per merged class ("Myotis myotis_Greater
    /// mouse-eared bat", "noise"): the species first, by scientific name, then
    /// the other classes.
    pub lines: Vec<String>,
    /// How many of `lines` are species.
    pub species: usize,
    /// For each head, the merged class of each of its outputs.
    pub maps: Vec<Vec<u16>>,
}

struct Class {
    /// The scientific name (or the whole line of a class that is no species),
    /// as first written.
    name: String,
    /// The common names the heads give it, with how many heads use each.
    common: Vec<(String, usize)>,
    is_species: bool,
}

/// A common name that names one species: "Daubentons or Whiskered bat" and
/// "Lesser horseshoe bat social" are a head's own groupings.
fn is_plain(common: &str) -> bool {
    !common.contains(" or ") && !common.ends_with(" social")
}

/// Merge label files (one per head, one label per non-empty line, in the
/// order of the head's outputs). Two lines are the same class when their
/// scientific names match, or their whole lines for a class that is no
/// species ("noise" and "NOISE"); case and outer spaces are ignored. Where
/// the heads differ on a species' common name, the name most of them use
/// wins, and a name for that species alone beats a grouped one.
pub fn merge_bat_labels(texts: &[&str]) -> MergedBatLabels {
    let mut classes: Vec<Class> = Vec::new();
    let mut index: HashMap<(bool, String), usize> = HashMap::new();
    // Each head's outputs as indexes into `classes`.
    let mut heads: Vec<Vec<usize>> = Vec::new();
    for text in texts {
        let mut outputs = Vec::new();
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let (name, common) = match line.split_once('_') {
                Some((s, c)) => (s.trim(), Some(c.trim())),
                None => (line, None),
            };
            let key = (common.is_some(), name.to_lowercase());
            let i = *index.entry(key).or_insert_with(|| {
                classes.push(Class {
                    name: name.to_string(),
                    common: Vec::new(),
                    is_species: common.is_some(),
                });
                classes.len() - 1
            });
            if let Some(c) = common {
                match classes[i].common.iter_mut().find(|(n, _)| n == c) {
                    Some((_, count)) => *count += 1,
                    None => classes[i].common.push((c.to_string(), 1)),
                }
            }
            outputs.push(i);
        }
        heads.push(outputs);
    }
    // Species first, by scientific name; the other classes keep their order.
    let mut order: Vec<usize> = (0..classes.len()).collect();
    order.sort_by_key(|&i| {
        let c = &classes[i];
        (!c.is_species, c.is_species.then(|| c.name.to_lowercase()))
    });
    let mut merged_index = vec![0u16; classes.len()];
    for (merged, &i) in order.iter().enumerate() {
        merged_index[i] = merged as u16;
    }
    let lines = order
        .iter()
        .map(|&i| {
            let c = &classes[i];
            // The first of the most-used names; `max_by_key` alone would give
            // the last.
            let common = c
                .common
                .iter()
                .rev()
                .max_by_key(|(name, count)| (is_plain(name), *count));
            match common {
                Some((common, _)) => format!("{}_{common}", c.name),
                None => c.name.clone(),
            }
        })
        .collect();
    MergedBatLabels {
        lines,
        species: classes.iter().filter(|c| c.is_species).count(),
        maps: heads
            .into_iter()
            .map(|h| h.into_iter().map(|i| merged_index[i]).collect())
            .collect(),
    }
}

/// Merge the logits of several BattyBirdNET heads into one list of `classes`
/// logits. `heads` holds each head's logits and, for each of them, its class
/// in the merged list. Classes below `species` are species; the rest are the
/// heads' other classes ("noise", "background", "silence" and so on).
///
/// A species gets its highest logit from any head. Another class gets its
/// lowest, and never more than the lowest of the heads' own highest
/// non-species logits: a head that finds a species scores its "noise" low, so
/// "noise" from the heads that do not know that species cannot hide it. A
/// class no head scores gets negative infinity (confidence 0).
pub fn merge_bat_logits(heads: &[(&[f32], &[u16])], species: usize, classes: usize) -> Vec<f32> {
    let mut out = vec![f32::NEG_INFINITY; classes];
    let mut other = vec![f32::INFINITY; classes];
    let mut cap = f32::INFINITY;
    for (logits, map) in heads {
        let mut head_other = f32::NEG_INFINITY;
        for (&logit, &class) in logits.iter().zip(map.iter()) {
            let class = class as usize;
            if class >= classes {
                continue;
            }
            if class < species {
                out[class] = out[class].max(logit);
            } else {
                other[class] = other[class].min(logit);
                head_other = head_other.max(logit);
            }
        }
        cap = cap.min(head_other);
    }
    for class in species.min(classes)..classes {
        if other[class].is_finite() {
            out[class] = other[class].min(cap);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_merge_by_scientific_name() {
        let uk = "Myotis daubentonii_Daubentons or Whiskered bat\nNyctalus noctula_Common noctule\nnoise\nsilence\n";
        let eu = "Nyctalus noctula_Noctule\nMyotis daubentonii_Daubenton's bat\nBarbastella barbastellus_Barbastelle\nNOISE\n\n";
        let se = "nyctalus noctula_Noctule\nrain\n";
        let m = merge_bat_labels(&[uk, eu, se]);
        assert_eq!(
            m.lines,
            vec![
                "Barbastella barbastellus_Barbastelle",
                // A name for the species alone, not the UK head's grouping.
                "Myotis daubentonii_Daubenton's bat",
                // Two heads say "Noctule", one "Common noctule".
                "Nyctalus noctula_Noctule",
                "noise",
                "silence",
                "rain",
            ]
        );
        assert_eq!(m.species, 3);
        assert_eq!(m.maps, vec![vec![1, 2, 3, 4], vec![2, 1, 0, 3], vec![2, 5]]);
    }

    #[test]
    fn a_tie_between_common_names_goes_to_the_first() {
        let m = merge_bat_labels(&[
            "Eptesicus serotinus_Serotine bat\n",
            "Eptesicus serotinus_Serotine\n",
        ]);
        assert_eq!(m.lines, vec!["Eptesicus serotinus_Serotine bat"]);
    }

    #[test]
    fn merged_species_take_the_highest_and_noise_the_lowest() {
        // Merged classes: 0 and 1 are species; 2 is "noise", 3 is "rain".
        // Head A knows species 0 and finds it. Head B knows only species 1
        // and hears noise. Head C has "rain" and is unsure.
        let a: (&[f32], &[u16]) = (&[2.0, -3.0], &[0, 2]);
        let b: (&[f32], &[u16]) = (&[-4.0, 5.0], &[1, 2]);
        let c: (&[f32], &[u16]) = (&[1.0, 0.5, 4.0], &[0, 2, 3]);
        let m = merge_bat_logits(&[a, b, c], 2, 4);
        assert_eq!(m[0], 2.0);
        assert_eq!(m[1], -4.0);
        // Noise: the lowest of -3, 5 and 0.5.
        assert_eq!(m[2], -3.0);
        // Rain is 4 in its one head, but head A's best non-species is -3.
        assert_eq!(m[3], -3.0);
        // Every head hears noise: it stays on top.
        let a: (&[f32], &[u16]) = (&[-2.0, 3.0], &[0, 2]);
        let b: (&[f32], &[u16]) = (&[-4.0, 5.0], &[1, 2]);
        let m = merge_bat_logits(&[a, b], 2, 3);
        assert_eq!(m, vec![-2.0, -4.0, 3.0]);
        // A class no head has.
        assert_eq!(merge_bat_logits(&[], 1, 2), vec![f32::NEG_INFINITY; 2]);
    }
}
