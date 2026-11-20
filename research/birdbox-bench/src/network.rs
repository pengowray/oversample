//! The released Just-Bird network (`Just-Bird_fp16.onnx`, BirdBox v2.1.0) in tract.
//!
//! The release file is the bare Ultralytics YOLO11s export: input `images`
//! float32 `[1, 3, 256, 256]`, output `output0` float32 `[1, 5, 1344]` (box centre x,
//! centre y, width, height in pixels, then the `bird` score after its sigmoid, for each
//! of the 1,344 anchors of the 32 x 32, 16 x 16 and 8 x 8 grids). Weights are float16,
//! with a Cast to float16 after the input and a Cast to float32 before the output.

use tract_onnx::prelude::*;
use tract_onnx::tract_core::internal::format_err;

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
        if t.data_type == ONNX_FLOAT16 {
            f16_bytes_to_f32(t);
            p.weights_to_f32 += 1;
        }
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

fn f16_bytes_to_f32(t: &mut tract_onnx::pb::TensorProto) {
    let halves: Vec<u16> = if !t.raw_data.is_empty() {
        t.raw_data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect()
    } else {
        t.int32_data.iter().map(|&v| v as u16).collect()
    };
    t.raw_data = halves.iter().flat_map(|&h| f16::from_bits(h).to_f32().to_le_bytes()).collect();
    t.int32_data.clear();
    t.data_type = ONNX_FLOAT;
}

/// Cut the bare network out of BirdBox's all-in-one demo graph
/// (`docs/models/Just-Bird_fp16.onnx`), in memory. That file is served with CORS headers
/// (GitHub Pages, raw.githubusercontent.com); the release asset is not. The network sits
/// in the graph's Loop body, from the Cast(to=float16) of the rendered image to the
/// Cast(to=float32) after the head. Keeps those nodes, the Constant nodes and
/// initializers they use, and gives the result the release file's interface
/// (`images` [1, 3, 256, 256] in, `output0` [1, 5, 1344] out). Float16 Constant values
/// become float32 here; `patch_proto` does the weights and casts. Python twin:
/// `cut_graph.py`.
pub fn cut_demo_graph(proto: &mut tract_onnx::pb::ModelProto) -> TractResult<()> {
    use tract_onnx::pb::{AttributeProto, GraphProto, NodeProto, ValueInfoProto};
    let main = proto.graph.as_ref().ok_or_else(|| format_err!("no graph"))?;
    let loop_node = main
        .node
        .iter()
        .find(|n| n.op_type == "Loop")
        .ok_or_else(|| format_err!("no Loop: not the demo graph"))?;
    let body: &GraphProto = loop_node
        .attribute
        .iter()
        .find(|a| a.name == "body")
        .and_then(|a| a.g.as_ref())
        .ok_or_else(|| format_err!("Loop without body"))?;
    let conv0 = body
        .node
        .iter()
        .find(|n| n.op_type == "Conv" && n.input.iter().any(|i| i == "b_core.network.model.0.conv.weight"))
        .ok_or_else(|| format_err!("first network Conv not found"))?;
    let x_in = conv0.input[0].clone();
    let mut consumers: std::collections::HashMap<&str, Vec<usize>> = Default::default();
    for (k, n) in body.node.iter().enumerate() {
        for i in &n.input {
            consumers.entry(i.as_str()).or_default().push(k);
        }
    }
    let is_cast_to = |n: &NodeProto, to: i32| n.op_type == "Cast" && n.attribute.iter().any(|a| a.name == "to" && a.i == to as i64);
    let mut keep = vec![false; body.node.len()];
    let mut out_val = None;
    let mut frontier = vec![x_in.clone()];
    while let Some(v) = frontier.pop() {
        for &k in consumers.get(v.as_str()).map(|v| v.as_slice()).unwrap_or(&[]) {
            if keep[k] {
                continue;
            }
            let n = &body.node[k];
            if is_cast_to(n, ONNX_FLOAT) {
                out_val = Some(n.input[0].clone());
                continue;
            }
            keep[k] = true;
            frontier.extend(n.output.iter().cloned());
        }
    }
    let out_val = out_val.ok_or_else(|| format_err!("network output Cast not found"))?;
    let used: std::collections::HashSet<&str> =
        body.node.iter().zip(&keep).filter(|(_, &k)| k).flat_map(|(n, _)| n.input.iter().map(String::as_str)).collect();
    let cast = |input: &str, output: &str, to: i32| NodeProto {
        op_type: "Cast".into(),
        input: vec![input.into()],
        output: vec![output.into()],
        attribute: vec![AttributeProto { name: "to".into(), r#type: 2, i: to as i64, ..Default::default() }],
        ..Default::default()
    };
    let mut nodes = vec![cast("images", &x_in, ONNX_FLOAT16)];
    for (n, &k) in body.node.iter().zip(&keep) {
        let is_const = n.op_type == "Constant" && n.output.iter().any(|o| used.contains(o.as_str()));
        if k || is_const {
            let mut n = n.clone();
            for a in n.attribute.iter_mut() {
                if let Some(t) = a.t.as_mut().filter(|t| t.data_type == ONNX_FLOAT16) {
                    f16_bytes_to_f32(t);
                }
            }
            nodes.push(n);
        }
    }
    nodes.push(cast(&out_val, "output0", ONNX_FLOAT));
    let initializer = body.initializer.iter().filter(|t| used.contains(t.name.as_str())).cloned().collect();
    let graph = GraphProto {
        name: "just_bird_network".into(),
        node: nodes,
        initializer,
        input: vec![ValueInfoProto { name: "images".into(), ..Default::default() }],
        output: vec![ValueInfoProto { name: "output0".into(), ..Default::default() }],
        ..Default::default()
    };
    proto.graph = Some(graph);
    proto.functions.clear();
    proto.opset_import.retain(|o| o.domain.is_empty() || o.domain == "ai.onnx");
    Ok(())
}

pub struct Network {
    pub plan: std::sync::Arc<TypedRunnableModel>,
    pub patched: Patched,
}

/// Parse, patch and optimise for one `[1, 3, 256, 256]` image. Takes the release
/// file, or the all-in-one demo graph (cut with [`cut_demo_graph`] first).
pub fn load(bytes: &[u8]) -> TractResult<Network> {
    let onnx = tract_onnx::onnx()
        .with_ignore_value_info(true)
        .with_ignore_output_shapes(true);
    let mut proto = onnx.proto_model_for_read(&mut std::io::Cursor::new(bytes))?;
    let is_demo_graph = proto
        .graph
        .as_ref()
        .is_some_and(|g| g.input.first().is_some_and(|i| i.name == "audio"));
    if is_demo_graph {
        cut_demo_graph(&mut proto)?;
    }
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
