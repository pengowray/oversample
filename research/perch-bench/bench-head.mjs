// Time Perch v2 as two files in the wasm-bindgen build: the embedding model in tract,
// then the int8 classifier (head.rs, from export_head.py). Each part is timed apart.
//
//   node bench-head.mjs PKG_NODE perch_v2_embed.tract.onnx perch_v2_head.int8.bin windows.f32 [out_prefix]
import { readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';

const [pkgDir, modelPath, headPath, winPath, outPrefix] = process.argv.slice(2);
const pkg = createRequire(import.meta.url)(resolve(pkgDir, 'perch_bench.js'));
const WIN = 160000;
const EMB = 1536;
const mb = () => (pkg.wasm_memory_bytes() / 1e6).toFixed(0);

let t = performance.now();
const net = pkg.Perch.with_spatial(readFileSync(modelPath));
console.log(`embedding model load ${(performance.now() - t).toFixed(0)} ms, WASM memory ${mb()} MB`);
t = performance.now();
const head = new pkg.PerchHead(readFileSync(headPath));
const C = head.classes();
console.log(`classifier load ${(performance.now() - t).toFixed(0)} ms (${C} classes), WASM memory ${mb()} MB`);

const wins = new Float32Array(readFileSync(winPath).buffer.slice(0));
const n = wins.length / WIN;
const emb = new Float32Array(n * EMB);
const lab = new Float32Array(n * C);
const tm = [];
const th = [];
for (let i = 0; i < n; i++) {
  t = performance.now();
  const out = net.run(wins.subarray(i * WIN, (i + 1) * WIN));
  tm.push(performance.now() - t);
  t = performance.now();
  const l = head.scores(out.subarray(EMB));
  th.push(performance.now() - t);
  emb.set(out.subarray(0, EMB), i * EMB);
  lab.set(l, i * C);
}
const med = (a) => a.slice(1).sort((x, y) => x - y)[(a.length - 1) >> 1];
const total = [...tm, ...th].reduce((a, b) => a + b, 0);
console.log(
  `per 5 s window, median: embedding model ${med(tm).toFixed(0)} ms, classifier ${med(th).toFixed(0)} ms; ` +
    `${n} windows in ${(total / 1000).toFixed(2)} s (${((n * 5) / (total / 1000)).toFixed(0)}x real time), ` +
    `WASM memory ${mb()} MB`,
);
if (outPrefix) {
  writeFileSync(outPrefix + '.emb.f32', Buffer.from(emb.buffer));
  writeFileSync(outPrefix + '.label.f32', Buffer.from(lab.buffer));
}
