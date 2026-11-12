//! Google's Perch 2.0, as two files (`huggingface.co/pengowray/perch-v2-tract`):
//!
//! - [`Embedding`]: the embedding model, in tract. One window of 160,000 samples
//!   (5 s at 32 kHz) in; out come the 1,536-value embedding and the spatial
//!   embedding (16 x 4 positions of 1,536 values).
//! - [`Classifier`]: the 14,795-class classifier, in plain Rust on int8 weights,
//!   scoring the spatial embedding.
//!
//! The ONNX file has its batch-norm folded into the convolutions
//! (`research/perch-bench/fix_for_tract.py`), so tract 0.23 loads it at batch 1.

mod classifier;
mod kernel;

pub use classifier::Classifier;

use oversample_ipc::classifier::{Score, PERCH_EMBEDDING_LEN, PERCH_WINDOW_SAMPLES, TOP_K};
use tract_onnx::prelude::*;

type Plan = std::sync::Arc<TypedRunnableModel>;

/// The embedding model, with only its `embedding` and `spatial_embedding`
/// outputs kept.
pub struct Embedding {
    plan: Plan,
}

/// What the embedding model gives for one window.
pub struct EmbeddingOut {
    pub embedding: Vec<f32>,
    /// 64 positions x 1,536 values, the classifier's input.
    pub spatial: Vec<f32>,
}

impl Embedding {
    pub fn load(bytes: &[u8]) -> TractResult<Embedding> {
        let mut m = tract_onnx::onnx()
            .with_ignore_value_info(true)
            .with_ignore_output_shapes(true)
            .model_for_read(&mut std::io::Cursor::new(bytes))?
            .with_input_fact(0, f32::fact([1, PERCH_WINDOW_SAMPLES]).into())?;
        let outlets = m.output_outlets()?.to_vec();
        let mut keep = Vec::new();
        for want in ["embedding", "spatial_embedding"] {
            let o = outlets
                .iter()
                .find(|o| m.outlet_label(**o) == Some(want))
                .ok_or_else(|| TractError::msg(format!("the model has no {want} output")))?;
            keep.push(*o);
        }
        m.select_output_outlets(&keep)?;
        let plan = m.into_optimized()?.into_runnable()?;
        Ok(Embedding { plan })
    }

    pub fn run(&self, window: &[f32]) -> TractResult<EmbeddingOut> {
        let input = Tensor::from_shape(&[1, PERCH_WINDOW_SAMPLES], window)?;
        let out = self.plan.run(tvec!(input.into()))?;
        let flat = |i: usize| -> TractResult<Vec<f32>> {
            Ok(out[i]
                .to_plain_array_view::<f32>()?
                .iter()
                .copied()
                .collect())
        };
        let embedding = flat(0)?;
        let spatial = flat(1)?;
        if embedding.len() != PERCH_EMBEDDING_LEN || spatial.len() % PERCH_EMBEDDING_LEN != 0 {
            return Err(TractError::msg(format!(
                "embedding has {} values and the spatial embedding {}; expected {PERCH_EMBEDDING_LEN} and a multiple of it",
                embedding.len(),
                spatial.len()
            )));
        }
        Ok(EmbeddingOut { embedding, spatial })
    }
}

/// The `TOP_K` highest logits as softmax probabilities over every class,
/// highest first. Perch's head was trained with a softmax, so its logits are
/// only meaningful relative to each other.
pub fn top_scores(logits: &[f32]) -> Vec<Score> {
    let Some(max) = logits.iter().copied().reduce(f32::max) else {
        return Vec::new();
    };
    let sum: f64 = logits.iter().map(|&l| ((l - max) as f64).exp()).sum();
    let mut top = crate::engine::top_scores(logits);
    for s in &mut top {
        s.confidence = (((logits[s.class as usize] - max) as f64).exp() / sum) as f32;
    }
    debug_assert!(top.len() <= TOP_K);
    top
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn softmax_scores() {
        let logits = [1.0f32, 3.0, 2.0, 3.0];
        let top = top_scores(&logits);
        let total: f32 = top.iter().map(|s| s.confidence).sum();
        assert!((total - 1.0).abs() < 1e-5);
        assert!((top[0].confidence - top[1].confidence).abs() < 1e-6);
        assert_eq!(top[2].class, 2);
    }
}
