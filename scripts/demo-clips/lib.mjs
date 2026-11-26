// Shared harness for the demo clips: opens Oversample in headless Chromium,
// records the page, draws a pointer, captions and title cards over it, then
// trims the recording and encodes it. See record.mjs for how to run it.
import { chromium, devices } from 'playwright';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, extname, join, resolve, sep } from 'node:path';

export const W = 1280;
export const H = 720;
/** Height of the caption band above the app, in CSS pixels. */
const BAND = 56;
/** Phone clips: a Pixel 7 (412 x 839 page, touch), recorded at twice its CSS pixels. */
const PHONE = devices['Pixel 7'];
const PHONE_BAND = 64;

const MIME = {
    '.html': 'text/html',
    '.js': 'text/javascript',
    '.mjs': 'text/javascript',
    '.wasm': 'application/wasm',
    '.css': 'text/css',
    '.json': 'application/json',
    '.png': 'image/png',
    '.jpg': 'image/jpeg',
    '.svg': 'image/svg+xml',
    '.ico': 'image/x-icon',
    '.txt': 'text/plain',
    '.tsv': 'text/tab-separated-values',
    '.webmanifest': 'application/manifest+json',
    '.woff2': 'font/woff2',
};

/**
 * Answers every request to `origin` from the files in `root`, so a clip can
 * run a built `dist` folder at the origin the search server accepts
 * (http://localhost:21259) without a server on that port.
 */
async function serveDist(ctx, origin, root) {
    await ctx.route(`${origin}/**`, (route) => {
        let path = decodeURIComponent(new URL(route.request().url()).pathname);
        if (path.endsWith('/')) path += 'index.html';
        const file = resolve(root, `.${path}`);
        if (!file.startsWith(root + sep) || !existsSync(file) || statSync(file).isDirectory()) {
            return route.fulfill({ status: 404, body: 'Not found' });
        }
        return route.fulfill({
            path: file,
            contentType: MIME[extname(file)] ?? 'application/octet-stream',
        });
    });
}

/**
 * Runs in the page before the app on every load: turns off the suggestion
 * cards, and adds the pointer, the caption band, the key badge and the title
 * card. Headless video has no pointer of its own.
 */
function overlay({ band, phone }) {
    try {
        localStorage.setItem('oversample_hfr_nudge_enabled', 'false');
        localStorage.setItem('oversample_gain_nudge_enabled', 'false');
    } catch {}
    const arrow =
        '<svg xmlns="http://www.w3.org/2000/svg" width="26" height="26" viewBox="0 0 20 20">' +
        '<path d="M2 2 L2 16 L6 12 L9 18 L11.5 17 L8.6 11 L14 11 Z" fill="white" stroke="black" stroke-width="1.2"/></svg>';
    // A fingertip on phones, an arrow elsewhere.
    const pointer = phone
        ? `width: 34px; height: 34px; margin: -17px 0 0 -17px; border-radius: 50%;
           background: rgba(255, 255, 255, .35); border: 2px solid rgba(255, 255, 255, .85);`
        : `width: 26px; height: 26px; margin: -3px 0 0 -3px;
           background: url("data:image/svg+xml,${encodeURIComponent(arrow)}") no-repeat;`;
    const press = phone
        ? `#demo-cursor.down { background: rgba(240, 192, 80, .55); border-color: #f0c050; }`
        : `#demo-cursor.down::after { content: ''; position: absolute; left: -14px; top: -14px;
             width: 28px; height: 28px; border: 3px solid #f0c050; border-radius: 50%; }`;
    const css = `
      #demo-cursor { position: fixed; z-index: 2147483647; pointer-events: none; left: 50%; top: 50%; ${pointer} }
      ${press}
      #demo-keys { position: absolute; left: 24px; top: 22px; display: none; gap: 4px; }
      #demo-keys.on { display: flex; }
      #demo-keys span { font: 600 13px/1 system-ui, sans-serif; color: #111; background: #f2f2f2;
        border: 1px solid #999; border-bottom-width: 3px; border-radius: 5px; padding: 4px 7px; }
      #demo-band { position: fixed; z-index: 2147483646; left: 0; right: 0; top: 0; height: ${band}px;
        display: flex; flex-direction: column; align-items: center; justify-content: center;
        background: #0c0c0c; border-bottom: 1px solid #2c2c2c; pointer-events: none;
        font-family: "Segoe UI", system-ui, sans-serif; }
      #demo-band .main { color: #f4f4f4; font-size: ${phone ? 17 : 21}px; font-weight: 600;
        line-height: 1.2; text-align: center; padding: 0 12px; transition: opacity .25s; }
      #demo-band .sub { color: #a8b0a8; font-size: ${phone ? 12 : 13}px; margin-top: 2px;
        text-align: center; padding: 0 12px; transition: opacity .25s; }
      #demo-band .sub:empty { display: none; }
      #demo-band.fade .main, #demo-band.fade .sub { opacity: 0; }
      #demo-card { position: fixed; z-index: 2147483646; inset: 0; display: flex; flex-direction: column;
        align-items: center; justify-content: center; gap: 14px; background: rgba(12, 12, 12, .96);
        font-family: "Segoe UI", system-ui, sans-serif; opacity: 0; pointer-events: none; transition: opacity .35s; }
      #demo-card.on { opacity: 1; }
      #demo-card .title { color: #f4f4f4; font-size: ${phone ? 40 : 64}px; font-weight: 700; letter-spacing: -.5px; }
      #demo-card .sub { color: #b8c0b8; font-size: ${phone ? 17 : 24}px; text-align: center; padding: 0 28px; }
      ${band ? `.app { height: calc(100dvh - ${band}px) !important; margin-top: ${band}px; }
        .app-toast-container { top: ${44 + band}px !important; }` : ''}
    `;
    const add = () => {
        if (document.getElementById('demo-cursor')) return;
        const style = document.createElement('style');
        style.textContent = css;
        document.head.appendChild(style);
        const cursor = document.createElement('div');
        cursor.id = 'demo-cursor';
        cursor.innerHTML = '<div id="demo-keys"></div>';
        document.body.appendChild(cursor);
        if (band) {
            const b = document.createElement('div');
            b.id = 'demo-band';
            b.innerHTML = '<div class="main"></div><div class="sub"></div>';
            document.body.appendChild(b);
        }
        const card = document.createElement('div');
        card.id = 'demo-card';
        card.innerHTML = '<div class="title"></div><div class="sub"></div>';
        document.body.appendChild(card);
        const move = (e) => {
            cursor.style.left = `${e.clientX}px`;
            cursor.style.top = `${e.clientY}px`;
        };
        addEventListener('pointermove', move, true);
        addEventListener('pointerdown', (e) => { move(e); cursor.classList.add('down'); }, true);
        addEventListener('pointerup', () => cursor.classList.remove('down'), true);
    };
    if (document.body) add();
    else addEventListener('DOMContentLoaded', add);
}

/** Where a target is: a CSS selector, a Playwright locator, or [x, y]. */
async function centreOf(page, target, fx = 0.5, fy = 0.5) {
    if (Array.isArray(target)) return target;
    const loc = typeof target === 'string' ? page.locator(target).first() : target;
    await loc.waitFor({ state: 'visible', timeout: 30000 });
    let b = await loc.boundingBox();
    const { width: vw, height: vh } = page.viewportSize();
    if (b.y < 0 || b.y + b.height > vh || b.x < 0 || b.x + b.width > vw) {
        // Off screen in a scrolling panel: scroll it into view as a person would.
        await loc.evaluate((el) => el.scrollIntoView({ behavior: 'smooth', block: 'center' }));
        await page.waitForTimeout(700);
        b = await loc.boundingBox();
    }
    return [b.x + b.width * fx, b.y + b.height * fy];
}

export class Demo {
    static async start({ url, dist, captions = true, phone = false }) {
        const d = new Demo();
        d.origin = new URL(url).origin;
        d.url = url;
        d.captions = captions;
        d.phone = phone;
        d.w = phone ? PHONE.viewport.width : W;
        d.h = phone ? PHONE.viewport.height : H;
        d.browser = await chromium.launch();
        d.videoDir = mkdtempSync(join(tmpdir(), 'oversample-demo-'));
        const view = phone
            ? { ...PHONE, deviceScaleFactor: 2 }
            : { viewport: { width: W, height: H } };
        const scale = phone ? 2 : 1;
        d.ctx = await d.browser.newContext({
            ...view,
            recordVideo: { dir: d.videoDir, size: { width: d.w * scale, height: d.h * scale } },
            serviceWorkers: 'block',
        });
        if (dist) await serveDist(d.ctx, d.origin, resolve(dist));
        const band = captions ? (phone ? PHONE_BAND : BAND) : 0;
        await d.ctx.addInitScript(overlay, { band, phone });
        d.page = await d.ctx.newPage();
        // The video starts when the page is made.
        d.t0 = Date.now();
        d.x = d.w / 2;
        d.y = d.h / 2;
        // Parts of the recording that make the clip: [start, end] in seconds.
        d.segments = [];
        d.beginOnCaption = false;
        d.posterShot = null;
        // Scales pointer moves, typing and holds (not waits for the app).
        d.pace = 1;
        return d;
    }

    now() {
        return (Date.now() - this.t0) / 1000;
    }

    /** Opens the app at `hash` (a new page load, unlike a change of hash alone). */
    async goto(hash = '') {
        await this.page.goto('about:blank');
        await this.page.goto(new URL(hash, this.url).href);
        await this.page.waitForSelector('.app', { timeout: 60000 });
    }

    /**
     * Opens a recording by its Xeno-canto number, so the title bar shows its
     * licence. Without the network, opens the local copy from bat-demo-sounds.
     */
    async openRecording(rec) {
        await this.goto(`#XC${rec.xc}`);
        try {
            await this.page.waitForSelector('.band-gutter', { timeout: 30000 });
        } catch {
            const file = join(rec.soundsDir, rec.file);
            console.warn(`XC${rec.xc} did not load from the network; opening ${file}`);
            await this.page.locator('input[type=file]').first().setInputFiles(file);
            await this.page.waitForSelector('.band-gutter', { timeout: 60000 });
        }
    }

    /** Snapshot from the app's read-only test hook (src/test_hook.rs). */
    state() {
        return this.page.evaluate(() => window.__oversample_test());
    }

    /** Screen position of a time (s) and frequency (kHz) on the main view. */
    async at(t, khz) {
        const s = await this.state();
        const b = await this.page.locator('.main-view .chart-stage > canvas').first().boundingBox();
        const x = b.x + ((t - s.scrollOffset) / s.visibleTime) * b.width;
        const lo = s.displayFreqLo / 1000;
        const hi = s.displayFreqHi / 1000;
        const y = b.y + (1 - (khz - lo) / (hi - lo)) * b.height;
        return [x, y];
    }

    /** Screen y of a frequency (kHz) on the frequency strip, and the strip's x. */
    async stripAt(khz, fx = 0.7) {
        const s = await this.state();
        const g = await this.page.locator('.band-gutter').first().boundingBox();
        const lo = s.displayFreqLo / 1000;
        const hi = s.displayFreqHi / 1000;
        return [g.x + g.width * fx, g.y + (1 - (khz - lo) / (hi - lo)) * g.height];
    }

    /** Screen x of a time (s) on the time strip, and the strip's y. */
    async timeStripAt(t) {
        const s = await this.state();
        const g = await this.page.locator('.time-gutter').first().boundingBox();
        return [g.x + ((t - s.scrollOffset) / s.visibleTime) * g.width, g.y + g.height / 2];
    }

    /** Waits for the app. */
    wait(ms) {
        return this.page.waitForTimeout(ms);
    }

    /** Holds still so the viewer can take something in (scaled by pace). */
    hold(ms) {
        return this.page.waitForTimeout(ms * this.pace);
    }

    /** Clip starts here. */
    begin() {
        this.segments.push([this.now(), null]);
    }

    /** Clip starts when the next caption has appeared, so its first frame has one. */
    beginWithCaption() {
        this.beginOnCaption = true;
    }

    /** Leaves out what follows (a download, say) until resume(). */
    cut() {
        const last = this.segments.at(-1);
        if (last && last[1] === null) last[1] = this.now();
    }

    resume() {
        this.begin();
    }

    /** Clip ends here. */
    end() {
        this.cut();
    }

    duration() {
        return this.segments.reduce((sum, [a, b]) => sum + (b - a), 0);
    }

    /** Takes the poster image now (default: when the clip ends). */
    async poster() {
        this.posterShot = await this.page.screenshot({ type: 'jpeg', quality: 85 });
    }

    /** Sets the caption band's text; `null` empties it. */
    async caption(main, sub = '') {
        if (this.captions) await this.showCaption(main, sub);
        if (this.beginOnCaption && !this.segments.length) {
            if (this.captions) await this.wait(300);
            this.begin();
        }
    }

    async showCaption(main, sub) {
        await this.page.evaluate(async ([main, sub]) => {
            const b = document.getElementById('demo-band');
            if (!b) return;
            const m = b.querySelector('.main');
            const s = b.querySelector('.sub');
            if (m.textContent === (main ?? '') && s.textContent === (sub ?? '')) return;
            b.classList.add('fade');
            await new Promise((r) => setTimeout(r, 250));
            m.textContent = main ?? '';
            s.textContent = sub ?? '';
            b.classList.remove('fade');
        }, [main, sub]);
    }

    /** Full-screen title card for `ms`, then fades away. */
    async card(title, sub, ms) {
        await this.page.evaluate(([title, sub]) => {
            const c = document.getElementById('demo-card');
            c.querySelector('.title').textContent = title;
            c.querySelector('.sub').textContent = sub ?? '';
            c.classList.add('on');
        }, [title, sub]);
        await this.wait(ms);
        await this.page.evaluate(() => document.getElementById('demo-card').classList.remove('on'));
        await this.wait(400);
    }

    /** Card that stays up (the end of a clip). */
    async endCard(title, sub, ms) {
        await this.page.evaluate(([title, sub]) => {
            const c = document.getElementById('demo-card');
            c.querySelector('.title').textContent = title;
            c.querySelector('.sub').textContent = sub ?? '';
            c.classList.add('on');
        }, [title, sub]);
        await this.wait(ms);
    }

    /** Shows key caps beside the pointer (`[]` hides them). */
    async keys(names) {
        await this.page.evaluate((names) => {
            const k = document.getElementById('demo-keys');
            k.innerHTML = names.map((n) => `<span>${n}</span>`).join('');
            k.classList.toggle('on', names.length > 0);
        }, names);
    }

    /** Puts the pointer somewhere at once (before a clip starts). */
    async place(x = this.w / 2, y = this.h * 0.6) {
        await this.page.mouse.move(x, y);
        this.x = x;
        this.y = y;
    }

    /** Moves the pointer to a target with an ease-in-out glide. */
    async glide(target, ms = 600) {
        const [tx, ty] = await centreOf(this.page, target);
        const { x, y } = this;
        // By the clock, not by step count: each move takes a while to send.
        const total = ms * this.pace;
        const t0 = Date.now();
        for (;;) {
            const k = Math.min(1, (Date.now() - t0) / total);
            const e = k < 0.5 ? 2 * k * k : 1 - Math.pow(-2 * k + 2, 2) / 2;
            await this.page.mouse.move(x + (tx - x) * e, y + (ty - y) * e);
            if (k >= 1) break;
            await this.page.waitForTimeout(12);
        }
        this.x = tx;
        this.y = ty;
    }

    /** Glides to a target and clicks it. */
    async click(target, { ms = 700, before = 220, after = 600 } = {}) {
        await this.glide(target, ms);
        await this.hold(before);
        await this.page.mouse.down();
        await this.wait(110);
        await this.page.mouse.up();
        await this.hold(after);
    }

    /** Presses at `from` and drags to `to`. */
    async drag(from, to, { ms = 1000, after = 500 } = {}) {
        await this.glide(from, 700);
        await this.hold(220);
        await this.page.mouse.down();
        await this.glide(to, ms);
        await this.wait(140);
        await this.page.mouse.up();
        await this.hold(after);
    }

    /** Scroll wheel ticks at the pointer, with keys held (for example ['Control']). */
    async wheel(ticks, { dy = 100, gap = 70, hold = [] } = {}) {
        for (const k of hold) await this.page.keyboard.down(k);
        for (let i = 0; i < ticks; i++) {
            await this.page.mouse.wheel(0, dy);
            await this.wait(gap);
        }
        for (const k of hold) await this.page.keyboard.up(k);
    }

    async type(text, delay = 75) {
        await this.page.keyboard.type(text, { delay: delay * this.pace });
    }

    /**
     * Closes the browser and writes `<out>.mp4` (H.264, no sound) and
     * `<out>-poster.jpg`, plus `<out>.gif` when `gif` is set.
     */
    async finish(out, { gif = false } = {}) {
        this.end();
        if (!this.segments.length) throw new Error('The clip never started (no begin())');
        if (!this.posterShot) await this.poster();
        const video = this.page.video();
        await this.ctx.close();
        await this.browser.close();
        const webm = await video.path();
        mkdirSync(dirname(resolve(out)), { recursive: true });
        writeFileSync(`${out}-poster.jpg`, this.posterShot);
        // Trimming by wall-clock offsets is close enough: the video clock
        // starts a moment after t0, and the clip starts on a settled page.
        const parts = this.segments.map(
            ([a, b], i) => `[0:v]trim=${a.toFixed(3)}:${b.toFixed(3)},setpts=PTS-STARTPTS[v${i}]`,
        );
        const joined = this.segments.map((_, i) => `[v${i}]`).join('');
        execFileSync('ffmpeg', [
            '-y', '-loglevel', 'error', '-i', webm,
            '-filter_complex', `${parts.join(';')};${joined}concat=n=${parts.length}:v=1:a=0[out]`,
            '-map', '[out]',
            '-an', '-c:v', 'libx264', '-preset', 'slow', '-crf', '26',
            '-pix_fmt', 'yuv420p', '-r', '30', '-movflags', '+faststart',
            `${out}.mp4`,
        ]);
        if (gif) {
            execFileSync('ffmpeg', [
                '-y', '-loglevel', 'error', '-i', `${out}.mp4`,
                '-vf',
                'fps=12,scale=960:-1:flags=lanczos,split[a][b];' +
                    '[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle',
                '-loop', '0', `${out}.gif`,
            ]);
        }
        rmSync(this.videoDir, { recursive: true, force: true });
        return this.duration();
    }
}
