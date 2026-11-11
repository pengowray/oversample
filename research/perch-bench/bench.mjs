// Time Perch v2 in tract compiled to wasm32-wasip1 (V8, single thread), as a stand-in
// for the browser / Tauri webview.
//
//   node bench.mjs perch_bench.wasm MODEL.onnx windows.f32 [out_prefix]
//
// Writes <out_prefix>.emb.f32 and, if the model has labels, <out_prefix>.label.f32.
import { readFileSync, writeFileSync } from 'node:fs';
import { WASI } from 'node:wasi';

const [wasmPath, modelPath, winPath, outPrefix] = process.argv.slice(2);
const WIN = 160000;
const EMB = 1536;

const wasi = new WASI({ version: 'preview1', args: [], env: {} });
const { instance } = await WebAssembly.instantiate(readFileSync(wasmPath), wasi.getImportObject());
wasi.initialize(instance);
const ex = instance.exports;
const mem = () => ex.memory.buffer;
const err = () => new TextDecoder().decode(new Uint8Array(mem(), ex.err_ptr(), ex.err_len()));

let model = readFileSync(modelPath);
const t0 = performance.now();
const mp = ex.alloc(model.length);
new Uint8Array(mem(), mp, model.length).set(model);
if (ex.load(mp, model.length, 1) !== 0) throw new Error(err());
ex.dealloc(mp, model.length);
const loadMs = performance.now() - t0;
const memAfterLoad = mem().byteLength;
model = null;
const per = ex.out_len();

const wins = new Float32Array(readFileSync(winPath).buffer.slice(0));
const n = wins.length / WIN;
const inp = ex.alloc(WIN * 4);
const out = ex.alloc(per * 4);
const all = new Float32Array(n * per);
const times = [];
for (let i = 0; i < n; i++) {
  new Float32Array(mem(), inp, WIN).set(wins.subarray(i * WIN, (i + 1) * WIN));
  const t = performance.now();
  if (ex.run(inp, out) !== 0) throw new Error(err());
  times.push(performance.now() - t);
  all.set(new Float32Array(mem(), out, per), i * per);
}
const total = times.reduce((a, b) => a + b, 0);
const sorted = times.slice(1).sort((a, b) => a - b);
console.log(
  `tract wasm, ${per - EMB} classes: load ${loadMs.toFixed(0)} ms, per 5 s window median ${sorted[sorted.length >> 1].toFixed(1)} ms, ` +
    `min ${sorted[0].toFixed(1)} ms, ${n} windows in ${(total / 1000).toFixed(2)} s (${((n * 5) / (total / 1000)).toFixed(0)}x real time), ` +
    `wasm memory ${(memAfterLoad / 1e6).toFixed(0)} MB after load, ${(mem().byteLength / 1e6).toFixed(0)} MB at end, ` +
    `process RSS ${(process.memoryUsage().rss / 1e6).toFixed(0)} MB`,
);
if (outPrefix) {
  const emb = new Float32Array(n * EMB);
  const lab = new Float32Array(n * (per - EMB));
  for (let i = 0; i < n; i++) {
    emb.set(all.subarray(i * per, i * per + EMB), i * EMB);
    lab.set(all.subarray(i * per + EMB, (i + 1) * per), i * (per - EMB));
  }
  writeFileSync(outPrefix + '.emb.f32', Buffer.from(emb.buffer));
  if (per > EMB) writeFileSync(outPrefix + '.label.f32', Buffer.from(lab.buffer));
}
