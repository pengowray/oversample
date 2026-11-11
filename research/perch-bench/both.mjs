// Time Perch v2 and BirdNET V2.4 in the same node process, taking turns, so both see
// the same machine load. Both are tract builds for wasm32-unknown-unknown + wasm-bindgen
// + SIMD, opt-level 3 (the app's target).
//
//   node both.mjs PERCH_PKG_NODE PERCH.onnx perch_windows.f32 \
//                 BIRDNET_WEB_PKG BIRDNET.onnx birdnet_windows.f32 [rounds]
//
// PERCH_PKG_NODE is `wasm-pack --target nodejs` output of this crate; BIRDNET_WEB_PKG is
// ../birdnet-bench/web/pkg (`--target web`). Give both the same audio (for example 3
// minutes: 36 Perch windows, 60 BirdNET windows) so the totals compare directly.
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const [perchPkg, perchModel, perchWin, bnPkg, bnModel, bnWin, roundsArg] = process.argv.slice(2);
const rounds = Number(roundsArg ?? 3);

const { Perch } = createRequire(import.meta.url)(resolve(perchPkg, 'perch_bench.js'));
const bn = await import(pathToFileURL(resolve(bnPkg, 'birdnet_bench.js')).href);
bn.initSync({ module: readFileSync(resolve(bnPkg, 'birdnet_bench_bg.wasm')) });

function windows(path, win) {
  const w = new Float32Array(readFileSync(path).buffer.slice(0));
  return Array.from({ length: w.length / win }, (_, i) => w.subarray(i * win, (i + 1) * win));
}

function timedLoad(name, make, path) {
  const bytes = readFileSync(path);
  const t = performance.now();
  const net = make(bytes);
  console.log(`${name}: model load (parse and optimise) ${(performance.now() - t).toFixed(0)} ms`);
  return net;
}

const perchName = `Perch v2 (${perchModel.split('/').pop()})`;
const models = [
  { name: perchName, net: timedLoad(perchName, (b) => new Perch(b), perchModel), wins: windows(perchWin, 160000), sec: 5 },
  { name: 'BirdNET V2.4', net: timedLoad('BirdNET V2.4', (b) => new bn.BirdNet(b), bnModel), wins: windows(bnWin, 144000), sec: 3 },
];

for (let r = 0; r < rounds; r++) {
  for (const m of models) {
    const times = [];
    for (const w of m.wins) {
      const t = performance.now();
      m.net.run(w);
      times.push(performance.now() - t);
    }
    const total = times.reduce((a, b) => a + b, 0);
    const med = times.slice(1).sort((a, b) => a - b)[times.length >> 1];
    const audio = m.wins.length * m.sec;
    console.log(
      `round ${r + 1} ${m.name}: ${m.wins.length} windows (${audio} s of audio) in ${(total / 1000).toFixed(2)} s, ` +
        `median ${med.toFixed(0)} ms per window, ${(audio / (total / 1000)).toFixed(0)}x real time`,
    );
  }
}
console.log(`process RSS ${(process.memoryUsage().rss / 1e6).toFixed(0)} MB`);
