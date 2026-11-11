// Load Perch as two files (tract embedding model + int8 classifier, wasm-bindgen build
// in ./pkg) and run windows. Used by the worker and, for comparison, directly on the
// main thread. Both files are cached in OPFS.
import init, { Perch, PerchHead } from './pkg/perch_bench.js';

const WIN = 160000;
const EMB = 1536;
const now = () => performance.now();

async function readCache(root, name) {
  try {
    const f = await (await root.getFileHandle(name)).getFile();
    if (f.size > 0) return new Uint8Array(await f.arrayBuffer());
  } catch {}
  return null;
}

async function writeCache(root, name, bytes, inWorker) {
  const fh = await root.getFileHandle(name, { create: true });
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

/** Fetch `url` or read it from OPFS; timings go into `info` under `key`. */
async function getFile(root, url, clear, inWorker, info, key) {
  const name = url.split('/').pop();
  if (root && clear) {
    try {
      await root.removeEntry(name);
    } catch {}
  }
  let t = now();
  let bytes = root ? await readCache(root, name) : null;
  if (bytes) {
    info[key + 'Source'] = 'opfs';
    info[key + 'ReadMs'] = +(now() - t).toFixed(0);
  } else {
    t = now();
    bytes = new Uint8Array(await (await fetch(url, { cache: 'no-store' })).arrayBuffer());
    info[key + 'Source'] = 'network';
    info[key + 'FetchMs'] = +(now() - t).toFixed(0);
    if (root) {
      t = now();
      await writeCache(root, name, bytes, inWorker);
      info[key + 'CacheWriteMs'] = +(now() - t).toFixed(0);
    }
  }
  info[key + 'Bytes'] = bytes.length;
  return bytes;
}

/** Returns { net, head, wasm, info } where info has the load timings in ms. */
export async function loadModel(urls, clear, inWorker) {
  const info = {};
  // WebKitGTK (the Tauri webview on Linux) has no navigator.storage: no OPFS cache.
  const root = navigator.storage?.getDirectory ? await navigator.storage.getDirectory() : null;
  if (!root) info.opfs = 'unavailable';
  const modelBytes = await getFile(root, urls.model, clear, inWorker, info, 'model');
  let t = now();
  const wasm = await init();
  info.wasmInitMs = +(now() - t).toFixed(0);
  t = now();
  const net = Perch.with_spatial(modelBytes);
  info.modelBuildMs = +(now() - t).toFixed(0);
  const headBytes = await getFile(root, urls.head, clear, inWorker, info, 'head');
  t = now();
  const head = new PerchHead(headBytes);
  info.headBuildMs = +(now() - t).toFixed(0);
  info.wasmMemoryBytes = wasm.memory.buffer.byteLength;
  return { net, head, wasm, info };
}

/** Run every window: the embedding model, then the classifier, each timed. Returns the
 *  top class and its softmax probability per window. `yieldBetween` yields to the event
 *  loop between windows (for main-thread runs). */
export async function runWindows(net, head, wasm, samples, onWindow, yieldBetween) {
  const n = samples.length / WIN;
  const model = [];
  const classifier = [];
  const top1 = [];
  for (let i = 0; i < n; i++) {
    let t = now();
    const out = net.run(samples.subarray(i * WIN, (i + 1) * WIN));
    model.push(now() - t);
    t = now();
    const logits = head.scores(out.subarray(EMB));
    classifier.push(now() - t);
    let k = 0;
    for (let c = 1; c < logits.length; c++) if (logits[c] > logits[k]) k = c;
    let sum = 0;
    for (let c = 0; c < logits.length; c++) sum += Math.exp(logits[c] - logits[k]);
    top1.push([k, Math.round(1000 / sum) / 1000]);
    onWindow?.(i, model[i] + classifier[i]);
    if (yieldBetween) await new Promise((r) => setTimeout(r, 0));
  }
  return { model, classifier, top1, wasmMemoryBytes: wasm.memory.buffer.byteLength };
}
