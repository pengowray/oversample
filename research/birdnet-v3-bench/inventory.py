"""List what an ONNX file is made of: opset, inputs and outputs (with any symbolic
dimensions), op types and counts, weight data types and sizes, and Cast targets.

  python -I inventory.py MODEL.onnx [MODEL2.onnx ...]
"""

import collections
import sys

import onnx
from onnx import TensorProto, numpy_helper

DT = {v: k for k, v in TensorProto.DataType.items()}


def dims(vi):
    t = vi.type.tensor_type
    out = []
    for d in t.shape.dim:
        out.append(d.dim_param if d.dim_param else d.dim_value)
    return f"{DT.get(t.elem_type, t.elem_type)}{out}"


def walk(graph, ops, casts, sub):
    for n in graph.node:
        ops[n.op_type] += 1
        if n.op_type == "Cast":
            to = [a.i for a in n.attribute if a.name == "to"][0]
            casts[DT.get(to, to)] += 1
        for a in n.attribute:
            if a.type == onnx.AttributeProto.GRAPH:
                sub.append(n.op_type)
                walk(a.g, ops, casts, sub)


for path in sys.argv[1:]:
    m = onnx.load(path, load_external_data=False)
    g = m.graph
    print(f"== {path}")
    print("  ir_version", m.ir_version, "opsets", [(o.domain or "ai.onnx", o.version) for o in m.opset_import])
    print("  producer", m.producer_name, m.producer_version)
    for i in g.input:
        print("  input ", i.name, dims(i))
    for o in g.output:
        print("  output", o.name, dims(o))
    ops, casts, sub = collections.Counter(), collections.Counter(), []
    walk(g, ops, casts, sub)
    print(f"  {sum(ops.values())} nodes, {len(ops)} op types")
    print("  ops:", ", ".join(f"{k} {v}" for k, v in sorted(ops.items(), key=lambda kv: -kv[1])))
    if casts:
        print("  Cast to:", dict(casts))
    if sub:
        print("  ops with subgraphs:", collections.Counter(sub))
    wt = collections.Counter()
    wbytes = collections.Counter()
    for t in g.initializer:
        wt[DT.get(t.data_type, t.data_type)] += 1
        wbytes[DT.get(t.data_type, t.data_type)] += numpy_helper.to_array(t).nbytes
    print("  initializers:", {k: f"{wt[k]} tensors, {wbytes[k] / 1e6:.1f} MB" for k in wt})
    consts = collections.Counter()
    for n in g.node:
        if n.op_type == "Constant":
            for a in n.attribute:
                if a.name == "value":
                    consts[DT.get(a.t.data_type, a.t.data_type)] += 1
    if consts:
        print("  Constant nodes by type:", dict(consts))
