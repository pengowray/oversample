# birdnet-bench

Throwaway speed and accuracy test for running BirdNET V2.4 in Oversample without a
server. Findings are in [../birdnet-on-device.md](../birdnet-on-device.md). Nothing here
is part of the app build: this folder has its own `[workspace]` and target directory.

No model files are committed. Download them into a folder of your own (for example
`~/birdnet-models`):

| File | Source |
|---|---|
| `BirdNET_GLOBAL_6K_V2.4_Model_FP32.tflite` | `huggingface.co/justinchuby/BirdNET-onnx` (copy of the BirdNET-Analyzer file) |
| `BirdNET_v2.4_fp32_foldconv.onnx`, `BirdNET_v2.4_int8_arm_foldconv.onnx`, `labels.txt` | `huggingface.co/tphakala/BirdNET-v2.4` |
| `birdnet-v2.4-embeddings-fp32.onnx`, `fp32/BattyBirdNET-*-256kHz_fp32.onnx`, `labels/BattyBirdNET-*_Labels.txt` | `huggingface.co/tphakala/BattyBirdNET-onnx` |

All are CC BY-NC-SA 4.0.

## Steps

Python (3.12): `pip install onnx onnxruntime numpy ai-edge-litert soundfile scipy`.

```bash
# 1. Audio to 3 s windows of 48 kHz float32
python -I prep.py birds.f32 bird1.wav bird2.wav
python -I prep.py --repeat 60 long.f32 bird1.wav      # 3 minutes, for timing

# 2. Reference: TFLite and Python onnxruntime, saves logits
python -I ref.py ~/birdnet-models birds.f32 --threads 1 --save birds

# 3. tract native and onnxruntime native (ort crate)
cargo build --release --features ort
./target/release/native tract ~/birdnet-models/BirdNET_v2.4_fp32_foldconv.onnx birds.f32 1 birds.tract.f32
./target/release/native ort   ~/birdnet-models/BirdNET_v2.4_fp32_foldconv.onnx birds.f32 1 birds.ort.f32

# 4. tract in wasm32 with SIMD, run by node (V8, one thread)
RUSTFLAGS="-C target-feature=+simd128" cargo build --release --lib --target wasm32-wasip1 --target-dir target/wasm-simd
node bench.mjs target/wasm-simd/wasm32-wasip1/release/birdnet_bench.wasm ~/birdnet-models/BirdNET_v2.4_fp32_foldconv.onnx birds.f32 birds.wasm.f32

# 5. onnxruntime-web, WASM backend, in node
(cd /some/dir && npm install onnxruntime-web)
node ortweb.mjs /some/dir ~/birdnet-models/BirdNET_v2.4_fp32_foldconv.onnx birds.f32 1 birds.ortweb.f32

# 6. Compare any run with TFLite
python -I compare.py birds.tflite.f32 birds.tract.f32 birds.wasm.f32 birds.ortweb.f32

# 7. BattyBirdNET on bat recordings (resampled to 256 kHz, read as 48 kHz)
python -I batty.py ~/birdnet-models EU bat1.wav bat2.wav
```

Step 4 uses `wasm32-wasip1` and a raw C ABI so that node can load the module without
wasm-bindgen. To time the app's own target, `wasm32-unknown-unknown` with wasm-bindgen
(`getrandom`'s `wasm_js` feature is set in `Cargo.toml` for that target):

```bash
RUSTFLAGS="-C target-feature=+simd128" CARGO_TARGET_DIR=target/wasm-bindgen \
  wasm-pack build --release --target nodejs --out-dir target/pkg-node --no-typescript
node bench-bindgen.mjs target/pkg-node ~/birdnet-models/BirdNET_v2.4_fp32_foldconv.onnx birds.f32 birds.bindgen.f32
```

Build output under `target/` is large (several GB with five target directories). Delete
it when done, since this folder may be synced.
