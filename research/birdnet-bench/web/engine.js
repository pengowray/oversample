// Load BirdNET (tract, wasm-bindgen build in ./pkg) and run windows. Used by the
// worker and, for comparison, directly on the main thread.
import init, { BirdNet } from './pkg/birdnet_bench.js';

const FILE = 'birdnet-v2.4-fp32-foldconv.onnx';
const WIN = 144000;
const CLASSES = 6522;
const now = () => performance.now();

async function readCache(root) {
  try {
    const f = await (await root.getFileHandle(FILE)).getFile();
    if (f.size > 0) return new Uint8Array(await f.arrayBuffer());
  } catch {}
  return null;
}

async function writeCache(root, bytes, inWorker) {
  const fh = await root.getFileHandle(FILE, { create: true });
  if (inWorker && fh.createSyncAccessHandle) {
    const h = await fh.createSyncAccessHandle();
    h.truncate(0);
    h.write(bytes, { at: 0 });
    h.flush();
    h.close();
  } else {
    const w = await fh.createWritable();
    await w.write(bytes);
    await w.close();
  }
}

/** Returns { net, wasm, info } where info has the load timings in ms. */
export async function loadModel(url, clear, inWorker) {
  const info = {};
  // WebKitGTK (the Tauri webview on Linux) has no navigator.storage: no OPFS cache.
  const root = navigator.storage?.getDirectory ? await navigator.storage.getDirectory() : null;
  if (!root) info.opfs = 'unavailable';
  if (root && clear) {
    try {
      await root.removeEntry(FILE);
    } catch {}
  }
  let t = now();
  let bytes = root ? await readCache(root) : null;
  if (bytes) {
    info.source = 'opfs';
    info.readMs = now() - t;
  } else {
    t = now();
    const resp = await fetch(url, { cache: 'no-store' });
    bytes = new Uint8Array(await resp.arrayBuffer());
    info.source = 'network';
    info.fetchMs = now() - t;
    if (root) {
      t = now();
      await writeCache(root, bytes, inWorker);
      info.cacheWriteMs = now() - t;
    }
  }
  info.modelBytes = bytes.length;
  t = now();
  const wasm = await init();
  info.wasmInitMs = now() - t;
  t = now();
  const net = new BirdNet(bytes);
  info.modelBuildMs = now() - t;
  info.wasmMemoryBytes = wasm.memory.buffer.byteLength;
  return { net, wasm, info };
}

/** Run every window. `onWindow(i, ms)` after each; `yieldEvery` > 0 yields to the
 *  event loop between windows (for main-thread runs). */
export async function runWindows(net, wasm, samples, onWindow, yieldBetween) {
  const n = samples.length / WIN;
  const times = [];
  const top1 = [];
  for (let i = 0; i < n; i++) {
    const t = now();
    const logits = net.run(samples.subarray(i * WIN, (i + 1) * WIN));
    const ms = now() - t;
    times.push(ms);
    let k = 0;
    for (let c = 1; c < CLASSES; c++) if (logits[c] > logits[k]) k = c;
    top1.push([k, Math.round(1000 / (1 + Math.exp(-logits[k]))) / 1000]);
    onWindow?.(i, ms);
    if (yieldBetween) await new Promise((r) => setTimeout(r, 0));
  }
  return { times, top1, wasmMemoryBytes: wasm.memory.buffer.byteLength };
}
