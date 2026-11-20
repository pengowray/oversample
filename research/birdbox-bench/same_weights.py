"""Check that the network inside the demo graph (docs/models/Just-Bird_fp16.onnx in the
BirdBox repository) has the same weights as the released network file
(Just-Bird_fp16.onnx, v2.1.0 release asset).

  python -I same_weights.py RELEASE.onnx DEMO_GRAPH.onnx
"""

import sys

import numpy as np
import onnx
from onnx import numpy_helper


def initializers(graph, out):
    for t in graph.initializer:
        out[t.name] = numpy_helper.to_array(t)
    for n in graph.node:
        for a in n.attribute:
            if a.type == onnx.AttributeProto.GRAPH:
                initializers(a.g, out)
    return out


rel = initializers(onnx.load(sys.argv[1]).graph, {})
demo = initializers(onnx.load(sys.argv[2]).graph, {})
prefix = "b_core.network."
demo_net = {k[len(prefix):]: v for k, v in demo.items() if k.startswith(prefix)}
rel_net = {k: v for k, v in rel.items() if k.startswith("model.")}
both = sorted(set(rel_net) & set(demo_net))
print(f"release network tensors {len(rel_net)}, demo graph network tensors {len(demo_net)}, shared names {len(both)}")
print("only in release:", sorted(set(rel_net) - set(demo_net))[:10])
print("only in demo:", sorted(set(demo_net) - set(rel_net))[:10])
worst = 0.0
identical = 0
for k in both:
    a, b = rel_net[k], demo_net[k]
    if a.shape != b.shape:
        print("shape differs", k, a.shape, b.shape)
        continue
    if a.dtype == b.dtype and np.array_equal(a, b):
        identical += 1
    d = float(np.max(np.abs(a.astype(np.float64) - b.astype(np.float64)))) if a.size else 0.0
    worst = max(worst, d)
print(f"bit-identical {identical} of {len(both)}; largest difference {worst:g}")
