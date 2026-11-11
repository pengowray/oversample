"""Make a Perch v2 ONNX file loadable by tract 0.23 with batch size 1.

With a fixed batch of 1, tract's optimiser removes the batch axis from each Conv and
then folds the Transpose -> Mul -> Add (batch-norm scale and shift) that follows it
into the Conv bias with the wrong shape:

    Bias should be scalar or a vector with one value per output channel.
    Output channels is 40, bias is 40,1,F32

Batch 2 loads fine. This script does the fold in the ONNX file instead, so tract has
nothing left to fold:

    Conv(x, W, b) -> Transpose(NCHW->NHWC) -> Mul(s) -> Add(t)
    becomes
    Conv(x, W * s, b * s + t) -> Transpose

The result is the same function up to float rounding.

  python fix_for_tract.py in.onnx out.onnx
"""

import sys

import numpy as np
import onnx
from onnx import numpy_helper

src, dst = sys.argv[1], sys.argv[2]
m = onnx.load(src)
g = m.graph
init = {t.name: t for t in g.initializer}
outputs = {o.name for o in g.output}


def consumers(name):
    return [n for n in g.node if name in n.input]


def const_vec(name, c):
    """Initializer `name` as a length-c vector if it is per-channel on the last axis."""
    t = init.get(name)
    if t is None:
        return None
    a = numpy_helper.to_array(t)
    if a.size == c and (a.ndim == 1 or all(d == 1 for d in a.shape[:-1])):
        return a.reshape(c).astype(np.float32)
    if a.size == 1:
        return np.full(c, a.reshape(()), np.float32)
    return None


def other_input(node, name):
    return node.input[1] if node.input[0] == name else node.input[0]


folded = 0
remove = []
for conv in list(g.node):
    if conv.op_type != "Conv" or len(conv.input) < 3:
        continue
    W = numpy_helper.to_array(init[conv.input[1]]).astype(np.float32)
    c = W.shape[0]
    b = numpy_helper.to_array(init[conv.input[2]]).astype(np.float32).reshape(c)
    cs = consumers(conv.output[0])
    if len(cs) != 1 or cs[0].op_type != "Transpose" or conv.output[0] in outputs:
        continue
    tr = cs[0]
    perm = [a.ints for a in tr.attribute if a.name == "perm"][0]
    if list(perm) != [0, 2, 3, 1]:
        continue
    cur = tr.output[0]
    scale, shift, chain = np.ones(c, np.float32), np.zeros(c, np.float32), []
    while True:
        nxt = consumers(cur)
        if len(nxt) != 1 or cur in outputs:
            break
        n = nxt[0]
        if n.op_type not in ("Mul", "Add"):
            break
        v = const_vec(other_input(n, cur), c)
        if v is None:
            break
        if n.op_type == "Mul":
            scale, shift = scale * v, shift * v
        else:
            shift = shift + v
        chain.append(n)
        cur = n.output[0]
    if not chain:
        continue
    W2 = W * scale[:, None, None, None]
    b2 = b * scale + shift
    for name, arr in ((conv.input[1], W2), (conv.input[2], b2)):
        new = numpy_helper.from_array(arr, name + f"_fold{folded}")
        g.initializer.append(new)
    conv.input[1] = conv.input[1] + f"_fold{folded}"
    conv.input[2] = conv.input[2] + f"_fold{folded}"
    # Consumers of the end of the chain now read the Transpose output.
    last = chain[-1].output[0]
    for n in g.node:
        for i, name in enumerate(n.input):
            if name == last:
                n.input[i] = tr.output[0]
    remove.extend(chain)
    folded += 1

for n in remove:
    g.node.remove(n)
# Drop initializers nothing uses any more.
used = {i for n in g.node for i in n.input}
keep = [t for t in g.initializer if t.name in used]
del g.initializer[:]
g.initializer.extend(keep)
del g.value_info[:]
onnx.save(m, dst)
print(f"{dst}: folded scale/shift into {folded} Convs, removed {len(remove)} nodes")
