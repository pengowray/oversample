"""Count multiply-adds per 3 s window in a shape-fixed BirdNET+ V3.0 ONNX file
(convert.py --shape), split into the spectrogram, the backbone (up to the `embeddings`
tensor) and the classifier heads, and list the heads' weights.

  python -I macs.py full.fp32.sim.onnx
"""

import sys

import numpy as np
import onnx
from onnx import numpy_helper, shape_inference

m = shape_inference.infer_shapes(onnx.load(sys.argv[1]))
g = m.graph
shapes = {}
for vi in list(g.value_info) + list(g.input) + list(g.output):
    shapes[vi.name] = [d.dim_value for d in vi.type.tensor_type.shape.dim]
init = {t.name: t for t in g.initializer}
for name, t in init.items():
    shapes[name] = list(t.dims)

# Nodes the embeddings depend on are spectrogram + backbone; the rest are heads.
prod = {o: n for n in g.node for o in n.output}
need, stack = set(), ["embeddings"]
while stack:
    nm = stack.pop()
    n = prod.get(nm)
    if n is None or id(n) in need:
        continue
    need.add(id(n))
    stack.extend(i for i in n.input if i)

tot = {"spectrogram": 0, "backbone": 0, "heads": 0}
head_w = []
first_conv = None
for n in g.node:
    out = shapes.get(n.output[0])
    if n.op_type == "Conv":
        w = shapes[n.input[1]]
        macs = int(np.prod(out)) * int(np.prod(w[1:]))
    elif n.op_type in ("MatMul", "Gemm"):
        a, b = shapes.get(n.input[0]), shapes.get(n.input[1])
        k = a[-1] if not (n.op_type == "Gemm" and any(at.name == "transA" and at.i for at in n.attribute)) else a[-2]
        macs = int(np.prod(out)) * k
    else:
        continue
    if id(n) in need:
        if first_conv is None and n.op_type == "Conv":
            first_conv = n.name
        part = "backbone" if first_conv else "spectrogram"
    else:
        part = "heads"
    tot[part] += macs
    if part == "heads" or n.op_type == "Gemm":
        ws = [i for i in n.input[1:] if i in init]
        head_w += [(i, shapes[i]) for i in ws]
print({k: f"{v / 1e9:.3f} GMAC" for k, v in tot.items()}, f"total {sum(tot.values()) / 1e9:.3f} GMAC")
emb_src = prod["embeddings"]
print("feature map before pooling:", [shapes.get(i) for n in g.node if n.op_type == "Conv" and "conv_head" in "".join(n.input) for i in n.output])
params = 0
for name, s in head_w:
    params += int(np.prod(s))
    print(f"  head weight {name}: {s}")
print(f"head weights: {params / 1e6:.1f} M values (float16 {params * 2 / 1e6:.0f} MB, int8 {params / 1e6:.0f} MB)")
bb = sum(int(np.prod(t.dims)) for t in g.initializer) - params
print(f"everything else: {bb / 1e6:.1f} M values (float16 {bb * 2 / 1e6:.0f} MB, float32 {bb * 4 / 1e6:.0f} MB)")
