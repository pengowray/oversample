// Records the demo clips in media/: short silent looping videos of one feature
// each, and a tour that goes through them in one session.
//
//   trunk serve --release --port 21259
//   node scripts/demo-clips/record.mjs                  (every clip)
//   node scripts/demo-clips/record.mjs tour hfr-demo    (some clips)
//
// Options:
//   --url URL       the app (default http://localhost:21259/). The species
//                   search only answers pages from localhost:21259 and the
//                   live site.
//   --dist DIR      serve a built dist folder at --url instead of using a
//                   server there, so rebuilds cannot reload a page mid-clip:
//                   trunk build --release --dist target/dist-demo
//                   node scripts/demo-clips/record.mjs --dist target/dist-demo
//   --out DIR       where to write the clips (default media)
//   --gif           also write a GIF of each clip (960 px wide, 12 fps)
//   --no-captions   no caption band above the app
//
// Each clip is <name>.mp4 (H.264, no sound, 1280 x 720) and
// <name>-poster.jpg. MP4, not GIF: on the 17.5 s HFR clip, MP4 was 810 KB,
// animated WebP 990 KB and GIF 5.0 MB, with the spectrogram's gradients
// banded by the GIF's 256 colours. Every browser plays
// <video autoplay muted loop playsinline poster=...> MP4.
//
// Needs Playwright (npm install), ffmpeg with libx264, and the network (the
// recordings open from Xeno-canto). Without the network the recordings open
// from bat-demo-sounds: set BAT_DEMO_SOUNDS when it is not at
// ../batchi-extras/bat-demo-sounds. The recordings and their licences are
// listed in media/README.md.
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Demo } from './lib.mjs';
import * as s from './steps.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const soundsDir = join(
    process.env.BAT_DEMO_SOUNDS ?? resolve(here, '../../../batchi-extras/bat-demo-sounds'),
    'sounds',
);
const rec = Object.fromEntries(
    Object.entries(s.RECORDINGS).map(([k, r]) => [k, { ...r, soundsDir }]),
);

/** Opens a recording, hides the file list, and waits for the spectrogram. */
async function opened(d, r) {
    await d.openRecording(r);
    await s.hideFiles(d, { quick: true });
    await d.place();
    await d.wait(3000);
    d.beginWithCaption();
}

const CLIPS = {
    'open-recording': async (d) => {
        await d.goto('');
        await d.place();
        await d.wait(2500);
        d.beginWithCaption();
        await s.openFromSearch(d, rec.pip);
        await d.wait(800);
    },
    'zoom-pan': async (d) => {
        await opened(d, rec.pip);
        await s.zoomAndPan(d);
    },
    'hfr-demo': async (d) => {
        await opened(d, rec.pip);
        await s.hearWithHfr(d, rec.pip);
    },
    'playback-modes': async (d) => {
        await opened(d, rec.pip);
        await s.setBand(d, rec.pip);
        await d.place();
        await d.wait(1500);
        await s.playbackModes(d);
    },
    views: async (d) => {
        await opened(d, rec.pip);
        await s.views(d);
    },
    'mark-call': async (d) => {
        await opened(d, rec.pip);
        await s.markCall(d, rec.pip);
    },
    'call-detection': async (d) => {
        await opened(d, rec.pip);
        await s.detectCalls(d);
    },
    'bat-book': async (d) => {
        await opened(d, rec.pip);
        await s.batBook(d);
    },
    tour: async (d) => {
        await d.goto('');
        await d.place();
        await d.wait(2500);
        // Faster than the short clips, to keep the tour under a minute.
        d.pace = 0.65;
        d.begin();
        await d.card(...s.TEXT.title, 2200);
        await s.openFromSearch(d, rec.pip);
        await s.hideFiles(d);
        await s.hearWithHfr(d, rec.pip, { timeStrip: false });
        await s.playbackModes(d, { backToPs: false });
        await s.markCall(d, rec.pip);
        await s.detectCalls(d);
        await s.batBook(d);
        await d.caption(null);
        await d.endCard(...s.TEXT.end, 2600);
    },
};

const args = process.argv.slice(2);
const opt = (name, fallback) => {
    const i = args.indexOf(name);
    if (i < 0) return fallback;
    const [, value] = args.splice(i, 2);
    return value;
};
const flag = (name) => {
    const i = args.indexOf(name);
    if (i >= 0) args.splice(i, 1);
    return i >= 0;
};
const url = opt('--url', 'http://localhost:21259/');
const dist = opt('--dist');
const outDir = opt('--out', 'media');
const gif = flag('--gif');
const captions = !flag('--no-captions');
const names = args.length ? args : Object.keys(CLIPS);
const unknown = names.filter((n) => !CLIPS[n]);
if (unknown.length) {
    console.error(`Unknown clip: ${unknown.join(', ')}. Clips: ${Object.keys(CLIPS).join(', ')}`);
    process.exit(2);
}

let failed = 0;
for (const name of names) {
    const d = await Demo.start({ url, dist, captions });
    try {
        await CLIPS[name](d);
        d.end();
        const secs = await d.finish(join(outDir, name), { gif });
        console.log(`${name}: ${secs.toFixed(1)} s`);
    } catch (e) {
        failed++;
        console.error(`${name} failed: ${e.message}`);
        await d.page.screenshot({ path: join(outDir, `${name}-failed.png`) }).catch(() => {});
        await d.browser.close().catch(() => {});
    }
}
process.exit(failed ? 1 : 0);
