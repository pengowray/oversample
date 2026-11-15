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
 * XForm with time expansion (TE). Playback only plays the recording slower,
 * so the spectrogram under XForm is the recording's own picture and only the
 * axis labels change: frequencies divided by the TE factor, times multiplied
 * by it.
 */

/**
 * The pixels of a part of the spectrogram that holds the calls: left of the
 * recording's end and clear of the axis labels on either edge.
 */
async function callPixels(page: Page): Promise<number[]> {
  return page.evaluate(() => {
    const canvas = document.querySelector(".chart-stage canvas") as HTMLCanvasElement;
    const ctx = canvas.getContext("2d")!;
    const x = 60;
    const y = Math.round(canvas.height * 0.25);
    const w = 300;
    const h = Math.round(canvas.height * 0.45);
    return Array.from(ctx.getImageData(x, y, w, h).data);
  });
}

/** The share of pixels that differ by more than a rounding step. */
function shareDifferent(a: number[], b: number[]): number {
  let diff = 0;
  for (let i = 0; i < a.length; i += 4) {
    if (
      Math.abs(a[i] - b[i]) > 2 ||
      Math.abs(a[i + 1] - b[i + 1]) > 2 ||
      Math.abs(a[i + 2] - b[i + 2]) > 2
    ) {
      diff++;
    }
  }
  return diff / (a.length / 4);
}

/** The share of pixels bright enough to be part of a call. */
function shareBright(a: number[]): number {
  let bright = 0;
  for (let i = 0; i < a.length; i += 4) {
    if (a[i] + a[i + 1] + a[i + 2] > 360) bright++;
  }
  return bright / (a.length / 4);
}

async function openWithTe10(page: Page) {
  await page.addInitScript(recordCanvasText);
  await page.setViewportSize({ width: 1400, height: 900 });
  await page.goto("/");
  await page
    .locator('input[type="file"][accept*=".wav"]')
    .first()
    .setInputFiles({ name: "sweeps.wav", mimeType: "audio/wav", buffer: sweepWav(256_000, 1.0) });
  await expect(page.locator(".chart-stage canvas").first()).toBeVisible();

  // TE, at 10x. The second click on the selected mode opens its settings.
  const te = page.getByTitle(/^TE —/).first();
  await te.click();
  await te.click();
  await page.locator(".factor-presets button").filter({ hasText: /^10x$/ }).first().click();
  await page.keyboard.press("Escape");
  // Park the pointer away from the spectrogram, so no hover text is drawn.
  await page.mouse.move(700, 20);
  await page.waitForTimeout(2000);
}

test("TE under XForm draws the recording's own picture", async ({ page }) => {
  await openWithTe10(page);
  const raw = await callPixels(page);
  expect(shareBright(raw)).toBeGreaterThan(0.002);

  await xformSwitch(page).click();
  await setRows(page, { EQ: "off", Notch: "off", NR: "off", Gain: "off", Resam: "off", Xform: "sam" });
  await page.mouse.move(700, 20);
  await expect
    .poll(async () => shareDifferent(raw, await callPixels(page)), { timeout: 15_000 })
    .toBeLessThan(0.005);
});

test("TE under XForm with every row on sam still shows the calls", async ({ page }) => {
  await openWithTe10(page);
  const raw = await callPixels(page);

  // Every row starts on "sam" the first time XForm is switched on.
  await xformSwitch(page).click();
  await page.mouse.move(700, 20);
  await page.waitForTimeout(3000);
  const shown = await callPixels(page);
  expect(shareBright(shown)).toBeGreaterThan(shareBright(raw) * 0.5);
});

test("TE under XForm labels the axes with the values heard", async ({ page }) => {
  await openWithTe10(page);
  const before = await drawnNow(page);
  const rawFreq = numberLabels(before, "band-gutter");
  const rawTime = numberLabels(before, "time-gutter", "s");
  expect(rawFreq.size).toBeGreaterThanOrEqual(5);
  expect(rawTime.size).toBeGreaterThanOrEqual(3);

  await xformSwitch(page).click();
  await setRows(page, { EQ: "off", Notch: "off", NR: "off", Gain: "off", Resam: "off", Xform: "sam" });
  await page.mouse.move(700, 20);
  await page.waitForTimeout(1500);
  const after = await drawnNow(page);

  // Frequency strip: each label is a tenth of the recording's frequency at
  // that height.
  const heardFreq = numberLabels(after, "band-gutter");
  expect(heardFreq.size).toBeGreaterThanOrEqual(5);
  for (const [khz, label] of heardFreq) {
    const at = rawFreq.get(Math.round(khz * 10));
    expect(at, `frequency strip label ${khz} has a recording label ${khz * 10}`).toBeTruthy();
    expect(Math.abs(at!.y - label.y)).toBeLessThanOrEqual(1);
  }

  // The frequency axis inside the spectrogram, drawn on its right edge under
  // XForm: its labels sit at the same heights as the strip's.
  const axis = numberLabels(after, "chart-stage");
  const canvasHeight = await page.evaluate(
    () => (document.querySelector(".chart-stage canvas") as HTMLCanvasElement).height,
  );
  const stripHeight = await page.evaluate(
    () => (document.querySelector(".band-gutter canvas") as HTMLCanvasElement).height,
  );
  expect(canvasHeight).toBe(stripHeight);
  let axisChecked = 0;
  for (const [khz, label] of axis) {
    const at = rawFreq.get(Math.round(khz * 10));
    if (!at) continue;
    // The strip centres a label on its line; the axis writes it above the line.
    expect(Math.abs(at.y - label.y)).toBeLessThanOrEqual(20);
    axisChecked++;
  }
  expect(axisChecked).toBeGreaterThanOrEqual(5);

  // Each relabelled axis says that its values are the heard ones.
  const notes = (where: string) =>
    after.filter((d) => d.where.includes(where) && d.text.startsWith("heard")).map((d) => d.text);
  expect(notes("chart-stage")).toContain("heard kHz = recording ÷10");
  expect(notes("time-gutter")).toContain("heard time = recording ×10");
  expect(before.some((d) => d.text.startsWith("heard"))).toBe(false);
  expect(axis.has(120), "the axis has no 120 kHz label").toBe(false);

  // Time strip: each label is ten times the recording's time at that place.
  const heardTime = numberLabels(after, "time-gutter", "s");
  expect(heardTime.size).toBeGreaterThanOrEqual(3);
  for (const [secs, label] of heardTime) {
    if (secs === 0) continue;
    const at = rawTime.get(Math.round(secs * 100) / 1000);
    expect(at, `time strip label ${secs}s has a recording label ${secs / 10}s`).toBeTruthy();
    expect(Math.abs(at!.x - label.x)).toBeLessThanOrEqual(1);
  }
  expect([...heardTime.keys()].some((s) => s >= 2)).toBe(true);

  // Switching XForm off puts the recording's own values back.
  await xformSwitch(page).click();
  await page.mouse.move(700, 20);
  const off = await drawnNow(page);
  expect([...numberLabels(off, "band-gutter").keys()].sort()).toEqual([...rawFreq.keys()].sort());
  expect([...numberLabels(off, "time-gutter", "s").keys()].sort()).toEqual([...rawTime.keys()].sort());
});
