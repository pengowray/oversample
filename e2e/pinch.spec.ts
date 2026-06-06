import { test, expect, type Page } from "@playwright/test";

/**
 * Directional pinch-zoom on the main spectrogram.
 *
 * Validates the axis-snap feature: a two-finger pinch zooms EITHER time
 * (horizontal) OR frequency (vertical) — whichever axis the fingers spread
 * more — and never both. Drives synthetic two-finger TouchEvents and reads
 * back zoom / display-freq via the `window.__oversample_test()` hook.
 *
 * Touch is required, so the whole file runs with a touch-capable context.
 */
test.use({ hasTouch: true });

type Snap = {
  zoom: number;
  minFreq: number | null;
  maxFreq: number | null;
  scrollOffset: number;
};

async function loadDemoBat(page: Page) {
  await page.goto("/");
  await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
  const loadDemo = page.locator("button.add-files-btn", { hasText: "Load demo" });
  await expect(loadDemo).toBeVisible({ timeout: 15_000 });
  await loadDemo.click();
  const randomBat = page.locator(".demo-random-bat").first();
  await expect(randomBat).toBeVisible({ timeout: 10_000 });
  await randomBat.click();
  await expect(page.locator(".chart-stage canvas").first()).toBeVisible({ timeout: 30_000 });
  await page.waitForFunction(
    () => typeof (window as { __oversample_test?: unknown }).__oversample_test === "function",
    undefined,
    { timeout: 15_000 },
  );
}

function snapshot(page: Page): Promise<Snap> {
  return page.evaluate(() => (window as unknown as { __oversample_test: () => Snap }).__oversample_test());
}

/**
 * Dispatch a synthetic two-finger pinch on the main spectrogram canvas.
 * `axis` "x" spreads the fingers horizontally (time), "y" vertically (freq).
 * The gap grows from startGap to endGap across several incremental moves so
 * the axis-lock (≥10px spread) engages just like a real gesture.
 */
async function pinch(page: Page, axis: "x" | "y", startGap: number, endGap: number) {
  await page.evaluate(
    ({ axis, startGap, endGap }) => {
      const canvas = document.querySelector(".chart-stage canvas") as HTMLElement;
      const r = canvas.getBoundingClientRect();
      const cx = r.left + r.width / 2;
      const cy = r.top + r.height / 2;
      const mk = (id: number, x: number, y: number) =>
        new Touch({
          identifier: id,
          target: canvas,
          clientX: x,
          clientY: y,
          pageX: x,
          pageY: y,
          screenX: x,
          screenY: y,
          radiusX: 1,
          radiusY: 1,
          rotationAngle: 0,
          force: 1,
        });
      const fire = (type: string, touches: Touch[]) =>
        canvas.dispatchEvent(
          new TouchEvent(type, {
            cancelable: true,
            bubbles: true,
            composed: true,
            touches,
            targetTouches: touches,
            changedTouches: touches,
          }),
        );
      const pts = (gap: number): Touch[] =>
        axis === "x"
          ? [mk(0, cx - gap / 2, cy), mk(1, cx + gap / 2, cy)]
          : [mk(0, cx, cy - gap / 2), mk(1, cx, cy + gap / 2)];

      fire("touchstart", pts(startGap));
      const steps = 6;
      for (let i = 1; i <= steps; i++) {
        fire("touchmove", pts(startGap + ((endGap - startGap) * i) / steps));
      }
      fire("touchend", []);
    },
    { axis, startGap, endGap },
  );
}

/** True if a display-freq value meaningfully changed (handles null = "auto"). */
function changed(a: number | null, b: number | null): boolean {
  if (a === null && b === null) return false;
  if (a === null || b === null) return true;
  return Math.abs(a - b) > Math.max(1e-6, Math.abs(a) * 1e-4);
}

test.describe("spectrogram pinch — directional axis snap", () => {
  test("horizontal pinch zooms time only; vertical pinch zooms frequency only", async ({ page }) => {
    test.setTimeout(90_000);
    await loadDemoBat(page);

    const s0 = await snapshot(page);

    // ── Horizontal spread → time zoom changes, frequency unchanged ──
    await pinch(page, "x", 120, 280);
    const s1 = await snapshot(page);
    expect(s1.zoom, `zoom should grow on horizontal spread (${s0.zoom} -> ${s1.zoom})`).toBeGreaterThan(s0.zoom);
    expect(changed(s0.minFreq, s1.minFreq), "min freq must NOT change on a horizontal pinch").toBe(false);
    expect(changed(s0.maxFreq, s1.maxFreq), "max freq must NOT change on a horizontal pinch").toBe(false);

    // ── Vertical spread → frequency changes, time zoom unchanged ──
    await pinch(page, "y", 120, 280);
    const s2 = await snapshot(page);
    expect(
      changed(s1.minFreq, s2.minFreq) || changed(s1.maxFreq, s2.maxFreq),
      "a vertical pinch should change the display frequency range",
    ).toBe(true);
    expect(changed(s1.zoom, s2.zoom), `zoom must NOT change on a vertical pinch (${s1.zoom} -> ${s2.zoom})`).toBe(false);
  });
});
