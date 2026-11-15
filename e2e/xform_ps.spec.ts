import { test, expect, type Page } from "@playwright/test";
import {
  drawnNow,
  numberLabels,
  recordCanvasText,
  setRows,
  sweepWav,
  xformSwitch,
} from "./xform_util";

/**
 * XForm with pitch shift (PS). Playback divides every frequency by the
 * factor and keeps the timing, so the spectrogram under XForm is a picture
 * of the shifted audio: each 90 to 30 kHz sweep of the test file is drawn
 * once, at 22.5 to 7.5 kHz at ÷4, in the place in time it has in the
 * recording. The frequency labels are the picture's own and are marked as
 * heard values. The time labels are the recording's.
 */

/** Opens the sweep file with PS at the factor preset `preset` ("4x"). */
async function openWithPs(page: Page, preset: string) {
  await page.addInitScript(recordCanvasText);
  await page.setViewportSize({ width: 1400, height: 900 });
  await page.goto("/");
  await page
    .locator('input[type="file"][accept*=".wav"]')
    .first()
    .setInputFiles({ name: "sweeps.wav", mimeType: "audio/wav", buffer: sweepWav(256_000, 1.0) });
  await expect(page.locator(".chart-stage canvas").first()).toBeVisible();

  // PS, at the preset. The second click on the selected mode opens its settings.
  const ps = page.getByTitle(/^PS —/).first();
  await ps.click();
  await ps.click();
  await page.locator(".algo-row button").filter({ hasText: /^PS$/ }).click();
  await page
    .locator(".factor-presets button")
    .filter({ hasText: new RegExp(`^${preset}$`) })
    .first()
    .click();
  await page.keyboard.press("Escape");
  // Park the pointer away from the spectrogram, so no hover text is drawn.
  await page.mouse.move(700, 20);
  await page.waitForTimeout(2000);
}

/** XForm on, with only its Xform row applied. */
async function xformRowOnly(page: Page) {
  await xformSwitch(page).click();
  await setRows(page, { EQ: "off", Notch: "off", NR: "off", Gain: "off", Resam: "off", Xform: "sam" });
  await page.mouse.move(700, 20);
}

interface Bright {
  /** Bright pixels in each pixel row, top row first. */
  rows: number[];
  /** The frequency at the bottom and top edges of the picture, in Hz. */
  minFreq: number;
  maxFreq: number;
}

/**
 * Counts the pixels bright enough to be part of a call in each pixel row of
 * the part of the spectrogram that holds the calls (left of the recording's
 * end, clear of the axis labels on either edge).
 */
async function brightRows(page: Page): Promise<Bright> {
  return page.evaluate(() => {
    const canvas = document.querySelector(".chart-stage canvas") as HTMLCanvasElement;
    const ctx = canvas.getContext("2d")!;
    const [x, w] = [60, 300];
    const data = ctx.getImageData(x, 0, w, canvas.height).data;
    const rows: number[] = [];
    for (let y = 0; y < canvas.height; y++) {
      let n = 0;
      for (let i = y * w * 4; i < (y + 1) * w * 4; i += 4) {
        if (data[i] + data[i + 1] + data[i + 2] > 360) n++;
      }
      rows.push(n);
    }
    const s = (window as any).__oversample_test();
    return { rows, minFreq: s.minFreq ?? 0, maxFreq: s.maxFreq ?? 128_000 };
  });
}

/** Bright pixels between two frequencies of the picture. */
function brightBetween(b: Bright, loHz: number, hiHz: number): number {
  const h = b.rows.length;
  const yOf = (hz: number) => Math.round(h * (1 - (hz - b.minFreq) / (b.maxFreq - b.minFreq)));
  let n = 0;
  for (let y = Math.max(0, yOf(hiHz)); y < Math.min(h, yOf(loHz)); y++) n += b.rows[y];
  return n;
}

test("PS under XForm draws each call once, with no copies and no gaps", async ({ page }) => {
  await openWithPs(page, "4x");
  await xformRowOnly(page);

  // At ÷4 the 90 to 30 kHz sweep is at 22.5 to 7.5 kHz.
  let bright!: Bright;
  await expect
    .poll(
      async () => {
        bright = await brightRows(page);
        return brightBetween(bright, 7_500, 22_500);
      },
      { timeout: 20_000 },
    )
    .toBeGreaterThan(100);
  expect(bright.maxFreq).toBe(128_000);
  const call = brightBetween(bright, 7_500, 22_500);

  // A resampler that does not band-limit adds mirror images of the call
  // about each multiple of 64 kHz: at 41.5 to 56.5 kHz, 71.5 to 86.5 kHz
  // and 105.5 to 120.5 kHz. Nothing of the call is drawn there, or anywhere
  // above the shifted audio's own range.
  expect(brightBetween(bright, 41_500, 56_500)).toBeLessThanOrEqual(call * 0.01);
  expect(brightBetween(bright, 71_500, 86_500)).toBeLessThanOrEqual(call * 0.01);
  // (The top 8 kHz of the picture holds the note about heard kHz.)
  expect(brightBetween(bright, 36_000, 120_000)).toBeLessThanOrEqual(call * 0.02);

  // The whole sweep is drawn: every 1.5 kHz step from 9 to 21 kHz has part
  // of it. A shifter that drops pieces of each call leaves steps empty.
  for (let hz = 9_000; hz < 21_000; hz += 1_500) {
    expect(brightBetween(bright, hz, hz + 1_500), `${hz / 1000} to ${hz / 1000 + 1.5} kHz`).toBeGreaterThan(0);
  }

  if (process.env.PS_SHOT_DIR) {
    await page.screenshot({ path: `${process.env.PS_SHOT_DIR}/after-ps-x4-xform.png` });
  }
});

test("PS under XForm marks the frequency labels as heard and keeps the time labels", async ({ page }) => {
  await openWithPs(page, "10x");
  const before = await drawnNow(page);
  const rawFreq = numberLabels(before, "band-gutter");
  const rawTime = numberLabels(before, "time-gutter", "s");
  expect(rawFreq.size).toBeGreaterThanOrEqual(5);
  expect(rawTime.size).toBeGreaterThanOrEqual(3);
  expect(before.some((d) => d.text.startsWith("heard"))).toBe(false);

  await xformRowOnly(page);
  await page.waitForTimeout(1500);
  const after = await drawnNow(page);

  // The picture is of the shifted audio on its own frequency scale, so the
  // labels keep their numbers and places: a call drawn at the 5 kHz label
  // is heard at 5 kHz.
  const heardFreq = numberLabels(after, "band-gutter");
  expect([...heardFreq.keys()].sort()).toEqual([...rawFreq.keys()].sort());
  for (const [khz, label] of heardFreq) {
    expect(Math.abs(rawFreq.get(khz)!.y - label.y)).toBeLessThanOrEqual(1);
  }

  // The spectrogram says that its frequencies are the heard ones and how
  // they relate to the recording's. The time strip has no such note: pitch
  // shift leaves time as it is.
  const notes = (where: string) =>
    after.filter((d) => d.where.includes(where) && d.text.startsWith("heard")).map((d) => d.text);
  expect(notes("chart-stage")).toContain("heard kHz = recording ÷10");
  expect(notes("time-gutter")).toEqual([]);
  const heardTime = numberLabels(after, "time-gutter", "s");
  expect([...heardTime.keys()].sort()).toEqual([...rawTime.keys()].sort());

  if (process.env.PS_SHOT_DIR) {
    await page.screenshot({ path: `${process.env.PS_SHOT_DIR}/after-ps-x10-xform.png` });
    // The same settings as the screenshot taken before the shifter was fixed.
    const ps = page.getByTitle(/^PS —/).first();
    await ps.click();
    await page.locator(".factor-presets button").filter({ hasText: /^16x$/ }).first().click();
    await page.keyboard.press("Escape");
    await page.mouse.move(700, 20);
    await page.waitForTimeout(4000);
    await page.screenshot({ path: `${process.env.PS_SHOT_DIR}/after-ps-x16-xform.png` });
  }
});
