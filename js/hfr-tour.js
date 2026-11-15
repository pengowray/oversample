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
            return setTimeout(function () {
                if (!tour || !tour.isActive()) return;
                tour.refresh();
                placeGesture();
            }, ms);
        });
    }

    function onKeydown(ev) {
        if (ev.key === 'Escape' && tour && tour.isActive()) tour.destroy();
    }

    // ── The hand that acts out a step's drag ────────────────────────────
    // A step with a gesture gets a large hand that drags along its control,
    // over and over, until the user starts a drag there: 'drag-up' for the
    // frequency strip, 'drag-right' for the time strip.
    //
    // The hand is a child of <body>, not of the control: the strips clip
    // their contents, and driver.css turns pointer events back on inside
    // the highlighted control, where the hand would take the user's drag.

    // Pointing hand, drawn here so it looks the same on every platform.
    // The fingertip is at (46, 6) in the 100 x 130 box; style.css puts the
    // transform origin there.
    var HAND_TIP_X = 0.46;
    var HAND_TIP_Y = 0.046;
    var HAND_SVG =
        '<svg class="tour-hand" viewBox="0 0 100 130" aria-hidden="true">' +
        '<path d="M38 14a8 8 0 0 1 16 0V46a6.5 6.5 0 0 1 13 0v4a6 6 0 0 1 12 0v5' +
        'a5.5 5.5 0 0 1 11 0V88c0 20-12 34-30 34h-6c-12 0-18-6-24-16L14 78' +
        'c-4-7 4-14 11-8L38 82Z"/>' +
        '<path class="tour-hand-lines" d="M54 46V62M67 50V64M79 55V66"/>' +
        '</svg>';
    // How long after a press that did not finish the step the hand returns.
    var GESTURE_RETURN_MS = 1200;

    var gesture = null; // { el, kind, node, timer }

    function removeGesture() {
        if (!gesture) return;
        clearTimeout(gesture.timer);
        gesture.node.remove();
        gesture = null;
    }

    function popoverRect() {
        var pop = document.querySelector('.driver-popover');
        var p = pop && pop.getBoundingClientRect();
        return p && p.width > 0 ? p : null;
    }

    // Up the middle of a tall strip. The hand hangs below the fingertip.
    function placeDragUp(r) {
        var size = Math.max(88, Math.min(150, r.height * 0.22));
        var x = r.left + r.width / 2;
        var startY = r.top + r.height * 0.8;
        var endY = r.top + r.height * 0.3;
        // Stop below the popover when the hand would pass under it.
        var p = popoverRect();
        if (p) {
            var below = p.bottom + size * 0.3;
            if (p.left < x + size && below > endY && startY - below >= 40) endY = below;
        }
        return { size: size, x: x, y: startY, dx: 0, dy: endY - startY, above: false };
    }

    // Along a wide strip at the bottom of the view. The hand hangs below the
    // fingertip when the window has room under the strip (a phone). When it
    // does not, the hand reaches down from above, and stops short of the
    // popover, which sits over the right end of the strip.
    function placeDragRight(r) {
        var size = Math.max(88, Math.min(150, r.width * 0.12));
        var y = r.top + r.height / 2;
        var startX = r.left + r.width * 0.1;
        var endX = r.left + r.width * 0.55;
        // The leaning hand is about 1.05 of its width across and down.
        var reach = size * 1.05;
        var room = window.innerHeight - y;
        var above = room < reach;
        if (above) {
            var p = popoverRect();
            if (p && p.bottom > y - reach) {
                var stop = p.left - reach - 8;
                if (stop - startX >= 60) {
                    endX = Math.min(endX, stop);
                } else {
                    // No room beside the popover either: a smaller hand below.
                    above = false;
                    size = Math.max(56, Math.min(size, room / 1.05));
                }
            }
        }
        return { size: size, x: startX, y: y, dx: endX - startX, dy: 0, above: above };
    }

    var PLACE = { 'drag-up': placeDragUp, 'drag-right': placeDragRight };

    // Put the hand at the start of its drag and set how far it travels.
    function placeGesture() {
        if (!gesture) return;
        var r = gesture.el.getBoundingClientRect();
        if (r.width <= 0 || r.height <= 0) return;
        var g = PLACE[gesture.kind](r);
        gesture.node.classList.toggle('tour-gesture-above', g.above);
        var s = gesture.node.style;
        s.left = g.x + 'px';
        s.top = g.y + 'px';
        s.setProperty('--tour-hand-size', g.size + 'px');
        s.setProperty('--tour-hand-left', -g.size * HAND_TIP_X + 'px');
        s.setProperty('--tour-hand-top', -g.size * 1.3 * HAND_TIP_Y + 'px');
        s.setProperty('--tour-drag-x', g.dx + 'px');
        s.setProperty('--tour-drag-y', g.dy + 'px');
    }

    function showGesture(el, kind) {
        removeGesture();
        if (!PLACE[kind] || !el) return;
        var node = document.createElement('div');
        node.className = 'tour-gesture';
        node.innerHTML = HAND_SVG;
        document.body.appendChild(node);
        gesture = { el: el, kind: kind, node: node, timer: 0 };
        placeGesture();
    }

    // The hand goes away while the user presses its control, so it is not in
    // the way of their own drag.
    function onPointerDown(ev) {
        if (!gesture || !gesture.el.contains(ev.target)) return;
        clearTimeout(gesture.timer);
        gesture.node.hidden = true;
    }

    // A finished drag moves the tutorial on, which removes the hand. After a
    // press that did not count (a tap), the hand comes back.
    function onPointerUp() {
        if (!gesture || !gesture.node.hidden) return;
        var g = gesture;
        clearTimeout(g.timer);
        g.timer = setTimeout(function () {
            if (gesture !== g) return;
            placeGesture();
            g.node.hidden = false;
        }, GESTURE_RETURN_MS);
    }

    function listen(on) {
        var f = on ? 'addEventListener' : 'removeEventListener';
        document[f]('keydown', onKeydown, true);
        document[f]('pointerdown', onPointerDown, true);
        document[f]('pointerup', onPointerUp, true);
        document[f]('pointercancel', onPointerUp, true);
        window[f]('resize', placeGesture);
    }

    window.oversampleTour = {
        // steps: [{ selector, title, description, side, align, wait_ms, gesture }]
        // gesture: 'drag-up' or 'drag-right' shows a hand dragging along
        // the control.
        // labels: { next, done, close, progress } ("{{current}} of {{total}}")
        // on_step(index) runs as each step starts; on_end() once at the end.
        start: function (steps, labels, onStep, onEnd) {
            return loadDriver().then(function (driverFn) {
                if (tour && tour.isActive()) tour.destroy();
                hooks = { onStep: onStep, onEnd: onEnd };
                var gestures = steps.map(function (st) { return st.gesture; });
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
                        removeGesture();
                        if (hooks && hooks.onStep) hooks.onStep(opts.index);
                    },
                    onHighlighted: function (el, step, opts) {
                        showGesture(el, gestures[opts.index]);
                        refreshSoon();
                    },
                    onDestroyed: function () {
                        listen(false);
                        removeGesture();
                        refreshTimers.forEach(clearTimeout);
                        var h = hooks;
                        hooks = null;
                        if (h && h.onEnd) h.onEnd();
                    },
                });
                listen(true);
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
