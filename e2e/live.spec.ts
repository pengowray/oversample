import { test, expect, type Page } from "@playwright/test";
import path from "path";
import { pinch, drag, changed } from "./touch";

/**
 * Live-listening behaviour, driven by Chromium's fake-audio capture (a chirp
 * generated in global-setup) so the web getUserMedia path produces a real,
 * varying mic stream. Reads state via the `window.__oversample_test()` hook.
 *
 * Covers:
 *  - the live overview repaints faster than 1/s (cadence fix),
 *  - a pinch keeps the waterfall scrolling (live pinch fix),
 *  - panning to the live edge re-engages auto-follow (pan-to-edge fix).
 */
const wavPath = path.resolve(process.cwd(), "e2e", "fixtures", "fake-audio.wav");

test.use({
  hasTouch: true,
  launchOptions: {
    args: [
      "--use-fake-device-for-media-stream",
      "--use-fake-ui-for-media-stream",
      `--use-file-for-fake-audio-capture=${wavPath}`,
    ],
  },
});

type Snap = {
  zoom: number;
  minFreq: number | null;
  maxFreq: number | null;
  scrollOffset: number;
  listening: boolean;
  recording: boolean;
  liveActive: boolean;
  liveTotalTime: number;
  liveDataCols: number;
  following: boolean;
  recordingTargetScroll: number;
  scrollUserPanUntil: number;
};

function snapshot(page: Page): Promise<Snap> {
  return page.evaluate(() => (window as unknown as { __oversample_test: () => Snap }).__oversample_test());
}

async function startListening(page: Page) {
  await page.goto("/");
  await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
  await page.waitForFunction(
    () => typeof (window as { __oversample_test?: unknown }).__oversample_test === "function",
    undefined,
    { timeout: 15_000 },
  );
  const listenBtn = page.locator('button[title="Toggle live listening (L)"]').first();
  await expect(listenBtn).toBeVisible({ timeout: 10_000 });
  await listenBtn.click();
  // Wait for the live waterfall to start producing real columns from the
  // fake-audio stream.
  await page.waitForFunction(
    () => {
      const s = (window as unknown as { __oversample_test: () => { liveActive: boolean; liveTotalTime: number } }).__oversample_test();
      return s.liveActive && s.liveTotalTime > 0.05;
    },
    undefined,
    { timeout: 25_000 },
  );
}

test.describe("live listening", () => {
  test("the waveform overview repaints faster than once per second", async ({ page }) => {
    test.setTimeout(60_000);
    await startListening(page);

    // Toggle the overview to the waveform view (button shows the CURRENT view,
    // so it reads "Spectrum" while the spectrogram is shown).
    const ovBtn = page.locator(".overview-strip button.layer-btn").first();
    await expect(ovBtn).toBeVisible({ timeout: 5_000 });
    if (((await ovBtn.textContent()) ?? "").includes("Spectrum")) {
      await ovBtn.click();
    }

    const cols0 = (await snapshot(page)).liveDataCols;

    // Sample the overview background canvas over ~1.2 s.
    const frames: string[] = [];
    for (let i = 0; i < 6; i++) {
      frames.push(
        await page.evaluate(() => {
          const c = document.querySelector(".overview-strip canvas") as HTMLCanvasElement;
          return c.toDataURL();
        }),
      );
      await page.waitForTimeout(200);
    }
    const cols1 = (await snapshot(page)).liveDataCols;

    // The capture pipeline advanced many columns in ~1.2 s (far more than 1/s).
    expect(cols1 - cols0, `liveDataCols should grow fast (${cols0} -> ${cols1})`).toBeGreaterThan(10);
    // The overview actually repainted multiple distinct frames — i.e. it is not
    // frozen between the old ~1 Hz snapshots.
    const distinct = new Set(frames).size;
    expect(distinct, `overview should repaint >1x/s (got ${distinct} distinct frames)`).toBeGreaterThanOrEqual(3);
  });

  test("a pinch does not stop the live scroll", async ({ page }) => {
    test.setTimeout(60_000);
    await startListening(page);

    // Following by default (no pan yet).
    await expect.poll(async () => (await snapshot(page)).following, { timeout: 5_000 }).toBe(true);

    const before = await snapshot(page);
    await pinch(page, "x", 120, 280);
    const after = await snapshot(page);

    expect(after.following, "a live pinch must NOT freeze the waterfall follow").toBe(true);
    expect(changed(before.zoom, after.zoom), "the pinch should still change zoom").toBe(true);
  });

  test("panning to the live edge re-engages auto-follow", async ({ page }) => {
    test.setTimeout(60_000);
    await startListening(page);
    await expect.poll(async () => (await snapshot(page)).following, { timeout: 5_000 }).toBe(true);

    // Wait until enough audio has streamed that there is real history to pan
    // INTO — i.e. the live edge (recordingTargetScroll == max_scroll) is well
    // past zero. Until then panning back trivially lands at the edge.
    await expect
      .poll(async () => (await snapshot(page)).recordingTargetScroll, { timeout: 30_000, intervals: [250] })
      .toBeGreaterThan(2.0);

    // Pan back into history (finger moves right) → follow suspends.
    await drag(page, 400);
    await expect.poll(async () => (await snapshot(page)).following, { timeout: 4_000 }).toBe(false);

    // Pan hard toward the live edge (finger moves far left) → follow re-engages.
    await drag(page, -2000);
    await expect.poll(async () => (await snapshot(page)).following, { timeout: 4_000 }).toBe(true);
  });
});
