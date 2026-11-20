#!/usr/bin/env bash
# Every comparison in the README, stage by stage. Writes to stdout.
#
#   bash compare_all.sh PYTHON WORK_DIR > results/compare.txt
#
# WORK_DIR holds ref/ (ref_images.py, ref_graph.py, cli_tsv.py output), rust/
# (`native detect`, `native fuse`, bench-bindgen.mjs output) and stage/ (`native decode`
# and `native fuse` on the reference's own inputs).
set -u
here=$(cd "$(dirname "$0")" && pwd)
py=$1 work=$2
W=$work/ref R=$work/rust S=$work/stage
P() { "$py" -I "$here/compare.py" "$@"; }
L="bat canary gentoo pauraque rain soundscape speech hawaii"

echo "# BirdBox Just-Bird spike: comparisons. Reference = BirdBox CLI v2.1.0 with the release"
echo "# Just-Bird_fp16.onnx in onnxruntime 1.27, conf 0.15, NMS IoU 0.7, window fusion, phrase gap 0.05 s."
echo; echo "## 0. The reference against itself: demo graph (onnxruntime) vs CLI"
echo "### window boxes"; P boxes raw "$W/{name}.cli.raw.tsv" "$W/{name}.graph.raw.tsv" $L
echo "### merged boxes"; P boxes merged "$W/{name}.cli.boxes.tsv" "$W/{name}.graph.boxes.tsv" $L
echo; echo "## 1. Image: CLI PNG (cv2, RGB, /255) vs Rust"
P images "$W/{name}.images.f32" "$R/{name}.images.f32" $L
echo; echo "## 2. Network alone, on the CLI's images"
echo "### tract (release fp16, float32 in memory) vs onnxruntime on a float32 copy"; P heads "$W/{name}.head.ort32.f32" "$W/{name}.head.tract16.f32" $L
echo "### tract vs onnxruntime on the release fp16 file (what the CLI runs)"; P heads "$W/{name}.head.ort16.f32" "$W/{name}.head.tract16.f32" $L
echo "### tract, network cut from the demo graph in memory, vs tract, release file"; P heads "$W/{name}.head.tract16.f32" "$W/{name}.head.tractcut.f32" canary
echo "### tract int8 vs onnxruntime int8"; P heads "$W/{name}.head.ortint8.f32" "$W/{name}.head.tractint8.f32" canary
echo "### onnxruntime int8 vs onnxruntime fp16"; P heads "$W/{name}.head.ort16.f32" "$W/{name}.head.ortint8.f32" canary gentoo soundscape
echo; echo "## 3. Rust NMS on onnxruntime's fp16 heads vs CLI window boxes"
P boxes raw "$W/{name}.cli.raw.tsv" "$S/{name}.dec.raw.tsv" $L
echo; echo "## 4. Rust fusion (gap 0.05) on the CLI's window boxes vs CLI merged boxes"
P boxes merged "$W/{name}.cli.boxes.tsv" "$S/{name}.fuse.boxes.tsv" $L
echo; echo "## 5. End to end: Rust image + tract + Rust NMS and fusion vs CLI"
echo "### effect of the image differences: tract on Rust images vs tract on CLI images"; P heads "$W/{name}.head.tract16.f32" "$R/{name}.head.f32" $L
echo "### window boxes"; P boxes raw "$W/{name}.cli.raw.tsv" "$R/{name}.raw.tsv" $L --per-file
echo "### merged boxes, phrase gap 0.05 (default)"; P boxes merged "$W/{name}.cli.boxes.tsv" "$R/{name}.boxes.tsv" $L --per-file
echo "### merged boxes, phrase gap 0 (call level), vs the demo graph at gap 0"; P boxes merged "$W/{name}.graph0.boxes.tsv" "$R/{name}.gap0.boxes.tsv" $L
echo; echo "## 6. soundscape only, merged boxes vs CLI: WASM in node, and the demo-graph cut (native and WASM)"
for t in wasm wasmcut cut; do echo "### $t"; P boxes merged "$W/{name}.cli.boxes.tsv" "$R/{name}.$t.boxes.tsv" soundscape; done
