import { test, expect, type Page } from "@playwright/test";
import path from "path";
import { pinch, drag, zoomIn, changed } from "./touch";

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
  overviewAxisStart?: number;
  overviewSpan?: number;
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

    // Toggle the overview to the waveform view. The toggle now lives in the
    // overview toolbar and previews the OTHER view, so it reads "Waveform"
    // while the spectrogram is shown — click it then to switch.
    const ovBtn = page.locator(".overview-toolbar button").first();
    await expect(ovBtn).toBeVisible({ timeout: 5_000 });
    if (((await ovBtn.textContent()) ?? "").includes("Waveform")) {
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

  test("a pinch while parked in history stays put (no jump to the start)", async ({ page }) => {
    test.setTimeout(60_000);
    await startListening(page);
    await expect.poll(async () => (await snapshot(page)).following, { timeout: 5_000 }).toBe(true);
    // Zoom in so the main view shows far less than the buffer — i.e. there is
    // real room to pan back into history (at the default live zoom the whole
    // ring fits on screen, so nothing is "history").
    await zoomIn(page, 14);
    await expect
      .poll(async () => (await snapshot(page)).recordingTargetScroll, { timeout: 30_000, intervals: [250] })
      .toBeGreaterThan(1.0);

    // Park the view back in history.
    await drag(page, 200);
    await expect.poll(async () => (await snapshot(page)).following, { timeout: 4_000 }).toBe(false);
    const before = await snapshot(page);
    expect(before.scrollOffset, "should be genuinely parked in history").toBeGreaterThan(0.1);

    // A pinch must anchor-zoom in place — not re-follow, not jump to 0.
    await pinch(page, "x", 120, 280);
    const after = await snapshot(page);
    expect(after.following, "a pinch in history must not re-engage follow").toBe(false);
    expect(
      after.scrollOffset,
      `pinch must not jump to the start (${before.scrollOffset} -> ${after.scrollOffset})`,
    ).toBeGreaterThan(before.scrollOffset * 0.5);
  });

  test("scrubbing the overview far-left lines up and doesn't snap back", async ({ page }) => {
    test.setTimeout(60_000);
    await startListening(page);
    // Zoom in so the main view is narrower than the overview window — otherwise
    // the whole ring is already on screen and any scrub trivially stays "live".
    await zoomIn(page, 14);
    // Wait until the ring has trimmed so the overview window's left edge
    // (axisStart) is well past 0 — only then can centering-vs-aligning differ.
    await expect
      .poll(async () => (await snapshot(page)).overviewAxisStart ?? 0, { timeout: 40_000, intervals: [300] })
      .toBeGreaterThan(0.5);

    const axisStart = (await snapshot(page)).overviewAxisStart!;

    // Click the far-left of the overview overlay canvas (the 2nd canvas).
    await page.evaluate(() => {
      const c = document.querySelector(".overview-strip canvas:nth-of-type(2)") as HTMLElement;
      const r = c.getBoundingClientRect();
      const x = r.left + 3;
      const y = r.top + r.height / 2;
      const opts = { clientX: x, clientY: y, bubbles: true, cancelable: true, pointerId: 1 };
      c.dispatchEvent(new PointerEvent("pointerdown", opts));
      c.dispatchEvent(new PointerEvent("pointerup", opts));
    });

    const after = await snapshot(page);
    // Far-left click should land at the window's left edge, not centered to its right.
    expect(
      Math.abs(after.scrollOffset - axisStart),
      `far-left scrub should align to axisStart (scroll ${after.scrollOffset} vs ${axisStart})`,
    ).toBeLessThan(0.6);
    // Scrubbing suspends follow (no immediate snap-back to the live edge).
    expect(after.following, "scrubbing must suspend the waterfall follow").toBe(false);
    await page.waitForTimeout(800);
    expect((await snapshot(page)).following, "must stay parked within the grace window").toBe(false);
  });
});
