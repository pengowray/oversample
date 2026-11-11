import { loadModel, runWindows } from './engine.js';

let state = null;

self.onmessage = async (e) => {
  const m = e.data;
  try {
    if (m.cmd === 'load') {
      state = await loadModel(m.model, m.clear, true);
      self.postMessage({ type: 'loaded', info: state.info });
    } else if (m.cmd === 'run') {
      const samples = new Float32Array(m.buffer);
      const res = await runWindows(state.net, state.wasm, samples, (i, ms) =>
        self.postMessage({ type: 'window', i, ms }),
      );
      self.postMessage({ type: 'done', ...res });
    }
  } catch (err) {
    self.postMessage({ type: 'error', message: `${err}\n${err?.stack ?? ''}` });
  }
};
