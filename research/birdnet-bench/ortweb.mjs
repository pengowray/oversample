// Time BirdNET V2.4 in onnxruntime-web's WASM backend under node (V8).
//
//   node ortweb.mjs NPM_DIR MODEL.onnx windows.f32 [threads] [logits_out.f32]
//
// NPM_DIR is a folder where `npm install onnxruntime-web` was run.
import { readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

const [npmDir, modelPath, winPath, threadsArg, outPath] = process.argv.slice(2);
const require = createRequire(join(npmDir, 'package.json'));
const ort = await import(pathToFileURL(require.resolve('onnxruntime-web')).href);
const WIN = 144000;
const CLASSES = 6522;

ort.env.wasm.numThreads = Number(threadsArg ?? 1);
const t0 = performance.now();
const session = await ort.InferenceSession.create(readFileSync(modelPath), {
  executionProviders: ['wasm'],
  graphOptimizationLevel: 'all',
});
const loadMs = performance.now() - t0;

const wins = new Float32Array(readFileSync(winPath).buffer.slice(0));
const n = wins.length / WIN;
const logits = new Float32Array(n * CLASSES);
const times = [];
for (let i = 0; i < n; i++) {
  const input = new ort.Tensor('float32', wins.subarray(i * WIN, (i + 1) * WIN), [1, WIN]);
  const t = performance.now();
  const res = await session.run({ input });
  times.push(performance.now() - t);
  logits.set(res.output.data, i * CLASSES);
}
const total = times.reduce((a, b) => a + b, 0);
const sorted = times.slice(1).sort((a, b) => a - b);
console.log(
  `onnxruntime-web ${ort.env.versions?.web ?? ''} wasm threads=${ort.env.wasm.numThreads}: ` +
    `load ${loadMs.toFixed(0)} ms, per window median ${sorted[sorted.length >> 1].toFixed(1)} ms, ` +
    `${n} windows in ${(total / 1000).toFixed(2)} s (${((n * 3) / (total / 1000)).toFixed(0)}x real time), ` +
    `process RSS ${(process.memoryUsage().rss / 1e6).toFixed(0)} MB`,
);
if (outPath) writeFileSync(outPath, Buffer.from(logits.buffer));
process.exit(0);
