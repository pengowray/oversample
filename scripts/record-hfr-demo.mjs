// Records the HFR demo clip: turn on HFR, drag the frequency gutter, drag the
// time gutter, press Play. Output is a muted, looping MP4 for web pages.
//
//   trunk serve --release --port 21411        (any free port)
//   node scripts/record-hfr-demo.mjs http://localhost:21411/ media/hfr-demo
//
// Set BAT_DEMO_SOUNDS to the bat-demo-sounds checkout when it is not at
// ../batchi-extras/bat-demo-sounds (for example from a git worktree).
//
// Writes <out>.mp4 (H.264) and <out>-poster.jpg. Needs Playwright (npm
// install) and ffmpeg with libx264. The recording is XC1087684 from
// bat-demo-sounds (CC BY-NC-SA 4.0); its title and licence stay in frame.
//
// MP4/H.264, not GIF or animated WebP. On the 17.5 s clip: MP4 810 KB at
// 30 fps; animated WebP 990 KB at 15 fps; GIF 5.0 MB at 15 fps, with the
// spectrogram's gradients banded by its 256 colours. Every browser plays
// <video autoplay muted loop playsinline poster=...> MP4.
import { chromium } from 'playwright';
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readdirSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const [url = 'http://localhost:21411/', out = 'media/hfr-demo'] = process.argv.slice(2);
const here = dirname(fileURLToPath(import.meta.url));
// The bat-demo-sounds checkout, a sibling of this repository (see CLAUDE.md).
const SOUNDS = process.env.BAT_DEMO_SOUNDS ?? resolve(here, '../../batchi-extras/bat-demo-sounds');
const FILE = join(SOUNDS, 'sounds/XC1087684 - Nut-colored Yellow Bat - Scotophilus nux.wav');
const W = 1280;
const H = 720;

mkdirSync(dirname(resolve(out)), { recursive: true });
const videoDir = mkdtempSync(join(tmpdir(), 'hfr-demo-'));
const browser = await chromium.launch();
const ctx = await browser.newContext({
    viewport: { width: W, height: H },
    recordVideo: { dir: videoDir, size: { width: W, height: H } },
});
const page = await ctx.newPage();
const t0 = Date.now();

await page.goto(url);
await page.waitForSelector('.app', { timeout: 60000 });
// No suggestion card in the clip.
await page.evaluate(() => localStorage.setItem('oversample_hfr_nudge_enabled', 'false'));
// Open it by its Xeno-canto number so the title bar shows its licence badge.
// Without network, fall back to the local copy (no licence badge).
// (about:blank first: a change of hash alone would not reload the page.)
await page.goto('about:blank');
await page.goto(new URL('#XC1087684', url).href);
await page.waitForSelector('.app', { timeout: 60000 });
try {
    await page.waitForSelector('.band-gutter', { timeout: 30000 });
} catch {
    console.warn('XC1087684 did not load from the network; using the local file');
    await page.locator('input[type=file]').first().setInputFiles(FILE);
    await page.waitForSelector('.band-gutter', { timeout: 60000 });
}
// Hide the sidebar so the spectrogram gets the width.
await page.locator('.toolbar-sidebar-tab-left').click();
await page.waitForTimeout(2500);

// Headless video has no pointer, so draw one, plus a ring on each press.
await page.addStyleTag({
    content: `
    #demo-cursor { position: fixed; z-index: 2147483647; pointer-events: none;
      width: 20px; height: 20px; margin: -2px 0 0 -2px;
      background: url("data:image/svg+xml,${encodeURIComponent(
          '<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><path d="M2 2 L2 16 L6 12 L9 18 L11.5 17 L8.6 11 L14 11 Z" fill="white" stroke="black" stroke-width="1.2"/></svg>',
      )}") no-repeat; }
    #demo-cursor.down::after { content: ''; position: absolute; left: -12px; top: -12px;
      width: 24px; height: 24px; border: 2px solid #f0c050; border-radius: 50%; }`,
});
await page.evaluate(() => {
    const c = document.createElement('div');
    c.id = 'demo-cursor';
    document.body.appendChild(c);
    const move = (e) => { c.style.left = e.clientX + 'px'; c.style.top = e.clientY + 'px'; };
    addEventListener('pointermove', move, true);
    addEventListener('pointerdown', (e) => { move(e); c.classList.add('down'); }, true);
    addEventListener('pointerup', () => c.classList.remove('down'), true);
});

let x = W / 2;
let y = H / 2;
async function glide(tx, ty, ms = 600) {
    const steps = Math.max(8, Math.round(ms / 16));
    for (let i = 1; i <= steps; i++) {
        const k = i / steps;
        const e = k < 0.5 ? 2 * k * k : 1 - Math.pow(-2 * k + 2, 2) / 2;
        await page.mouse.move(x + (tx - x) * e, y + (ty - y) * e);
        await page.waitForTimeout(16);
    }
    x = tx;
    y = ty;
}
async function centre(sel) {
    const b = await page.locator(sel).first().boundingBox();
    return [b.x + b.width / 2, b.y + b.height / 2, b];
}

await page.mouse.move(x, y);
await page.waitForTimeout(300);
const clipStart = (Date.now() - t0) / 1000;
await page.waitForTimeout(700);

// 1. HFR on.
let [hx, hy] = await centre('.hearing-bar .band-cell-hfr');
await glide(hx, hy, 900);
await page.waitForTimeout(250);
await page.mouse.down();
await page.waitForTimeout(120);
await page.mouse.up();
await page.waitForTimeout(1100);

// 2. Drag the frequency gutter over the calls (about 25–60 kHz here).
const [, , g] = await centre('.band-gutter');
const gx = g.x + g.width * 0.7;
const yAt = (khz) => g.y + g.height * (1 - khz / 192);
await glide(gx, yAt(60), 900);
await page.waitForTimeout(250);
await page.mouse.down();
await glide(gx, yAt(25), 1100);
await page.waitForTimeout(150);
await page.mouse.up();
await page.waitForTimeout(1100);

// 3. Drag the time gutter across the first calls.
const [, , tg] = await centre('.time-gutter');
const ty = tg.y + tg.height / 2;
await glide(tg.x + tg.width * 0.08, ty, 900);
await page.waitForTimeout(250);
await page.mouse.down();
await glide(tg.x + tg.width * 0.55, ty, 1200);
await page.waitForTimeout(150);
await page.mouse.up();
await page.waitForTimeout(900);

// 4. Play, and let the playhead run.
const [px, py] = await centre('.play-main-btn');
await glide(px, py, 900);
await page.waitForTimeout(250);
await page.mouse.down();
await page.waitForTimeout(120);
await page.mouse.up();
await page.waitForTimeout(1800);
const clipEnd = (Date.now() - t0) / 1000;

await page.screenshot({ path: `${out}-poster.jpg`, type: 'jpeg', quality: 85 });
const video = page.video();
await ctx.close();
await browser.close();
const webm = await video.path();

// The video clock starts slightly after t0; trimming by wall-clock offsets is
// close enough for a demo (the first frames are the settled page anyway).
execFileSync('ffmpeg', [
    '-y', '-loglevel', 'error',
    '-ss', clipStart.toFixed(2), '-to', clipEnd.toFixed(2), '-i', webm,
    '-an', '-c:v', 'libx264', '-preset', 'slow', '-crf', '26',
    '-pix_fmt', 'yuv420p', '-r', '30', '-movflags', '+faststart',
    `${out}.mp4`,
]);
for (const f of readdirSync(videoDir)) rmSync(join(videoDir, f));
rmSync(videoDir, { recursive: true });
console.log(`wrote ${out}.mp4 and ${out}-poster.jpg (${(clipEnd - clipStart).toFixed(1)} s)`);
