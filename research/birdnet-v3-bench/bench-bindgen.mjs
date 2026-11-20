// Time BirdNET+ V3.0 in tract built for wasm32-unknown-unknown (the app's browser
// target) with wasm-bindgen, run by node (V8, one thread).
//
//   CARGO_PROFILE_RELEASE_DEBUG=false CARGO_PROFILE_RELEASE_STRIP=true \
//     RUSTFLAGS="-C target-feature=+simd128" \
//     wasm-pack build --release --target nodejs --out-dir PKG --no-typescript
//   node bench-bindgen.mjs PKG MODEL.onnx windows.f32 [out_prefix] [max_windows]
//
// Writes <out_prefix>.emb.f32 and <out_prefix>.score.f32 like the native binary.
import { readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';

const [pkgDir, modelPath, winPath, outPrefix, maxArg] = process.argv.slice(2);
const pkg = createRequire(import.meta.url)(resolve(pkgDir, 'birdnet_v3_bench.js'));
const WIN = 96000;
const EMB = 1280;

let bytes = readFileSync(modelPath);
const t0 = performance.now();
const net = new pkg.BirdNetV3(bytes);
const loadMs = performance.now() - t0;
bytes = null;
const per = net.out_len();

const wins = new Float32Array(readFileSync(winPath).buffer.slice(0));
let n = wins.length / WIN;
if (maxArg) n = Math.min(n, Number(maxArg));
const emb = new Float32Array(n * EMB);
const sc = new Float32Array(n * (per - EMB));
const times = [];
for (let i = 0; i < n; i++) {
  const t = performance.now();
  const out = net.run(wins.subarray(i * WIN, (i + 1) * WIN));
  times.push(performance.now() - t);
  emb.set(out.subarray(0, EMB), i * EMB);
  sc.set(out.subarray(EMB), i * (per - EMB));
}
const total = times.reduce((a, b) => a + b, 0);
const sorted = times.slice(1).sort((a, b) => a - b);
console.log(
  `tract wasm32-unknown-unknown (wasm-bindgen), ${per - EMB} classes: load ${loadMs.toFixed(0)} ms, ` +
    `per 3 s window median ${sorted[sorted.length >> 1].toFixed(1)} ms, first ${times[0].toFixed(0)} ms, ` +
    `${n} windows in ${(total / 1000).toFixed(2)} s (${((n * 3) / (total / 1000)).toFixed(0)}x real time), ` +
    `WASM memory ${(pkg.wasm_memory_bytes() / 1e6).toFixed(0)} MB, process RSS ${(process.memoryUsage().rss / 1e6).toFixed(0)} MB`,
);
if (outPrefix) {
  writeFileSync(outPrefix + '.emb.f32', Buffer.from(emb.buffer));
  writeFileSync(outPrefix + '.score.f32', Buffer.from(sc.buffer));
}
