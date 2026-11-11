// Time Perch v2 in tract built for wasm32-unknown-unknown (the app's browser target)
// with wasm-bindgen, run by node.
//
//   RUSTFLAGS="-C target-feature=+simd128" CARGO_TARGET_DIR=target/wasm-bindgen \
//     wasm-pack build --release --target nodejs --out-dir target/pkg-node --no-typescript
//   node bench-bindgen.mjs target/pkg-node MODEL.onnx windows.f32 [out_prefix]
import { readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';

const [pkgDir, modelPath, winPath, outPrefix] = process.argv.slice(2);
const { Perch } = createRequire(import.meta.url)(resolve(pkgDir, 'perch_bench.js'));
const WIN = 160000;
const EMB = 1536;

const t0 = performance.now();
const net = new Perch(readFileSync(modelPath));
const loadMs = performance.now() - t0;
const per = net.out_len();

const wins = new Float32Array(readFileSync(winPath).buffer.slice(0));
const n = wins.length / WIN;
const emb = new Float32Array(n * EMB);
const lab = new Float32Array(n * (per - EMB));
const times = [];
for (let i = 0; i < n; i++) {
  const t = performance.now();
  const out = net.run(wins.subarray(i * WIN, (i + 1) * WIN));
  times.push(performance.now() - t);
  emb.set(out.subarray(0, EMB), i * EMB);
  lab.set(out.subarray(EMB), i * (per - EMB));
}
const total = times.reduce((a, b) => a + b, 0);
const sorted = times.slice(1).sort((a, b) => a - b);
console.log(
  `tract wasm32-unknown-unknown (wasm-bindgen), ${per - EMB} classes: load ${loadMs.toFixed(0)} ms, ` +
    `per 5 s window median ${sorted[sorted.length >> 1].toFixed(1)} ms, ` +
    `${n} windows in ${(total / 1000).toFixed(2)} s (${((n * 5) / (total / 1000)).toFixed(0)}x real time), ` +
    `process RSS ${(process.memoryUsage().rss / 1e6).toFixed(0)} MB`,
);
if (outPrefix) {
  writeFileSync(outPrefix + '.emb.f32', Buffer.from(emb.buffer));
  if (per > EMB) writeFileSync(outPrefix + '.label.f32', Buffer.from(lab.buffer));
}
