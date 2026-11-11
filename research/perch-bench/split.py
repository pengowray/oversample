"""Split Perch v2 ONNX into an embedding model and a plain classifier head.

The Perch v2 export has one classifier: a prototype head (ProtoPNet) with 4 prototypes
per class, applied to the spatial embedding (16 x 4 positions of 1536 values):

    s = spatial_embedding / (L2 norm per position + 1e-5)
    p = s @ P                        P: [1536, 14795 * 4]
    m = max over the 64 positions    -> [14795, 4]
    label = sum(m * W, axis=prototype) + b      W: [14795, 4], b: [14795]

This script writes:
  OUT/perch_v2_embed.onnx   the graph up to `embedding` and `spatial_embedding`
  OUT/perch_v2_head.npz     P, W, b as float32

  python split.py perch_v2.onnx OUT_DIR
"""

import os
import sys

import numpy as np
import onnx
from onnx import numpy_helper

src, out = sys.argv[1], sys.argv[2]
m = onnx.load(src)
init = {t.name: t for t in m.graph.initializer}

P = W = b = None
for n in m.graph.node:
    if "heads_protopnet" not in n.name and not any("heads_protopnet" in i for i in n.input):
        continue
    for i in n.input:
        if i not in init:
            continue
        a = numpy_helper.to_array(init[i])
        if n.op_type == "MatMul" and a.ndim == 2 and a.shape[0] == 1536:
            P = a
        elif n.op_type == "Mul" and a.shape[-1] == 4 and a.ndim == 3:
            W = a.reshape(-1, 4)
        elif n.op_type == "Add" and n.output[0] == "label":
            b = a.reshape(-1)
assert P is not None and W is not None and b is not None, (P is None, W is None, b is None)
np.savez(os.path.join(out, "perch_v2_head.npz"), P=P.astype(np.float32), W=W.astype(np.float32), b=b.astype(np.float32))
print("head", P.shape, W.shape, b.shape)

# Keep only the embedding outputs and drop every node they don't need.
keep = {"embedding", "spatial_embedding"}
outs = [o for o in m.graph.output if o.name in keep]
del m.graph.output[:]
m.graph.output.extend(outs)
needed = set(keep)
nodes = []
for n in reversed(m.graph.node):
    if set(n.output) & needed:
        nodes.append(n)
        needed.update(n.input)
nodes.reverse()
del m.graph.node[:]
m.graph.node.extend(nodes)
inits = [t for t in m.graph.initializer if t.name in needed]
del m.graph.initializer[:]
m.graph.initializer.extend(inits)
vi = [v for v in m.graph.value_info if v.name in needed]
del m.graph.value_info[:]
m.graph.value_info.extend(vi)
dst = os.path.join(out, os.path.basename(src).replace(".onnx", "_embed.onnx"))
onnx.save(m, dst)
print(dst, os.path.getsize(dst) / 1e6, "MB,", len(nodes), "nodes")
