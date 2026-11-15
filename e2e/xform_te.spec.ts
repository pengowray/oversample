import { test, expect, type Page } from "@playwright/test";

/**
 * XForm with time expansion (TE). Playback only plays the recording slower,
 * so the spectrogram under XForm is the recording's own picture and only the
 * axis labels change: frequencies divided by the TE factor, times multiplied
 * by it.
 */

/** Mono 16-bit WAV of FM sweeps from 90 kHz down to 30 kHz over faint noise. */
function sweepWav(rate: number, seconds: number): Buffer {
  const n = Math.round(rate * seconds);
  const data = Buffer.alloc(n * 2);
  let seed = 1234567;
  const rnd = () => ((seed = (seed * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff) * 2 - 1;
  const period = Math.round(0.06 * rate);
  const len = Math.round(0.008 * rate);
  let phase = 0;
  for (let i = 0; i < n; i++) {
    const k = i % period;
    let s = rnd() * 0.004;
    if (k < len) {
      const u = k / len;
      phase += (2 * Math.PI * 90_000 * Math.pow(30 / 90, u)) / rate;
      s += Math.sin(phase) * 0.5 * Math.sin(Math.PI * u);
    } else {
      phase = 0;
    }
    data.writeInt16LE(Math.round(Math.max(-1, Math.min(1, s)) * 32767), i * 2);
  }
  const h = Buffer.alloc(44);
  h.write("RIFF", 0);
  h.writeUInt32LE(36 + data.length, 4);
  h.write("WAVE", 8);
  h.write("fmt ", 12);
  h.writeUInt32LE(16, 16);
  h.writeUInt16LE(1, 20);
  h.writeUInt16LE(1, 22);
  h.writeUInt32LE(rate, 24);
  h.writeUInt32LE(rate * 2, 28);
  h.writeUInt16LE(2, 32);
  h.writeUInt16LE(16, 34);
  h.write("data", 36);
  h.writeUInt32LE(data.length, 40);
  return Buffer.concat([h, data]);
}

interface Drawn {
  /** Class of the canvas's parent element: which strip or view drew it. */
  where: string;
  text: string;
  x: number;
  y: number;
}

/**
 * Records every string drawn with fillText on any canvas, with its position
 * and the class of the canvas's parent, so a test can read the axis labels.
 * Runs in the page before the app loads.
 */
function recordCanvasText() {
  const drawn: Drawn[] = [];
  (window as any).__canvasText = drawn;
  const fillText = CanvasRenderingContext2D.prototype.fillText;
  CanvasRenderingContext2D.prototype.fillText = function (text: string, x: number, y: number, ...rest: any[]) {
    const where = this.canvas?.parentElement?.className ?? "";
    drawn.push({ where: String(where), text: String(text), x, y });
    if (drawn.length > 20_000) drawn.splice(0, 10_000);
    return (fillText as any).call(this, text, x, y, ...rest);
  };
}

/** Clears the record, waits for the strips to be redrawn, and returns what was drawn. */
async function drawnNow(page: Page): Promise<Drawn[]> {
  // A resize makes every canvas redraw, the strips included. Only the draws
  // at the final size are kept.
  const size = page.viewportSize()!;
  await page.setViewportSize({ width: size.width - 40, height: size.height - 20 });
  await page.waitForTimeout(600);
  await page.evaluate(() => ((window as any).__canvasText as unknown[]).splice(0));
  await page.setViewportSize(size);
  await page.waitForTimeout(1200);
  return page.evaluate(() => (window as any).__canvasText as Drawn[]);
}

/** Labels made only of a number, with an optional unit, drawn on one canvas. */
function numberLabels(drawn: Drawn[], where: string, unit = ""): Map<number, Drawn> {
  const out = new Map<number, Drawn>();
  const re = new RegExp(`^(\\d+(?:\\.\\d+)?)${unit}$`);
  for (const d of drawn) {
    if (!d.where.includes(where)) continue;
    const m = re.exec(d.text);
    if (m) out.set(Number(m[1]), d);
  }
  return out;
}

const xformSwitch = (page: Page) => page.getByRole("switch", { name: "XForm" });

/** One row of the XForm menu, by its label. */
const row = (page: Page, label: string) =>
  page.locator(".dsp-filter-row", {
    has: page.locator(".dsp-filter-label", { hasText: new RegExp(`^${label}$`) }),
  });

/** Sets rows of the XForm menu, then closes the menu. */
async function setRows(page: Page, rows: Record<string, string>) {
  const menu = page.getByTitle("XForm settings");
  await menu.click();
  for (const [label, value] of Object.entries(rows)) {
    await row(page, label).getByRole("button", { name: value, exact: true }).click();
  }
  await menu.click();
}

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
  await setRows(page, {
    "PASS filter": "Off",
    "Notch filter": "Off",
    "Noise reduction": "Off",
    Gain: "Off",
    Resampling: "Off",
    "Playback mode": "As heard",
  });
  await page.mouse.move(700, 20);
  await expect
    .poll(async () => shareDifferent(raw, await callPixels(page)), { timeout: 15_000 })
    .toBeLessThan(0.005);
});

test("TE under XForm with every row on As heard still shows the calls", async ({ page }) => {
  await openWithTe10(page);
  const raw = await callPixels(page);

  // Every row starts on "As heard" the first time XForm is switched on.
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
  await setRows(page, {
    "PASS filter": "Off",
    "Notch filter": "Off",
    "Noise reduction": "Off",
    Gain: "Off",
    Resampling: "Off",
    "Playback mode": "As heard",
  });
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
