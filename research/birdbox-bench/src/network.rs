//! The released Just-Bird network (`Just-Bird_fp16.onnx`, BirdBox v2.1.0) in tract.
//!
//! The release file is the bare Ultralytics YOLO11s export: input `images`
//! float32 `[1, 3, 256, 256]`, output `output0` float32 `[1, 5, 1344]` (box centre x,
//! centre y, width, height in pixels, then the `bird` score after its sigmoid, for each
//! of the 1,344 anchors of the 32 x 32, 16 x 16 and 8 x 8 grids). Weights are float16,
//! with a Cast to float16 after the input and a Cast to float32 before the output.

use tract_onnx::prelude::*;

pub const IMAGE: usize = 256;
pub const ANCHORS: usize = 1344;
pub const HEAD: usize = 5 * ANCHORS;

const ONNX_FLOAT: i32 = 1;
const ONNX_FLOAT16: i32 = 10;

/// What [`patch_proto`] changed.
#[derive(Debug, Default)]
pub struct Patched {
    pub weights_to_f32: usize,
    pub casts_to_f32: usize,
}

/// Make the float16 release file compute in float32, in memory, before tract reads it
/// (the same idea as `research/birdnet-v3-bench`): float16 weights become float32 and
/// Cast(to=float16) becomes Cast(to=float32). So the official file can be downloaded
/// unchanged. The int8 file has no float16 parts and is left as it is.
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
        if n.op_type == "Cast" {
            for a in n.attribute.iter_mut() {
                if a.name == "to" && a.i == ONNX_FLOAT16 as i64 {
                    a.i = ONNX_FLOAT as i64;
                    p.casts_to_f32 += 1;
                }
            }
        }
    }
    p
}

pub struct Network {
    pub plan: std::sync::Arc<TypedRunnableModel>,
    pub patched: Patched,
}

/// Parse, patch and optimise for one `[1, 3, 256, 256]` image.
pub fn load(bytes: &[u8]) -> TractResult<Network> {
    let onnx = tract_onnx::onnx()
        .with_ignore_value_info(true)
        .with_ignore_output_shapes(true);
    let mut proto = onnx.proto_model_for_read(&mut std::io::Cursor::new(bytes))?;
    let patched = patch_proto(&mut proto);
    let plan = onnx
        .model_for_proto_model(&proto)?
        .with_input_fact(0, f32::fact([1, 3, IMAGE, IMAGE]).into())?
        .into_optimized()?
        .into_runnable()?;
    Ok(Network { plan, patched })
}

/// One image (3 x 256 x 256, RGB planes, 0..1) in; the 5 x 1344 head out.
pub fn run(net: &Network, image: &[f32]) -> TractResult<Vec<f32>> {
    let input = Tensor::from_shape(&[1, 3, IMAGE, IMAGE], image)?;
    let out = net.plan.run(tvec!(input.into()))?;
    let t = out[0].cast_to::<f32>()?;
    Ok(t.to_plain_array_view::<f32>()?.iter().copied().collect())
}
