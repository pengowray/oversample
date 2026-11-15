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
 * heard values. The time labels are the recording's. The frequency view
 * goes to the heard range (0 to 11, 22 or 44 kHz) while in that state.
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
  await setRows(page, {
    "PASS filter": "Off",
    "Notch filter": "Off",
    "Noise reduction": "Off",
    Gain: "Off",
    Resampling: "Off",
    "Playback mode": "As heard",
  });
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
  // The shifted audio reaches 32 kHz, so the view is 0 to 44 kHz.
  expect([bright.minFreq, bright.maxFreq]).toEqual([0, 44_000]);
  const call = brightBetween(bright, 7_500, 22_500);

  // A resampler that does not band-limit adds mirror images of the call
  // about each multiple of 64 kHz, the first at 41.5 to 56.5 kHz. Nothing
  // of the call is drawn there, or anywhere above the shifted audio's own
  // range. (The top 2 kHz of the picture holds the note about heard kHz.)
  expect(brightBetween(bright, 36_000, 42_000)).toBeLessThanOrEqual(call * 0.01);

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

  // The picture is of the shifted audio on its own frequency scale, drawn
  // from 0 to 22 kHz. The axis on its right edge is that scale's: a call
  // drawn at the 6 kHz label is heard at 6 kHz. Each label sits just above
  // its frequency's height.
  const height = await page.evaluate(
    () => (document.querySelector(".chart-stage canvas") as HTMLCanvasElement).height,
  );
  const axis = numberLabels(after, "chart-stage");
  expect(axis.size).toBeGreaterThanOrEqual(5);
  expect(Math.max(...axis.keys())).toBeLessThanOrEqual(22);
  for (const [khz, label] of axis) {
    const line = height * (1 - khz / 22);
    expect(Math.abs(line - label.y), `${khz} kHz label`).toBeLessThanOrEqual(20);
  }

  // The frequency strip on the left is where the band of the recording is
  // set. It keeps the recording's whole range and labels.
  const strip = numberLabels(after, "band-gutter");
  expect([...strip.keys()].sort()).toEqual([...rawFreq.keys()].sort());
  for (const [khz, label] of strip) {
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

/** The bottom and top of the frequency view, in Hz. `null` is the file's full range. */
async function freqView(page: Page): Promise<[number | null, number | null]> {
  return page.evaluate(() => {
    const s = (window as any).__oversample_test();
    return [s.minFreq, s.maxFreq] as [number | null, number | null];
  });
}

/** Zooms the frequency view in by one step, with shift and the wheel over the frequency strip. */
async function zoomFreqIn(page: Page) {
  const box = (await page.locator(".band-gutter canvas").first().boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height * 0.75);
  await page.keyboard.down("Shift");
  await page.mouse.wheel(0, -100);
  await page.keyboard.up("Shift");
  await page.mouse.move(700, 20);
}

/** Sets the PS factor from the preset buttons of the PS settings. */
async function setPsFactor(page: Page, preset: string) {
  await page.getByTitle(/^PS —/).first().click();
  await page
    .locator(".factor-presets button")
    .filter({ hasText: new RegExp(`^${preset}$`) })
    .first()
    .click();
  await page.keyboard.press("Escape");
  await page.mouse.move(700, 20);
}

test("PS under XForm shows the heard range, keeps the user's zoom, and puts the view back", async ({ page }) => {
  await openWithPs(page, "10x");
  // The range the user had before: zoomed in one step from the full range.
  await zoomFreqIn(page);
  await expect.poll(async () => (await freqView(page))[1]).not.toBeNull();
  const earlier = await freqView(page);
  expect(earlier[1]!).toBeGreaterThan(100_000);

  // At ÷10 the 256 kHz file's 128 kHz becomes 12.8 kHz: the view is 0 to 22 kHz.
  await xformRowOnly(page);
  await expect.poll(() => freqView(page)).toEqual([0, 22_000]);
  await page.waitForTimeout(2500);
  if (process.env.PS_SHOT_DIR) {
    await page.screenshot({ path: `${process.env.PS_SHOT_DIR}/view-ps-x10-xform.png` });
  }

  // The user's own zoom in that state stays.
  await zoomFreqIn(page);
  await expect.poll(async () => (await freqView(page))[1]).toBeLessThan(21_000);
  const own = await freqView(page);
  await page.waitForTimeout(1500);
  expect(await freqView(page)).toEqual(own);

  // A new factor sets the view again. At ÷16 the top is 8 kHz: 0 to 11 kHz.
  await setPsFactor(page, "16x");
  await expect.poll(() => freqView(page)).toEqual([0, 11_000]);
  await page.waitForTimeout(2500);
  if (process.env.PS_SHOT_DIR) {
    await page.screenshot({ path: `${process.env.PS_SHOT_DIR}/view-ps-x16-xform.png` });
  }

  // Switching XForm off puts back the range the user had before.
  await xformSwitch(page).click();
  await expect.poll(() => freqView(page)).toEqual(earlier);

  // So does leaving PS for another mode while XForm stays on.
  await xformSwitch(page).click();
  await expect.poll(() => freqView(page)).toEqual([0, 11_000]);
  await page.getByTitle(/^TE —/).first().click();
  await page.keyboard.press("Escape");
  await expect.poll(() => freqView(page)).toEqual(earlier);
});
