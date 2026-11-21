// Live view (experimental) for BirdNET or Perch, drawn with Apache ECharts
// (vendor/, Apache-2.0), loaded the first time the view is shown.
//
// Called from src/live_embedding.rs:
//   oversampleLiveEmbedding.setMapHost(el, host)  (where the map goes)
//   oversampleLiveEmbedding.setPins(el, pinsJson)  (pins on the component maps)
//   oversampleLiveEmbedding.update(el, json, speciesJson, pcJson)
//   oversampleLiveEmbedding.dispose(el)  (also when the model changes)
// The reading's top classes are drawn by the Call detection tab itself
// (live_view.rs); `top` below is not drawn here.
// `json` is { min, top: [[name, score, unlikelyHere]...],
//   trail: [[x, y, secondsAgo, score, umapX, umapY]...] (x, y on the t-SNE layout),
//   rows: [name...], rowsUnlikely: [bool...], heard (any reading in the history),
//   strip: [[secondsAgo, row, score]...],
//   change: [[secondsAgo, distance]...], history,
//   pcHist: [[secondsAgo, percentile on PC1, on PC2, ...]...] (0 to 1),
//   preview: null, or while nothing plays the reading of the 3 s Play would
//     start with: { at: { x, y, ux, uy, s } or null, range ("12.4s to 15.4s") };
//     `top` is then that reading's,
//   pending: paused and that reading not in yet; `top` is then empty and the
//     map shows no marker,
//   model: { name, hearing (said for bat recordings), jump: [t-SNE, UMAP]
//     (placement radius on each layout) } }
// `speciesJson` is [[x, y, name, scientific]...] for the map's species (t-SNE
// places), or null when it was sent before. `pcJson` comes with it:
// { variance: [share...] (empty without components), ranks: [[0..255 per
// species]...], basis: 'rows' | 'recordings', umap: [[x, y]...] or null (UMAP
// places), ends: [[low end, high end]...] per component, or empty }.
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
  // The "unlikely here" flag: the colour of .unlikely-flag (style.css) and
  // the legend's wording (species_map.rs, classifier_section.rs).
  const FLAG_COLOUR = '#ffcc88';
  const FLAG_GLYPH = '\u2691';
  const UNLIKELY_TEXT = 'Unlikely at this place and month';
  // The app's pin icon (Icon::Pin in icons.rs).
  const PIN_SVG =
    '<svg class="icon" viewBox="0 0 24 24" width="1em" height="1em" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 17v5"/><path class="icon-pin-head" d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"/></svg>';
  // The paused reading's marker: a play triangle in the blue of the
  // spectrogram's dashed "here" line (rgba(100, 160, 255) in
  // spectrogram.rs). A triangle, not a ring, so it differs from the dashed
  // ring of a live reading below the minimum confidence, and from the blue
  // of the component colours.
  const PLAY_BLUE = '#64a0ff';
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
    const unlikely = d.rowsUnlikely || [];
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
            // The "unlikely here" flag after the name, in the colour of
            // .unlikely-flag. A text glyph: axis labels are drawn on canvas.
            formatter: (v, i) => (unlikely[i] ? `${v} {flag|${FLAG_GLYPH}}` : v),
            rich: { flag: { color: FLAG_COLOUR, fontSize: 11 } },
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
        ...(rows.length || !d.heard
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
              `${esc(rows[p.value[1]])}<br>${p.value[2].toFixed(2)} <span style="color:${MUTED}">${ago(p.value[0])}</span>` +
              (unlikely[p.value[1]] ? `<br><span style="color:${FLAG_COLOUR}">${FLAG_GLYPH}</span> ${UNLIKELY_TEXT}` : ''),
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


  // ── Layout ────────────────────────────────────────────────────────────
  // The species are drawn at their UMAP places (species_umap.py) or their
  // t-SNE places (research/species-map). The choice is saved on this device.
  // `?pcmap=tsne` or `?pcmap=umap` in the address sets it for one visit.
  const query = new URLSearchParams(window.location.search);
  const LAYOUT_KEY = 'oversample.liveMap.layout';
  const LAYOUTS = [
    ['umap', 'UMAP'],
    ['tsne', 't-SNE'],
  ];
  let layout = 'umap';
  try {
    const saved = query.get('pcmap') || window.localStorage.getItem(LAYOUT_KEY);
    if (saved === 'tsne' || saved === 'umap') layout = saved;
  } catch (_) {
    // Storage is blocked: the default layout is used.
  }
  const isUmap = (el) => layout === 'umap' && !!(el.__pc && el.__pc.umap);
  // Jumps longer than this on the map are not joined by the trail line, so
  // it never crosses the gap between unrelated groups (the same radius as
  // placement: species sit closer together on the UMAP layout).
  const jump = (el) => {
    const j = (el.__last && el.__last.model && el.__last.model.jump) || [0.15, 0.11];
    return isUmap(el) ? j[1] : j[0];
  };

  // The species' places on the active layout: [[x, y]...].
  function points(el) {
    const name = isUmap(el) ? 'umap' : 'tsne';
    if (!el.__pts || el.__pts.name !== name) {
      el.__pts = { name, pts: name === 'umap' ? el.__pc.umap : el.__species };
      el.__grid = null;
      el.__colours = null;
    }
    return el.__pts.pts;
  }

  // The trail on the active layout: [[x, y, secondsAgo, score]...].
  const trailOf = (el, d) => (isUmap(el) ? d.trail.map((p) => [p[4], p[5], p[2], p[3]]) : d.trail);

  // The paused reading's place on the active layout, [x, y], or null.
  function previewAt(el, d) {
    const a = d.preview && d.preview.at;
    if (!a) return null;
    return isUmap(el) ? [a.ux, a.uy] : [a.x, a.y];
  }

  // Where the map's marker is: the paused reading's place while it is
  // shown, else the newest live reading's. [x, y] or undefined.
  function markerAt(el, d) {
    if (d.pending) return undefined;
    if (d.preview) return previewAt(el, d) || undefined;
    const trail = trailOf(el, d);
    return trail[trail.length - 1];
  }

  function mapSeries(el, d) {
    // While the paused reading is shown, it is the only marker: the trail
    // is of the audio heard before the pause.
    const trail = d.preview || d.pending ? [] : trailOf(el, d);
    const last = trail[trail.length - 1];
    const play = previewAt(el, d);
    const hit = last && last[3] >= d.min;
    const far = jump(el);
    // The line joins readings that are close on the map; a null breaks it.
    const line = [];
    trail.forEach((p, i) => {
      const q = trail[i - 1];
      if (q && Math.hypot(p[0] - q[0], p[1] - q[1]) > far) line.push(null);
      line.push(p);
    });
    return [
      {
        id: 'trailLine',
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
        type: 'scatter',
        silent: true,
        data: play
          ? [
              {
                value: play,
                // A triangle pointing right, as a path so no rotation is needed.
                symbol: 'path://M0,0 L10,6 L0,12 Z',
                symbolSize: [14, 16],
                itemStyle: {
                  color: 'transparent',
                  borderColor: PLAY_BLUE,
                  borderWidth: 2,
                  shadowColor: 'rgba(0,0,0,0.9)',
                  shadowBlur: 3,
                },
              },
            ]
          : last
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

  // ECharts draws only the moving marker and trail, and handles zoom and
  // pan. The species are drawn on a canvas under it (drawSpecies), so they
  // can be coloured by a component and are redrawn only when the view moves.
  function mapOptions(el, d) {
    return {
      animation: false,
      backgroundColor: 'transparent',
      grid: { left: 2, right: 2, top: 2, bottom: 2 },
      xAxis: { type: 'value', show: false, min: -FULL, max: FULL },
      yAxis: { type: 'value', show: false, min: -FULL, max: FULL },
      dataZoom: [
        { type: 'inside', xAxisIndex: 0, filterMode: 'none' },
        { type: 'inside', yAxisIndex: 0, filterMode: 'none' },
      ],
      // No ECharts tooltip: pointing anywhere lists the nearest species
      // (mapExplore below).
      tooltip: { show: false },
      series: mapSeries(el, d),
    };
  }

  // Pointing at the species map. Hovering (mouse) or tapping (touch)
  // anywhere lists the species nearest that point, nearest first, with the
  // nearest ringed on the map. A click or tap keeps the list. A kept list
  // has a Close button and a "find recordings" button beside each species,
  // and closes on Escape, on a press anywhere outside it, and on a pan.
  // Names are also written on the map: the few species nearest the current
  // reading, and every species in view once zoomed in far enough. Lookups use
  // a grid of the species, built once per layout; the overlay is plain DOM
  // and canvas over the ECharts map.
  const FULL = 1.05; // the map runs from -FULL to FULL on both axes
  const NEAR_PX = 48; // how far from the pointer a species can be listed
  const NEAR_MAX = 5; // species in the list
  const NEAR_READING = 3; // species named beside the current reading
  // Name species in view once zoomed in this far (the share of the whole
  // map's width in view), nearest the middle of the view first, up to
  // LABEL_MAX that fit without overlapping.
  const LABEL_ZOOM = 0.5;
  const LABEL_MAX = 12; // most names on the map at once
  const GRID_N = 40;
  const gridCell = (v, n) => Math.min(n - 1, Math.max(0, Math.floor(((v + FULL) / (2 * FULL)) * n)));

  function speciesGrid(pts) {
    const cells = Array.from({ length: GRID_N * GRID_N }, () => []);
    pts.forEach((s, i) => cells[gridCell(s[1], GRID_N) * GRID_N + gridCell(s[0], GRID_N)].push(i));
    return cells;
  }

  // Species within `r` (map units) of (x, y), nearest first, at most `max`.
  function nearestSpecies(el, x, y, r, max) {
    const pts = points(el);
    const cells = el.__grid || (el.__grid = speciesGrid(pts));
    const span = Math.ceil((r / (2 * FULL)) * GRID_N);
    const cx = gridCell(x, GRID_N);
    const cy = gridCell(y, GRID_N);
    const found = [];
    for (let j = Math.max(0, cy - span); j <= Math.min(GRID_N - 1, cy + span); j++)
      for (let i = Math.max(0, cx - span); i <= Math.min(GRID_N - 1, cx + span); i++)
        for (const k of cells[j * GRID_N + i]) {
          const d = Math.hypot(pts[k][0] - x, pts[k][1] - y);
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

  // The part of the map in view: { x0, x1, y0, y1 } in map units.
  function viewOf(map, div) {
    const [x0, y1] = map.convertFromPixel({ gridIndex: 0 }, [0, 0]);
    const [x1, y0] = map.convertFromPixel({ gridIndex: 0 }, [div.clientWidth, div.clientHeight]);
    return { x0, x1, y0, y1 };
  }
  const WHOLE = { x0: -FULL, x1: FULL, y0: -FULL, y1: FULL };
  // Share of the whole map's width in view: 1 zoomed out.
  const viewShare = (v) => Math.min(1, Math.abs(v.x1 - v.x0) / (2 * FULL));

  const SEARCH_ICON =
    '<svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true"><circle cx="7" cy="7" r="4.5" fill="none" stroke="currentColor" stroke-width="1.6"/><path d="M10.5 10.5 14 14" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>';

  // Common name, then the scientific name when it differs. In a kept list
  // each row also has a button that opens Find recordings for the species.
  function speciesRow(s, k, kept) {
    const sci = s[3] && s[3] !== s[2] ? `<span class="live-emb-near-sci">${esc(s[3])}</span>` : '';
    const find = kept
      ? `<button class="live-emb-near-find" data-find="${k}" title="Find recordings of ${esc(s[2])}" aria-label="Find recordings of ${esc(s[2])}">${SEARCH_ICON}</button>`
      : '';
    return `<div class="live-emb-near-row"><span class="live-emb-near-text"><span class="live-emb-near-name">${esc(s[2])}</span>${sci}</span>${find}</div>`;
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
    const key = mk('div', 'live-emb-key');
    let touch = false;
    let pinned = false;
    let shown = null; // the pixel the list was last shown for
    const hide = () => {
      box.style.display = ring.style.display = 'none';
      box.classList.remove('kept');
      pinned = false;
      shown = null;
    };
    const show = (px, py, keep) => {
      if (!el.__species) return;
      const [x, y] = map.convertFromPixel({ gridIndex: 0 }, [px, py]);
      const near = nearestSpecies(el, x, y, NEAR_PX * unitsPerPx(map), NEAR_MAX);
      if (!near.length) return hide();
      const sp = el.__species;
      const pts = points(el);
      box.classList.toggle('kept', !!keep);
      box.innerHTML =
        (keep
          ? '<div class="live-emb-near-head"><span>Nearest species</span><button class="live-emb-near-close" title="Close" aria-label="Close">×</button></div>'
          : '') + near.map((k) => speciesRow(sp[k], k, keep)).join('');
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
      const [rx, ry] = map.convertToPixel({ gridIndex: 0 }, [pts[near[0]][0], pts[near[0]][1]]);
      ring.style.display = 'block';
      ring.style.transform = `translate(${rx}px, ${ry}px)`;
      shown = [px, py];
      pinned = !!keep;
    };
    div.addEventListener('pointerdown', (e) => (touch = e.pointerType !== 'mouse'), true);
    box.addEventListener('click', (e) => {
      const find = e.target.closest('[data-find]');
      if (find) {
        const s = el.__species[Number(find.dataset.find)];
        // The dialog opens over the app, so leave full screen first.
        setFull(el, false);
        window.dispatchEvent(new CustomEvent('oversample-find-species', { detail: s[3] || s[2] }));
        hide();
      } else if (e.target.closest('.live-emb-near-close')) hide();
    });
    // A press outside a kept list closes it. Presses on the map are left to
    // the map's own click below, so a pan does not count.
    const outside = (e) => {
      if (pinned && !div.contains(e.target)) hide();
    };
    const escape = (e) => {
      if (e.key !== 'Escape') return;
      if (pinned) hide();
      else setFull(el, false);
    };
    document.addEventListener('pointerdown', outside, true);
    document.addEventListener('keydown', escape);
    const zr = map.getZr();
    // A mouse shows the list while it moves over the map. A click or tap
    // keeps the list for that spot.
    // The pointer's place is measured from the map's box at the time of the
    // event: the map moves down the sidebar when the strip above it gains a
    // row, and zrender's own offsets can then be a row out.
    const at = (e) => {
      const r = div.getBoundingClientRect();
      const n = e.event;
      return n && n.clientX != null ? [n.clientX - r.left, n.clientY - r.top] : [e.offsetX, e.offsetY];
    };
    zr.on('mousemove', (e) => {
      if (!touch && !pinned) show(...at(e), false);
    });
    zr.on('globalout', () => {
      if (!pinned) hide();
    });
    // zrender sends click only for a tap or click that did not move, so a
    // pan or pinch does not open the list.
    zr.on('click', (e) => {
      if (pinned) {
        hide();
        if (!touch) show(...at(e), false);
        return;
      }
      show(...at(e), true);
    });
    map.on('datazoom', () => {
      hide();
      viewChanged(el);
    });
    div.__explore = {
      labels,
      key,
      hide,
      remove() {
        document.removeEventListener('pointerdown', outside, true);
        document.removeEventListener('keydown', escape);
      },
    };
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
    const pts = points(el);
    const toPx = (i) => map.convertToPixel({ gridIndex: 0 }, [pts[i][0], pts[i][1]]);
    const taken = [];
    const free = (r) =>
      r.x >= 0 && r.y >= 0 && r.x + r.w <= w && r.y + r.h <= h &&
      !taken.some((t) => r.x < t.x + t.w && t.x < r.x + r.w && r.y < t.y + t.h && t.y < r.y + r.h);
    // Names keep clear of the colour key in the corner.
    if (ex.key.style.display === 'flex') {
      taken.push({ x: ex.key.offsetLeft, y: ex.key.offsetTop, w: ex.key.offsetWidth, h: ex.key.offsetHeight });
    }
    const want = [];
    const last = markerAt(el, el.__last);
    if (last) {
      const [mx, my] = map.convertToPixel({ gridIndex: 0 }, [last[0], last[1]]);
      taken.push({ x: mx - 12, y: my - 12, w: 24, h: 24 });
      want.push(...nearestSpecies(el, last[0], last[1], jump(el), NEAR_READING));
    }
    const v = viewOf(map, div);
    if (viewShare(v) <= LABEL_ZOOM) {
      const inView = [];
      for (let i = 0; i < pts.length; i++) {
        const s = pts[i];
        if (s[0] >= v.x0 && s[0] <= v.x1 && s[1] >= v.y0 && s[1] <= v.y1) inView.push(i);
      }
      const mx = (v.x0 + v.x1) / 2;
      const my = (v.y0 + v.y1) / 2;
      const d = (i) => Math.hypot(pts[i][0] - mx, pts[i][1] - my);
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
      const [px, py] = toPx(k);
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

  // ── Component colours ─────────────────────────────────────────────────
  // Each principal component (of BirdNET's class rows, pc_bands.py; or with
  // ?pc=rec of real recordings' embeddings, pc_rec.py) ranks the species.
  // The species are split into five equal fifths per component, coloured
  // from the low end to the high end: blue, grey in the middle, orange.
  // Blue against orange survives red-green colour blindness, and the outer
  // fifths are lighter than the inner ones, so each arm also steps in
  // lightness. The two steps within an arm are close (dataviz validator: 10
  // to 11 apart where 15 is the target), so the key beside each graph, in
  // the same order, says which is which.
  //
  // A map coloured by a component has two looks, blended by zoom:
  //   areas   (zoomed out) each species takes the colour of the fifth that
  //           is concentrated around it on the map (its share of the species
  //           in the surrounding 3 x 3 grid cells furthest above 1 in 5),
  //           stronger the further above; species in mixed areas are a
  //           faint grey. The map shows a few coloured areas, not speckle.
  //   species (zoomed in) each species has the colour of its own fifth.
  // The blend runs from all areas with 60% of the map's width in view to
  // all species at 30%.
  const PC_COLOURS = ['#4a8ee0', '#2f66b8', '#6a6963', '#b25e18', '#e8843c'];
  const PC_FIFTHS = ['lowest', 'low', 'middle', 'high', 'highest'];
  const PC_MIXED = '#555555';
  const PC_PLAIN = 'rgba(170,170,170,0.3)';
  // Component maps per row: at least 4 (the sidebar), more where they fit
  // at PC_MIN_CELL px or wider (full screen), with the rows filled evenly.
  // 12 maps: 3 rows of 4 in the sidebar, 2 rows of 6 in a 440 px column.
  // 8 maps: 2 rows of 4 in both.
  const PC_MIN_COLS = 4;
  const PC_MIN_CELL = 70;
  function pcColumns(n, width, gap) {
    const fit = Math.max(PC_MIN_COLS, Math.floor((width + gap) / (PC_MIN_CELL + gap)));
    const rows = Math.ceil(n / Math.min(fit, n));
    return Math.ceil(n / rows);
  }
  // Cells per side of the grid that areas are worked out on.
  const PC_GRID = 24;
  const PC_GRID_REACH = 1; // cells around a cell that count as its surroundings
  // Below this strength (0 at chance, 1 when the surrounding cells are all
  // one fifth) an area is mixed.
  const PC_AREA_MIN = 0.15;
  const AREAS_FROM = 0.6;
  const SPECIES_AT = 0.3;
  // Middle fifth first and the outer fifths last, so where points overlap
  // the ends of the component show.
  const PC_ORDER = [2, 1, 3, 0, 4];

  // Per component: each species' own fifth, and the fifth and strength of
  // the area around it. Worked out once per layout.
  function colours(el) {
    const pts = points(el);
    if (el.__colours) return el.__colours;
    const pc = el.__pc;
    const rows = pc.basis === 'rows';
    const top = rows ? 256 : 255;
    const n = PC_COLOURS.length;
    const cellOf = pts.map((p) => gridCell(p[1], PC_GRID) * PC_GRID + gridCell(p[0], PC_GRID));
    el.__colours = pc.ranks.map((ranks) => {
      // -1: no place on this component (recordings basis only).
      const band = ranks.map((r) => (!rows && r === 255 ? -1 : Math.min(n - 1, Math.floor((r * n) / top))));
      const counts = Array.from({ length: n + 1 }, () => new Float32Array(PC_GRID * PC_GRID));
      band.forEach((b, i) => {
        if (b < 0) return;
        counts[b][cellOf[i]] += 1;
        counts[n][cellOf[i]] += 1;
      });
      const around = (arr, c) => {
        const cx = c % PC_GRID;
        const cy = Math.floor(c / PC_GRID);
        let t = 0;
        for (let dy = -PC_GRID_REACH; dy <= PC_GRID_REACH; dy++)
          for (let dx = -PC_GRID_REACH; dx <= PC_GRID_REACH; dx++) {
            const x = cx + dx;
            const y = cy + dy;
            if (x >= 0 && y >= 0 && x < PC_GRID && y < PC_GRID) t += arr[y * PC_GRID + x];
          }
        return t;
      };
      // Per grid cell: the fifth with the largest share of the species in
      // and around the cell, that share, and how many species that is of.
      const cells = Array.from({ length: PC_GRID * PC_GRID }, (_, c) => {
        const all = around(counts[n], c);
        let best = 0;
        let share = 0;
        for (let j = 0; j < n; j++) {
          const sh = all ? around(counts[j], c) / all : 0;
          if (sh > share) {
            share = sh;
            best = j;
          }
        }
        const chance = 1 / n;
        // 0 at chance, 1 when the area is all one fifth.
        const strength = Math.min(1, Math.max(0, (share - chance) / (1 - chance)));
        return { all, best, share, strength };
      });
      // Species indices by look: own[j] by their own fifth; area[j] by the
      // area's fifth, with a strength each; mixed and unplaced apart.
      const own = Array.from({ length: n }, () => []);
      const area = Array.from({ length: n }, () => []);
      const mixed = [];
      const unplaced = [];
      band.forEach((b, i) => {
        if (b < 0) return unplaced.push(i);
        own[b].push(i);
        const c = cells[cellOf[i]];
        if (c.strength < PC_AREA_MIN) mixed.push(i);
        else area[c.best].push([i, Math.min(1, 0.45 + c.strength)]);
      });
      const placed = band.length - unplaced.length;
      return { band, cells, own, area, mixed, unplaced, mixedShare: placed ? mixed.length / placed : 1 };
    });
    return el.__colours;
  }

  // 0: the areas look; 1: every species in its own fifth's colour.
  const speciesBlend = (v) =>
    Math.min(1, Math.max(0, (AREAS_FROM - viewShare(v)) / (AREAS_FROM - SPECIES_AT)));
  // Points grow a little as the view zooms in.
  const pointScale = (v) => Math.min(1.5, 1 / Math.sqrt(viewShare(v)));

  // Draw the species into a `w` x `h` pixel canvas showing the part `v` of
  // the map, plain grey or (with `comp`, one of colours(el)) coloured.
  function paintSpecies(g, w, h, v, pts, comp, size) {
    const ax = w / (v.x1 - v.x0);
    const ay = h / (v.y1 - v.y0);
    const half = size / 2;
    const put = (i) => {
      const x = (pts[i][0] - v.x0) * ax;
      const y = (v.y1 - pts[i][1]) * ay;
      if (x < -size || y < -size || x > w + size || y > h + size) return;
      g.fillRect(x - half, y - half, size, size);
    };
    g.clearRect(0, 0, w, h);
    if (!comp) {
      g.globalAlpha = 1;
      g.fillStyle = PC_PLAIN;
      for (let i = 0; i < pts.length; i++) put(i);
      return;
    }
    const t = speciesBlend(v);
    g.fillStyle = PC_MIXED;
    g.globalAlpha = 0.3;
    comp.unplaced.forEach(put);
    if (t < 1) {
      g.globalAlpha = 0.45 * (1 - t);
      comp.mixed.forEach(put);
      for (const j of PC_ORDER) {
        g.fillStyle = PC_COLOURS[j];
        for (const [i, a] of comp.area[j]) {
          g.globalAlpha = a * (1 - t);
          put(i);
        }
      }
    }
    if (t > 0) {
      g.globalAlpha = 0.9 * t;
      for (const j of PC_ORDER) {
        g.fillStyle = PC_COLOURS[j];
        comp.own[j].forEach(put);
      }
    }
    g.globalAlpha = 1;
  }

  // The species on the large map: a canvas under the ECharts layers,
  // coloured by the component map that is selected (el.__sel), if any.
  function drawSpecies(el) {
    const p = el.__parts;
    const map = p && window.echarts.getInstanceByDom(p.map);
    if (!map || !map.__ready || !el.__species) return;
    const div = p.map;
    let c = div.__species;
    if (!c) {
      c = div.__species = document.createElement('canvas');
      c.className = 'live-emb-species';
      div.insertBefore(c, div.firstChild);
    }
    const dpr = window.devicePixelRatio || 1;
    const w = Math.round(div.clientWidth * dpr);
    const h = Math.round(div.clientHeight * dpr);
    if (c.width !== w || c.height !== h) {
      c.width = w;
      c.height = h;
    }
    const v = viewOf(map, div);
    const comp = el.__sel != null && el.__pc ? colours(el)[el.__sel] : null;
    // 3 px points on a sidebar map, larger on a full-screen one.
    const size = Math.max(3, div.clientWidth / 160);
    paintSpecies(c.getContext('2d'), w, h, v, points(el), comp, size * dpr * pointScale(v));
    // The key for the colours, in a corner of the map.
    const key = div.__explore && div.__explore.key;
    if (key) {
      key.style.display = comp ? 'flex' : 'none';
      if (comp && key.__for !== el.__sel) {
        key.__for = el.__sel;
        key.innerHTML =
          `<span>PC${el.__sel + 1}</span><span>low</span>` +
          PC_COLOURS.map((col) => `<i style="background:${col}"></i>`).join('') +
          '<span>high</span>';
      }
    }
  }

  // The view moved (zoom, pan, resize) or the layout or colours changed:
  // redraw what does not move with each reading, once per animation frame.
  function viewChanged(el) {
    if (el.__frame) return;
    el.__frame = requestAnimationFrame(() => {
      el.__frame = 0;
      const p = el.__parts;
      if (!p || !window.echarts) return;
      const map = window.echarts.getInstanceByDom(p.map);
      if (!map || !map.__ready) return;
      drawSpecies(el);
      drawLabels(el, map, p.map);
      pcStatic(el, viewOf(map, p.map));
      if (el.__last) pcDraw(el, el.__last);
    });
  }

  // ── Component maps ────────────────────────────────────────────────────
  // Small copies of the species map, one per component, each coloured by
  // that component and showing the same part of the map as the large one.
  // Pressing one colours the large map the same way; pressing it again
  // removes the colours. On each, a dot marks the current reading, in the
  // colour of the nearest species' own fifth. Under each, a graph of the
  // reading's percentile on that component over the last 20 s (newest on
  // the right, like the strip above), with the five colours as a key at its
  // right edge. Plain canvas: a map's species are drawn once per view into
  // an offscreen canvas, so a reading costs one drawImage per map.
  const GRAPH_H = 22; // px
  const KEY_W = 3; // px, the colour key at the right edge of a graph

  // A pie with `share` (0 to 1) filled, starting at 12 o'clock.
  function pie(share) {
    const a = Math.min(0.9999, Math.max(0, share)) * 2 * Math.PI;
    const x = 5 + 5 * Math.sin(a);
    const y = 5 - 5 * Math.cos(a);
    return (
      '<svg class="live-emb-pie" viewBox="0 0 10 10" width="10" height="10" aria-hidden="true">' +
      '<circle cx="5" cy="5" r="5" fill="#444"/>' +
      `<path d="M5 5 L5 0 A5 5 0 ${a > Math.PI ? 1 : 0} 1 ${x.toFixed(2)} ${y.toFixed(2)} Z" fill="#ddd"/></svg>`
    );
  }

  const ordinal = (n) => {
    const t = n % 100;
    const s = t >= 11 && t <= 13 ? 'th' : ['th', 'st', 'nd', 'rd'][n % 10] || 'th';
    return `${n}${s}`;
  };
  const swatch = (j) => `<i class="live-emb-swatch" style="background:${PC_COLOURS[j]}"></i>`;

  // The pin on each component map (el.__pins: one bool per component, or
  // null when the rows above the spectrogram cannot show these components).
  // The same look and wording as the pins in the Call detection tab's lists
  // (classifier_section.rs, .cd-pin).
  function markPins(el) {
    const div = el.__parts && el.__parts.pcs;
    const b = div && div.__built;
    if (!b || !el.__pins) return;
    b.maps.forEach((m, k) => {
      const pin = m.cell.querySelector('.live-emb-pc-pin');
      if (!pin) return;
      const on = !!el.__pins[k];
      const name = `PC${k + 1}`;
      const text = on ? `Remove ${name} from above the spectrogram` : `Show ${name} above the spectrogram`;
      pin.classList.toggle('on', on);
      pin.setAttribute('aria-pressed', String(on));
      pin.setAttribute('aria-label', text);
      pin.title = text;
    });
  }

  function pcBuild(el) {
    const div = el.__parts.pcs;
    const pc = el.__pc;
    const n = pc.variance.length;
    if (n === 0) {
      // No components for this model: no small maps.
      div.innerHTML = '';
      return null;
    }
    const gap = 4;
    const cols = pcColumns(n, div.clientWidth, gap);
    div.style.gridTemplateColumns = `repeat(${cols}, minmax(0, 1fr))`;
    const size = Math.floor((div.clientWidth - gap * (cols - 1)) / cols) - 2;
    if (size < 20) return null;
    const dpr = window.devicePixelRatio || 1;
    const px = Math.round(size * dpr);
    const rows = pc.basis === 'rows';
    const model = (el.__last && el.__last.model && el.__last.model.name) || 'BirdNET';
    const between = rows ? `${model}'s classes` : 'recordings';
    div.innerHTML = '';
    const tip = document.createElement('div');
    tip.className = 'live-emb-near live-emb-pc-tip';
    const showTip = (cell, html) => {
      tip.innerHTML = html;
      tip.style.display = 'block';
      const w = div.clientWidth;
      const left = Math.max(0, Math.min(w - tip.offsetWidth, cell.offsetLeft));
      tip.style.left = `${left}px`;
      // Under the cell, or above it when that would leave the window.
      const below = cell.offsetTop + cell.offsetHeight + 4;
      const room = div.getBoundingClientRect().top + below + tip.offsetHeight < window.innerHeight;
      tip.style.top = `${room ? below : cell.offsetTop - tip.offsetHeight - 4}px`;
    };
    const maps = [];
    for (let k = 0; k < n; k++) {
      const name = `PC${k + 1}`;
      // A cell holds two buttons (pick the component, pin its row), so it
      // is not a button itself; a press on its map or graph also picks it.
      const cell = document.createElement('div');
      cell.className = 'live-emb-pc';
      const label = document.createElement('div');
      label.className = 'live-emb-pc-label';
      const pick = document.createElement('button');
      pick.type = 'button';
      pick.className = 'live-emb-pc-pick';
      pick.innerHTML = `${pie(pc.variance[k])}<span>${name}</span>`;
      label.appendChild(pick);
      // The pin sits on the map's top-right corner: four maps to a row
      // leave no room for it beside the name.
      let pin = null;
      if (el.__pins) {
        pin = document.createElement('button');
        pin.type = 'button';
        pin.className = 'cd-pin live-emb-pc-pin';
        pin.innerHTML = PIN_SVG;
        pin.addEventListener('click', (e) => {
          e.stopPropagation();
          window.dispatchEvent(new CustomEvent('oversample-pc-pin', { detail: k }));
        });
      }
      const canvas = document.createElement('canvas');
      canvas.className = 'live-emb-pc-map';
      canvas.width = canvas.height = px;
      canvas.style.width = canvas.style.height = `${size}px`;
      const graph = document.createElement('canvas');
      graph.className = 'live-emb-pc-graph';
      graph.width = px;
      graph.height = Math.round(GRAPH_H * dpr);
      graph.style.width = `${size}px`;
      graph.style.height = `${GRAPH_H}px`;
      cell.append(label, canvas, graph);
      if (pin) cell.appendChild(pin);
      div.appendChild(cell);
      const img = document.createElement('canvas');
      img.width = img.height = px;
      const m = { cell, pick, img, ctx: canvas.getContext('2d'), graph: graph.getContext('2d') };
      maps.push(m);

      const share = `${(pc.variance[k] * 100).toFixed(1)}% of the variation between ${between}`;
      // What lies at the two ends of the component (pc_bands.txt), if known.
      const [lo, hi] = (rows && pc.ends && pc.ends[k]) || ['', ''];
      const ends = lo
        ? `<div class="live-emb-tip-line">${swatch(0)}Low end: ${esc(lo)}</div><div class="live-emb-tip-line">${swatch(4)}High end: ${esc(hi)}</div>`
        : '';
      pick.setAttribute('aria-label', `${name}: ${share}`);
      // Over the map: what the colour under the pointer stands for.
      canvas.addEventListener('pointermove', (e) => {
        if (e.pointerType !== 'mouse') return;
        const v = el.__view || WHOLE;
        const r = canvas.getBoundingClientRect();
        const x = v.x0 + ((e.clientX - r.left) / r.width) * (v.x1 - v.x0);
        const y = v.y1 - ((e.clientY - r.top) / r.height) * (v.y1 - v.y0);
        const c = colours(el)[k].cells[gridCell(y, PC_GRID) * PC_GRID + gridCell(x, PC_GRID)];
        let here = '';
        if (c.all >= 1 && speciesBlend(v) < 1) {
          here =
            c.strength < PC_AREA_MIN
              ? `<div class="live-emb-tip-line"><i class="live-emb-swatch" style="background:${PC_MIXED}"></i>Around the pointer: no fifth of ${name} stands out</div>`
              : `<div class="live-emb-tip-line">${swatch(c.best)}Around the pointer: ${Math.round(c.share * 100)}% of ${Math.round(c.all)} species are in the ${PC_FIFTHS[c.best]} fifth of ${name}</div>`;
        } else if (speciesBlend(v) >= 1) {
          here = `<div class="live-emb-tip-line">Each species has the colour of its own fifth of ${name}</div>`;
        }
        showTip(cell, `<div>${name}: ${share}</div>${ends}${here}`);
      });
      // Over the graph: the reading under the pointer.
      graph.addEventListener('pointermove', (e) => {
        const d = el.__last;
        if (e.pointerType !== 'mouse' || !d || !d.pcHist.length) return;
        const r = graph.getBoundingClientRect();
        const t = (((e.clientX - r.left) / (r.width - KEY_W)) - 1) * d.history;
        let best = d.pcHist[0];
        for (const row of d.pcHist) if (Math.abs(row[0] - t) < Math.abs(best[0] - t)) best = row;
        const p = best[k + 1];
        const fifth = Math.min(4, Math.floor(p * 5));
        showTip(
          cell,
          `<div class="live-emb-tip-line">${swatch(fifth)}${name}: ${ordinal(Math.round(p * 100))} percentile, ${ago(best[0])}</div>`,
        );
      });
      cell.addEventListener('pointerleave', () => (tip.style.display = 'none'));
      cell.addEventListener('click', (e) => {
        if (e.target.closest('.live-emb-pc-pin')) return;
        el.__sel = el.__sel === k ? null : k;
        maps.forEach((o, j) => o.cell.classList.toggle('selected', el.__sel === j));
        maps.forEach((o, j) => o.pick.setAttribute('aria-pressed', String(el.__sel === j)));
        viewChanged(el);
      });
      cell.classList.toggle('selected', el.__sel === k);
      pick.setAttribute('aria-pressed', String(el.__sel === k));
    }
    div.appendChild(tip);
    markPins(el);
    return { width: div.clientWidth, maps, px, dpr, layout: points(el) };
  }

  // Draw every component map's species for the part `v` of the map.
  function pcStatic(el, v) {
    const pc = el.__pc;
    const p = el.__parts;
    if (!pc || !el.__species || !p) return;
    el.__view = v;
    const div = p.pcs;
    let b = div.__built;
    if (!b || b.width !== div.clientWidth) b = div.__built = pcBuild(el);
    if (!b) return;
    const comps = colours(el);
    const pts = points(el);
    b.layout = pts;
    b.maps.forEach((m, k) => {
      paintSpecies(m.img.getContext('2d'), b.px, b.px, v, pts, comps[k], Math.max(1, b.dpr) * pointScale(v));
    });
  }

  function pcDraw(el, d) {
    const p = el.__parts;
    const pc = el.__pc;
    if (!pc || !el.__species || !p) return;
    const div = p.pcs;
    if (!div.__built || div.__built.width !== div.clientWidth || div.__built.layout !== points(el)) {
      pcStatic(el, el.__view || WHOLE);
    }
    const b = div.__built;
    if (!b) return;
    const v = el.__view || WHOLE;
    const comps = colours(el);
    const last = markerAt(el, d);
    // The species nearest the reading: the dot takes its colour.
    const near = last ? nearestSpecies(el, last[0], last[1], jump(el), 1)[0] : undefined;
    const u = b.dpr;
    b.maps.forEach((m, k) => {
      m.ctx.clearRect(0, 0, b.px, b.px);
      m.ctx.drawImage(m.img, 0, 0);
      if (last) {
        const x = ((last[0] - v.x0) / (v.x1 - v.x0)) * b.px;
        const y = ((v.y1 - last[1]) / (v.y1 - v.y0)) * b.px;
        const band = near === undefined ? -1 : comps[k].band[near];
        m.ctx.beginPath();
        if (d.preview) {
          // The paused reading: the same blue play triangle as the large
          // map, outlined in black so it shows on the coloured areas.
          const r = 4.5 * u;
          m.ctx.moveTo(x + r, y);
          m.ctx.lineTo(x - r / 2, y - r * 0.87);
          m.ctx.lineTo(x - r / 2, y + r * 0.87);
          m.ctx.closePath();
          m.ctx.lineJoin = 'round';
          m.ctx.lineWidth = 3 * u;
          m.ctx.strokeStyle = '#000000';
          m.ctx.stroke();
          m.ctx.lineWidth = 1.5 * u;
          m.ctx.strokeStyle = PLAY_BLUE;
          m.ctx.stroke();
        } else {
          m.ctx.arc(x, y, 3 * u, 0, 2 * Math.PI);
          m.ctx.fillStyle = band < 0 ? PC_MIXED : PC_COLOURS[band];
          m.ctx.fill();
          m.ctx.lineWidth = u;
          m.ctx.strokeStyle = '#ffffff';
          m.ctx.stroke();
        }
      }
      // The graph: percentile (low at the bottom) over the last d.history
      // seconds, each stretch in the colour of the fifth it is in.
      const g = m.graph;
      const w = b.px;
      const h = g.canvas.height;
      const keyW = KEY_W * u;
      const plotW = w - keyW - u;
      g.clearRect(0, 0, w, h);
      g.fillStyle = '#2c2c2c';
      g.fillRect(0, Math.round(h / 2), plotW, 1);
      PC_COLOURS.forEach((c, j) => {
        g.fillStyle = c;
        g.fillRect(w - keyW, h - ((j + 1) * h) / 5, keyW, h / 5 - (j ? u / 2 : 0));
      });
      const at = (row) => [(1 + row[0] / d.history) * plotW, (1 - row[k + 1]) * (h - 2 * u) + u];
      g.lineWidth = 1.5 * u;
      g.lineJoin = 'round';
      for (let i = 1; i < d.pcHist.length; i++) {
        const a = d.pcHist[i - 1];
        const c = d.pcHist[i];
        // A gap in the readings (playback paused) is not joined.
        if (c[0] - a[0] > 1.5) continue;
        const [x0, y0] = at(a);
        const [x1, y1] = at(c);
        g.strokeStyle = PC_COLOURS[Math.min(4, Math.floor(c[k + 1] * 5))];
        g.beginPath();
        g.moveTo(x0, y0);
        g.lineTo(x1, y1);
        g.stroke();
      }
      const now = d.pcHist[d.pcHist.length - 1];
      if (now) {
        // The newest value, as a white mark across the key.
        g.fillStyle = '#ffffff';
        g.fillRect(w - keyW - 2 * u, at(now)[1] - u, keyW + 2 * u, 2 * u);
      }
    });
  }

  // ── Full screen ───────────────────────────────────────────────────────
  // The whole "At the play position" section (.live-emb, from the Call
  // detection tab) fills the window (CSS, .live-emb-full), and where the
  // browser allows it the screen. Android's web view has no full-screen
  // support for page elements, so there the view only fills the app's window.
  const fullTarget = (el) => el.closest('.live-emb') || el;

  function setFull(el, on) {
    const t = fullTarget(el);
    if (t.classList.contains('live-emb-full') === on) return;
    t.classList.toggle('live-emb-full', on);
    const b = el.__parts && el.__parts.fullBtn;
    if (b) b.textContent = on ? 'Exit full screen' : 'Full screen';
    try {
      if (on && t.requestFullscreen) {
        const r = t.requestFullscreen();
        if (r && r.catch) r.catch(() => {});
      } else if (!on && document.fullscreenElement === t) document.exitFullscreen();
    } catch (_) {
      // The window-filling view is shown either way.
    }
    viewChanged(el);
  }

  function tools(el, div) {
    const group = document.createElement('div');
    group.className = 'live-emb-seg';
    group.setAttribute('role', 'group');
    group.setAttribute('aria-label', 'Map layout');
    const buttons = LAYOUTS.map(([id, text]) => {
      const b = document.createElement('button');
      b.type = 'button';
      b.className = 'peak-src-btn';
      b.textContent = text;
      b.addEventListener('click', () => {
        layout = id;
        try {
          window.localStorage.setItem(LAYOUT_KEY, id);
        } catch (_) {
          // Not saved; the choice lasts for this visit.
        }
        mark();
        const p = el.__parts;
        const map = window.echarts && window.echarts.getInstanceByDom(p.map);
        if (map && map.__ready && el.__last) {
          if (p.map.__explore) p.map.__explore.hide();
          map.setOption({ series: mapSeries(el, el.__last) });
        }
        viewChanged(el);
      });
      group.appendChild(b);
      return [id, b];
    });
    const mark = () =>
      buttons.forEach(([id, b]) => {
        const on = (isUmap(el) ? 'umap' : 'tsne') === id;
        b.classList.toggle('sel', on);
        b.setAttribute('aria-pressed', String(on));
      });
    const full = document.createElement('button');
    full.type = 'button';
    full.className = 'live-emb-full-btn';
    full.textContent = 'Full screen';
    full.addEventListener('click', () => setFull(el, !fullTarget(el).classList.contains('live-emb-full')));
    // Leaving the browser's full screen (Escape, a system gesture) also
    // leaves the window-filling view.
    const onChange = () => {
      const t = fullTarget(el);
      if (!document.fullscreenElement && el.__wasFull) setFull(el, false);
      el.__wasFull = document.fullscreenElement === t;
    };
    document.addEventListener('fullscreenchange', onChange);
    div.append(group, full);
    div.__mark = mark;
    div.__remove = () => document.removeEventListener('fullscreenchange', onChange);
    return full;
  }

  // The strip goes in `el`; the map's tools, the map and the component maps
  // go in el.__mapHost (the "Map and components" fold) when there is one.
  function parts(el) {
    if (!el.__parts) {
      el.innerHTML = '';
      const host = el.__mapHost || el;
      if (host !== el) host.innerHTML = '';
      const mk = (cls, parent) => {
        const div = document.createElement('div');
        div.className = cls;
        parent.appendChild(div);
        return div;
      };
      const p = {
        strip: mk('live-emb-strip', el),
        tools: mk('live-emb-tools', host),
        map: mk('live-emb-map', host),
        pcs: mk('live-emb-pcs', host),
      };
      p.fullBtn = tools(el, p.tools);
      el.__parts = p;
    }
    return el.__parts;
  }

  function chartFor(div, onResize) {
    let chart = window.echarts.getInstanceByDom(div);
    if (!chart) {
      chart = window.echarts.init(div, null, { renderer: 'canvas' });
      div.__resize = new ResizeObserver(() => {
        chart.resize();
        if (onResize) onResize();
      });
      div.__resize.observe(div);
    }
    return chart;
  }

  window.oversampleLiveEmbedding = {
    // Where the map and component maps go (see parts). Call before the
    // first update.
    setMapHost(el, host) {
      el.__mapHost = host;
    },
    // Which component maps carry a pin, and which pins are on: a JSON array
    // of bools, or null for no pins. Rebuilds the maps when pins come or go.
    setPins(el, json) {
      const pins = json ? JSON.parse(json) : null;
      const had = !!el.__pins;
      el.__pins = pins;
      const div = el.__parts && el.__parts.pcs;
      if (div && had !== !!pins) div.__built = null;
      markPins(el);
    },
    async update(el, json, speciesJson, pcJson) {
      const d = JSON.parse(json);
      el.__last = d;
      if (speciesJson) {
        el.__species = JSON.parse(speciesJson);
        el.__liveSpecies = true;
        el.__pts = null;
      }
      if (pcJson) {
        el.__pc = JSON.parse(pcJson);
        el.__pts = null;
      }
      await loadEcharts();
      if (el.__last !== d) return; // a newer reading arrived while loading
      const p = parts(el);
      p.tools.__mark();
      // The strip keeps the most rows it has had, so the map under it does
      // not move up and down as classes come and go.
      el.__rows = Math.max(el.__rows || 1, d.rows.length);
      p.strip.style.height = `${el.__rows * ROW_H + CHANGE_H + 36}px`;
      // Nothing heard yet (a file open and paused shows only the reading of
      // where Play starts): the strip and its change trace would be empty
      // frames, so they are hidden until the first reading.
      p.strip.style.display = d.heard ? '' : 'none';
      if (d.heard) {
        const strip = chartFor(p.strip);
        strip.setOption(stripOptions(d), { replaceMerge: ['series', 'yAxis', 'grid', 'xAxis', 'graphic'] });
      }
      // The map's fold is closed: nothing to draw there. Opening it draws
      // the map again (live_embedding::redraw).
      if (p.map.clientWidth === 0) return;
      const map = chartFor(p.map, () => viewChanged(el));
      const t0 = performance.now();
      if (!map.__ready && el.__species) {
        map.setOption(mapOptions(el, d));
        map.__ready = true;
        mapExplore(el, map, p.map);
        viewChanged(el);
      } else if (map.__ready) {
        map.setOption({ series: mapSeries(el, d) });
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
      setFull(el, false);
      if (el.__frame) cancelAnimationFrame(el.__frame);
      el.__frame = 0;
      if (el.__parts.map.__explore) el.__parts.map.__explore.remove();
      el.__parts.tools.__remove();
      for (const div of [el.__parts.strip, el.__parts.map]) {
        if (div.__resize) div.__resize.disconnect();
        const chart = window.echarts && window.echarts.getInstanceByDom(div);
        if (chart) chart.dispose();
      }
      el.__parts = null;
      el.__rows = 0;
      el.__liveSpecies = false;
      // The next update may be of another model: its species, layouts and
      // components replace these.
      el.__species = null;
      el.__pc = null;
      el.__pts = null;
      el.__grid = null;
      el.__colours = null;
      el.__sel = null;
      el.__view = null;
      el.__last = null;
      el.innerHTML = '';
      if (el.__mapHost) el.__mapHost.innerHTML = '';
    },
  };
})();
