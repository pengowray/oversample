# perch-bench

Throwaway speed, accuracy, bat and species-map tests for Perch 2.0 (Google's
`perch_v2`) in Oversample. Findings are in [../perch.md](../perch.md). Copied from
[../birdnet-bench](../birdnet-bench) and changed for Perch. Nothing here is part of the
app build: this folder has its own `[workspace]`.

No model files are committed. Download them into a folder of your own (for example
`~/perch-models`; Apache 2.0 unless noted):

| File | Source |
|---|---|
| `perch_v2.onnx`, `perch_v2_no_dft.onnx`, `perch_v2.tflite` | `huggingface.co/justinchuby/Perch-onnx` |
| `labels.csv`, `perch_v2_ebird_classes.csv` | `huggingface.co/cgeorgiaw/Perch` (`assets/`), same as the Kaggle files |
| `perch_v2_central-europe_no_dft_fp32.onnx`, `central-europe_indices.txt` | `huggingface.co/tphakala/Perch-v2-Models` (`regional/central-europe/`) |
| `BirdNET_v2.4_fp32_foldconv.onnx`, `birdnet_labels.txt` (renamed from `labels.txt`) | `huggingface.co/tphakala/BirdNET-v2.4` (CC BY-NC-SA 4.0) |
| `birdnet-v2.4-embeddings-fp32.onnx` | `huggingface.co/tphakala/BattyBirdNET-onnx` (CC BY-NC-SA 4.0) |

`ref_tf.py` downloads Google's `perch_v2_cpu` SavedModel itself, through kagglehub.

## Steps

Python 3.12: `pip install onnx onnxruntime numpy ai-edge-litert soundfile scipy
scikit-learn umap-learn matplotlib`. For `ref_tf.py`, a separate environment with
`pip install "tensorflow-cpu~=2.20.0" kagglehub`.

Keep `CARGO_TARGET_DIR` outside this folder, since it is synced; the targets below
take several GB.

```bash
M=~/perch-models
# 1. Embedding model + classifier prototypes, and tract-loadable copies
python -I split.py $M/perch_v2.onnx $M          # perch_v2_embed.onnx, perch_v2_head.npz
for m in perch_v2 perch_v2_embed perch_v2_central-europe_no_dft_fp32; do
  python -I fix_for_tract.py $M/$m.onnx $M/$m.tract.onnx
done

# 2. Audio to 5 s windows of 32 kHz float32
python -I prep.py birds.f32 bird1.wav bird2.wav
python -I prep.py --repeat 36 long.f32 bird1.wav                       # 3 minutes
python -I prep.py --rate 48000 --win 144000 --repeat 60 long48.f32 bird1.wav   # same, for BirdNET

# 3. References: Google's SavedModel (TF), TFLite, onnxruntime, numpy classifier
python -I ref_tf.py birds.f32 out/birds.tf
python -I ref.py $M birds.f32 --save out/birds

# 4. tract native
cargo build --release
$CARGO_TARGET_DIR/release/native $M/perch_v2.tract.onnx birds.f32 1 out/birds.tract
$CARGO_TARGET_DIR/release/native $M/perch_v2_central-europe_no_dft_fp32.tract.onnx birds.f32 1 out/birds.ce

# 5. tract wasm32-wasip1 + SIMD in node (reports WASM memory)
RUSTFLAGS="-C target-feature=+simd128" cargo build --release --lib --target wasm32-wasip1
node bench.mjs $CARGO_TARGET_DIR/wasm32-wasip1/release/perch_bench.wasm $M/perch_v2.tract.onnx birds.f32 out/birds.wasm

# 6. tract wasm32-unknown-unknown + wasm-bindgen (the app's target)
CARGO_PROFILE_RELEASE_DEBUG=false CARGO_PROFILE_RELEASE_STRIP=true RUSTFLAGS="-C target-feature=+simd128" \
  wasm-pack build --release --target nodejs --out-dir $CARGO_TARGET_DIR/pkg-node --no-typescript
node bench-bindgen.mjs $CARGO_TARGET_DIR/pkg-node $M/perch_v2.tract.onnx birds.f32 out/birds.bindgen

# 7. Perch and BirdNET taking turns in one process (same machine load)
node both.mjs $CARGO_TARGET_DIR/pkg-node $M/perch_v2_embed.tract.onnx long.f32 \
  ../birdnet-bench/web/pkg $M/BirdNET_v2.4_fp32_foldconv.onnx long48.f32 3

# 8. Compare any run with the TF reference
python -I compare.py out/birds.tf out/birds.tract out/birds.wasm out/birds.bindgen
python -I compare.py out/birds.tf out/birds.ce --indices $M/central-europe_indices.txt

# 9. Bats: Perch vs BirdNET embeddings on bat-demo-sounds
python -I bats.py $M ../../../batchi-extras/bat-demo-sounds/sounds out/bats.npz

# 10. Species map (JSON + PNG) and the class-vector checks
python -I species_map.py $M out out/birds.tf

# 11. Placing detections on the map: embeddings against class vectors
python -I placement.py $M birds.f32

# 12. The classifier stored in float16 and int8: same answers?
python -I head_precision.py $M birds.f32

# 13. Embedding model in tract + int8 classifier in plain Rust (src/head.rs)
python -I export_head.py $M                     # perch_v2_head.int8.bin, 91 MB
$CARGO_TARGET_DIR/release/native $M/perch_v2_embed.tract.onnx birds.f32 1 out/birds.head $M/perch_v2_head.int8.bin
node bench-head.mjs $CARGO_TARGET_DIR/pkg-node $M/perch_v2_embed.tract.onnx $M/perch_v2_head.int8.bin birds.f32 out/birds.bindgen.head
python -I compare.py out/birds.tf out/birds.head out/birds.bindgen.head

# 14. The same on frogs, grasshoppers and mammals, by group (reference: onnxruntime)
python -I prep.py --max-windows 6 --list nobat.list nobat.f32 ../../../batchi-extras/no-bat-sounds/sounds/*.wav
python -I ref.py $M nobat.f32 --only ort_full --save out/nobat
$CARGO_TARGET_DIR/release/native $M/perch_v2_embed.tract.onnx nobat.f32 1 out/nobat.head $M/perch_v2_head.int8.bin
python -I recheck.py $M nobat.list out/nobat.ort_full out/nobat.head

# 15. Sound events: rain, traffic, wind, voices from Wikimedia Commons (kept in
#     batchi-extras/sound-event-sounds), converted with ffmpeg since libsndfile can't
#     read some of the Ogg files
E=../../../batchi-extras/sound-event-sounds
python -I fetch_commons.py $E
mkdir -p events && for f in $E/sounds/*; do ffmpeg -nostdin -i "$f" -ac 1 -ar 32000 "events/$(basename "${f%.*}").wav"; done
python -I prep.py --max-windows 6 --list events.list events.f32 events/*.wav
python -I ref.py $M events.f32 --only ort_full --save out/events
$CARGO_TARGET_DIR/release/native $M/perch_v2_embed.tract.onnx events.f32 1 out/events.head $M/perch_v2_head.int8.bin
python -I recheck.py $M events.list out/events.ort_full out/events.head
```

`cargo test --release --lib` checks every classifier kernel this CPU can run against
the portable one.

Step 7 loads BirdNET's `--target web` build from `../birdnet-bench/web/pkg`. If that
folder is missing, build it as described in `../birdnet-bench/README.md`.
