// 3D spectrogram surface ("View in 3D" in the selection menu).
//
// Plotly's gl3d bundle is 1.7 MB, so it is loaded on first use rather than at
// startup. It is vendored (js/plotly-gl3d-*.min.js, copied into dist by Trunk)
// so the view also works offline in the desktop and Android apps.
(function () {
    var PLOTLY_SRC = 'plotly-gl3d-4.1.2.min.js';
    var loading = null;

    function loadPlotly() {
        if (window.Plotly) return Promise.resolve(window.Plotly);
        if (loading) return loading;
        loading = new Promise(function (resolve, reject) {
            var s = document.createElement('script');
            s.src = PLOTLY_SRC;
            s.async = true;
            s.onload = function () { resolve(window.Plotly); };
            s.onerror = function () {
                loading = null;
                reject(new Error('Could not load ' + PLOTLY_SRC));
            };
            document.head.appendChild(s);
        });
        return loading;
    }

    function hasWebGL() {
        try {
            var c = document.createElement('canvas');
            return !!(c.getContext('webgl2') || c.getContext('webgl'));
        } catch (e) {
            return false;
        }
    }

    // Starting camera position. In a tall, narrow box (a phone held upright)
    // the default view crops the wide time axis, so move the camera back.
    function startEye(el) {
        var w = el.clientWidth || 1;
        var h = el.clientHeight || 1;
        // 0.9 on a wide screen: Plotly's default distance leaves wide empty
        // margins around the surface.
        var k = Math.max(0.9, (h / w) * 1.2);
        return { x: 1.5 * k, y: -1.6 * k, z: 1.1 * k };
    }

    // d: { z: Float32Array (rows = frequencies, row-major), nx, ny,
    //      times_ms: Float64Array(nx), freqs_khz: Float64Array(ny),
    //      level: Float32Array, unclamped z; start_s: selection start in seconds,
    //      floor_db, colorscale: [[0..1, 'rgb(...)'], ...] }
    function show(el, d) {
        if (!hasWebGL()) {
            return Promise.reject(new Error('The 3D view needs WebGL, which is turned off or unavailable in this browser'));
        }
        return loadPlotly().then(function (Plotly) {
            var z = new Array(d.ny);
            // Hover shows the measured level (z is clamped to the floor) and
            // the time in the recording, so a peak can be found on the timeline.
            var custom = new Array(d.ny);
            for (var j = 0; j < d.ny; j++) {
                z[j] = Array.prototype.slice.call(d.z, j * d.nx, (j + 1) * d.nx);
                custom[j] = new Array(d.nx);
                for (var i = 0; i < d.nx; i++) {
                    custom[j][i] = [d.level[j * d.nx + i], d.start_s + d.times_ms[i] / 1000];
                }
            }
            var x = Array.prototype.slice.call(d.times_ms);
            var narrow = el.clientWidth < 600;
            var y = Array.prototype.slice.call(d.freqs_khz);
            var axis = function (title) {
                return {
                    title: { text: title, font: { size: 12 } },
                    color: '#bbb',
                    gridcolor: '#3a3a3a',
                    zerolinecolor: '#555',
                    backgroundcolor: '#1e1e1e',
                    showbackground: true,
                    showspikes: false,
                };
            };
            var yAxis = axis('Frequency (kHz)');
            // Fewer frequency ticks on a phone, where 5 kHz steps touch.
            if (narrow) yAxis.nticks = 5;
            // The long title is clipped at the left edge on a phone.
            var zAxis = axis(narrow ? 'dB' : 'Level (dB, 0 = loudest)');
            zAxis.range = [d.floor_db, 0];
            var trace = {
                type: 'surface',
                x: x,
                y: y,
                z: z,
                cmin: d.floor_db,
                cmax: 0,
                colorscale: d.colorscale,
                showscale: false,
                customdata: custom,
                hovertemplate: '%{customdata[1]:.4f} s (+%{x:.1f} ms)<br>%{y:.1f} kHz<br>%{customdata[0]:.1f} dB<extra></extra>',
                contours: { z: { show: false } },
                lighting: { ambient: 0.75, diffuse: 0.5, specular: 0.05, roughness: 0.9 },
            };
            var layout = {
                paper_bgcolor: '#1e1e1e',
                // Plotly draws 3D axis text on a canvas, where 'inherit' is not a font.
                font: { color: '#ccc', family: getComputedStyle(document.body).fontFamily, size: 11 },
                margin: { l: 0, r: 0, t: 0, b: 0 },
                scene: {
                    xaxis: axis('Time after selection start (ms)'),
                    yaxis: yAxis,
                    zaxis: zAxis,
                    aspectmode: 'manual',
                    aspectratio: { x: 1.6, y: 1, z: 0.6 },
                    // Aim slightly below the middle: the peaks rise above the
                    // box, so a centred view leaves the top empty.
                    camera: { eye: startEye(el), center: { x: 0, y: 0, z: -0.15 } },
                    dragmode: 'turntable',
                },
            };
            var config = {
                displaylogo: false,
                responsive: true,
                modeBarButtonsToRemove: ['orbitRotation', 'tableRotation', 'zoom3d', 'pan3d', 'resetCameraLastSave3d', 'hoverClosest3d', 'sendChartToCloud'],
                // The "Share chart..." button uploads the data to Plotly Cloud.
                showSendToCloud: false,
                toImageButtonOptions: { filename: 'spectrogram-3d' },
            };
            // Keep the user's rotation and zoom when the floor changes: react()
            // puts the camera back to the layout's, so set the live one again.
            var live = el._fullLayout && el._fullLayout.scene && el._fullLayout.scene._scene;
            var camera = live && live.getCamera ? live.getCamera() : null;
            return Plotly.react(el, [trace], layout, config).then(function () {
                if (camera) return Plotly.relayout(el, { 'scene.camera': camera });
            });
        });
    }

    function purge(el) {
        // Frees the WebGL context; browsers allow only a few at once.
        if (window.Plotly && el) window.Plotly.purge(el);
    }

    window.oversampleSurface3d = { show: show, purge: purge };
})();
