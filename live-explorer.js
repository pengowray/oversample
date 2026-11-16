// Live explorer tab (experimental): views of BirdNET's live readings, drawn
// with Apache ECharts (vendor/, Apache-2.0). UMAP uses umap-js (vendor/, MIT),
// loaded only when "Run UMAP" is pressed.
//
// Called from src/live_explorer.rs:
//   oversampleLiveExplorer.update(el, json)
//   oversampleLiveExplorer.dispose(el)
// `json` is { status, min, spectrum: { pts: [[kHz, dB]...], peaks: [[kHz, dB]...], rate } | null,
//   top: [[name, score]...], map: { pts: [[x, y, secondsAgo, name, score]...], share: [a, b], n } | null,
//   times: [secondsAgo...], sim: [[i, j, similarity]...], dims: [dim...],
//   emb: [[i, row, z, value]...] }
(function () {
  const loaded = {};
  function load(src, test) {
    if (test()) return Promise.resolve();
    if (!loaded[src]) {
      loaded[src] = new Promise((resolve, reject) => {
        const s = document.createElement('script');
        s.src = src;
        s.onload = () => resolve();
        s.onerror = () => {
          loaded[src] = null;
          reject(new Error(`Could not load ${src}`));
        };
        document.head.appendChild(s);
      });
    }
    return loaded[src];
  }
  const loadEcharts = () => load('echarts.min.js', () => !!window.echarts);
  const loadUmap = () => load('umap-js-1.4.0.min.js', () => !!(window.UMAP && window.UMAP.UMAP));

  const esc = (t) =>
    String(t).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);
  const ago = (s) => (s > -0.05 ? 'now' : `${(-s).toFixed(1)} s ago`);

  // Same colours as live-embedding.js and species-map.js.
  const INK = '#dddddd';
  const MUTED = '#9a9a9a';
  const GRID = '#333333';
  const HIT = '#50dc96';
  const TOOLTIP = {
    confine: true,
    backgroundColor: '#262626',
    borderColor: '#444',
    textStyle: { color: INK, fontSize: 12 },
  };
  const axisText = { color: MUTED, fontSize: 10 };
  // Sequential (one hue, dark to light) for similarity, diverging (blue,
  // grey, amber) for the embedding's z-scores.
  const SEQ = ['#1b1b1b', '#1f4a38', '#2f8a60', '#50dc96', '#c9f5df'];
  const DIV = ['#3b7dd8', '#2a3f5c', '#2a2a2a', '#6b5326', '#e0a030'];

  const PANELS = [
    ['spectrum', 'Frequency peaks', 'Power spectrum of the last 3 s. The strongest peaks are marked in kHz.'],
    ['top', 'Top classes now', 'BirdNET’s highest scores for the last 3 s, up to 15. The dashed line is the minimum confidence.'],
    ['map', 'This session’s readings', 'One dot per reading, newest brightest. Dots close together sound alike to BirdNET.'],
    ['sim', 'Similarity, last 15 s', 'How alike each pair of readings is. Bright squares away from the diagonal are a sound that came back.'],
    ['emb', 'Embedding, last 15 s', 'The 48 of BirdNET’s 1,024 values that changed most, one row each, scaled per row.'],
  ];

  function parts(el) {
    if (el.__parts) return el.__parts;
    el.innerHTML = '';
    const p = {};
    for (const [key, title, note] of PANELS) {
      const sec = document.createElement('section');
      sec.className = 'live-exp-panel';
      sec.innerHTML = `<div class="live-exp-title">${title}</div><div class="live-exp-note">${note}</div>`;
      if (key === 'map') {
        const bar = document.createElement('div');
        bar.className = 'live-exp-mapbar';
        bar.innerHTML =
          '<button class="psd-btn live-exp-pca active">PCA</button>' +
          '<button class="psd-btn live-exp-umap">UMAP</button>' +
          '<span class="live-exp-mapinfo"></span>';
        sec.appendChild(bar);
        p.mapbar = bar;
      }
      const chart = document.createElement('div');
      chart.className = `live-exp-chart-${key}`;
      sec.appendChild(chart);
      const empty = document.createElement('div');
      empty.className = 'live-exp-empty';
      sec.appendChild(empty);
      el.appendChild(sec);
      p[key] = chart;
      p[key + 'Empty'] = empty;
    }
    // PCA follows each reading; UMAP is a snapshot made on request.
    p.mapbar.querySelector('.live-exp-pca').onclick = () => {
      el.__mapMode = 'pca';
      el.__umap = null;
      if (el.__last) draw(el, el.__last, true);
    };
    p.mapbar.querySelector('.live-exp-umap').onclick = () => runUmap(el);
    el.__parts = p;
    return p;
  }

  function chartFor(div) {
    let chart = window.echarts.getInstanceByDom(div);
    if (!chart) {
      chart = window.echarts.init(div, null, { renderer: 'canvas' });
      div.__resize = new ResizeObserver(() => chart.resize());
      div.__resize.observe(div);
    }
    return chart;
  }

  function setEmpty(p, key, text) {
    p[key + 'Empty'].textContent = text || '';
    p[key].style.display = text ? 'none' : '';
  }

  // The axis ends at the highest frequency within 40 dB of the loudest bin
  // (rounded up to 5 kHz, at least 10 kHz), so a bird in a 192 kHz file is
  // not squeezed into the left edge. Bats keep their range.
  function spectrumMax(s) {
    const top = Math.max(...s.pts.map((p) => p[1]));
    let hi = 0;
    for (const [k, db] of s.pts) if (db > top - 40) hi = k;
    return Math.min(s.rate / 2000, Math.max(10, Math.ceil(hi / 5) * 5));
  }

  // Peaks to mark: none below 0.2 kHz (DC and rumble), and none within 4%
  // of the axis width of a stronger one, so labels do not overlap. The Rust
  // side sends them strongest first.
  function spreadPeaks(s) {
    const gap = spectrumMax(s) * 0.04;
    const out = [];
    for (const p of s.peaks) {
      if (p[0] < 0.2 || p[0] > spectrumMax(s)) continue;
      if (out.every((q) => Math.abs(q[0] - p[0]) > gap)) out.push(p);
    }
    return out;
  }

  function spectrumOptions(s) {
    return {
      animation: false,
      grid: { left: 34, right: 26, top: 18, bottom: 22 },
      tooltip: { ...TOOLTIP, trigger: 'axis', formatter: (ps) => `${ps[0].value[0].toFixed(2)} kHz<br>${ps[0].value[1].toFixed(1)} dB` },
      xAxis: {
        type: 'value',
        min: 0,
        max: spectrumMax(s),
        axisLabel: { ...axisText, formatter: (v) => `${v}` },
        name: 'kHz',
        nameLocation: 'end',
        nameGap: 4,
        nameTextStyle: axisText,
        splitLine: { lineStyle: { color: GRID } },
      },
      yAxis: {
        type: 'value',
        scale: true,
        name: 'dB',
        nameGap: 4,
        nameTextStyle: { ...axisText, align: 'right' },
        axisLabel: axisText,
        splitLine: { lineStyle: { color: GRID } },
      },
      // Peaks can sit close together; their labels must not overlap.
      labelLayout: { hideOverlap: true },
      series: [
        {
          type: 'line',
          data: s.pts.filter((p) => p[0] <= spectrumMax(s)),
          showSymbol: false,
          lineStyle: { color: INK, width: 1.2 },
          markPoint: {
            symbol: 'circle',
            symbolSize: 6,
            itemStyle: { color: HIT },
            label: { show: true, position: 'top', distance: 3, color: INK, fontSize: 10, formatter: (m) => `${m.value}` },
            data: spreadPeaks(s).map(([k, db]) => ({ coord: [k, db], value: k.toFixed(1) })),
          },
        },
      ],
    };
  }

  function topOptions(d) {
    const rows = d.top;
    return {
      animation: false,
      grid: { left: 4, right: 34, top: 2, bottom: 18 },
      tooltip: { ...TOOLTIP, trigger: 'item', formatter: (p) => `${esc(p.name)}<br>${p.value.toFixed(3)}` },
      xAxis: { type: 'value', min: 0, max: 1, axisLabel: axisText, splitLine: { lineStyle: { color: GRID } } },
      yAxis: { type: 'category', inverse: true, data: rows.map((r) => r[0]), show: false },
      series: [
        {
          type: 'bar',
          barWidth: 12,
          data: rows.map(([n, s]) => ({
            name: n,
            value: s,
            itemStyle: { color: s >= d.min ? HIT : '#3a5a4a', borderRadius: [0, 2, 2, 0] },
          })),
          // Names over the bars: the sidebar is too narrow for a label column.
          label: {
            show: true,
            position: 'insideLeft',
            formatter: (p) => `${p.name}  ${p.value.toFixed(2)}`,
            color: INK,
            fontSize: 10,
            textShadowColor: '#000',
            textShadowBlur: 3,
          },
          markLine: {
            silent: true,
            symbol: 'none',
            label: { show: false },
            lineStyle: { color: MUTED, type: 'dashed' },
            data: [{ xAxis: d.min }],
          },
        },
      ],
    };
  }

  function mapOptions(pts, xName, yName) {
    const newest = pts.length ? pts[pts.length - 1] : null;
    return {
      animation: false,
      grid: { left: 20, right: 8, top: 8, bottom: 22 },
      tooltip: {
        ...TOOLTIP,
        trigger: 'item',
        formatter: (p) => `${ago(p.value[2])}<br>${esc(p.value[3])} ${Number(p.value[4]).toFixed(2)}`,
      },
      xAxis: {
        type: 'value',
        scale: true,
        name: xName,
        nameLocation: 'middle',
        nameGap: 6,
        nameTextStyle: axisText,
        axisLabel: { show: false },
        splitLine: { show: false },
        axisLine: { lineStyle: { color: GRID } },
      },
      yAxis: {
        type: 'value',
        scale: true,
        name: yName,
        nameLocation: 'middle',
        nameGap: 8,
        nameTextStyle: axisText,
        axisLabel: { show: false },
        splitLine: { show: false },
        axisLine: { lineStyle: { color: GRID } },
      },
      // Age on a colour ramp: the newest readings are brightest.
      visualMap: {
        show: false,
        dimension: 2,
        min: pts.length ? Math.min(...pts.map((p) => p[2])) : -1,
        max: 0,
        inRange: { color: ['#3a3a3a', '#2f8a60', HIT] },
      },
      series: [
        { id: 'pts', type: 'scatter', data: pts, symbolSize: 5 },
        {
          id: 'now',
          type: 'scatter',
          data: newest ? [newest] : [],
          symbolSize: 12,
          itemStyle: { color: HIT, borderColor: '#fff', borderWidth: 2 },
          z: 3,
        },
      ],
    };
  }

  function heatOptions(d, cells, yData, range, colors, tip) {
    const t = d.times;
    return {
      animation: false,
      grid: { left: 4, right: 16, top: 4, bottom: 20 },
      tooltip: { ...TOOLTIP, trigger: 'item', formatter: tip },
      xAxis: {
        type: 'category',
        data: t.map((_, i) => i),
        axisLabel: {
          ...axisText,
          interval: (i) => i === t.length - 1 || (i > 0 && Math.floor(t[i]) !== Math.floor(t[i - 1]) && Math.round(-t[i]) % 5 === 0),
          formatter: (i) => (t[i] > -0.05 ? 'now' : `${Math.round(t[i])} s`),
        },
        axisTick: { show: false },
        axisLine: { show: false },
      },
      yAxis: { type: 'category', data: yData, show: false, inverse: true },
      visualMap: { show: false, min: range[0], max: range[1], dimension: 2, inRange: { color: colors } },
      series: [{ type: 'heatmap', data: cells, progressive: 0 }],
    };
  }

  // The colour scale starts at the lowest similarity between two different
  // readings, so the structure shows whether readings are 0.5 or 0.95 alike.
  function simFloor(cells) {
    let lo = 1;
    for (const [i, j, v] of cells) if (i !== j && v < lo) lo = v;
    return Math.min(lo, 0.99);
  }

  async function runUmap(el) {
    const get = window.__oversampleExplorerEmbeddings;
    const info = el.__parts.mapbar.querySelector('.live-exp-mapinfo');
    if (!get) return;
    const e = get();
    if (e.n < 10) {
      info.textContent = 'UMAP needs 10 or more readings.';
      return;
    }
    info.textContent = `Running UMAP on ${e.n} readings…`;
    try {
      await loadUmap();
      const rows = [];
      for (let i = 0; i < e.n; i++) rows.push(Array.from(e.data.subarray(i * e.dim, (i + 1) * e.dim)));
      const umap = new window.UMAP.UMAP({
        nComponents: 2,
        nNeighbors: Math.min(15, e.n - 1),
        minDist: 0.1,
      });
      const y = await umap.fitAsync(rows);
      el.__umap = {
        pts: y.map((p, i) => [p[0], p[1], e.ago[i], e.names[i], 0]),
        n: e.n,
        made: new Date(),
      };
      el.__mapMode = 'umap';
      if (el.__last) draw(el, el.__last, true);
    } catch (err) {
      info.textContent = `UMAP failed: ${err.message}`;
    }
  }

  // Shortest gap between redraws of each chart, in ms. Redrawing all five at
  // every reading (4 a second) cost playback frames.
  const GAP = { spectrum: 450, top: 0, map: 900, sim: 1500, emb: 1500 };
  function due(el, key, force) {
    const now = performance.now();
    el.__drawn = el.__drawn || {};
    if (!force && now - (el.__drawn[key] || 0) < GAP[key]) return false;
    el.__drawn[key] = now;
    return true;
  }

  function draw(el, d, force) {
    const p = parts(el);
    // Spectrum
    if (d.spectrum && d.spectrum.pts.length) {
      setEmpty(p, 'spectrum');
      if (due(el, 'spectrum', force)) chartFor(p.spectrum).setOption(spectrumOptions(d.spectrum), true);
    } else {
      setEmpty(p, 'spectrum', 'Play a file or listen to see the spectrum.');
    }
    const noBirdnet =
      d.status === 'download'
        ? 'Needs BirdNET.'
        : d.status === 'loading'
          ? 'Loading BirdNET…'
          : d.status === 'failed'
            ? 'BirdNET stopped.'
            : null;
    // Top classes
    if (d.top.length) {
      setEmpty(p, 'top');
      p.top.style.height = `${d.top.length * 17 + 26}px`;
      chartFor(p.top).setOption(topOptions(d), true);
    } else {
      setEmpty(p, 'top', noBirdnet || 'No reading yet.');
    }
    // Session map
    const pca = el.__mapMode !== 'umap';
    p.mapbar.querySelector('.live-exp-pca').classList.toggle('active', pca);
    p.mapbar.querySelector('.live-exp-umap').classList.toggle('active', !pca);
    const info = p.mapbar.querySelector('.live-exp-mapinfo');
    if (pca) {
      if (d.map && d.map.pts.length >= 3) {
        setEmpty(p, 'map');
        const [a, b] = d.map.share.map((s) => Math.round(s * 100));
        info.textContent = `${d.map.n} readings`;
        if (due(el, 'map', force))
          chartFor(p.map).setOption(mapOptions(d.map.pts, `PC1, ${a}% of the variation`, `PC2, ${b}%`), true);
      } else {
        info.textContent = '';
        setEmpty(p, 'map', noBirdnet || 'Needs 3 or more readings.');
      }
    } else if (el.__umap) {
      setEmpty(p, 'map');
      const u = el.__umap;
      info.textContent = `${u.n} readings at ${u.made.toLocaleTimeString()}. Press UMAP to update.`;
      if (el.__umapDrawn !== u) {
        chartFor(p.map).setOption(mapOptions(u.pts, 'UMAP 1', 'UMAP 2'), true);
        el.__umapDrawn = u;
      }
    }
    // Similarity and embedding
    if (d.times.length >= 2) {
      setEmpty(p, 'sim');
      setEmpty(p, 'emb');
      if (due(el, 'sim', force)) chartFor(p.sim).setOption(
        heatOptions(d, d.sim, d.times.map((_, i) => i), [simFloor(d.sim), 1], SEQ, (q) =>
          `${ago(d.times[q.value[0]])} and ${ago(d.times[q.value[1]])}<br>Similarity ${q.value[2].toFixed(2)}`
        ),
        true
      );
      p.sim.style.height = `${Math.max(120, Math.min(220, el.clientWidth))}px`;
      if (due(el, 'emb', force)) chartFor(p.emb).setOption(
        heatOptions(d, d.emb, d.dims, [-3, 3], DIV, (q) =>
          `Value ${d.dims[q.value[1]] + 1} of 1,024, ${ago(d.times[q.value[0]])}<br>${q.value[3].toFixed(3)} (${q.value[2] >= 0 ? '+' : ''}${q.value[2].toFixed(1)} SD from its mean)`
        ),
        true
      );
    } else {
      setEmpty(p, 'sim', noBirdnet || 'Needs 2 or more readings.');
      setEmpty(p, 'emb', noBirdnet || 'Needs 2 or more readings.');
    }
  }

  window.oversampleLiveExplorer = {
    async update(el, json) {
      const d = JSON.parse(json);
      el.__last = d;
      await loadEcharts();
      draw(el, d);
    },
    dispose(el) {
      if (!el.__parts) return;
      for (const key of ['spectrum', 'top', 'map', 'sim', 'emb']) {
        const div = el.__parts[key];
        if (div.__resize) div.__resize.disconnect();
        const chart = window.echarts && window.echarts.getInstanceByDom(div);
        if (chart) chart.dispose();
      }
      el.__parts = null;
      el.innerHTML = '';
    },
  };
})();
