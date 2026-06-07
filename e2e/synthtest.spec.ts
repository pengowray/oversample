import { test, expect, type Page } from "@playwright/test";

/**
 * Synthetic live-waterfall test mode. The `#synthtest-<signal>[-rateKHz]` URL
 * hash drives the real listen pipeline with generated audio (no mic / no
 * fake-device flags needed — see src/audio/synthetic_mic.rs), so the waterfall
 * column generation + direct canvas render path can be exercised and rough
 * throughput observed in CI.
 *
 * Reads state via the same `window.__oversample_test()` hook live.spec uses.
 */

type Snap = {
  listening: boolean;
  liveActive: boolean;
  liveTotalTime: number;
  liveDataCols: number;
};

function snapshot(page: Page): Promise<Snap> {
  return page.evaluate(() => (window as unknown as { __oversample_test: () => Snap }).__oversample_test());
}

async function gotoSynth(page: Page, hash: string) {
  await page.goto(`/${hash}`);
  await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
  await page.waitForFunction(
    () => typeof (window as { __oversample_test?: unknown }).__oversample_test === "function",
    undefined,
    { timeout: 15_000 },
  );
  // The launcher waits ~400ms for layout, then starts feeding columns.
  await page.waitForFunction(
    () => {
      const s = (window as unknown as { __oversample_test: () => { liveActive: boolean; liveTotalTime: number } }).__oversample_test();
      return s.liveActive && s.liveTotalTime > 0.1;
    },
    undefined,
    { timeout: 25_000 },
  );
}

test.describe("synthetic live waterfall", () => {
  test("a chirp signal drives the waterfall and keeps producing columns", async ({ page }) => {
    test.setTimeout(60_000);
    await gotoSynth(page, "#synthtest-chirp-256");

    const cols0 = (await snapshot(page)).liveDataCols;
    await page.waitForTimeout(1000);
    const cols1 = (await snapshot(page)).liveDataCols;

    // The synthetic feeder advanced many columns in ~1s (256 kHz / hop ⇒ hundreds/s).
    expect(cols1 - cols0, `liveDataCols should grow fast (${cols0} -> ${cols1})`).toBeGreaterThan(20);
  });

  test("the spectrogram canvas actually repaints (waterfall scrolls)", async ({ page }) => {
    test.setTimeout(60_000);
    await gotoSynth(page, "#synthtest-pulses-256");

    // The main waterfall canvas lives in .chart-stage; .spectrogram-container
    // also holds the static frequency/time gutter canvases, which precede it.
    const grab = () =>
      page.evaluate(() => {
        const c = document.querySelector(".chart-stage canvas") as HTMLCanvasElement | null;
        return c ? c.toDataURL() : "";
      });

    const frames: string[] = [];
    for (let i = 0; i < 5; i++) {
      frames.push(await grab());
      await page.waitForTimeout(200);
    }
    const distinct = new Set(frames.filter((f) => f.length > 0)).size;
    expect(distinct, `waterfall canvas should repaint over ~1s (got ${distinct} distinct frames)`).toBeGreaterThanOrEqual(3);
  });
});
