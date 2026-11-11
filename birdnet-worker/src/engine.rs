//! The models, through tract: the BirdNET V2.4 backbone with its embeddings
//! output, and the BattyBirdNET regional heads.

use oversample_ipc::classifier::{Score, EMBEDDING_LEN, TOP_K, WINDOW_SAMPLES};
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

pub fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// The `TOP_K` highest logits as confidences, highest first.
pub fn top_scores(logits: &[f32]) -> Vec<Score> {
    let mut idx: Vec<usize> = (0..logits.len()).collect();
    let k = TOP_K.min(idx.len());
    if k == 0 {
        return Vec::new();
    }
    idx.select_nth_unstable_by(k - 1, |&a, &b| logits[b].total_cmp(&logits[a]));
    idx.truncate(k);
    idx.sort_by(|&a, &b| logits[b].total_cmp(&logits[a]));
    idx.into_iter()
        .map(|i| Score {
            class: i as u16,
            confidence: sigmoid(logits[i]),
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
    fn top_scores_fewer_classes_than_k() {
        let top = top_scores(&[1.0, 3.0, 2.0]);
        assert_eq!(
            top.iter().map(|s| s.class).collect::<Vec<_>>(),
            vec![1, 2, 0]
        );
    }
}
