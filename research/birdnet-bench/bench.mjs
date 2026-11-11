// Time BirdNET V2.4 in tract compiled to wasm32 (V8, single thread), as a stand-in
// for the browser / Tauri webview.
//
//   node bench.mjs birdnet_bench.wasm MODEL.onnx windows.f32 [logits_out.f32]
import { readFileSync, writeFileSync } from 'node:fs';
import { WASI } from 'node:wasi';

const [wasmPath, modelPath, winPath, outPath] = process.argv.slice(2);
const WIN = 144000;
const CLASSES = 6522;

const wasi = new WASI({ version: 'preview1', args: [], env: {} });
const { instance } = await WebAssembly.instantiate(readFileSync(wasmPath), wasi.getImportObject());
wasi.initialize(instance);
const ex = instance.exports;
const mem = () => ex.memory.buffer;
const err = () => new TextDecoder().decode(new Uint8Array(mem(), ex.err_ptr(), ex.err_len()));

const model = readFileSync(modelPath);
const t0 = performance.now();
const mp = ex.alloc(model.length);
new Uint8Array(mem(), mp, model.length).set(model);
if (ex.load(mp, model.length, 1) !== 0) throw new Error(err());
const loadMs = performance.now() - t0;

const wins = new Float32Array(readFileSync(winPath).buffer.slice(0));
const n = wins.length / WIN;
const inp = ex.alloc(WIN * 4);
const out = ex.alloc(CLASSES * 4);
const logits = new Float32Array(n * CLASSES);
const times = [];
for (let i = 0; i < n; i++) {
  new Float32Array(mem(), inp, WIN).set(wins.subarray(i * WIN, (i + 1) * WIN));
  const t = performance.now();
  if (ex.run(inp, out) !== 0) throw new Error(err());
  times.push(performance.now() - t);
  logits.set(new Float32Array(mem(), out, CLASSES), i * CLASSES);
}
const total = times.reduce((a, b) => a + b, 0);
const sorted = times.slice(1).sort((a, b) => a - b);
console.log(
  `tract wasm (${wasmPath.includes('simd') && !wasmPath.includes('nosimd') ? 'simd128' : 'no simd'}): ` +
    `load ${loadMs.toFixed(0)} ms, per window median ${sorted[sorted.length >> 1].toFixed(1)} ms, ` +
    `${n} windows in ${(total / 1000).toFixed(2)} s (${((n * 3) / (total / 1000)).toFixed(0)}x real time), ` +
    `wasm memory ${(mem().byteLength / 1e6).toFixed(0)} MB, process RSS ${(process.memoryUsage().rss / 1e6).toFixed(0)} MB`,
);
if (outPath) writeFileSync(outPath, Buffer.from(logits.buffer));
