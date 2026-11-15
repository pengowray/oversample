import { type Page } from "@playwright/test";

/**
 * Shared by the XForm specs: a test recording of bat-like sweeps, a record of
 * the text drawn on canvases (the axis labels), and the Display Processing
 * menu's rows.
 */

/** Mono 16-bit WAV of FM sweeps from 90 kHz down to 30 kHz over faint noise. */
export function sweepWav(rate: number, seconds: number): Buffer {
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

export interface Drawn {
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
export function recordCanvasText() {
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
export async function drawnNow(page: Page): Promise<Drawn[]> {
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
export function numberLabels(drawn: Drawn[], where: string, unit = ""): Map<number, Drawn> {
  const out = new Map<number, Drawn>();
  const re = new RegExp(`^(\\d+(?:\\.\\d+)?)${unit}$`);
  for (const d of drawn) {
    if (!d.where.includes(where)) continue;
    const m = re.exec(d.text);
    if (m) out.set(Number(m[1]), d);
  }
  return out;
}

export const xformSwitch = (page: Page) => page.getByRole("switch", { name: "XForm" });

/** One row of the Display Processing menu, by its label. */
export const row = (page: Page, label: string) =>
  page.locator(".dsp-filter-row", {
    has: page.locator(".dsp-filter-label", { hasText: new RegExp(`^${label}$`) }),
  });

/** Sets rows of the Display Processing menu, then closes the menu. */
export async function setRows(page: Page, rows: Record<string, string>) {
  const menu = page.getByTitle("Display Processing settings");
  await menu.click();
  for (const [label, value] of Object.entries(rows)) {
    await row(page, label).getByRole("button", { name: value }).click();
  }
  await menu.click();
}
