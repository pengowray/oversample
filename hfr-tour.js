// Step-by-step tutorial overlay (the HFR tutorial in src/components/hfr_tour.rs).
//
// driver.js draws the dimmed overlay, the cut-out around the control and the
// popover. It is loaded on first use, and vendored (js/driver-1.10.2.*, MIT,
// copied into dist by Trunk) so the tutorial also works offline in the
// desktop and Android apps.
//
// Rust owns the steps and decides when one is done (HFR turned on, a drag on a
// gutter finished); this file only shows them. The page stays usable inside
// the cut-out, so the user does each step on the real control.
(function () {
    var JS_SRC = 'driver-1.10.2.iife.js';
    var CSS_HREF = 'driver-1.10.2.css';
    var loading = null;
    var tour = null;
    var hooks = null;
    var refreshTimers = [];

    function loadDriver() {
        if (window.driver && window.driver.js) return Promise.resolve(window.driver.js.driver);
        if (loading) return loading;
        loading = new Promise(function (resolve, reject) {
            var link = document.createElement('link');
            link.rel = 'stylesheet';
            link.href = CSS_HREF;
            // Before style.css, so the app's .oversample-tour rules win ties.
            document.head.insertBefore(link, document.head.firstChild);
            var s = document.createElement('script');
            s.src = JS_SRC;
            s.async = true;
            s.onload = function () { resolve(window.driver.js.driver); };
            s.onerror = function () {
                loading = null;
                reject(new Error('Could not load ' + JS_SRC));
            };
            document.head.appendChild(s);
        });
        return loading;
    }

    // First match that is laid out on screen. Hidden copies exist: the phone
    // Hear sheet is always mounted, and the desktop bar keeps its own HFR.
    function firstVisible(selector) {
        return function () {
            var all = document.querySelectorAll(selector);
            for (var i = 0; i < all.length; i++) {
                var r = all[i].getBoundingClientRect();
                if (r.width > 0 && r.height > 0) return all[i];
            }
            return all[0];
        };
    }

    // Re-measure after layout settles: the Hear sheet slides in over 0.2 s,
    // and a step can start while the view is still resizing.
    function refreshSoon() {
        refreshTimers.forEach(clearTimeout);
        refreshTimers = [250, 600].map(function (ms) {
            return setTimeout(function () { if (tour && tour.isActive()) tour.refresh(); }, ms);
        });
    }

    function onKeydown(ev) {
        if (ev.key === 'Escape' && tour && tour.isActive()) tour.destroy();
    }

    window.oversampleTour = {
        // steps: [{ selector, title, description, side, align, wait_ms }]
        // labels: { next, done, close, progress } ("{{current}} of {{total}}")
        // on_step(index) runs as each step starts; on_end() once at the end.
        start: function (steps, labels, onStep, onEnd) {
            return loadDriver().then(function (driverFn) {
                if (tour && tour.isActive()) tour.destroy();
                hooks = { onStep: onStep, onEnd: onEnd };
                tour = driverFn({
                    steps: steps.map(function (st) {
                        return {
                            element: firstVisible(st.selector),
                            waitForElement: st.wait_ms || 0,
                            popover: {
                                title: st.title,
                                description: st.description,
                                side: st.side,
                                align: st.align,
                            },
                        };
                    }),
                    showProgress: steps.length > 1,
                    progressText: labels.progress,
                    nextBtnText: labels.next,
                    doneBtnText: labels.done,
                    closeBtnLabel: labels.close,
                    // No Back: each step is something done on the real
                    // control, and going back would undo nothing.
                    showButtons: ['next', 'close'],
                    popoverClass: 'oversample-tour',
                    // Light enough that the calls in the spectrogram stay
                    // visible: the gutter steps ask the user to find them.
                    overlayOpacity: 0.35,
                    stagePadding: 4,
                    stageRadius: 6,
                    // A drag on a gutter can end outside the cut-out; that
                    // release must not close the tutorial.
                    overlayClickBehavior: 'none',
                    // The app scrolls with the arrow keys. Escape is handled
                    // below so it closes the tutorial and nothing else changes.
                    allowKeyboardControl: false,
                    smoothScroll: false,
                    onHighlightStarted: function (el, step, opts) {
                        if (hooks && hooks.onStep) hooks.onStep(opts.index);
                    },
                    onHighlighted: refreshSoon,
                    onDestroyed: function () {
                        document.removeEventListener('keydown', onKeydown, true);
                        refreshTimers.forEach(clearTimeout);
                        var h = hooks;
                        hooks = null;
                        if (h && h.onEnd) h.onEnd();
                    },
                });
                document.addEventListener('keydown', onKeydown, true);
                tour.drive(0);
            });
        },
        // Go to the next step, or finish after the last one.
        next: function () {
            if (!tour || !tour.isActive()) return;
            if (tour.isLastStep()) tour.destroy();
            else tour.moveNext();
        },
        refresh: function () {
            if (tour && tour.isActive()) tour.refresh();
        },
        stop: function () {
            if (tour && tour.isActive()) tour.destroy();
        },
    };
})();
