// The recordings the clips show, the captions, and one function per feature.
// A step starts from whatever the previous step left, so the tour can run them
// in a row; each one also works on a freshly opened recording.

/**
 * Recordings from bat-demo-sounds. Each clip keeps the title bar and its
 * licence badge in frame; media/README.md credits the recordist.
 */
export const RECORDINGS = {
    // Cornwall, UK. Stephan Risch, CC BY-NC-SA 4.0. 256 kHz, 10.3 s.
    pip: {
        xc: 1037039,
        file: 'XC1037039 - Soprano Pipistrelle - Pipistrellus pygmaeus.wav',
        // HFR band dragged on the frequency strip, top to bottom (kHz).
        band: [85, 45],
        // Start times (s) of calls that show clearly, from the in-built detector.
        calls: [0.232, 0.453, 0.56, 0.772, 0.928, 1.036, 2.033, 2.117, 2.195],
    },
};

/**
 * Recordings joined in the timeline clip, opened from bat-demo-sounds: one
 * night in Sweden (2025-09-27), all 500 kHz, by Alessandra Munari, CC BY-NC-SA
 * 4.0. Files at the same rate keep the same colours on one timeline.
 */
export const TIMELINE_FILES = [
    'XC1084197 - Western Barbastelle - Barbastella barbastellus.wav',
    'XC1084199 - Eurasian Particolored Bat - Vespertilio murinus.wav',
    'XC1084195 - Natterer\'s Myotis - Myotis nattereri.wav',
];

export const TEXT = {
    title: ['Oversample', 'Free app for recordings of bats and other wildlife'],
    open: 'Find recordings on Xeno-canto by species name',
    zoom: ['Zoom in and move along the recording', 'Zoom: Ctrl + scroll wheel'],
    hear: 'Hear bat calls at a lower pitch',
    modes: ['Four ways to make bat calls audible', 'Heterodyne, time expansion, pitch shift, zero crossing'],
    views: 'Switch between spectrogram, waveform and dot plot',
    mark: 'Select a bat call and annotate it',
    detect: 'Detect bat calls automatically',
    book: 'Bat book: typical calls of UK bats',
    view3d: 'Select bat calls and view them in 3D',
    timeline: "Create a timeline from a night's recordings",
    live: ['Hear bat calls live at a lower pitch', 'Test signal here: a replayed bat recording'],
    end: ['app.oversample.com', 'Also for Windows, macOS, Linux and Android'],
};

const MAIN = '.main-view .chart-stage > canvas';

/**
 * A visible mode button (view or playback mode) by its label. Their
 * accessible names come from their titles, and the phone layout has hidden
 * copies.
 */
const button = (page, text, scope = '') =>
    page
        .locator(`${scope} button.mode-radio-btn:visible`.trim(), {
            has: page.locator('.mode-radio-label', { hasText: new RegExp(`^${text}$`) }),
        })
        .first();

/** Hides the file list so the spectrogram gets the width. */
export async function hideFiles(d, { quick = false } = {}) {
    const files = d.page.locator('.sidebar:not(.right-sidebar)').first();
    const open = async () => {
        const b = (await files.isVisible()) ? await files.boundingBox() : null;
        return !!b && b.x + b.width > 1;
    };
    // On phones the list is a panel that slides off the left edge by itself
    // once a recording opens.
    for (let i = 0; d.phone && i < 30 && (await open()); i++) await d.wait(100);
    if (!(await open())) return;
    if (quick) await d.page.locator('.toolbar-sidebar-tab-left').click();
    else await d.click('.toolbar-sidebar-tab-left', { ms: 500, after: 400 });
}

/** Find recordings: search for a species and load one of its recordings. */
export async function openFromSearch(d, rec) {
    const { page } = d;
    await d.caption(TEXT.open);
    const find = page.getByRole('button', { name: 'Find recordings' });
    // On phones it is in the Files panel, which starts closed.
    if (!(await find.isVisible())) await d.click('.toolbar-sidebar-tab-left', { ms: 700, after: 600 });
    await d.click(find, { ms: 800 });
    await d.click('.find-search', { ms: 600, after: 200 });
    await d.type('soprano pip', 90);
    const species = page.locator('.find-species-row', { hasText: 'Soprano Pipistrelle' }).first();
    await species.waitFor({ timeout: 20000 });
    await d.hold(500);
    await d.click(species, { ms: 700 });
    // XC1037039, from Cornwall. The list comes from the search server, so
    // find the row by its place, not its position.
    const row = page.locator('.find-rec-row', { hasText: 'Caervallack' }).first();
    await row.waitFor({ timeout: 20000 });
    await d.hold(400);
    await d.glide(row, 700);
    await d.hold(300);
    await d.click(row.locator('.find-rec-load'), { ms: 500, after: 700 });
    // The download from Xeno-canto takes a while: leave it out of the clip.
    d.cut();
    await page.waitForSelector('.band-gutter', { timeout: 90000 });
    await d.wait(2500);
    d.resume();
    await d.hold(1500);
}

/** Ctrl + scroll wheel to zoom out to the whole recording and back in, then drag along. */
export async function zoomAndPan(d) {
    await d.caption(...TEXT.zoom);
    const b = await d.page.locator(MAIN).first().boundingBox();
    await d.glide([b.x + b.width * 0.45, b.y + b.height * 0.55], 600);
    await d.keys(['Ctrl']);
    await d.wait(250);
    let ticks = 0;
    await d.page.keyboard.down('Control');
    for (;;) {
        const s = await d.state();
        if (s.visibleTime >= s.duration * 0.97 || ticks > 40) break;
        await d.page.mouse.wheel(0, 100);
        ticks++;
        await d.wait(45);
    }
    await d.hold(900);
    for (let i = 0; i < ticks; i++) {
        await d.page.mouse.wheel(0, -100);
        await d.wait(45);
    }
    await d.page.keyboard.up('Control');
    await d.wait(250);
    await d.keys([]);
    await d.wait(300);
    // Drag left to move later in the recording.
    const y = b.y + b.height * 0.55;
    await d.drag([b.x + b.width * 0.8, y], [b.x + b.width * 0.2, y], { ms: 1100, after: 900 });
}

/** HFR on, choose the band on the frequency strip and the part on the time strip, Play. */
export async function hearWithHfr(d, rec, { timeStrip = true } = {}) {
    const { page } = d;
    await d.caption(TEXT.hear);
    const s = await d.state();
    if (!s.hfrOn) {
        await d.click('.hearing-bar .hfr-switch .switch-btn-top', { ms: 800, after: 900 });
    }
    const [top, bottom] = rec.band;
    await d.drag(await d.stripAt(top), await d.stripAt(bottom), { ms: 1100, after: 900 });
    if (timeStrip) {
        const v = await d.state();
        const t0 = v.scrollOffset + v.visibleTime * 0.06;
        const t1 = v.scrollOffset + v.visibleTime * 0.85;
        await d.drag(await d.timeStripAt(t0), await d.timeStripAt(t1), { ms: 1300, after: 700 });
    }
    await d.click('.play-main-btn', { ms: 800, after: 2800 });
}

/** HFR on with the recording's band, at once (before a clip starts). */
export async function setBand(d, rec) {
    if (!(await d.state()).hfrOn) {
        await d.page.locator('.hearing-bar .hfr-switch .switch-btn-top').click();
        await d.wait(500);
    }
    const [top, bottom] = rec.band;
    const from = await d.stripAt(top);
    const to = await d.stripAt(bottom);
    await d.page.mouse.move(...from);
    await d.page.mouse.down();
    await d.page.mouse.move(...to, { steps: 8 });
    await d.page.mouse.up();
    await d.wait(500);
}

/**
 * XForm on (the spectrogram of the sound as played), then HET, TE, PS and
 * ZC. Leaves XForm off. Needs HFR on with a band round the calls.
 */
export async function playbackModes(d, { backToPs = true } = {}) {
    const { page } = d;
    await d.caption(...TEXT.modes);
    const xform = page.getByRole('switch', { name: /^XForm/ });
    await d.click(xform, { ms: 800, after: 0 });
    await d.wait(Math.max(1200, 1600 * d.pace));
    // XForm takes about 1.5 s to redraw after each change.
    for (const m of ['HET', 'TE', 'PS', 'ZC']) {
        await d.click(button(page, m, '.hearing-bar'), { ms: 450, after: 0 });
        await d.wait(Math.max(1700, 2000 * d.pace));
    }
    // PS is the mode HFR starts in, so the short clip loops back to its start.
    if (backToPs) await d.click(button(page, 'PS', '.hearing-bar'), { ms: 450, after: 200 });
    await d.click(xform, { ms: 500, after: 1200 });
}

/** Spec, Wave, Dots, back to Spec. */
export async function views(d) {
    const { page } = d;
    await d.caption(TEXT.views);
    for (const name of ['Wave', 'Dots', 'Spec']) {
        await d.click(button(page, name), { ms: 600, after: 1500 });
    }
}

/** Selection tool, box round a call, "..." > Annotate Region, type a label. */
export async function markCall(d, rec) {
    const { page } = d;
    await d.caption(TEXT.mark);
    await d.click('.tool-combo-btn', { ms: 800, after: 400 });
    await d.click(page.locator('.layer-panel button', { hasText: 'Selection' }), { ms: 500, after: 500 });
    // Zoom in so the box round a call (a few milliseconds long) is wide enough to see.
    const c = await d.page.locator(MAIN).first().boundingBox();
    await d.glide([c.x + c.width * 0.4, c.y + c.height * 0.45], 600);
    await d.keys(['Ctrl']);
    await d.page.keyboard.down('Control');
    for (let i = 0; i < 30 && (await d.state()).visibleTime > 0.75; i++) {
        await d.page.mouse.wheel(0, -100);
        await d.wait(45);
    }
    await d.page.keyboard.up('Control');
    await d.keys([]);
    await d.wait(500);
    const v = await d.state();
    const mid = v.scrollOffset + v.visibleTime * 0.4;
    const t = rec.calls
        .filter((c) => c > v.scrollOffset + v.visibleTime * 0.1 && c < v.scrollOffset + v.visibleTime * 0.75)
        .sort((a, b) => Math.abs(a - mid) - Math.abs(b - mid))[0];
    if (t === undefined) throw new Error('No call in view to mark');
    await d.drag(await d.at(t - 0.01, 90), await d.at(t + 0.035, 45), { ms: 900, after: 500 });
    await d.click('.canvas-overflow-btn', { ms: 600, after: 500 });
    await d.click(page.locator('.canvas-overflow-item', { hasText: 'Annotate Region' }), { ms: 500, after: 500 });
    await page.locator('.annotation-inline-label-editor').waitFor({ timeout: 10000 });
    await d.type('Soprano Pipistrelle', 65);
    await page.keyboard.press('Enter');
    await d.hold(1600);
}

/** Opens the Call detection panel; its boxes appear round the calls. */
export async function detectCalls(d) {
    const { page } = d;
    await d.caption(TEXT.detect);
    if (!(await page.locator('.right-sidebar').isVisible())) {
        await d.click('.toolbar-sidebar-tab-right', { ms: 900, after: 500 });
    }
    await d.click('.sidebar-tab-dropdown', { ms: 600, after: 400 });
    await d.click(page.locator('.sidebar-tab-option', { hasText: 'Call detection' }), { ms: 500, after: 600 });
    // Point at the count of bat calls while the boxes are drawn.
    const bats = page
        .locator('.right-sidebar')
        .getByText('Bat call', { exact: true })
        .filter({ visible: true })
        .first();
    await bats.waitFor({ timeout: 30000 });
    await d.glide(bats, 900);
    await d.hold(2800);
}

/** Bat book on, then a second species to compare. */
export async function batBook(d) {
    const { page } = d;
    await d.caption(TEXT.book);
    await d.click(page.locator('label.overlay-check', { hasText: 'Bat book' }), { ms: 800, after: 2000 });
    const other = page.locator('.bat-book-chip', { hasText: 'Common Pipistrelle' }).first();
    await d.click(other, { ms: 800, after: 2200 });
}

/** Selection round a few calls, then "..." > View in 3D, then drag to turn it. */
export async function view3d(d, rec) {
    const { page } = d;
    await d.caption(TEXT.view3d);
    await d.click('.tool-combo-btn', { ms: 800, after: 400 });
    await d.click(page.locator('.layer-panel button', { hasText: 'Selection' }), { ms: 500, after: 400 });
    const v = await d.state();
    const t0 = v.scrollOffset + v.visibleTime * 0.08;
    const t1 = v.scrollOffset + v.visibleTime * 0.32;
    await d.drag(await d.at(t0, 95), await d.at(t1, 15), { ms: 1000, after: 500 });
    await d.click('.canvas-overflow-btn', { ms: 600, after: 500 });
    await d.click(page.locator('.canvas-overflow-item', { hasText: 'View in 3D' }), { ms: 500, after: 300 });
    // Software WebGL in headless Chromium takes a while to draw the surface.
    d.cut();
    await page.locator('.surface-3d-plot .main-svg').first().waitFor({ state: 'attached', timeout: 120000 });
    await page.locator('.surface-3d-status').waitFor({ state: 'detached', timeout: 120000 });
    await d.wait(3000);
    d.resume();
    await d.hold(800);
    // A small turn: a long drag spins it to a top-down view.
    const plot = await page.locator('.surface-3d-plot').boundingBox();
    const cy = plot.y + plot.height * 0.3;
    await d.drag([plot.x + plot.width * 0.58, cy], [plot.x + plot.width * 0.46, cy], { ms: 1800, after: 2200 });
}

/** Files tab: Select, pick three recordings, Create Timeline. */
export async function timeline(d, names) {
    const { page } = d;
    await d.caption(TEXT.timeline);
    const select = page.getByRole('button', { name: 'Select', exact: true });
    await d.click(select, { ms: 800, after: 500 });
    for (const n of names) {
        await d.click(page.locator('.file-item', { hasText: n.slice(0, 9) }).first(), { ms: 500, after: 350 });
    }
    await d.hold(500);
    await d.click(page.getByRole('button', { name: 'Create Timeline' }), { ms: 700, after: 2500 });
}

/**
 * Starts live listening fed by the app's Replay test signal (the open
 * recording, looped), zoomed in so the calls are wide enough to see. Run
 * before the clip starts: the title bar says "Test signal: Replay" throughout.
 */
export async function startReplay(d) {
    await d.page.evaluate(() => window.__synthStart('replay'));
    await d.page.waitForFunction(() => window.__oversample_test().liveActive, null, { timeout: 30000 });
    const b = await d.page.locator(MAIN).first().boundingBox();
    await d.page.mouse.move(b.x + b.width / 2, b.y + b.height / 2);
    await d.page.keyboard.down('Control');
    for (let i = 0; i < 30 && (await d.state()).visibleTime > 2.6; i++) {
        await d.page.mouse.wheel(0, -100);
        await d.wait(40);
    }
    await d.page.keyboard.up('Control');
    await d.wait(3000);
}

/** Live view scrolling; HFR on and a band chosen on the frequency strip. */
export async function listenLive(d, rec) {
    await d.caption(...TEXT.live);
    await d.hold(2200);
    if (!(await d.state()).hfrOn) {
        await d.click('.hearing-bar .hfr-switch .switch-btn-top', { ms: 800, after: 900 });
    }
    const [top, bottom] = rec.band;
    await d.drag(await d.stripAt(top), await d.stripAt(bottom), { ms: 1100, after: 2600 });
}
