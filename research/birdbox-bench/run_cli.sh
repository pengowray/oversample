#!/usr/bin/env bash
# Reference 1: BirdBox's own CLI (src/inference/detect_birds.py at tag v2.1.0) with the
# released Just-Bird_fp16.onnx, default settings (conf 0.15, NMS IoU 0.7, window fusion,
# phrase gap 0.05 s), once merged and once with --no-merge (the per-window boxes).
#
#   bash run_cli.sh VENV_PYTHON BIRDBOX_REPO MODEL.onnx AUDIO_DIR OUT_DIR
set -eu
py=$1 repo=$2 model=$3 audio=$4 out=$5
mkdir -p "$out"
for f in "$audio"/*.wav; do
  n=$(basename "$f" .wav)
  for mode in merged raw; do
    extra=()
    [ "$mode" = raw ] && extra=(--no-merge)
    rm -rf "$out/$n.$mode"
    "$py" -I "$repo/src/inference/detect_birds.py" --audio "$f" --model "$model" \
      --species-mapping Just-Bird --output-path "$out/$n.$mode" \
      --output-format json-with-algorithm-metadata --num-workers 1 "${extra[@]}" >"$out/$n.$mode.log" 2>&1
    # merged: with_algorithm_metadata.json; --no-merge: raw_detections.json
    echo "$n $mode: $(grep -c '"time_start"' "$out/$n.$mode"/*.json) boxes"
  done
done
