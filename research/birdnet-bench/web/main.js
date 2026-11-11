// Test page driver. URL parameters:
//   phases=cold,warm,main   phases to run, one page load each (default: all three)
//     cold: clear the OPFS cache, download the model, run in a Web Worker
//     warm: load the model from OPFS, run in a Web Worker
//     main: load from OPFS, run on the main thread (yielding between windows)
//     idle: fetch the windows file and wait 5 s, with no model (memory baseline)
//   i=0                      index of the current phase
//   label=...                name for the results file
//   audio=data/audio/long.f32  windows file (N x 144000 float32)
import { loadModel, runWindows } from './engine.js';

// The Tauri window opens the bare dev URL, so run.py leaves the query in a file.
if (!location.search) {
  const r = await fetch('data/default-query', { cache: 'no-store' });
  if (r.ok) location.search = (await r.text()).trim();
}
const q = new URLSearchParams(location.search);
const phases = (q.get('phases') || 'cold,warm,main').split(',');
const idx = Number(q.get('i') || 0);
const phase = phases[idx];
const label = q.get('label') || '';
const modelUrl = q.get('model') || 'data/models/BirdNET_v2.4_fp32_foldconv.onnx';
const audioUrl = q.get('audio') || 'data/audio/long.f32';
const out = document.getElementById('out');
const spin = document.getElementById('spin');
const log = (s) => (out.textContent += s + '\n');

// Frame monitor: a spinning square drawn every animation frame. Gaps between frames
// show how long the main thread was blocked.
let gaps = null;
let last = performance.now();
let angle = 0;
function frame(t) {
  if (gaps) gaps.push(t - last);
  last = t;
  angle = (angle + 6) % 360;
  spin.style.transform = `rotate(${angle}deg)`;
  requestAnimationFrame(frame);
}
requestAnimationFrame(frame);

async function measureFrames(ms) {
  gaps = [];
  await new Promise((r) => setTimeout(r, ms));
  const g = gaps;
  gaps = null;
  return g;
}

function frameStats(g) {
  if (!g.length) return { frames: 0 };
  const s = [...g].sort((a, b) => a - b);
  return {
    frames: g.length,
    medianMs: +s[s.length >> 1].toFixed(1),
    maxMs: +s[s.length - 1].toFixed(1),
    over50ms: g.filter((x) => x > 50).length,
  };
}

function stats(times) {
  const s = times.slice(1).sort((a, b) => a - b);
  const total = times.reduce((a, b) => a + b, 0);
  return {
    windows: times.length,
    firstMs: +times[0].toFixed(1),
    medianMs: +s[s.length >> 1].toFixed(1),
    totalS: +(total / 1000).toFixed(2),
    realTimeX: +((times.length * 3) / (total / 1000)).toFixed(1),
  };
}

async function post(result) {
  log(JSON.stringify(result, null, 1));
  try {
    await fetch('/result', { method: 'POST', body: JSON.stringify(result) });
  } catch (e) {
    log('could not post result: ' + e);
  }
}

async function run() {
  const result = {
    label,
    phase,
    userAgent: navigator.userAgent,
    hardwareConcurrency: navigator.hardwareConcurrency,
    simd: WebAssembly.validate(
      new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0, 1, 5, 1, 96, 0, 1, 123, 3, 2, 1, 0, 10, 10, 1, 8, 0, 65, 0, 253, 15, 253, 98, 11]),
    ),
  };
  log(`phase ${phase} (${idx + 1} of ${phases.length})`);
  const audio = new Float32Array(await (await fetch(audioUrl, { cache: 'no-store' })).arrayBuffer());
  result.idleFrames = frameStats(await measureFrames(1000));

  const tLoad = performance.now();
  if (phase === 'idle') {
    // Baseline for memory: the same page held open with no model loaded.
    await measureFrames(5000);
    result.audioBytes = audio.byteLength;
  } else if (phase === 'main') {
    const { net, wasm, info } = await loadModel(modelUrl, false, false);
    result.load = info;
    result.loadTotalMs = +(performance.now() - tLoad).toFixed(0);
    gaps = [];
    const res = await runWindows(net, wasm, audio, null, true);
    result.runFrames = frameStats(gaps);
    gaps = null;
    Object.assign(result, { run: stats(res.times), top1: res.top1, wasmMemoryBytes: res.wasmMemoryBytes });
  } else {
    const worker = new Worker('worker.js', { type: 'module' });
    const msg = (cmd, extra, transfer) =>
      new Promise((resolve, reject) => {
        worker.onmessage = (e) => {
          if (e.data.type === 'error') reject(new Error(e.data.message));
          else if (e.data.type !== 'window') resolve(e.data);
        };
        worker.postMessage({ cmd, ...extra }, transfer || []);
      });
    // Frame gaps are measured during the whole load too: compiling the model happens
    // in the worker, so the page should keep drawing.
    gaps = [];
    const loaded = await msg('load', { model: modelUrl, clear: phase === 'cold' });
    result.loadFrames = frameStats(gaps);
    result.load = loaded.info;
    result.loadTotalMs = +(performance.now() - tLoad).toFixed(0);
    gaps = [];
    const done = await msg('run', { buffer: audio.buffer }, [audio.buffer]);
    result.runFrames = frameStats(gaps);
    gaps = null;
    Object.assign(result, { run: stats(done.times), top1: done.top1, wasmMemoryBytes: done.wasmMemoryBytes });
    worker.terminate();
  }
  await post(result);
  if (idx + 1 < phases.length) {
    q.set('i', String(idx + 1));
    location.search = q.toString();
  } else {
    await post({ label, phase: 'finished' });
    log('finished');
  }
}

run().catch((e) => post({ label, phase, error: `${e}\n${e?.stack ?? ''}` }));
