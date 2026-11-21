//! BirdBox's Just-Bird network (BirdBox v2.1.0) in tract.
//!
//! The network is the bare Ultralytics YOLO11s export: input `images` float32
//! `[1, 3, 256, 256]`, output `output0` float32 `[1, 5, 1344]` (box centre x,
//! centre y, width, height in pixels, then the `bird` score after its sigmoid,
//! for each of the 1,344 anchors). Its weights are float16, with a Cast to
//! float16 after the input and a Cast to float32 before the output.
//!
//! The app downloads BirdBox's all-in-one demo graph (audio in, boxes out),
//! which is served with CORS headers. [`cut_demo_graph`] takes the network out
//! of it in memory and [`patch_proto`] makes it compute in float32, so the
//! official file is used unchanged. The release file (`Just-Bird_fp16.onnx`)
//! loads too, without the cut. Ported from `research/birdbox-bench/src/network.rs`.

use oversample_ipc::classifier::boxes::{HEAD, IMAGE};
use tract_onnx::prelude::*;

const ONNX_FLOAT: i32 = 1;
const ONNX_FLOAT16: i32 = 10;
/// AttributeProto type INT.
const ATTR_INT: i32 = 2;
/// The first convolution of the network, in the demo graph's Loop body.
const FIRST_CONV_WEIGHT: &str = "b_core.network.model.0.conv.weight";

/// What [`patch_proto`] changed.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Patched {
    pub weights_to_f32: usize,
    pub casts_to_f32: usize,
}

/// Float16 weights become float32 and Cast(to=float16) becomes
/// Cast(to=float32), in memory before tract builds the model (the same idea
/// as `crate::v3::patch_proto`).
pub fn patch_proto(proto: &mut tract_onnx::pb::ModelProto) -> Patched {
    let mut p = Patched::default();
    let Some(g) = proto.graph.as_mut() else {
        return p;
    };
    for t in g.initializer.iter_mut() {
        if t.data_type == ONNX_FLOAT16 {
            crate::v3::f16_tensor_to_f32(t);
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

/// True for BirdBox's all-in-one demo graph, whose input is audio.
pub fn is_demo_graph(proto: &tract_onnx::pb::ModelProto) -> bool {
    proto
        .graph
        .as_ref()
        .is_some_and(|g| g.input.first().is_some_and(|i| i.name == "audio"))
}

/// Cut the bare network out of BirdBox's all-in-one demo graph
/// (`docs/models/Just-Bird_fp16.onnx`), in memory. The network sits in the
/// graph's Loop body, from the Cast(to=float16) of the rendered image to the
/// Cast(to=float32) after the head. Keeps those nodes and the Constant nodes
/// and initializers they use, and gives the result the release file's
/// interface (`images` [1, 3, 256, 256] in, `output0` [1, 5, 1344] out).
/// Float16 Constant values become float32 here; [`patch_proto`] does the
/// weights and casts.
pub fn cut_demo_graph(proto: &mut tract_onnx::pb::ModelProto) -> TractResult<()> {
    use tract_onnx::pb::{AttributeProto, GraphProto, NodeProto, ValueInfoProto};
    let main = proto.graph.as_ref().ok_or_else(|| TractError::msg("no graph"))?;
    let loop_node = main
        .node
        .iter()
        .find(|n| n.op_type == "Loop")
        .ok_or_else(|| TractError::msg("no Loop: not BirdBox's demo graph"))?;
    let body: &GraphProto = loop_node
        .attribute
        .iter()
        .find(|a| a.name == "body")
        .and_then(|a| a.g.as_ref())
        .ok_or_else(|| TractError::msg("Loop without body"))?;
    let conv0 = body
        .node
        .iter()
        .find(|n| n.op_type == "Conv" && n.input.iter().any(|i| i == FIRST_CONV_WEIGHT))
        .ok_or_else(|| TractError::msg("first network Conv not found"))?;
    let x_in = conv0.input[0].clone();
    let mut consumers: std::collections::HashMap<&str, Vec<usize>> = Default::default();
    for (k, n) in body.node.iter().enumerate() {
        for i in &n.input {
            consumers.entry(i.as_str()).or_default().push(k);
        }
    }
    let is_cast_to = |n: &NodeProto, to: i32| {
        n.op_type == "Cast" && n.attribute.iter().any(|a| a.name == "to" && a.i == to as i64)
    };
    // Every node downstream of the network's input, up to the Cast back to
    // float32.
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
    let out_val = out_val.ok_or_else(|| TractError::msg("network output Cast not found"))?;
    let used: std::collections::HashSet<&str> = body
        .node
        .iter()
        .zip(&keep)
        .filter(|(_, &k)| k)
        .flat_map(|(n, _)| n.input.iter().map(String::as_str))
        .collect();
    let cast = |input: &str, output: &str, to: i32| NodeProto {
        op_type: "Cast".into(),
        input: vec![input.into()],
        output: vec![output.into()],
        attribute: vec![AttributeProto {
            name: "to".into(),
            r#type: ATTR_INT,
            i: to as i64,
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut nodes = vec![cast("images", &x_in, ONNX_FLOAT16)];
    for (n, &k) in body.node.iter().zip(&keep) {
        let is_const = n.op_type == "Constant" && n.output.iter().any(|o| used.contains(o.as_str()));
        if k || is_const {
            let mut n = n.clone();
            for a in n.attribute.iter_mut() {
                if let Some(t) = a.t.as_mut().filter(|t| t.data_type == ONNX_FLOAT16) {
                    crate::v3::f16_tensor_to_f32(t);
                }
            }
            nodes.push(n);
        }
    }
    nodes.push(cast(&out_val, "output0", ONNX_FLOAT));
    let initializer = body
        .initializer
        .iter()
        .filter(|t| used.contains(t.name.as_str()))
        .cloned()
        .collect();
    let graph = GraphProto {
        name: "just_bird_network".into(),
        node: nodes,
        initializer,
        // tract needs the input's type declared.
        input: vec![ValueInfoProto {
            name: "images".into(),
            r#type: Some(tract_onnx::pb::TypeProto {
                value: Some(tract_onnx::pb::type_proto::Value::TensorType(
                    tract_onnx::pb::type_proto::Tensor {
                        elem_type: ONNX_FLOAT,
                        shape: None,
                    },
                )),
                ..Default::default()
            }),
            ..Default::default()
        }],
        output: vec![ValueInfoProto {
            name: "output0".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    proto.graph = Some(graph);
    proto.functions.clear();
    proto
        .opset_import
        .retain(|o| o.domain.is_empty() || o.domain == "ai.onnx");
    Ok(())
}

/// The network, built for one image.
pub struct Network {
    plan: std::sync::Arc<TypedRunnableModel>,
}

impl Network {
    /// Parse, cut (for the demo graph), patch and optimise.
    pub fn load(bytes: &[u8]) -> TractResult<Network> {
        let onnx = tract_onnx::onnx()
            .with_ignore_value_info(true)
            .with_ignore_output_shapes(true);
        let mut proto = onnx.proto_model_for_read(&mut std::io::Cursor::new(bytes))?;
        if is_demo_graph(&proto) {
            cut_demo_graph(&mut proto)?;
        }
        patch_proto(&mut proto);
        let m = onnx
            .model_for_proto_model(&proto)?
            .with_input_fact(0, f32::fact([1, 3, IMAGE, IMAGE]).into())?;
        drop(proto);
        let plan = m.into_optimized()?.into_runnable()?;
        Ok(Network { plan })
    }

    /// One image (3 x 256 x 256: R, G, B planes, 0 to 1) in; the
    /// [`HEAD`] values of the network's output out.
    pub fn run(&self, image: &[f32]) -> TractResult<Vec<f32>> {
        let input = Tensor::from_shape(&[1, 3, IMAGE, IMAGE], image)?;
        let out = self.plan.run(tvec!(input.into()))?;
        let t = out[0].cast_to::<f32>()?;
        let head: Vec<f32> = t.to_plain_array_view::<f32>()?.iter().copied().collect();
        if head.len() != HEAD {
            return Err(TractError::msg(format!(
                "the box network gave {} values; expected {HEAD}",
                head.len()
            )));
        }
        Ok(head)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tract_onnx::pb::{
        AttributeProto, GraphProto, ModelProto, NodeProto, TensorProto, ValueInfoProto,
    };

    fn node(op: &str, inputs: &[&str], outputs: &[&str]) -> NodeProto {
        NodeProto {
            op_type: op.into(),
            input: inputs.iter().map(|s| s.to_string()).collect(),
            output: outputs.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    fn cast(input: &str, output: &str, to: i32) -> NodeProto {
        let mut n = node("Cast", &[input], &[output]);
        n.attribute.push(AttributeProto {
            name: "to".into(),
            r#type: ATTR_INT,
            i: to as i64,
            ..Default::default()
        });
        n
    }

    fn half_tensor(name: &str, v: &[f32]) -> TensorProto {
        TensorProto {
            name: name.into(),
            dims: vec![v.len() as i64],
            data_type: ONNX_FLOAT16,
            raw_data: v
                .iter()
                .flat_map(|&x| f16::from_f32(x).to_bits().to_le_bytes())
                .collect(),
            ..Default::default()
        }
    }

    /// A small graph shaped like BirdBox's demo graph: audio in, a Loop whose
    /// body renders an image (`spec`), casts it to float16, runs the network
    /// (two nodes, one with a float16 Constant), casts the head to float32
    /// and post-processes it (NMS).
    fn demo_like() -> ModelProto {
        let mut constant = node("Constant", &[], &["k"]);
        constant.attribute.push(AttributeProto {
            name: "value".into(),
            t: Some(half_tensor("", &[0.5])),
            ..Default::default()
        });
        let body = GraphProto {
            node: vec![
                node("Gather", &["audio_seg", "lut"], &["spec"]),
                cast("spec", "spec16", ONNX_FLOAT16),
                node("Conv", &["spec16", FIRST_CONV_WEIGHT], &["c0"]),
                constant,
                node("Mul", &["c0", "k"], &["head16"]),
                cast("head16", "head32", ONNX_FLOAT),
                node("NonMaxSuppression", &["head32"], &["sel"]),
                node("Constant", &[], &["unused"]),
            ],
            initializer: vec![
                half_tensor(FIRST_CONV_WEIGHT, &[1.0, -2.0]),
                half_tensor("lut", &[0.0]),
            ],
            ..Default::default()
        };
        let mut lp = node("Loop", &["n", "cond"], &["out"]);
        lp.attribute.push(AttributeProto {
            name: "body".into(),
            g: Some(body),
            ..Default::default()
        });
        ModelProto {
            graph: Some(GraphProto {
                node: vec![node("Pad", &["audio"], &["padded"]), lp],
                input: vec![ValueInfoProto {
                    name: "audio".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            functions: vec![Default::default()],
            opset_import: vec![
                tract_onnx::pb::OperatorSetIdProto {
                    domain: String::new(),
                    version: 17,
                },
                tract_onnx::pb::OperatorSetIdProto {
                    domain: "birdbox.stream".into(),
                    version: 1,
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn the_cut_keeps_the_network_between_the_two_casts() {
        let mut p = demo_like();
        assert!(is_demo_graph(&p));
        cut_demo_graph(&mut p).unwrap();
        assert!(!is_demo_graph(&p));
        let g = p.graph.as_ref().unwrap();
        let ops: Vec<&str> = g.node.iter().map(|n| n.op_type.as_str()).collect();
        // Cast in, the network and its Constant, Cast out: no image making,
        // no NMS, no unused Constant.
        assert_eq!(ops, ["Cast", "Conv", "Constant", "Mul", "Cast"]);
        assert_eq!(g.node[0].input, ["images"]);
        assert_eq!(g.node[0].output, ["spec16"]);
        assert_eq!(g.node[4].input, ["head16"]);
        assert_eq!(g.node[4].output, ["output0"]);
        assert_eq!(g.input[0].name, "images");
        assert_eq!(g.output[0].name, "output0");
        // Only the network's weight; the image's colour table is left out.
        let names: Vec<&str> = g.initializer.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, [FIRST_CONV_WEIGHT]);
        // The Constant's float16 value became float32.
        let k = g.node[2].attribute[0].t.as_ref().unwrap();
        assert_eq!(k.data_type, ONNX_FLOAT);
        assert_eq!(k.raw_data, 0.5f32.to_le_bytes());
        assert!(p.functions.is_empty());
        assert_eq!(p.opset_import.len(), 1);
        // The patch then turns the weights and the first Cast to float32.
        let patched = patch_proto(&mut p);
        assert_eq!(
            patched,
            Patched {
                weights_to_f32: 1,
                casts_to_f32: 1
            }
        );
        let g = p.graph.as_ref().unwrap();
        assert_eq!(g.initializer[0].data_type, ONNX_FLOAT);
        assert_eq!(g.node[0].attribute[0].i, ONNX_FLOAT as i64);
    }

    #[test]
    fn the_cut_refuses_other_graphs() {
        let mut p = ModelProto {
            graph: Some(GraphProto {
                node: vec![node("Conv", &["images", "w"], &["y"])],
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(!is_demo_graph(&p));
        assert!(cut_demo_graph(&mut p).is_err());
        // A Loop whose body has no network.
        let mut p = demo_like();
        if let Some(g) = p.graph.as_mut() {
            if let Some(body) = g.node[1].attribute[0].g.as_mut() {
                body.node.retain(|n| n.op_type != "Conv");
            }
        }
        assert!(cut_demo_graph(&mut p).is_err());
    }
}
