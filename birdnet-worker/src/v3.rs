//! BirdNET+ V3.0 through tract: one window of 96,000 samples (3 s at 32 kHz)
//! in; a 1,280-value embedding and 11,725 confidences out.
//!
//! The app downloads the official pruned float16 file unchanged. tract 0.23
//! cannot run it as published, so [`patch_proto`] changes the graph in memory
//! before tract builds it (`research/birdnet-v3-bench/README.md`, "tract:
//! obstacles and fixes"):
//!
//! - float16 weights become float32 (tract has no fast float16 convolution,
//!   and a Cast kept in the plan converts the weights on every run);
//! - Cast(to=float16) becomes Cast(to=float32);
//! - the head's ConstantOfShape with no value gets an explicit float32 zero
//!   (tract makes float64 zeros there, and the last Sigmoid fails on them).
//!
//! The scores come out of the model's own sigmoid, so they are confidences
//! already: [`crate::engine::top_confidences`] ranks them without another one.

use oversample_ipc::classifier::{V3_EMBEDDING_LEN, V3_WINDOW_SAMPLES};
use tract_onnx::prelude::*;

const ONNX_FLOAT: i32 = 1;
const ONNX_FLOAT16: i32 = 10;
const ATTR_TENSOR: i32 = 4;

/// What [`patch_proto`] changed.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Patched {
    pub weights_to_f32: usize,
    pub casts_to_f32: usize,
    pub zeros_typed: usize,
}

/// Make a BirdNET+ V3.0 ONNX graph run in float32 in tract (the three changes
/// in the module notes). A float32 file only gets the last one.
pub fn patch_proto(proto: &mut tract_onnx::pb::ModelProto) -> Patched {
    let mut p = Patched::default();
    let Some(g) = proto.graph.as_mut() else {
        return p;
    };
    for t in g.initializer.iter_mut() {
        if t.data_type != ONNX_FLOAT16 {
            continue;
        }
        f16_tensor_to_f32(t);
        p.weights_to_f32 += 1;
    }
    for n in g.node.iter_mut() {
        match n.op_type.as_str() {
            "Cast" => {
                for a in n.attribute.iter_mut() {
                    if a.name == "to" && a.i == ONNX_FLOAT16 as i64 {
                        a.i = ONNX_FLOAT as i64;
                        p.casts_to_f32 += 1;
                    }
                }
            }
            "ConstantOfShape" if !n.attribute.iter().any(|a| a.name == "value") => {
                n.attribute.push(tract_onnx::pb::AttributeProto {
                    name: "value".into(),
                    r#type: ATTR_TENSOR,
                    t: Some(tract_onnx::pb::TensorProto {
                        dims: vec![1],
                        data_type: ONNX_FLOAT,
                        raw_data: 0f32.to_le_bytes().to_vec(),
                        ..Default::default()
                    }),
                    ..Default::default()
                });
                p.zeros_typed += 1;
            }
            _ => {}
        }
    }
    p
}

/// Turn a float16 tensor into float32 in place (also used for BirdBox's
/// network, `crate::birdbox::network`).
pub(crate) fn f16_tensor_to_f32(t: &mut tract_onnx::pb::TensorProto) {
    let halves: Vec<u16> = if !t.raw_data.is_empty() {
        t.raw_data
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect()
    } else {
        // ONNX keeps float16 values in int32_data when raw_data is not used.
        t.int32_data.iter().map(|&v| v as u16).collect()
    };
    t.raw_data = halves
        .iter()
        .flat_map(|&h| f16::from_bits(h).to_f32().to_le_bytes())
        .collect();
    t.int32_data.clear();
    t.data_type = ONNX_FLOAT;
}

/// True when every sample is exactly zero (digital silence). The pruned model
/// scores such a window up to 0.25 for birds that are not there, so it is not
/// run. Quiet noise is fine: the model normalises the level.
pub fn is_digital_silence(window: &[f32]) -> bool {
    window.iter().all(|&s| s == 0.0)
}

/// BirdNET+ V3.0, built for one window.
pub struct Model {
    plan: std::sync::Arc<TypedRunnableModel>,
}

/// What the model gives for one window.
pub struct ModelOut {
    pub embedding: Vec<f32>,
    /// One confidence (0 to 1) per class, in label order.
    pub confidences: Vec<f32>,
}

impl Model {
    pub fn load(bytes: &[u8]) -> TractResult<Model> {
        let onnx = tract_onnx::onnx()
            .with_ignore_value_info(true)
            .with_ignore_output_shapes(true);
        let mut proto = onnx.proto_model_for_read(&mut std::io::Cursor::new(bytes))?;
        patch_proto(&mut proto);
        let mut m = onnx
            .model_for_proto_model(&proto)?
            .with_input_fact(0, f32::fact([1, V3_WINDOW_SAMPLES]).into())?;
        // The parsed file is about as large as the model; free it before
        // tract optimises.
        drop(proto);
        let outlets = m.output_outlets()?.to_vec();
        let mut keep = Vec::with_capacity(2);
        for want in ["embeddings", "predictions"] {
            let o = outlets
                .iter()
                .find(|o| m.outlet_label(**o) == Some(want))
                .ok_or_else(|| TractError::msg(format!("the model has no {want} output")))?;
            keep.push(*o);
        }
        m.select_output_outlets(&keep)?;
        let plan = m
            .into_typed()?
            .into_decluttered()?
            .into_optimized()?
            .into_runnable()?;
        Ok(Model { plan })
    }

    pub fn run(&self, window: &[f32]) -> TractResult<ModelOut> {
        let input = Tensor::from_shape(&[1, V3_WINDOW_SAMPLES], window)?;
        let out = self.plan.run(tvec!(input.into()))?;
        let flat = |i: usize| -> TractResult<Vec<f32>> {
            let t = out[i].cast_to::<f32>()?;
            Ok(t.to_plain_array_view::<f32>()?.iter().copied().collect())
        };
        let embedding = flat(0)?;
        if embedding.len() != V3_EMBEDDING_LEN {
            return Err(TractError::msg(format!(
                "embedding has {} values; expected {V3_EMBEDDING_LEN}",
                embedding.len()
            )));
        }
        Ok(ModelOut {
            embedding,
            confidences: flat(1)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_exact_zeros_are_digital_silence() {
        assert!(is_digital_silence(&[0.0; 16]));
        assert!(is_digital_silence(&[0.0, -0.0, 0.0]));
        assert!(!is_digital_silence(&[0.0, 1e-9, 0.0]));
        let mut w = vec![0.0; V3_WINDOW_SAMPLES];
        assert!(is_digital_silence(&w));
        w[V3_WINDOW_SAMPLES - 1] = -3e-5;
        assert!(!is_digital_silence(&w));
    }

    #[test]
    fn patch_makes_float16_weights_casts_and_zeros_float32() {
        use tract_onnx::pb::{AttributeProto, GraphProto, ModelProto, NodeProto, TensorProto};
        let half = |v: f32| f16::from_f32(v).to_bits().to_le_bytes();
        let mut raw = half(1.5).to_vec();
        raw.extend(half(-2.0));
        let mut proto = ModelProto {
            graph: Some(GraphProto {
                initializer: vec![
                    TensorProto {
                        data_type: ONNX_FLOAT16,
                        raw_data: raw,
                        ..Default::default()
                    },
                    TensorProto {
                        data_type: ONNX_FLOAT16,
                        int32_data: vec![half(0.25)[0] as i32 | (half(0.25)[1] as i32) << 8],
                        ..Default::default()
                    },
                ],
                node: vec![
                    NodeProto {
                        op_type: "Cast".into(),
                        attribute: vec![AttributeProto {
                            name: "to".into(),
                            i: ONNX_FLOAT16 as i64,
                            ..Default::default()
                        }],
                        ..Default::default()
                    },
                    NodeProto {
                        op_type: "ConstantOfShape".into(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }),
            ..Default::default()
        };
        let p = patch_proto(&mut proto);
        assert_eq!(
            p,
            Patched {
                weights_to_f32: 2,
                casts_to_f32: 1,
                zeros_typed: 1
            }
        );
        let g = proto.graph.unwrap();
        let f32s = |t: &TensorProto| -> Vec<f32> {
            t.raw_data
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect()
        };
        assert_eq!(g.initializer[0].data_type, ONNX_FLOAT);
        assert_eq!(f32s(&g.initializer[0]), vec![1.5, -2.0]);
        assert_eq!(f32s(&g.initializer[1]), vec![0.25]);
        assert!(g.initializer[1].int32_data.is_empty());
        assert_eq!(g.node[0].attribute[0].i, ONNX_FLOAT as i64);
        let zero = g.node[1].attribute[0].t.as_ref().unwrap();
        assert_eq!(zero.data_type, ONNX_FLOAT);
        assert_eq!(f32s(zero), vec![0.0]);
    }
}
