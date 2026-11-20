#!/usr/bin/env bash
# Does a browser fetch of each URL work? Prints each hop's status and its
# Access-Control-Allow-Origin header for a ranged GET with an Origin header (a browser
# needs that header on every hop of a redirect, or it blocks the response).
#
#   bash cors.sh
set -u
urls=(
  # The release asset (the bare network the spike runs)
  "https://github.com/birdnet-team/BirdBox/releases/download/v2.1.0/Just-Bird_fp16.onnx"
  "https://github.com/birdnet-team/BirdBox/releases/download/v2.1.0/Just-Bird_int8.onnx"
  # Copies of the all-in-one demo graph (docs/models/Just-Bird_fp16.onnx)
  "https://birdnet-team.github.io/BirdBox/models/Just-Bird_fp16.onnx"
  "https://raw.githubusercontent.com/birdnet-team/BirdBox/v2.1.0/docs/models/Just-Bird_fp16.onnx"
  "https://cdn.jsdelivr.net/gh/birdnet-team/BirdBox@v2.1.0/docs/models/Just-Bird_fp16.onnx"
)
for u in "${urls[@]}"; do
  echo "== $u"
  curl -sS -L -o /dev/null -D - -H "Origin: https://example.org" -H "Range: bytes=0-99" "$u" |
    grep -i -E "^HTTP|^access-control-allow-origin|^content-range" | tr -d '\r'
done
