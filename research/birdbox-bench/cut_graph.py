"""Cut the bare network out of BirdBox's all-in-one demo graph
(docs/models/Just-Bird_fp16.onnx at v2.1.0), the copy that is served with CORS headers.

The network sits inside the graph's ONNX Loop body: from the Cast(to=float16) that
takes the rendered image to the Cast(to=float32) after the head. This finds those two
values, keeps the body nodes between them and the initializers they use, and writes a
graph with one input `images` [1, 3, 256, 256] and one output `output0` [1, 5, 1344],
the same interface as the release file. Python here to check the idea against
onnxruntime; the Rust side does the same in memory (`cut_demo_graph` in
src/network.rs).

  python -I cut_graph.py DEMO_GRAPH.onnx OUT.onnx
"""

import sys

import onnx
from onnx import TensorProto, helper

m = onnx.load(sys.argv[1])
loop = next(n for n in m.graph.node if n.op_type == "Loop")
body = next(a.g for a in loop.attribute if a.name == "body")
first_w = "b_core.network.model.0.conv.weight"
conv0 = next(n for n in body.node if n.op_type == "Conv" and first_w in n.input)
# The network input: the value conv0 reads (after the Cast to float16)
x_in = conv0.input[0]
producer = {o: n for n in body.node for o in n.output}
cast_in = producer[x_in]
print("first conv reads", x_in, "made by", cast_in.op_type, [a.i for a in cast_in.attribute], "from", cast_in.input)
# The network output: the Concat of box and score rows whose consumer casts to float32.
consumers = {}
for n in body.node:
    for i in n.input:
        consumers.setdefault(i, []).append(n)
# Walk forward from conv0 over nodes that touch network weights; the last value is the head.
net_nodes = set()
frontier = [x_in]
seen = set()
while frontier:
    v = frontier.pop()
    for n in consumers.get(v, []):
        if id(n) in seen:
            continue
        seen.add(id(n))
        if n.op_type == "Cast" and any(a.name == "to" and a.i == TensorProto.FLOAT for a in n.attribute):
            print("network ends at", n.input[0], "-> Cast(float32) ->", n.output[0])
            out_val = n.input[0]
            continue
        net_nodes.add(id(n))
        frontier.extend(n.output)
nodes = [n for n in body.node if id(n) in net_nodes]
used = {i for n in nodes for i in n.input}
inits = [t for t in body.initializer if t.name in used]
# Constants defined outside the kept nodes but used by them (Constant nodes in the body)
const_nodes = [n for n in body.node if n.op_type == "Constant" and n.output[0] in used and id(n) not in net_nodes]
print(f"{len(nodes)} network nodes, {len(const_nodes)} constants, {len(inits)} initializers")
g = helper.make_graph(
    [helper.make_node("Cast", ["images"], [x_in], to=TensorProto.FLOAT16)] + const_nodes + nodes,
    "just_bird_network",
    [helper.make_tensor_value_info("images", TensorProto.FLOAT, [1, 3, 256, 256])],
    [helper.make_tensor_value_info("output0_f16", TensorProto.FLOAT16, None)],
    inits,
)
g.node.append(helper.make_node("Cast", [out_val], ["output0"], to=TensorProto.FLOAT))
g.output[0].CopyFrom(helper.make_tensor_value_info("output0", TensorProto.FLOAT, [1, 5, 1344]))
cut = helper.make_model(g, opset_imports=[o for o in m.opset_import if o.domain in ("", "ai.onnx")])
cut.ir_version = m.ir_version
onnx.checker.check_model(cut)
onnx.save(cut, sys.argv[2])
print("wrote", sys.argv[2])
