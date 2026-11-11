// Time BirdNET V2.4 in tract built for wasm32-unknown-unknown (the app's browser
// target) with wasm-bindgen, run by node.
//
//   RUSTFLAGS="-C target-feature=+simd128" CARGO_TARGET_DIR=target/wasm-bindgen \
//     wasm-pack build --release --target nodejs --out-dir target/pkg-node --no-typescript
//   node bench-bindgen.mjs target/pkg-node MODEL.onnx windows.f32 [logits_out.f32]
import { readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';

const [pkgDir, modelPath, winPath, outPath] = process.argv.slice(2);
const { BirdNet } = createRequire(import.meta.url)(resolve(pkgDir, 'birdnet_bench.js'));
const WIN = 144000;
const CLASSES = 6522;

const t0 = performance.now();
const net = new BirdNet(readFileSync(modelPath));
const loadMs = performance.now() - t0;

const wins = new Float32Array(readFileSync(winPath).buffer.slice(0));
const n = wins.length / WIN;
const logits = new Float32Array(n * CLASSES);
const times = [];
for (let i = 0; i < n; i++) {
  const t = performance.now();
  const out = net.run(wins.subarray(i * WIN, (i + 1) * WIN));
  times.push(performance.now() - t);
  logits.set(out, i * CLASSES);
}
const total = times.reduce((a, b) => a + b, 0);
const sorted = times.slice(1).sort((a, b) => a - b);
console.log(
  `tract wasm32-unknown-unknown (wasm-bindgen): load ${loadMs.toFixed(0)} ms, ` +
    `per window median ${sorted[sorted.length >> 1].toFixed(1)} ms, ` +
    `${n} windows in ${(total / 1000).toFixed(2)} s (${((n * 3) / (total / 1000)).toFixed(0)}x real time), ` +
    `process RSS ${(process.memoryUsage().rss / 1e6).toFixed(0)} MB`,
);
if (outPath) writeFileSync(outPath, Buffer.from(logits.buffer));
