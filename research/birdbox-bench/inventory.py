"""List what a BirdBox ONNX file is made of: opsets, inputs and outputs (with symbolic
dimensions), metadata, local functions, op types per graph level (main graph, each
subgraph of Loop/If/Scan, each local function), weight types and sizes.

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


def walk(graph, where, levels):
    ops = levels.setdefault(where, collections.Counter())
    for n in graph.node:
        ops[n.op_type if not n.domain else f"{n.domain}:{n.op_type}"] += 1
        for a in n.attribute:
            if a.type == onnx.AttributeProto.GRAPH:
                walk(a.g, f"{where} > {n.op_type}({n.name}).{a.name}", levels)
            elif a.type == onnx.AttributeProto.GRAPHS:
                for i, g in enumerate(a.graphs):
                    walk(g, f"{where} > {n.op_type}({n.name}).{a.name}[{i}]", levels)


for path in sys.argv[1:]:
    m = onnx.load(path, load_external_data=False)
    g = m.graph
    print(f"== {path}")
    print("  ir_version", m.ir_version, "opsets", [(o.domain or "ai.onnx", o.version) for o in m.opset_import])
    print("  producer", m.producer_name, m.producer_version)
    for p in m.metadata_props:
        v = p.value if len(p.value) < 300 else p.value[:300] + f"... ({len(p.value)} chars)"
        print(f"  meta {p.key} = {v}")
    for i in g.input:
        print("  input ", i.name, dims(i))
    for o in g.output:
        print("  output", o.name, dims(o))
    levels = {}
    walk(g, "main", levels)
    for f in m.functions:
        fl = {}
        fg = onnx.GraphProto(node=f.node)
        walk(fg, f"function {f.domain}:{f.name}({', '.join(f.input)}) -> {', '.join(f.output)}", fl)
        levels.update(fl)
    for where, ops in levels.items():
        print(f"  [{where}] {sum(ops.values())} nodes:", ", ".join(f"{k} {v}" for k, v in sorted(ops.items(), key=lambda kv: -kv[1])))
    wt, wbytes = collections.Counter(), collections.Counter()
    for t in g.initializer:
        k = DT.get(t.data_type, t.data_type)
        wt[k] += 1
        wbytes[k] += numpy_helper.to_array(t).nbytes
    print("  initializers:", {k: f"{wt[k]} tensors, {wbytes[k] / 1e6:.2f} MB" for k in wt})
    big = sorted(g.initializer, key=lambda t: -numpy_helper.to_array(t).nbytes)[:5]
    for t in big:
        a = numpy_helper.to_array(t)
        print(f"    largest: {t.name} {DT.get(t.data_type)} {list(a.shape)}")
    consts = collections.Counter()
    for n in g.node:
        if n.op_type == "Constant":
            for a in n.attribute:
                if a.name == "value":
                    consts[DT.get(a.t.data_type, a.t.data_type)] += 1
    if consts:
        print("  Constant nodes by type:", dict(consts))
