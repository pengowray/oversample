// Time the BirdBox pipeline in tract built for wasm32-unknown-unknown (the app's browser
// target) with wasm-bindgen, run by node (V8, one thread). Same pattern as
// research/birdnet-v3-bench/bench-bindgen.mjs.
//
//   CARGO_PROFILE_RELEASE_DEBUG=false CARGO_PROFILE_RELEASE_STRIP=true \
//     RUSTFLAGS="-C target-feature=+simd128" \
//     wasm-pack build --release --target nodejs --out-dir PKG --no-typescript
//   node bench-bindgen.mjs PKG MODEL.onnx AUDIO.f32 [out.boxes.tsv]
//
// AUDIO.f32 is raw little-endian float32 at 32 kHz (mono). Prints the time per step and
// writes the merged boxes (t0, t1, f0, f1, avg_conf, max_conf, merged) if asked.
import { readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';

const [pkgDir, modelPath, audioPath, outPath] = process.argv.slice(2);
const pkg = createRequire(import.meta.url)(resolve(pkgDir, 'birdbox_bench.js'));

let bytes = readFileSync(modelPath);
let t = performance.now();
const bb = new pkg.BirdBox(bytes);
const loadMs = performance.now() - t;
bytes = null;

const audio = new Float32Array(readFileSync(audioPath).buffer.slice(0));
t = performance.now();
const n = bb.frames(audio);
const specMs = performance.now() - t;
const img = [], net = [], dec = [];
for (let c = 0; c < n; c++) {
  t = performance.now();
  const image = bb.image(c);
  img.push(performance.now() - t);
  t = performance.now();
  const head = bb.network(image);
  net.push(performance.now() - t);
  t = performance.now();
  bb.decode(c, head, 0.15);
  dec.push(performance.now() - t);
}
t = performance.now();
const boxes = bb.fuse(0.05);
const fuseMs = performance.now() - t;
const med = (v) => v.slice(1).sort((a, b) => a - b)[(v.length - 1) >> 1];
const sum = (v) => v.reduce((a, b) => a + b, 0);
const total = specMs + sum(img) + sum(net) + sum(dec) + fuseMs;
const secs = audio.length / 32000;
console.log(
  `tract wasm32 (wasm-bindgen): ${n} windows, ${boxes.length / 7} merged boxes; load ${loadMs.toFixed(0)} ms; per window: ` +
    `spectrogram ${(specMs / n).toFixed(2)} ms, floor+image median ${med(img).toFixed(2)} ms, ` +
    `network median ${med(net).toFixed(1)} ms (first ${net[0].toFixed(0)} ms), NMS median ${med(dec).toFixed(3)} ms; ` +
    `fusion ${fuseMs.toFixed(2)} ms; total ${(total / 1000).toFixed(2)} s for ${secs.toFixed(1)} s (${(secs * 1000 / total).toFixed(0)}x real time); ` +
    `WASM memory ${(pkg.wasm_memory_bytes() / 1e6).toFixed(0)} MB`,
);
if (outPath) {
  const rows = [];
  for (let i = 0; i < boxes.length; i += 7) rows.push(Array.from(boxes.subarray(i, i + 7), (v) => v.toFixed(6)).join('\t'));
  writeFileSync(outPath, rows.join('\n') + (rows.length ? '\n' : ''));
}
