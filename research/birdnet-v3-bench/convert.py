"""Rewrite a BirdNET+ V3.0 FP16 ONNX file (release 3.0.1, mixed precision) so that every
operation runs in float32.

The 3.0.1 FP16 files store most weights as float16 and wrap each convolution in
Cast(to=float16) ... Cast(to=float32). This script:

1. Turns every Cast(to=float16) into a Cast(to=float32), so all activations stay float32.
2. Either converts every float16 weight to float32 (--weights fp32, the default), or
   keeps it float16 and puts one Cast(to=float32) after it (--weights fp16), which keeps
   the file about half the size; the runtime converts the weights once when it loads.
3. Converts float16 Constant nodes the same way.
4. With --shape, fixes the input at [1, 96000] (3 s at 32 kHz) and runs onnx-simplifier,
   which folds the shape arithmetic (Shape, Range, ConstantOfShape, Expand) into
   constants. onnx-simplifier also folds the weight Casts back into float32 weights, so
   --shape and --weights fp16 don't mix; use --shape on its own.
5. Gives every ConstantOfShape without a value an explicit float32 zero (tract 0.23
   otherwise makes float64 zeros there; see the README).
6. With --outputs, keeps only the named outputs (for example `embeddings` for the
   backbone alone).

  python -I convert.py in.onnx out.onnx [--weights fp32|fp16|asis] [--shape] [--outputs embeddings]
"""

import argparse

import numpy as np
import onnx
from onnx import TensorProto, helper, numpy_helper

ap = argparse.ArgumentParser()
ap.add_argument("src")
ap.add_argument("dst")
ap.add_argument(
    "--weights",
    choices=["fp32", "fp16", "asis"],
    default="fp32",
    help="asis: leave the float16 casts and weights alone (only the ConstantOfShape fix)",
)
ap.add_argument("--shape", action="store_true", help="fix the input at [1, 96000] and simplify")
ap.add_argument("--outputs", nargs="+", default=None)
a = ap.parse_args()

m = onnx.load(a.src)
g = m.graph

casts, zeros_fixed = 0, 0
for n in g.node:
    if n.op_type == "Cast" and a.weights != "asis":
        for at in n.attribute:
            if at.name == "to" and at.i == TensorProto.FLOAT16:
                at.i = TensorProto.FLOAT
                casts += 1
    if n.op_type == "Constant" and a.weights != "asis":
        for at in n.attribute:
            if at.name == "value" and at.t.data_type == TensorProto.FLOAT16:
                arr = numpy_helper.to_array(at.t).astype(np.float32)
                at.t.CopyFrom(numpy_helper.from_array(arr, at.t.name))
    if n.op_type == "ConstantOfShape":
        values = [at for at in n.attribute if at.name == "value"]
        if not values:
            # ONNX says a missing value means float32 zeros, but tract 0.23 makes f64
            # zeros, and the Add that follows turns the class scores into f64.
            n.attribute.append(
                helper.make_attribute("value", numpy_helper.from_array(np.zeros(1, np.float32)))
            )
            zeros_fixed += 1
        for at in values if a.weights != "asis" else []:
            if at.t.data_type == TensorProto.FLOAT16:
                arr = numpy_helper.to_array(at.t).astype(np.float32)
                at.t.CopyFrom(numpy_helper.from_array(arr, at.t.name))

converted, wrapped = 0, 0
new_nodes = []
for i, t in enumerate(g.initializer):
    if t.data_type != TensorProto.FLOAT16 or a.weights == "asis":
        continue
    arr = numpy_helper.to_array(t)
    if a.weights == "fp32":
        g.initializer[i].CopyFrom(numpy_helper.from_array(arr.astype(np.float32), t.name))
        converted += 1
    else:
        # Keep the float16 bytes under a new name; a Cast gives the old name as float32.
        old = t.name
        g.initializer[i].name = old + "__f16"
        new_nodes.append(
            helper.make_node("Cast", [old + "__f16"], [old], to=TensorProto.FLOAT, name=old + "__to_f32")
        )
        wrapped += 1
if new_nodes:
    for n in reversed(new_nodes):
        g.node.insert(0, n)

if a.outputs:
    keep = [o for o in g.output if o.name in a.outputs]
    missing = set(a.outputs) - {o.name for o in keep}
    if missing:
        raise SystemExit(f"no outputs named {missing}")
    del g.output[:]
    g.output.extend(keep)

del g.value_info[:]

if a.outputs:
    # Drop nodes nothing reaches from the kept outputs.
    prod = {o: n for n in g.node for o in n.output}
    need, stack = set(), [o.name for o in g.output]
    while stack:
        name = stack.pop()
        n = prod.get(name)
        if n is None or id(n) in need:
            continue
        need.add(id(n))
        stack.extend(i for i in n.input if i)
    keep_nodes = [n for n in g.node if id(n) in need]
    del g.node[:]
    g.node.extend(keep_nodes)
    used = {i for n in g.node for i in n.input}
    keep_init = [t for t in g.initializer if t.name in used]
    del g.initializer[:]
    g.initializer.extend(keep_init)

if a.shape:
    from onnxsim import simplify

    m, ok = simplify(m, overwrite_input_shapes={g.input[0].name: [1, 96000]})
    if not ok:
        raise SystemExit("onnx-simplifier could not check the simplified model")

onnx.save(m, a.dst)
ops = sorted({n.op_type for n in m.graph.node})
print(
    f"{a.dst}: {casts} casts to float16 now float32, {zeros_fixed} ConstantOfShape given a "
    f"float32 value, {converted} weights converted, "
    f"{wrapped} weights kept float16 behind a Cast; {len(m.graph.node)} nodes; ops: {', '.join(ops)}"
)
