// Live BirdNET view (experimental), drawn with Apache ECharts (vendor/,
// Apache-2.0), loaded the first time the view is shown.
//
// Called from src/live_embedding.rs:
//   oversampleLiveEmbedding.update(el, json, speciesJson)
//   oversampleLiveEmbedding.dispose(el)
// `json` is { min, top: [[name, score]...], trail: [[x, y, secondsAgo, score]...],
//   rows: [name...], strip: [[secondsAgo, row, score]...],
//   change: [[secondsAgo, distance]...], history }
// `speciesJson` is [[x, y, name, scientific]...] for the map's grey points, or null when
// it was sent before. `pcJson` comes with it: { variance: [share...],
// ranks: [[0..255 per point]...], basis: 'recordings' | 'rows',
// umap: [[x, y]...] or null } for the component maps; `json.pc` then
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
        silent: true,
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
        silent: true,
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
      // No ECharts tooltip: pointing anywhere lists the nearest species
      // (mapExplore below), and the trail's points no longer take the hover.
      tooltip: { show: false },
      series: [
        {
          id: 'species',
          type: 'scatter',
          data: species.map((s) => [s[0], s[1]]),
          // Its own canvas layer (zlevel 0), drawn once: the moving marker
          // and trail are on zlevel 1, so an update redraws only them.
          zlevel: 0,
          large: true,
          silent: true,
          symbolSize: 3,
          itemStyle: { color: 'rgba(170,170,170,0.3)' },
          z: 1,
        },
        ...mapSeries(d),
      ],
    };
  }

  // Pointing at the species map. Hovering (mouse) or tapping (touch)
  // anywhere lists the species nearest that point, nearest first, with the
  // nearest ringed on the map; a click's or tap's list stays until the next
  // click, tap or pan.
  // Names are also written on the map: the few species nearest the current
  // reading, and every species in view once zoomed in far enough. Lookups use
  // a grid of the species, built once; the overlay is plain DOM and canvas
  // over the ECharts map, so the map's own layers are not redrawn for it.
  const NEAR_PX = 48; // how far from the pointer a species can be listed
  const NEAR_MAX = 5; // species in the list
  const NEAR_READING = 3; // species named beside the current reading
  // Name species in view once zoomed in this far (the share of the whole
  // map's width in view), nearest the middle of the view first, up to
  // LABEL_MAX that fit without overlapping.
  const LABEL_ZOOM = 0.5;
  const LABEL_MAX = 12; // most names on the map at once
  const GRID_N = 40;

  function speciesGrid(species) {
    const cells = Array.from({ length: GRID_N * GRID_N }, () => []);
    const cell = (v) => Math.min(GRID_N - 1, Math.max(0, Math.floor(((v + 1.05) / 2.1) * GRID_N)));
    species.forEach((s, i) => cells[cell(s[1]) * GRID_N + cell(s[0])].push(i));
    return { cells, cell };
  }

  // Species within `r` (map units) of (x, y), nearest first, at most `max`.
  function nearestSpecies(el, x, y, r, max) {
    const sp = el.__species;
    const g = el.__grid || (el.__grid = speciesGrid(sp));
    const span = Math.ceil((r / 2.1) * GRID_N);
    const cx = g.cell(x);
    const cy = g.cell(y);
    const found = [];
    for (let j = Math.max(0, cy - span); j <= Math.min(GRID_N - 1, cy + span); j++)
      for (let i = Math.max(0, cx - span); i <= Math.min(GRID_N - 1, cx + span); i++)
        for (const k of g.cells[j * GRID_N + i]) {
          const d = Math.hypot(sp[k][0] - x, sp[k][1] - y);
          if (d <= r) found.push([d, k]);
        }
    found.sort((a, b) => a[0] - b[0]);
    return found.slice(0, max).map((f) => f[1]);
  }

  // Map units per screen pixel at the current zoom.
  function unitsPerPx(map) {
    const a = map.convertFromPixel({ gridIndex: 0 }, [0, 0]);
    const b = map.convertFromPixel({ gridIndex: 0 }, [100, 0]);
    return Math.abs(b[0] - a[0]) / 100;
  }

  // Common name, then the scientific name when it differs.
  function speciesRow(s) {
    const sci = s[3] && s[3] !== s[2] ? `<span class="live-emb-near-sci">${esc(s[3])}</span>` : '';
    return `<div class="live-emb-near-row"><span class="live-emb-near-name">${esc(s[2])}</span>${sci}</div>`;
  }

  function mapExplore(el, map, div) {
    if (div.__explore) return div.__explore;
    const mk = (tag, cls) => {
      const e = document.createElement(tag);
      e.className = cls;
      div.appendChild(e);
      return e;
    };
    const labels = mk('canvas', 'live-emb-labels');
    const ring = mk('div', 'live-emb-near-ring');
    const box = mk('div', 'live-emb-near');
    let touch = false;
    let pinned = false;
    const hide = () => {
      box.style.display = ring.style.display = 'none';
      pinned = false;
    };
    const show = (px, py) => {
      if (!el.__species) return;
      const [x, y] = map.convertFromPixel({ gridIndex: 0 }, [px, py]);
      const near = nearestSpecies(el, x, y, NEAR_PX * unitsPerPx(map), NEAR_MAX);
      if (!near.length) return hide();
      const sp = el.__species;
      box.innerHTML = near.map((k) => speciesRow(sp[k])).join('');
      box.style.display = 'block';
      // Beside the pointer, kept inside the map; above a finger on touch.
      const w = div.clientWidth;
      const h = div.clientHeight;
      const bw = box.offsetWidth;
      const bh = box.offsetHeight;
      let bx = px + 14;
      if (bx + bw > w) bx = px - 14 - bw;
      let by = touch ? py - bh - 24 : py + 14;
      if (by + bh > h) by = py - bh - 14;
      box.style.left = `${Math.max(0, Math.min(w - bw, bx))}px`;
      box.style.top = `${Math.max(0, Math.min(h - bh, by))}px`;
      const [rx, ry] = map.convertToPixel({ gridIndex: 0 }, [sp[near[0]][0], sp[near[0]][1]]);
      ring.style.display = 'block';
      ring.style.transform = `translate(${rx}px, ${ry}px)`;
    };
    div.addEventListener('pointerdown', (e) => (touch = e.pointerType !== 'mouse'), true);
    const zr = map.getZr();
    // A mouse shows the list while it moves over the map. A click or tap
    // keeps the list for that spot until the next click, tap or pan.
    zr.on('mousemove', (e) => {
      if (!touch && !pinned) show(e.offsetX, e.offsetY);
    });
    zr.on('globalout', () => {
      if (!pinned) hide();
    });
    // zrender sends click only for a tap or click that did not move, so a
    // pan or pinch does not open the list.
    zr.on('click', (e) => {
      if (pinned) {
        hide();
        if (!touch) show(e.offsetX, e.offsetY);
        return;
      }
      show(e.offsetX, e.offsetY);
      pinned = box.style.display === 'block';
    });
    map.on('datazoom', () => {
      hide();
      drawLabels(el, map, div);
    });
    div.__explore = { labels, hide };
    return div.__explore;
  }

  // Names on the map, without overlapping each other or the reading's
  // marker: first the species nearest the current reading, then, when
  // zoomed in so few species are in view, the rest of them.
  function drawLabels(el, map, div) {
    const ex = div.__explore;
    if (!ex || !el.__species || !el.__last) return;
    const c = ex.labels;
    const dpr = window.devicePixelRatio || 1;
    const w = div.clientWidth;
    const h = div.clientHeight;
    if (c.width !== Math.round(w * dpr) || c.height !== Math.round(h * dpr)) {
      c.width = Math.round(w * dpr);
      c.height = Math.round(h * dpr);
      c.style.width = `${w}px`;
      c.style.height = `${h}px`;
    }
    const g = c.getContext('2d');
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    g.clearRect(0, 0, w, h);
    const sp = el.__species;
    const toPx = (s) => map.convertToPixel({ gridIndex: 0 }, [s[0], s[1]]);
    const taken = [];
    const free = (r) =>
      r.x >= 0 && r.y >= 0 && r.x + r.w <= w && r.y + r.h <= h &&
      !taken.some((t) => r.x < t.x + t.w && t.x < r.x + r.w && r.y < t.y + t.h && t.y < r.y + r.h);
    const want = [];
    const last = el.__last.trail[el.__last.trail.length - 1];
    if (last) {
      const [mx, my] = map.convertToPixel({ gridIndex: 0 }, [last[0], last[1]]);
      taken.push({ x: mx - 12, y: my - 12, w: 24, h: 24 });
      want.push(...nearestSpecies(el, last[0], last[1], 0.15, NEAR_READING));
    }
    const [x0, y1] = map.convertFromPixel({ gridIndex: 0 }, [0, 0]);
    const [x1, y0] = map.convertFromPixel({ gridIndex: 0 }, [w, h]);
    if ((x1 - x0) / 2.1 <= LABEL_ZOOM) {
      const inView = [];
      for (let i = 0; i < sp.length; i++) {
        const s = sp[i];
        if (s[0] >= x0 && s[0] <= x1 && s[1] >= y0 && s[1] <= y1) inView.push(i);
      }
      const mx = (x0 + x1) / 2;
      const my = (y0 + y1) / 2;
      const d = (i) => Math.hypot(sp[i][0] - mx, sp[i][1] - my);
      // Only the nearest few are tried, so a dense view costs little.
      want.push(...inView.sort((a, b) => d(a) - d(b)).slice(0, LABEL_MAX * 4));
    }
    g.font = '10px system-ui, sans-serif';
    g.textBaseline = 'middle';
    g.lineJoin = 'round';
    let drawn = 0;
    const seen = new Set();
    for (const k of want) {
      if (drawn >= LABEL_MAX || seen.has(k)) continue;
      seen.add(k);
      const [px, py] = toPx(sp[k]);
      const text = sp[k][2];
      const tw = g.measureText(text).width;
      const th = 12;
      // Beside the point, or further out when the reading's marker covers it
      // (the species nearest the reading are usually under the marker).
      const tries = [5, 14].flatMap((o) => [
        { x: px + o, y: py - th / 2 },
        { x: px - o - tw, y: py - th / 2 },
        { x: px - tw / 2, y: py - o - th },
        { x: px - tw / 2, y: py + o },
      ]);
      const spot = tries.map((t) => ({ ...t, w: tw, h: th })).find(free);
      if (!spot) continue;
      taken.push(spot);
      g.strokeStyle = 'rgba(0,0,0,0.85)';
      g.lineWidth = 3;
      g.strokeText(text, spot.x, spot.y + th / 2);
      g.fillStyle = INK;
      g.fillText(text, spot.x, spot.y + th / 2);
      drawn++;
    }
  }

  // Component maps: small copies of the species map, one per principal
  // component of real recordings' embeddings (research/live-embedding/
  // pc_rec.py; with ?pc=rows, of BirdNET's class rows, pc_bands.py). Each
  // lights the species whose place on that component is near the reading's
  // percentile among real recordings; species with no place stay dim. The
  // points are at their t-SNE places, as on the main map, or with
  // ?pcmap=umap at their UMAP places (species_umap.py). Plain canvas: each
  // group of points is drawn once into an offscreen canvas, so a reading
  // costs a few drawImage calls per map.
  //
  // What is lit (?pclight=):
  //   areas   (default) like regions, but each point takes the colour of the
  //           fifth concentrated around it on the map, so each map shows a
  //           few coloured areas; mixed areas stay grey. See PC_MODES.
  //   regions every species is always coloured by its fifth of
  //           the component (equal counts, blue at the low end, grey in the
  //           middle, orange at the high end), dimmed. The fifth the reading
  //           is in lights up at full strength, none while the reading is in
  //           the middle fifth. The line under each map uses the same
  //           colours.
  //   regions5 the same, with the 5% band at the reading in white on top.
  //   typical the outer third's species that sit among others of the same
  //           third, faint blue, plus the 5% band in yellow (see below).
  //   two     the third of the species the reading is in, faint,
  //           and the 5% band at the reading, bright on top. The middle
  //           third is not lit: only the ends of an axis say much about a
  //           sound, but the 5% band still shows where the reading is.
  //   outer   only the outer thirds, never the middle one.
  //   halves  the half below or above the median; nothing between the 40th
  //           and 60th percentiles.
  //   fine    5% bands, fading over 10% from the reading.
  //   typical like two, but the faint layer lights only the third's species
  //           that sit among others of the same third (the share of the
  //           third in the surrounding grid cells, above the 1 in 3 chance),
  //           so it shows the regions typical of that end of the axis.
  const query = new URLSearchParams(window.location.search);
  const PC_LIGHT = ['regions', 'areas', 'regions5', 'two', 'outer', 'halves', 'fine', 'typical'].includes(query.get('pclight'))
    ? query.get('pclight')
    : 'areas';
  const PC_UMAP = query.get('pcmap') === 'umap';
  // Two-level colours: a dark blue for the third and a light yellow for the
  // 5% band, so they differ in lightness as well as hue.
  const PC_THIRD = '#3f7fc0';
  const PC_FINE = '#ffe680';
  const PC_FINE_BANDS = 20;
  const PC_PER_ROW = 4;
  // Each mode: band layers to build (count, colour) and the middle zone in
  // which the coarse layer is not lit, as a distance from the median.
  // Five colours from the low end of a component to the high end: blue,
  // grey in the middle, orange. Blue against orange survives red-green
  // colour blindness, and the outer fifths are lighter than the inner ones,
  // so each arm also steps in lightness. The two steps within an arm are
  // close (checked with the dataviz validator: 10 to 11 apart where 15 is
  // the target), so the line under each map, in the same order, is the key.
  const PC_REGION_COLOURS = ['#4a8ee0', '#2f66b8', '#6a6963', '#b25e18', '#e8843c'];
  const PC_MODES = {
    regions: {
      regions: PC_REGION_COLOURS,
      dimAlpha: 0.3,
      middle: 0.1,
    },
    // The same with the 5% band at the reading in white on top. Off by
    // default: at this size its points scatter over the whole map.
    // Colour by area: each point takes the colour of the fifth that is
    // concentrated around it on the map (its share of the surrounding grid
    // cells furthest above 1 in 5), stronger the further above; points in
    // mixed areas stay grey.
    areas: {
      regions: PC_REGION_COLOURS,
      areas: true,
      dimAlpha: 0.3,
      middle: 0.1,
    },
    regions5: {
      regions: PC_REGION_COLOURS,
      dimAlpha: 0.3,
      fine: [PC_FINE_BANDS, '#ffffff'],
      fineFall: 0,
      middle: 0.1,
    },
    two: { coarse: [3, PC_THIRD], coarseAlpha: 0.75, fine: [PC_FINE_BANDS, PC_FINE], fineFall: 0.02, middle: 1 / 6 },
    outer: { coarse: [3, HIT], coarseAlpha: 1, middle: 1 / 6 },
    halves: { coarse: [2, HIT], coarseAlpha: 1, middle: 0.1 },
    fine: { fine: [PC_FINE_BANDS, HIT], fineFall: 0.1, middle: 0 },
    typical: {
      coarse: [3, PC_THIRD],
      coarseAlpha: 1,
      typical: true,
      fine: [PC_FINE_BANDS, PC_FINE],
      fineFall: 0.02,
      middle: 1 / 6,
    },
  };
  // Grid for the typical regions: cells per side, and how many cells around
  // a point count as its surroundings (1: a 3 x 3 block).
  const PC_GRID = 24;
  // Areas look: below this strength (0 at chance, 1 when the surrounding
  // cells are all one fifth) a point stays grey.
  const PC_AREA_MIN = 0.15;
  const PC_GRID_REACH = 1;
  const PC_MODE = PC_MODES[PC_LIGHT];
  // What lies at the two ends of each class-row component (pc_bands.txt).
  const PC_ENDS_ROWS = [
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
    const pts = PC_UMAP && pc.umap ? pc.umap : species;
    // `list` holds points [x, y] or, with per-point strength, [x, y, alpha].
    const layer = (list, color, r) => {
      const c = document.createElement('canvas');
      c.width = c.height = px;
      const g = c.getContext('2d');
      g.fillStyle = color;
      for (const p of list) {
        if (p.length > 2) {
          if (p[2] <= 0) continue;
          g.globalAlpha = p[2];
        }
        g.fillRect(at(p[0]) - r / 2, at(-p[1]) - r / 2, r, r);
      }
      return c;
    };
    const cellOf = (p) => {
      const c = (v) => Math.min(PC_GRID - 1, Math.max(0, Math.floor(((v + 1.05) / 2.1) * PC_GRID)));
      return [c(p[0]), c(p[1])];
    };
    const dot = Math.max(1, dpr);
    const base = layer(pts, 'rgba(170,170,170,0.22)', dot);
    const rows = pc.basis === 'rows';
    const top = rows ? 256 : 255;
    // Species per band, for `count` equal bands of the placed species.
    const bandOf = (r, count) => Math.min(count - 1, Math.floor((r * count) / top));
    const groups = (k, count) => {
      const g = Array.from({ length: count }, () => []);
      pc.ranks[k].forEach((r, i) => {
        if (!rows && r === 255) return; // no place on this component
        g[bandOf(r, count)].push(pts[i]);
      });
      return g;
    };
    // Each band's points with a strength: how far the band's share of the
    // placed species in the surrounding cells is above chance (1 / count).
    const typicalGroups = (k, count) => {
      const counts = Array.from({ length: count + 1 }, () => new Float32Array(PC_GRID * PC_GRID));
      const cells = pts.map(cellOf);
      pc.ranks[k].forEach((r, i) => {
        if (!rows && r === 255) return;
        const [cx, cy] = cells[i];
        counts[bandOf(r, count)][cy * PC_GRID + cx] += 1;
        counts[count][cy * PC_GRID + cx] += 1;
      });
      const around = (arr, cx, cy) => {
        let s = 0;
        for (let dy = -PC_GRID_REACH; dy <= PC_GRID_REACH; dy++)
          for (let dx = -PC_GRID_REACH; dx <= PC_GRID_REACH; dx++) {
            const x = cx + dx;
            const y = cy + dy;
            if (x >= 0 && y >= 0 && x < PC_GRID && y < PC_GRID) s += arr[y * PC_GRID + x];
          }
        return s;
      };
      const g = Array.from({ length: count }, () => []);
      const chance = 1 / count;
      pc.ranks[k].forEach((r, i) => {
        if (!rows && r === 255) return;
        const b = bandOf(r, count);
        const [cx, cy] = cells[i];
        const share = around(counts[b], cx, cy) / Math.max(1, around(counts[count], cx, cy));
        g[b].push([pts[i][0], pts[i][1], Math.min(1, (2 * (share - chance)) / (1 - chance))]);
      });
      return g;
    };
    div.innerHTML = '';
    const maps = [];
    // Share of species in mixed areas per component (areas look only).
    const neutral = [];
    for (let k = 0; k < n; k++) {
      const m = PC_MODE;
      const coarseGroups = m.coarse && (m.typical ? typicalGroups : groups)(k, m.coarse[0]);
      let coarse = coarseGroups ? coarseGroups.map((g) => layer(g, m.coarse[1], dot)) : [];
      // Regions: one dim image of all five fifths, one bright image each.
      let dim = null;
      if (m.areas) {
        // Dominant fifth around each point, and how far above chance.
        const n = m.regions.length;
        const counts = Array.from({ length: n + 1 }, () => new Float32Array(PC_GRID * PC_GRID));
        const cells = pts.map(cellOf);
        const bands = pc.ranks[k].map((r) => (!rows && r === 255 ? -1 : bandOf(r, n)));
        bands.forEach((b, i) => {
          if (b < 0) return;
          const [cx, cy] = cells[i];
          counts[b][cy * PC_GRID + cx] += 1;
          counts[n][cy * PC_GRID + cx] += 1;
        });
        const around = (arr, cx, cy) => {
          let t = 0;
          for (let dy = -PC_GRID_REACH; dy <= PC_GRID_REACH; dy++)
            for (let dx = -PC_GRID_REACH; dx <= PC_GRID_REACH; dx++) {
              const x = cx + dx;
              const y = cy + dy;
              if (x >= 0 && y >= 0 && x < PC_GRID && y < PC_GRID) t += arr[y * PC_GRID + x];
            }
          return t;
        };
        const chance = 1 / n;
        const byBand = Array.from({ length: n }, () => []);
        const grey = [];
        let placed = 0;
        bands.forEach((b, i) => {
          if (b < 0) return;
          placed++;
          const [cx, cy] = cells[i];
          const all = Math.max(1, around(counts[n], cx, cy));
          let best = 0;
          let bestShare = -1;
          for (let j = 0; j < n; j++) {
            const sh = around(counts[j], cx, cy) / all;
            if (sh > bestShare) {
              bestShare = sh;
              best = j;
            }
          }
          // 0 at chance, 1 when the area is all one fifth.
          const strength = Math.min(1, (bestShare - chance) / (1 - chance));
          if (strength < PC_AREA_MIN) grey.push(pts[i]);
          else byBand[best].push([pts[i][0], pts[i][1], Math.min(1, 0.35 + strength)]);
        });
        coarse = byBand.map((g, i) => layer(g, m.regions[i], dot));
        dim = document.createElement('canvas');
        dim.width = dim.height = px;
        const dg = dim.getContext('2d');
        // Mixed areas: a fainter grey than the middle fifth's colour, so the
        // two do not read as the same thing.
        dg.globalAlpha = 0.45;
        dg.drawImage(layer(grey, '#555555', dot), 0, 0);
        dg.globalAlpha = m.dimAlpha * 1.5;
        [2, 1, 3, 0, 4].forEach((i) => dg.drawImage(coarse[i], 0, 0));
        neutral.push(placed ? grey.length / placed : 1);
      } else if (m.regions) {
        const fifths = groups(k, m.regions.length);
        coarse = fifths.map((g, i) => layer(g, m.regions[i], dot));
        dim = document.createElement('canvas');
        dim.width = dim.height = px;
        const dg = dim.getContext('2d');
        dg.globalAlpha = m.dimAlpha;
        // Middle fifth first and the outer fifths last, so where points
        // overlap the ends of the component show.
        [2, 1, 3, 0, 4].forEach((i) => coarse[i] && dg.drawImage(coarse[i], 0, 0));
      }
      const fine = m.fine ? groups(k, m.fine[0]).map((g) => layer(g, m.fine[1], dot)) : [];
      const cell = document.createElement('div');
      cell.className = 'live-emb-pc';
      const [lo, hi] = (rows && PC_ENDS_ROWS[k]) || ['', ''];
      const between = rows ? "BirdNET's classes" : 'recordings';
      cell.title =
        `PC${k + 1}: ${(pc.variance[k] * 100).toFixed(1)}% of the variation between ${between}.` +
        (lo ? `\nLow end: ${lo}.\nHigh end: ${hi}.` : '');
      const label = document.createElement('div');
      label.className = 'live-emb-pc-label';
      // Whole percent, so the label fits a quarter of the sidebar.
      label.textContent = `PC${k + 1} ${Math.round(pc.variance[k] * 100)}%`;
      const canvas = document.createElement('canvas');
      canvas.width = canvas.height = px;
      canvas.style.width = canvas.style.height = `${size}px`;
      // The reading's percentile on this component, 0 at the left.
      const line = document.createElement('canvas');
      line.className = 'live-emb-pc-line';
      line.width = px;
      line.height = Math.round(7 * dpr);
      line.style.width = `${size}px`;
      line.style.height = '7px';
      cell.append(label, canvas, line);
      div.appendChild(cell);
      maps.push({ ctx: canvas.getContext('2d'), coarse, dim, fine, line: line.getContext('2d') });
    }
    return { width: div.clientWidth, base, maps, px, dpr, neutral };
  }

  // Draw band layers lit by distance from the reading `p`: 1 inside the
  // band, fading to 0 at `fall` away (0: only the band holding `p`).
  function drawBands(ctx, layers, p, fall, alpha) {
    const count = layers.length;
    layers.forEach((band, i) => {
      const dist = Math.max(0, i / count - p, p - (i + 1) / count);
      const a = fall > 0 ? 1 - dist / fall : dist > 0 ? 0 : 1;
      if (a <= 0) return;
      ctx.globalAlpha = a * alpha;
      ctx.drawImage(band, 0, 0);
    });
  }

  function pcDraw(el, d) {
    const div = el.__parts.pcs;
    const pc = el.__pc;
    if (!pc || !el.__species || !d.pc || !d.pc.length) return;
    if (!div.__built || div.__built.width !== div.clientWidth) div.__built = pcBuild(div, el.__species, pc);
    const b = div.__built;
    if (!b) return;
    const mode = PC_MODE;
    b.maps.forEach((m, k) => {
      const p = d.pc[k];
      const inMiddle = Math.abs(p - 0.5) < mode.middle;
      m.ctx.globalAlpha = 1;
      m.ctx.clearRect(0, 0, b.px, b.px);
      m.ctx.drawImage(b.base, 0, 0);
      if (mode.regions) {
        m.ctx.drawImage(m.dim, 0, 0);
        const fifth = Math.min(mode.regions.length - 1, Math.floor(p * mode.regions.length));
        // No fifth lights while the reading is in the middle one.
        if (fifth !== (mode.regions.length - 1) / 2) m.ctx.drawImage(m.coarse[fifth], 0, 0);
      } else if (!inMiddle) drawBands(m.ctx, m.coarse, p, 0, mode.coarseAlpha);
      drawBands(m.ctx, m.fine, p, mode.fineFall || 0, 1);
      // The percentile line: a grey rule, a tick at the median, the middle
      // zone (where the coarse layer is not lit) a little lighter, and the
      // reading as a mark.
      const g = m.line;
      const w = b.px;
      const h = g.canvas.height;
      const y = Math.round(h / 2);
      const u = b.dpr;
      g.clearRect(0, 0, w, h);
      g.fillStyle = '#555';
      g.fillRect(0, y - u / 2, w, u);
      if (mode.regions) {
        // The line is the key: one segment per fifth, in its colour.
        const n = mode.regions.length;
        mode.regions.forEach((c, i) => {
          g.fillStyle = c;
          g.fillRect((i * w) / n, y - u, w / n - u, 2 * u);
        });
      } else if (mode.middle > 0) {
        g.fillStyle = '#777';
        g.fillRect(w * (0.5 - mode.middle), y - u, w * 2 * mode.middle, 2 * u);
      }
      if (!mode.regions) {
        g.fillStyle = '#888';
        g.fillRect(Math.round(w / 2) - u / 2, 0, u, h);
      }
      g.fillStyle = mode.regions ? '#ffffff' : mode.fine ? mode.fine[1] : inMiddle ? '#bbbbbb' : HIT;
      const x = Math.min(w - 2 * u, Math.max(0, p * w - u));
      g.fillRect(x, 0, 2 * u, h);
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
        mapExplore(el, map, p.map);
      } else if (map.__ready) {
        map.setOption({ series: mapSeries(d) });
      }
      el.__ms = performance.now() - t0;
      const tl = performance.now();
      if (map.__ready) drawLabels(el, map, p.map);
      el.__labelMs = performance.now() - tl;
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
