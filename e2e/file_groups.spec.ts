import { test, expect, type Page } from "@playwright/test";

/**
 * Which settings follow which files, for sequences and multitrack channels
 * (the rules are in src/scope.rs, the name parsing in file_groups.rs):
 * a numbered series shares gain and not the view; channels of one recording
 * share the view and not the gain; a file loaded later takes its group's
 * values and does not reset them.
 */

type Snap = {
  currentFile: string | null;
  scrollOffset: number;
  zoom: number;
  gainDb: number;
};

/** Mono 16-bit WAV of quiet noise. */
function noiseWav(sampleRate: number, seconds: number): Buffer {
  const n = Math.floor(sampleRate * seconds);
  const data = Buffer.alloc(n * 2);
  let seed = 4321;
  for (let i = 0; i < n; i++) {
    seed = (seed * 1664525 + 1013904223) >>> 0;
    data.writeInt16LE(Math.round((seed / 0xffffffff - 0.5) * 0.05 * 32767), i * 2);
  }
  const header = Buffer.alloc(44);
  header.write("RIFF", 0);
  header.writeUInt32LE(36 + data.length, 4);
  header.write("WAVEfmt ", 8);
  header.writeUInt32LE(16, 16);
  header.writeUInt16LE(1, 20);
  header.writeUInt16LE(1, 22);
  header.writeUInt32LE(sampleRate, 24);
  header.writeUInt32LE(sampleRate * 2, 28);
  header.writeUInt16LE(2, 32);
  header.writeUInt16LE(16, 34);
  header.write("data", 36);
  header.writeUInt32LE(data.length, 40);
  return Buffer.concat([header, data]);
}

function snapshot(page: Page): Promise<Snap> {
  return page.evaluate(() =>
    (window as unknown as { __oversample_test: () => Snap }).__oversample_test(),
  );
}

async function open(page: Page, name: string) {
  await page
    .locator('input[type="file"][accept*=".wav"]')
    .first()
    .setInputFiles({ name, mimeType: "audio/wav", buffer: noiseWav(192_000, 3) });
  await expect(page.locator(".file-item", { hasText: name })).toBeVisible({ timeout: 30_000 });
  await expect.poll(async () => (await snapshot(page)).currentFile).toBe(name);
}

async function show(page: Page, name: string) {
  await page.locator(".file-item", { hasText: name }).click();
  await expect.poll(async () => (await snapshot(page)).currentFile).toBe(name);
}

/** A two-finger pinch that zooms the time axis in. */
async function zoomIn(page: Page) {
  await page.evaluate(() => {
    const canvas = document.querySelector(".chart-stage canvas") as HTMLElement;
    const r = canvas.getBoundingClientRect();
    const cx = r.left + r.width / 2;
    const cy = r.top + r.height / 2;
    const pts = (gap: number) =>
      [-1, 1].map(
        (side, id) =>
          new Touch({
            identifier: id,
            target: canvas,
            clientX: cx + (side * gap) / 2,
            clientY: cy,
            pageX: cx + (side * gap) / 2,
            pageY: cy,
          }),
      );
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
    fire("touchstart", pts(60));
    for (let i = 1; i <= 6; i++) fire("touchmove", pts(60 + 40 * i));
    fire("touchend", []);
  });
}

async function setManualGain(page: Page, db: number) {
  await page.locator('button[title="Gain settings"]').click();
  await page.locator('.layer-panel input[type="range"][max="60"]').fill(String(db));
  await page.getByRole("button", { name: /^Manual/ }).click();
  await page.locator('button[title="Gain settings"]').click();
}

async function start(page: Page, first: string) {
  await page.goto("/");
  await open(page, first);
  await expect(page.locator(".chart-stage canvas").first()).toBeVisible({ timeout: 30_000 });
}

test("a numbered series shares gain and keeps each file's own scroll", async ({ page }) => {
  await start(page, "260212_0041.wav");
  await open(page, "260212_0042.wav");
  await show(page, "260212_0041.wav");
  await zoomIn(page);
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("ArrowRight");
  await setManualGain(page, 9);
  const first = await snapshot(page);
  expect(first.scrollOffset).toBeGreaterThan(0);
  expect(first.gainDb).toBe(9);

  await show(page, "260212_0042.wav");
  const second = await snapshot(page);
  expect(second.gainDb).toBe(9);
  expect(second.scrollOffset).toBe(0);
});

test("channels of one recording share the scroll and keep each file's own gain", async ({
  page,
}) => {
  await start(page, "260305_0058_1-2.wav");
  await open(page, "260305_0058_3.wav");
  await show(page, "260305_0058_1-2.wav");
  await zoomIn(page);
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("ArrowRight");
  await setManualGain(page, 9);
  const first = await snapshot(page);
  expect(first.scrollOffset).toBeGreaterThan(0);

  await show(page, "260305_0058_3.wav");
  const second = await snapshot(page);
  expect(second.scrollOffset).toBeCloseTo(first.scrollOffset, 3);
  expect(second.gainDb).not.toBe(9);
});

test("a take loaded later takes the sequence's gain and does not reset it", async ({ page }) => {
  await start(page, "260401_0010_1.wav");
  await setManualGain(page, 9);
  await open(page, "walk.wav");
  await open(page, "260401_0011_1.wav");
  expect((await snapshot(page)).gainDb).toBe(9);
  // Leaving the new take must not write defaults over the first one.
  await show(page, "walk.wav");
  await show(page, "260401_0010_1.wav");
  expect((await snapshot(page)).gainDb).toBe(9);
});

test("a channel loaded later takes the recording's scroll and does not reset it", async ({
  page,
}) => {
  await start(page, "260305_0058_3.wav");
  await zoomIn(page);
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("ArrowRight");
  const scrolled = (await snapshot(page)).scrollOffset;
  expect(scrolled).toBeGreaterThan(0);
  await open(page, "walk.wav");
  await open(page, "260305_0058_MIX.wav");
  expect((await snapshot(page)).scrollOffset).toBeCloseTo(scrolled, 3);
  await show(page, "walk.wav");
  await show(page, "260305_0058_3.wav");
  expect((await snapshot(page)).scrollOffset).toBeCloseTo(scrolled, 3);
});
