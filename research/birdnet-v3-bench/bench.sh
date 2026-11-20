#!/usr/bin/env bash
# Timing round: each model in tract natively and in wasm32 (node), taking turns with
# BirdNET V2.4 so every round sees the same machine load. Run from the folder that
# holds long.f32 (V3.0, 60 windows) and long48.f32 (V2.4, 60 windows).
#
#   [PERCH=perch_v2_embed.tract.onnx] bash bench.sh NATIVE_BIN PKG_NODE_DIR V24_MODEL.onnx ROUNDS MODEL.onnx [MODEL.onnx ...]
#
# With PERCH set (and long160.f32, 5 s windows at 32 kHz, in the folder), Perch's
# embedding model is timed too.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
native=$1 pkg=$2 v24=$3 rounds=$4
shift 4
for r in $(seq "$rounds"); do
  echo "## round $r, load average $(cut -d' ' -f1-3 /proc/loadavg)"
  echo "V2.4 native: $("$native" plain "$v24" long48.f32 144000)"
  echo "V2.4 wasm:   $(node "$here/bench-v24.mjs" "$pkg" "$v24" long48.f32)"
  if [ -n "${PERCH:-}" ]; then
    echo "Perch embedding native: $("$native" plain "$PERCH" long160.f32 160000)"
    echo "Perch embedding wasm:   $(node "$here/bench-v24.mjs" "$pkg" "$PERCH" long160.f32 160000)"
  fi
  for m in "$@"; do
    echo "$(basename "$m") native: $("$native" "$m" long.f32 | tr '\n' ' ')"
    echo "$(basename "$m") wasm:   $(node "$here/bench-bindgen.mjs" "$pkg" "$m" long.f32)"
  done
done
