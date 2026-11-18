// Live BirdNET view (experimental), drawn with Apache ECharts (vendor/,
// Apache-2.0), loaded the first time the view is shown.
//
// Called from src/live_embedding.rs:
//   oversampleLiveEmbedding.update(el, json, speciesJson)
//   oversampleLiveEmbedding.dispose(el)
// `json` is { min, top: [[name, score]...], trail: [[x, y, secondsAgo, score]...],
//   rows: [name...], strip: [[secondsAgo, row, score]...],
//   change: [[secondsAgo, distance]...], history }
// `speciesJson` is [[x, y, name]...] for the map's grey points, or null when
// it was sent before. `pcJson` comes with it: { variance: [share...],
// ranks: [[0..255 per point]...] } for the component maps; `json.pc` then
// holds the reading's percentile (0 to 1) on each component.
(function () {
  let loading = null;
  function loadEcharts() {
    if (window.echarts) return Promise.resolve();
    if (!loading) {
      loading = new Promise((resolve, reject) => {
        const s = document.createElement('script');
        s.src = 'echarts.min.js';
        s.onload = () => resolve();
        s.onerror = () => {
          loading = null;
          reject(new Error('Could not load the chart library'));
        };
        document.head.appendChild(s);
      });
    }
    return loading;
  }

  const esc = (t) =>
    String(t).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);

  // Same colours as species-map.js: green for a species hit.
  const INK = '#dddddd';
  const MUTED = '#9a9a9a';
  const GRID = '#333333';
  const HIT = '#50dc96';
  // Jumps longer than this on the map are not joined by the trail line, so it
  // never crosses the gap between unrelated groups (same radius as placement).
  const JUMP = 0.15;
  const TOOLTIP = {
    confine: true,
    backgroundColor: '#262626',
    borderColor: '#444',
    textStyle: { color: INK, fontSize: 12 },
  };
  const ago = (s) => (s > -0.05 ? 'now' : `${(-s).toFixed(1)} s ago`);

  // A score's colour in the strip: green, more opaque for higher scores, with a
  // step at the minimum confidence so a hit stands out from a near miss.
  function cellColor(s, min) {
    const a = s >= min ? 0.6 + 0.4 * s : 0.08 + 0.4 * (s / Math.max(min, 0.01));
    return `rgba(80,220,150,${a.toFixed(3)})`;
  }

  // Rows are the full width of the sidebar, with each name written over the
  // left end of its row: the sidebar is too narrow for a label column.
  const ROW_H = 18;
  const CHANGE_H = 30;

  function stripOptions(d) {
    const rows = d.rows;
    const rowsH = Math.max(rows.length, 1) * ROW_H;
    // Each reading's cell runs to the next reading's time (the newest to 0).
    const times = [...new Set(d.strip.map((c) => c[0]))].sort((a, b) => a - b);
    const next = new Map(times.map((t, i) => [t, i + 1 < times.length ? times[i + 1] : 0]));
    const xAxis = (gridIndex, show) => ({
      gridIndex,
      type: 'value',
      min: -d.history,
      max: 0,
      interval: 5,
      axisLabel: { show, color: MUTED, fontSize: 10, formatter: (v) => (v === 0 ? 'now' : `${v} s`) },
      axisLine: { show, lineStyle: { color: GRID } },
      axisTick: { show: false },
      splitLine: { show: true, lineStyle: { color: GRID, opacity: 0.5 } },
    });
    return {
      animation: false,
      backgroundColor: 'transparent',
      tooltip: { ...TOOLTIP, trigger: 'item' },
      grid: [
        { left: 16, right: 14, top: 0, height: rowsH },
        { left: 16, right: 14, top: rowsH + 16, height: CHANGE_H },
      ],
      xAxis: [xAxis(0, false), xAxis(1, true)],
      yAxis: [
        {
          gridIndex: 0,
          type: 'category',
          data: rows,
          inverse: true,
          axisLabel: {
            inside: true,
            color: INK,
            fontSize: 11,
            textShadowColor: '#000',
            textShadowBlur: 3,
          },
          axisLine: { show: false },
          axisTick: { show: false },
          z: 10,
        },
        {
          gridIndex: 1,
          type: 'value',
          // Cosine distance between readings 0.25 s apart: a steady sound
          // stays under 0.1, a new sound reaches 0.2 to 0.5. Higher values
          // are drawn at the top; the tooltip has the exact value.
          min: 0,
          max: 0.5,
          show: false,
        },
      ],
      graphic: [
        ...(rows.length
          ? []
          : [
              {
                type: 'text',
                left: 16,
                top: 3,
                style: { text: 'No class at 0.10 or above', fill: MUTED, fontSize: 11 },
              },
            ]),
        {
          type: 'text',
          left: 16,
          top: rowsH + 3,
          style: { text: 'Change in latent space', fill: MUTED, fontSize: 10 },
        },
      ],
      series: [
        {
          id: 'strip',
          type: 'custom',
          xAxisIndex: 0,
          yAxisIndex: 0,
          data: d.strip,
          encode: { x: 0, y: 1 },
          renderItem: (params, api) => {
            const t = api.value(0);
            const row = api.value(1);
            const s = api.value(2);
            const a = api.coord([t, row]);
            const b = api.coord([next.get(t) ?? 0, row]);
            const h = api.size([0, 1])[1];
            return {
              type: 'rect',
              shape: { x: a[0], y: a[1] - h / 2 + 1, width: Math.max(b[0] - a[0], 1), height: h - 2 },
              style: { fill: cellColor(s, d.min) },
            };
          },
          tooltip: {
            formatter: (p) =>
              `${esc(rows[p.value[1]])}<br>${p.value[2].toFixed(2)} <span style="color:${MUTED}">${ago(p.value[0])}</span>`,
          },
        },
        {
          id: 'change',
          type: 'line',
          xAxisIndex: 1,
          yAxisIndex: 1,
          data: d.change.map(([t, v]) => [t, Math.min(v, 0.5), v]),
          encode: { x: 0, y: 1 },
          // Small dots, so each reading can be hovered.
          showSymbol: true,
          symbolSize: 3,
          itemStyle: { color: INK },
          lineStyle: { color: INK, width: 1.5 },
          areaStyle: { color: 'rgba(221,221,221,0.12)' },
          tooltip: {
            formatter: (p) =>
              `Change from the reading before: ${p.value[2].toFixed(2)}<br><span style="color:${MUTED}">${ago(p.value[0])}</span>`,
          },
        },
      ],
    };
  }

  function mapSeries(d) {
    const trail = d.trail;
    const last = trail[trail.length - 1];
    const hit = last && last[3] >= d.min;
    // The line joins readings that are close on the map; a null breaks it.
    const line = [];
    trail.forEach((p, i) => {
      const q = trail[i - 1];
      if (q && Math.hypot(p[0] - q[0], p[1] - q[1]) > JUMP) line.push(null);
      line.push(p);
    });
    return [
      {
        id: 'trailLine',
        zlevel: 1,
        type: 'line',
        data: line.map((p) => (p ? [p[0], p[1]] : [null, null])),
        connectNulls: false,
        showSymbol: false,
        silent: true,
        lineStyle: { color: HIT, width: 1, opacity: 0.35 },
        z: 2,
      },
      {
        id: 'trail',
        zlevel: 1,
        type: 'scatter',
        data: trail.slice(0, -1).map((p) => ({
          value: p,
          itemStyle: {
            color: p[3] >= d.min ? HIT : MUTED,
            opacity: Math.max(0.15, 1 + p[2] / 15),
          },
        })),
        symbolSize: 5,
        z: 3,
      },
      {
        id: 'now',
        zlevel: 1,
        type: 'scatter',
        data: last
          ? [
              {
                value: last,
                // Below the minimum confidence: a large dashed ring with no
                // fill, so it reads as "somewhere near here" and not as a hit.
                symbolSize: hit ? 10 + 10 * last[3] : 18,
                itemStyle: hit
                  ? { color: HIT, borderColor: '#ffffff', borderWidth: 2 }
                  : { color: 'transparent', borderColor: INK, borderWidth: 1.5, borderType: 'dashed' },
              },
            ]
          : [],
        z: 4,
      },
    ];
  }

  function mapOptions(species, d) {
    return {
      animation: false,
      backgroundColor: 'transparent',
      grid: { left: 2, right: 2, top: 2, bottom: 2 },
      xAxis: { type: 'value', show: false, min: -1.05, max: 1.05 },
      yAxis: { type: 'value', show: false, min: -1.05, max: 1.05 },
      dataZoom: [
        { type: 'inside', xAxisIndex: 0, filterMode: 'none' },
        { type: 'inside', yAxisIndex: 0, filterMode: 'none' },
      ],
      tooltip: {
        ...TOOLTIP,
        trigger: 'item',
        formatter: (p) => {
          if (p.seriesId === 'species') return esc(p.value[2]);
          const v = p.value;
          return `Reading ${ago(v[2])}<br>Highest score ${v[3].toFixed(2)}`;
        },
      },
      series: [
        {
          id: 'species',
          type: 'scatter',
          data: species,
          // Its own canvas layer (zlevel 0), drawn once: the moving marker
          // and trail are on zlevel 1, so an update redraws only them.
          zlevel: 0,
          large: true,
          symbolSize: 3,
          itemStyle: { color: 'rgba(170,170,170,0.3)' },
          emphasis: { scale: 2, itemStyle: { color: INK } },
          z: 1,
        },
        ...mapSeries(d),
      ],
    };
  }

  // Component maps: small copies of the species map, one per principal
  // component of BirdNET's class rows (research/live-embedding/pc_bands.py).
  // Each lights the species whose percentile on that component is near the
  // reading's percentile among real recordings. Plain canvas: each point
  // group is drawn once into an offscreen canvas, so a reading costs a few
  // drawImage calls per map.
  const PC_BANDS = 20; // 5 % of the species each
  const PC_FALLOFF = 0.1; // a band this far from the reading is not lit
  const PC_PER_ROW = 4;
  // What lies at the two ends of each component (pc_bands.txt).
  const PC_ENDS = [
    ['Amazonian antshrikes and antbirds', 'larks'],
    ['waterbirds: cormorants, gulls, herons', 'Andean tanagers and warblers'],
    ['southern seabirds and waterfowl', 'Asian flycatchers and drongos'],
    ['African larks and scrub robins', 'North American woodpeckers and warblers'],
    ['North American warblers and sparrows', 'African and Asian forest birds'],
    ['North American orioles and thrashers', 'a mix: canastero, flowerpecker, bay owl, hornbill'],
    ['dry-country thrashers, meadowlark, wheatear', 'South Asian sunbirds, coucal, crow'],
    ['terns and Asian forest birds', 'African doves, turacos, coucals'],
  ];

  function pcBuild(div, species, pc) {
    const n = pc.variance.length;
    const gap = 4;
    const size = Math.floor((div.clientWidth - gap * (PC_PER_ROW - 1)) / PC_PER_ROW);
    if (size < 20) return null;
    const dpr = window.devicePixelRatio || 1;
    const px = Math.round(size * dpr);
    const at = (v) => ((v + 1.05) / 2.1) * px;
    const layer = (pts, color, r) => {
      const c = document.createElement('canvas');
      c.width = c.height = px;
      const g = c.getContext('2d');
      g.fillStyle = color;
      for (const p of pts) g.fillRect(at(p[0]) - r / 2, at(-p[1]) - r / 2, r, r);
      return c;
    };
    const base = layer(species, 'rgba(170,170,170,0.22)', Math.max(1, dpr));
    div.innerHTML = '';
    const maps = [];
    for (let k = 0; k < n; k++) {
      const groups = Array.from({ length: PC_BANDS }, () => []);
      pc.ranks[k].forEach((r, i) => groups[Math.min(PC_BANDS - 1, Math.floor((r * PC_BANDS) / 256))].push(species[i]));
      const bands = groups.map((g) => layer(g, HIT, Math.max(1, dpr)));
      const cell = document.createElement('div');
      cell.className = 'live-emb-pc';
      const [lo, hi] = PC_ENDS[k] || ['', ''];
      cell.title =
        `PC${k + 1}: ${(pc.variance[k] * 100).toFixed(1)}% of the variation between BirdNET's classes.` +
        (lo ? `\nLow end: ${lo}.\nHigh end: ${hi}.` : '');
      const label = document.createElement('div');
      label.className = 'live-emb-pc-label';
      label.textContent = `PC${k + 1} ${(pc.variance[k] * 100).toFixed(1)}%`;
      const canvas = document.createElement('canvas');
      canvas.width = canvas.height = px;
      canvas.style.width = canvas.style.height = `${size}px`;
      cell.append(label, canvas);
      div.appendChild(cell);
      maps.push({ ctx: canvas.getContext('2d'), bands });
    }
    return { width: div.clientWidth, base, maps, px };
  }

  function pcDraw(el, d) {
    const div = el.__parts.pcs;
    const pc = el.__pc;
    if (!pc || !el.__species || !d.pc || !d.pc.length) return;
    if (!div.__built || div.__built.width !== div.clientWidth) div.__built = pcBuild(div, el.__species, pc);
    const b = div.__built;
    if (!b) return;
    b.maps.forEach((m, k) => {
      const p = d.pc[k];
      m.ctx.globalAlpha = 1;
      m.ctx.clearRect(0, 0, b.px, b.px);
      m.ctx.drawImage(b.base, 0, 0);
      m.bands.forEach((band, i) => {
        const a = 1 - Math.abs((i + 0.5) / PC_BANDS - p) / PC_FALLOFF;
        if (a <= 0) return;
        m.ctx.globalAlpha = a;
        m.ctx.drawImage(band, 0, 0);
      });
    });
  }

  function readout(el, d) {
    const [first, ...rest] = d.top;
    if (!first) return;
    const others = rest.map(([n, s]) => `${esc(n)} ${s.toFixed(2)}`).join(', ');
    let html;
    let next;
    if (first[1] >= d.min) {
      html = `<span class="live-emb-name">${esc(first[0])}</span> <span class="live-emb-score">${first[1].toFixed(2)}</span>`;
      next = others ? `Next: ${others}` : '';
    } else {
      html = `<span class="live-emb-none">No species at ${d.min.toFixed(2)} or above</span>`;
      next = `Highest: ${esc(first[0])} ${first[1].toFixed(2)}${others ? ', ' + others : ''}`;
      // Bat recordings: say why nothing is found.
      if (d.ultrasonic) next += '<br>BirdNET hears up to 15 kHz, so bat calls do not show here.';
    }
    el.innerHTML = html + (next ? `<div class="live-emb-next">${next}</div>` : '');
    const n = el.querySelector('.live-emb-next');
    if (n) n.title = n.textContent;
  }

  function parts(el) {
    if (!el.__parts) {
      el.innerHTML = '';
      const mk = (cls) => {
        const div = document.createElement('div');
        div.className = cls;
        el.appendChild(div);
        return div;
      };
      el.__parts = {
        readout: mk('live-emb-readout'),
        strip: mk('live-emb-strip'),
        map: mk('live-emb-map'),
        pcs: mk('live-emb-pcs'),
      };
    }
    return el.__parts;
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

  window.oversampleLiveEmbedding = {
    async update(el, json, speciesJson, pcJson) {
      const d = JSON.parse(json);
      el.__last = d;
      if (speciesJson) {
        el.__species = JSON.parse(speciesJson);
        el.__liveSpecies = true;
      }
      if (pcJson) el.__pc = JSON.parse(pcJson);
      await loadEcharts();
      const p = parts(el);
      readout(p.readout, d);
      p.strip.style.height = `${Math.max(d.rows.length, 1) * ROW_H + CHANGE_H + 36}px`;
      const strip = chartFor(p.strip);
      strip.setOption(stripOptions(d), { replaceMerge: ['series', 'yAxis', 'grid', 'xAxis', 'graphic'] });
      const map = chartFor(p.map);
      const t0 = performance.now();
      if (!map.__ready && el.__species) {
        map.setOption(mapOptions(el.__species, d));
        map.__ready = true;
      } else if (map.__ready) {
        map.setOption({ series: mapSeries(d) });
      }
      el.__ms = performance.now() - t0;
      const t1 = performance.now();
      pcDraw(el, d);
      el.__pcMs = performance.now() - t1;
    },
    dispose(el) {
      if (!el.__parts) return;
      for (const div of [el.__parts.strip, el.__parts.map]) {
        if (div.__resize) div.__resize.disconnect();
        const chart = window.echarts && window.echarts.getInstanceByDom(div);
        if (chart) chart.dispose();
      }
      el.__parts = null;
      el.__liveSpecies = false;
      el.innerHTML = '';
    },
  };
})();
