//! The models, through tract: the BirdNET V2.4 backbone with its embeddings
//! output, and the BattyBirdNET regional heads. Also the top-k ranking that
//! BirdNET+ V3.0 (`crate::v3`) shares.

use oversample_ipc::classifier::{
    merge_bat_logits, BatHeadPart, Score, EMBEDDING_LEN, TOP_K, WINDOW_SAMPLES,
};
use tract_onnx::prelude::*;

type Plan = std::sync::Arc<TypedRunnableModel>;

/// BirdNET V2.4 with an embeddings output: one window of 144,000 samples in;
/// out come 6,522 logits (output 0) and a 1,024-value embedding (output 1).
pub struct Backbone {
    plan: Plan,
}

/// What the backbone gives for one window.
pub struct BackboneOut {
    pub logits: Vec<f32>,
    pub embedding: Vec<f32>,
}

impl Backbone {
    pub fn load(bytes: &[u8]) -> TractResult<Backbone> {
        // Some conversions store intermediate shapes with a symbolic batch that
        // clash with a fixed batch; ignore them and let tract infer its own.
        let plan = tract_onnx::onnx()
            .with_ignore_value_info(true)
            .with_ignore_output_shapes(true)
            .model_for_read(&mut std::io::Cursor::new(bytes))?
            .with_input_fact(0, f32::fact([1, WINDOW_SAMPLES]).into())?
            .into_optimized()?
            .into_runnable()?;
        Ok(Backbone { plan })
    }

    pub fn run(&self, window: &[f32]) -> TractResult<BackboneOut> {
        let input = Tensor::from_shape(&[1, WINDOW_SAMPLES], window)?;
        let out = self.plan.run(tvec!(input.into()))?;
        if out.len() < 2 {
            return Err(TractError::msg(format!(
                "the model has {} output(s); expected logits and an embedding",
                out.len()
            )));
        }
        let logits: Vec<f32> = out[0]
            .to_plain_array_view::<f32>()?
            .iter()
            .copied()
            .collect();
        let embedding: Vec<f32> = out[1]
            .to_plain_array_view::<f32>()?
            .iter()
            .copied()
            .collect();
        if embedding.len() != EMBEDDING_LEN {
            return Err(TractError::msg(format!(
                "embedding has {} values; expected {EMBEDDING_LEN}",
                embedding.len()
            )));
        }
        Ok(BackboneOut { logits, embedding })
    }
}

/// A BattyBirdNET regional head: the 1,024-value embedding in, one logit per
/// species of the region out.
pub struct Head {
    plan: Plan,
}

impl Head {
    pub fn load(bytes: &[u8]) -> TractResult<Head> {
        let plan = tract_onnx::onnx()
            .with_ignore_value_info(true)
            .with_ignore_output_shapes(true)
            .model_for_read(&mut std::io::Cursor::new(bytes))?
            .with_input_fact(0, f32::fact([1, EMBEDDING_LEN]).into())?
            .into_optimized()?
            .into_runnable()?;
        Ok(Head { plan })
    }

    pub fn run(&self, embedding: &[f32]) -> TractResult<Vec<f32>> {
        let input = Tensor::from_shape(&[1, EMBEDDING_LEN], embedding)?;
        let out = self.plan.run(tvec!(input.into()))?;
        Ok(out[0]
            .to_plain_array_view::<f32>()?
            .iter()
            .copied()
            .collect())
    }
}

/// Several regional heads scored as one: each species gets the highest logit
/// any head gives it (`merge_bat_logits` has the whole rule).
pub struct MergedHeads {
    /// Each head and, for each of its outputs, the class in the merged list.
    heads: Vec<(Head, Vec<u16>)>,
    species: usize,
    classes: usize,
}

impl MergedHeads {
    /// Build the heads from their files, back to back in `bytes`. Fails when
    /// a head's outputs do not line up with its `classes`.
    pub fn load(
        bytes: &[u8],
        parts: &[BatHeadPart],
        species: usize,
        classes: usize,
    ) -> TractResult<MergedHeads> {
        let mut heads = Vec::with_capacity(parts.len());
        let mut at = 0usize;
        for (i, part) in parts.iter().enumerate() {
            let end = at + part.len as usize;
            let file = bytes.get(at..end).ok_or_else(|| {
                TractError::msg(format!("the bytes end before regional model {}", i + 1))
            })?;
            at = end;
            let head = Head::load(file)?;
            let outputs = head.run(&[0.0; EMBEDDING_LEN])?.len();
            if outputs != part.classes.len() {
                return Err(TractError::msg(format!(
                    "regional model {} has {outputs} outputs; its label list has {}",
                    i + 1,
                    part.classes.len()
                )));
            }
            if part.classes.iter().any(|&c| c as usize >= classes) {
                return Err(TractError::msg(format!(
                    "regional model {} names a class past the {classes} merged ones",
                    i + 1
                )));
            }
            heads.push((head, part.classes.clone()));
        }
        if heads.is_empty() {
            return Err(TractError::msg("no regional models were sent"));
        }
        Ok(MergedHeads {
            heads,
            species,
            classes,
        })
    }

    /// The merged logits for one embedding.
    pub fn run(&self, embedding: &[f32]) -> TractResult<Vec<f32>> {
        let logits = self
            .heads
            .iter()
            .map(|(h, _)| h.run(embedding))
            .collect::<TractResult<Vec<_>>>()?;
        let heads: Vec<(&[f32], &[u16])> = logits
            .iter()
            .zip(&self.heads)
            .map(|(l, (_, map))| (l.as_slice(), map.as_slice()))
            .collect();
        Ok(merge_bat_logits(&heads, self.species, self.classes))
    }
}

pub fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// The `TOP_K` highest logits as confidences (their sigmoid), highest first.
pub fn top_scores(logits: &[f32]) -> Vec<Score> {
    top_k(logits, sigmoid)
}

/// The `TOP_K` highest of values that are confidences already (BirdNET+
/// V3.0's outputs), highest first, unchanged.
pub fn top_confidences(confidences: &[f32]) -> Vec<Score> {
    top_k(confidences, |c| c)
}

/// The `TOP_K` highest values, highest first, each through `confidence`.
fn top_k(values: &[f32], confidence: impl Fn(f32) -> f32) -> Vec<Score> {
    let mut idx: Vec<usize> = (0..values.len()).collect();
    let k = TOP_K.min(idx.len());
    if k == 0 {
        return Vec::new();
    }
    idx.select_nth_unstable_by(k - 1, |&a, &b| values[b].total_cmp(&values[a]));
    idx.truncate(k);
    idx.sort_by(|&a, &b| values[b].total_cmp(&values[a]));
    idx.into_iter()
        .map(|i| Score {
            class: i as u16,
            confidence: confidence(values[i]),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_scores_order() {
        let logits: Vec<f32> = (0..50).map(|i| ((i * 37) % 50) as f32 - 25.0).collect();
        let top = top_scores(&logits);
        assert_eq!(top.len(), TOP_K);
        assert!(top.windows(2).all(|w| w[0].confidence >= w[1].confidence));
        let best = logits
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap()
            .0;
        assert_eq!(top[0].class as usize, best);
        assert!((sigmoid(0.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn confidences_are_ranked_without_another_sigmoid() {
        let probs = [0.05, 0.902, 0.3, 0.0, 0.71];
        let top = top_confidences(&probs);
        assert_eq!(
            top.iter().map(|s| s.class).collect::<Vec<_>>(),
            vec![1, 4, 2, 0, 3]
        );
        // As given: a second sigmoid would turn 0.902 into about 0.71.
        assert_eq!(top[0].confidence, 0.902);
        assert_eq!(top[4].confidence, 0.0);
        assert!((top_scores(&probs)[0].confidence - sigmoid(0.902)).abs() < 1e-6);
    }

    #[test]
    fn top_scores_fewer_classes_than_k() {
        let top = top_scores(&[1.0, 3.0, 2.0]);
        assert_eq!(
            top.iter().map(|s| s.class).collect::<Vec<_>>(),
            vec![1, 2, 0]
        );
    }
}
