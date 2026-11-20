// Time BirdNET V2.4 (the app's `birdnet-v2.4-embeddings-fp32.onnx`, 144,000 samples
// in) or Perch's embedding model (`perch_v2_embed.tract.onnx`, 160,000 samples in)
// with the same wasm-bindgen build as bench-bindgen.mjs, for comparison.
//
//   node bench-v24.mjs PKG MODEL.onnx windows.f32 [WIN, default 144000]
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';

const [pkgDir, modelPath, winPath, winArg] = process.argv.slice(2);
const pkg = createRequire(import.meta.url)(resolve(pkgDir, 'birdnet_v3_bench.js'));
const WIN = Number(winArg || 144000);
const t0 = performance.now();
const net = new pkg.Plain(readFileSync(modelPath), WIN);
const loadMs = performance.now() - t0;
const wins = new Float32Array(readFileSync(winPath).buffer.slice(0));
const n = wins.length / WIN;
const times = [];
for (let i = 0; i < n; i++) {
  const t = performance.now();
  net.run(wins.subarray(i * WIN, (i + 1) * WIN));
  times.push(performance.now() - t);
}
const sorted = times.slice(1).sort((a, b) => a - b);
console.log(
  `load ${loadMs.toFixed(0)} ms, ${n} windows, median ${sorted[sorted.length >> 1].toFixed(1)} ms, ` +
    `WASM memory ${(pkg.wasm_memory_bytes() / 1e6).toFixed(0)} MB`,
);
