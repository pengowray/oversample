// Species map for the Call detection tab: BirdNET's 6,522 classes as faint
// points (positions from research/species-map), the windows of a run as
// highlighted dots, hover for names. Drawn with Apache ECharts (vendor/,
// Apache-2.0), which is loaded the first time a map opens.
//
// Called from src/components/file_sidebar/species_map.rs:
//   oversampleSpeciesMap.render(el, json, onPick) -> Promise
//   oversampleSpeciesMap.resetZoom(el)
//   oversampleSpeciesMap.dispose(el)
// `json` is { species: [[x, y, common, scientific]...], found: [i...],
//   windows: [[x, y, window, start, end, [[name, conf]...]]...], selected }
// `onPick(window)` runs when a window's dot is clicked.
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
  const time = (s) => `${s.toFixed(1)} s`;

  // Colours match the label strip on the spectrogram: green for windows,
  // amber for the selected one (which is also larger, with a white ring).
  const INK = '#dddddd';
  const MUTED = '#9a9a9a';
  const SURFACE = '#1e1e1e';
  const WINDOW = '#50dc96';
  const SELECTED = '#ffc850';

  function options(d) {
    const found = new Set(d.found);
    return {
      animation: false,
      backgroundColor: 'transparent',
      grid: { left: 4, right: 4, top: 4, bottom: 4 },
      xAxis: { type: 'value', show: false, min: -1.05, max: 1.05 },
      yAxis: { type: 'value', show: false, min: -1.05, max: 1.05 },
      dataZoom: [
        { type: 'inside', xAxisIndex: 0, filterMode: 'none' },
        { type: 'inside', yAxisIndex: 0, filterMode: 'none' },
      ],
      tooltip: {
        trigger: 'item',
        confine: true,
        backgroundColor: '#262626',
        borderColor: '#444',
        textStyle: { color: INK, fontSize: 12 },
        formatter: (p) => {
          const v = p.data.value || p.data;
          if (p.seriesId === 'windows') {
            const rows = v[5]
              .map(([n, c]) => `${esc(n)} <span style="float:right;margin-left:12px">${c.toFixed(2)}</span>`)
              .join('<br>');
            return `<b>Window ${v[2] + 1}</b>, ${time(v[3])} to ${time(v[4])}<br>${rows}`;
          }
          const sci = v[3] && v[3] !== v[2] ? `<br><i style="color:${MUTED}">${esc(v[3])}</i>` : '';
          return `${esc(v[2])}${sci}`;
        },
      },
      series: [
        {
          id: 'species',
          type: 'scatter',
          data: d.species,
          symbolSize: 4,
          itemStyle: { color: 'rgba(170,170,170,0.35)' },
          label: {
            show: false,
            position: 'right',
            formatter: (p) => p.data[2],
            color: MUTED,
            fontSize: 10,
          },
          labelLayout: { hideOverlap: true },
          emphasis: { scale: 2, itemStyle: { color: INK } },
          z: 1,
        },
        {
          // The species the windows name, ringed and labelled.
          id: 'found',
          type: 'scatter',
          data: d.species.filter((_, i) => found.has(i)),
          symbolSize: 9,
          itemStyle: { color: 'transparent', borderColor: INK, borderWidth: 1.5 },
          label: {
            show: true,
            position: 'right',
            formatter: (p) => p.data[2],
            color: INK,
            fontSize: 11,
          },
          labelLayout: { hideOverlap: true },
          z: 2,
        },
        {
          id: 'windows',
          type: 'scatter',
          data: d.windows.map((w) => ({
            value: w,
            symbolSize: w[2] === d.selected ? 13 : 8,
            itemStyle:
              w[2] === d.selected
                ? { color: SELECTED, borderColor: '#ffffff', borderWidth: 2 }
                : { color: WINDOW, borderColor: SURFACE, borderWidth: 2 },
            z: w[2] === d.selected ? 4 : 3,
          })),
          emphasis: { scale: 1.4 },
          z: 3,
        },
      ],
    };
  }

  window.oversampleSpeciesMap = {
    async render(el, json, onPick) {
      await loadEcharts();
      const d = JSON.parse(json);
      let chart = window.echarts.getInstanceByDom(el);
      if (!chart) {
        chart = window.echarts.init(el, null, { renderer: 'canvas' });
        el.__resize = new ResizeObserver(() => chart.resize());
        el.__resize.observe(el);
        chart.on('click', (p) => {
          if (p.seriesId === 'windows') el.__onPick && el.__onPick(p.data.value[2]);
        });
        // Name the grey species once zoomed in to a fifth of the map or less;
        // further out there are too many to read.
        chart.on('datazoom', () => {
          const z = chart.getOption().dataZoom;
          const span = Math.max(...z.map((d) => d.end - d.start));
          const show = span <= 20;
          if (show !== el.__namesShown) {
            el.__namesShown = show;
            chart.setOption({ series: [{ id: 'species', label: { show } }] });
          }
        });
        chart.setOption(options(d));
      } else {
        // Keep the zoom: replace only the series.
        chart.setOption({ series: options(d).series }, { replaceMerge: ['series'] });
        if (el.__namesShown) chart.setOption({ series: [{ id: 'species', label: { show: true } }] });
      }
      el.__onPick = onPick;
    },
    resetZoom(el) {
      const chart = window.echarts && window.echarts.getInstanceByDom(el);
      if (chart) {
        chart.dispatchAction({ type: 'dataZoom', dataZoomIndex: 0, start: 0, end: 100 });
        chart.dispatchAction({ type: 'dataZoom', dataZoomIndex: 1, start: 0, end: 100 });
      }
    },
    dispose(el) {
      if (el.__resize) el.__resize.disconnect();
      const chart = window.echarts && window.echarts.getInstanceByDom(el);
      if (chart) chart.dispose();
    },
  };
})();
