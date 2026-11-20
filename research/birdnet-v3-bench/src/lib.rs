//! BirdNET+ V3.0 (ONNX, 3 s of 32 kHz mono in) through tract.
//!
//! Every V3.0 ONNX file has the outputs `predictions` (class scores, already
//! through a sigmoid) and `embeddings` (1,280 values). `run_window` returns the
//! embedding followed by the scores.
//!
//! The same code builds natively and for wasm32-unknown-unknown with wasm-bindgen
//! (the app's browser target), driven from node by `bench-bindgen.mjs`.

use tract_onnx::prelude::*;

/// 3 s at 32 kHz.
pub const WIN: usize = 96_000;
pub const EMB: usize = 1280;

pub struct Model {
    pub plan: std::sync::Arc<TypedRunnableModel>,
    pub classes: usize,
}

const ONNX_FLOAT: i32 = 1;
const ONNX_FLOAT16: i32 = 10;
const ATTR_TENSOR: i32 = 4;

/// What [`patch_proto`] changed.
#[derive(Debug, Default)]
pub struct Patched {
    pub weights_to_f32: usize,
    pub casts_to_f32: usize,
    pub zeros_typed: usize,
}

/// Make a BirdNET+ V3.0 ONNX file run in float32 in tract, in memory, before tract
/// reads it (the same three changes as `convert.py`, done in Rust so the app can
/// download the official float16 file and still compute in float32):
///
/// - float16 weights (initializers) become float32;
/// - Cast(to=float16) becomes Cast(to=float32);
/// - a ConstantOfShape with no value gets an explicit float32 zero (tract 0.23 makes
///   float64 zeros there, and the Sigmoid at the end then fails on float64).
///
/// A float32 file only gets the last change.
pub fn patch_proto(proto: &mut tract_onnx::pb::ModelProto) -> Patched {
    let mut p = Patched::default();
    let Some(g) = proto.graph.as_mut() else {
        return p;
    };
    for t in g.initializer.iter_mut() {
        if t.data_type != ONNX_FLOAT16 {
            continue;
        }
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

/// Parse and optimise the model for one window of `WIN` samples, after
/// [`patch_proto`].
pub fn load_model(bytes: &[u8]) -> TractResult<Model> {
    let onnx = tract_onnx::onnx()
        .with_ignore_value_info(true)
        .with_ignore_output_shapes(true);
    let mut proto = onnx.proto_model_for_read(&mut std::io::Cursor::new(bytes))?;
    patch_proto(&mut proto);
    let mut m = onnx
        .model_for_proto_model(&proto)?
        .with_input_fact(0, f32::fact([1, WIN]).into())?;
    drop(proto);
    let outlets = m.output_outlets()?.to_vec();
    let mut keep = vec![];
    for want in ["embeddings", "predictions"] {
        match outlets.iter().find(|o| m.outlet_label(**o) == Some(want)) {
            Some(o) => keep.push(*o),
            // The backbone cut (convert.py --outputs embeddings) has no scores.
            None if want == "predictions" => {}
            None => {
                let names: Vec<_> = outlets.iter().map(|o| m.outlet_label(*o)).collect();
                return Err(tract_onnx::tract_core::internal::format_err!(
                    "no {want} output among {names:?}"
                ));
            }
        }
    }
    m.select_output_outlets(&keep)?;
    let typed = m.into_typed()?.into_decluttered()?;
    let classes = if keep.len() > 1 {
        typed
            .outlet_fact(typed.output_outlets()?[1])?
            .shape
            .as_concrete()
            .map_or(0, |s| s[1..].iter().product())
    } else {
        0
    };
    let plan = typed.into_optimized()?.into_runnable()?;
    Ok(Model { plan, classes })
}

impl Model {
    pub fn out_len(&self) -> usize {
        EMB + self.classes
    }
}

/// One window in; EMB embedding values then the class scores out.
pub fn run_window(m: &Model, samples: &[f32]) -> TractResult<Vec<f32>> {
    let input = Tensor::from_shape(&[1, WIN], samples)?;
    let out = m.plan.run(tvec!(input.into()))?;
    let mut v = Vec::with_capacity(m.out_len());
    for t in out.iter() {
        let t = t.cast_to::<f32>()?;
        v.extend(t.to_plain_array_view::<f32>()?.iter().copied());
    }
    Ok(v)
}

/// Any model with one `[1, win]` float input, all outputs kept, loaded the way
/// `birdnet-worker/src/engine.rs` loads BirdNET V2.4. For timing V2.4 on the same
/// machine and in the same process as V3.0.
pub fn load_plain(bytes: &[u8], win: usize) -> TractResult<std::sync::Arc<TypedRunnableModel>> {
    tract_onnx::onnx()
        .with_ignore_value_info(true)
        .with_ignore_output_shapes(true)
        .model_for_read(&mut std::io::Cursor::new(bytes))?
        .with_input_fact(0, f32::fact([1, win]).into())?
        .into_optimized()?
        .into_runnable()
}

/// One window through a `load_plain` model; the outputs, flattened, back to back.
pub fn run_plain(plan: &std::sync::Arc<TypedRunnableModel>, samples: &[f32]) -> TractResult<Vec<f32>> {
    let input = Tensor::from_shape(&[1, samples.len()], samples)?;
    let out = plan.run(tvec!(input.into()))?;
    let mut v = vec![];
    for t in out.iter() {
        v.extend(t.cast_to::<f32>()?.to_plain_array_view::<f32>()?.iter().copied());
    }
    Ok(v)
}

/// The Geomodel (latitude, longitude, week in; one score per species out), loaded the
/// way `birdnet-worker/src/geomodel.rs` loads it.
pub fn geomodel_scores(bytes: &[u8], lat: f32, lon: f32, week: f32) -> TractResult<Vec<f32>> {
    let plan = tract_onnx::onnx()
        .with_ignore_value_info(true)
        .with_ignore_output_shapes(true)
        .model_for_read(&mut std::io::Cursor::new(bytes))?
        .with_input_fact(0, f32::fact([1, 3]).into())?
        .into_optimized()?
        .into_runnable()?;
    let input = Tensor::from_shape(&[1, 3], &[lat, lon, week])?;
    let out = plan.run(tvec!(input.into()))?;
    Ok(out[0].to_plain_array_view::<f32>()?.iter().copied().collect())
}

/// wasm32-unknown-unknown (the app's browser target), through wasm-bindgen.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod bindgen {
    use super::*;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    pub struct BirdNetV3(Model);

    #[wasm_bindgen]
    impl BirdNetV3 {
        #[wasm_bindgen(constructor)]
        pub fn new(model: &[u8]) -> Result<BirdNetV3, JsError> {
            load_model(model)
                .map(BirdNetV3)
                .map_err(|e| JsError::new(&format!("{e:#}")))
        }

        pub fn out_len(&self) -> usize {
            self.0.out_len()
        }

        /// One 3 s window (96,000 samples) in; embedding then scores out.
        pub fn run(&self, window: &[f32]) -> Result<Vec<f32>, JsError> {
            run_window(&self.0, window).map_err(|e| JsError::new(&format!("{e:#}")))
        }
    }

    /// A `load_plain` model (BirdNET V2.4 for comparison).
    #[wasm_bindgen]
    pub struct Plain(std::sync::Arc<TypedRunnableModel>);

    #[wasm_bindgen]
    impl Plain {
        #[wasm_bindgen(constructor)]
        pub fn new(model: &[u8], win: usize) -> Result<Plain, JsError> {
            load_plain(model, win)
                .map(Plain)
                .map_err(|e| JsError::new(&format!("{e:#}")))
        }

        pub fn run(&self, window: &[f32]) -> Result<Vec<f32>, JsError> {
            run_plain(&self.0, window).map_err(|e| JsError::new(&format!("{e:#}")))
        }
    }

    /// Bytes of WASM memory in use (it only grows).
    #[wasm_bindgen]
    pub fn wasm_memory_bytes() -> usize {
        core::arch::wasm32::memory_size(0) * 65536
    }
}
