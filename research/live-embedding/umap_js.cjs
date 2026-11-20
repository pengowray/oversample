// UMAP of a float32 matrix with the app's vendored umap-js (MIT), for
// perch_live_data.py, which runs it because umap-learn is not installed here.
//
//   node umap_js.cjs IN.f32 ROWS COLS OUT.f32 NEIGHBOURS MIN_DIST SEED
//
// IN.f32 holds ROWS x COLS float32 values, little endian, row by row. Distance
// is euclidean (umap-js's default); give it unit-length rows for cosine.
// Writes ROWS x 2 float32 to OUT.f32.
const fs = require('fs');
const path = require('path');
const { UMAP } = require(path.join(__dirname, '../../vendor/umap-js-1.4.0.min.js'));

const [inPath, rows, cols, outPath, nn, minDist, seed] = process.argv.slice(2);
const n = Number(rows);
const d = Number(cols);
const buf = fs.readFileSync(inPath);
const all = new Float32Array(buf.buffer, buf.byteOffset, n * d);
const data = [];
for (let i = 0; i < n; i++) data.push(Array.from(all.subarray(i * d, (i + 1) * d)));

// mulberry32: a seeded random source, so the layout is the same each run.
let s = Number(seed) >>> 0;
const random = () => {
  s = (s + 0x6d2b79f5) >>> 0;
  let t = s;
  t = Math.imul(t ^ (t >>> 15), t | 1);
  t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
};

const t0 = Date.now();
const umap = new UMAP({ nComponents: 2, nNeighbors: Number(nn), minDist: Number(minDist), random });
const xy = umap.fit(data);
const out = new Float32Array(n * 2);
xy.forEach((p, i) => {
  out[2 * i] = p[0];
  out[2 * i + 1] = p[1];
});
fs.writeFileSync(outPath, Buffer.from(out.buffer));
console.log(`umap-js n_neighbors=${nn} min_dist=${minDist}: ${((Date.now() - t0) / 1000).toFixed(1)} s`);
