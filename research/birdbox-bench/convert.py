"""Write a float32 copy of the released float16 network, for onnxruntime checks only:
float16 weights become float32 and Cast(to=float16) becomes Cast(to=float32). The Rust
code makes the same change in memory (`patch_proto` in src/lib.rs), so the app would
never need this file.

  python -I convert.py Just-Bird_fp16.onnx OUT.onnx
"""

import sys

import numpy as np
import onnx
from onnx import TensorProto, numpy_helper

m = onnx.load(sys.argv[1])
g = m.graph
weights = casts = 0
for i, t in enumerate(g.initializer):
    if t.data_type == TensorProto.FLOAT16:
        g.initializer[i].CopyFrom(numpy_helper.from_array(numpy_helper.to_array(t).astype(np.float32), t.name))
        weights += 1
for n in g.node:
    if n.op_type == "Cast":
        for at in n.attribute:
            if at.name == "to" and at.i == TensorProto.FLOAT16:
                at.i = TensorProto.FLOAT
                casts += 1
    for at in n.attribute:
        if at.type == onnx.AttributeProto.TENSOR and at.t.data_type == TensorProto.FLOAT16:
            raise SystemExit(f"float16 constant in {n.name}: not handled")
# onnxruntime checks the declared types of intermediate values (tract is told to ignore them)
infos = 0
for vi in list(g.value_info) + list(g.input) + list(g.output):
    if vi.type.tensor_type.elem_type == TensorProto.FLOAT16:
        vi.type.tensor_type.elem_type = TensorProto.FLOAT
        infos += 1
print(f"{infos} declared value types changed to float32")
# No onnx.checker here: the released file itself fails it (its input Cast node comes
# after the first Conv in the node list), and onnxruntime and tract both accept that.
onnx.save(m, sys.argv[2])
print(f"{weights} weights and {casts} casts changed to float32")
