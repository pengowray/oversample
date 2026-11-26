import { test, expect, type Page } from "@playwright/test";

/**
 * The play button beside a selection's or annotation's "..." button plays just
 * that range, and an annotation's own "Play with" setting (from its "..." menu)
 * is used by that button, saved with the annotation and kept after a reload.
 *
 * (src/components/item_play.rs, src/audio/playback.rs `play_item`,
 * `Annotation::play` in src/annotations.rs.)
 */

type Range = { timeStart: number; timeEnd: number; freqLow: number | null; freqHigh: number | null };
type Stream = {
  startSecs: number;
  endSecs: number;
  mode: string;
  hetFreq: number;
  bandLow: number | null;
  bandHigh: number | null;
};
type Snap = {
  currentFile: string | null;
  scrollOffset: number;
  zoom: number;
  sampleRate: number;
  duration: number;
  playbackMode: string;
  isPlaying: boolean;
  regions: Range[];
  selection: Range | null;
  lastStream: Stream | null;
};

const snap = (page: Page): Promise<Snap> =>
  page.evaluate(() => (window as unknown as { __oversample_test: () => Snap }).__oversample_test());

function noiseWav(sampleRate: number, seconds: number, seed = 777): Buffer {
  const n = Math.floor(sampleRate * seconds);
  const data = Buffer.alloc(n * 2);
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

/** Every saved annotation file's text. */
const savedTexts = (page: Page): Promise<string[]> =>
  page.evaluate(async () => {
    const root = await navigator.storage.getDirectory();
    const out: string[] = [];
    let dir: FileSystemDirectoryHandle;
    try {
      dir = await root.getDirectoryHandle("oversample-annotations");
    } catch {
      return out;
    }
    const entries = (dir as unknown as { entries: () => AsyncIterable<[string, FileSystemFileHandle]> }).entries();
    for await (const [, handle] of entries) {
      try {
        out.push(await (await handle.getFile()).text());
      } catch {}
    }
    return out;
  });

const NAME = "item-play.wav";

async function open(page: Page) {
  await page
    .locator('input[type="file"][accept*=".wav"]')
    .first()
    .setInputFiles({ name: NAME, mimeType: "audio/wav", buffer: noiseWav(96_000, 8) });
  await expect.poll(async () => (await snap(page)).currentFile).toBe(NAME);
  await expect(page.locator(".chart-stage canvas").first()).toBeVisible({ timeout: 30_000 });
}

/** Canvas position of fraction `f` of the visible time, and of the height. */
async function canvasPoint(page: Page) {
  const box = (await page.locator(".main-view .chart-stage > canvas").boundingBox())!;
  const s = await snap(page);
  const visible = (box.width / s.zoom) * (512 / s.sampleRate);
  const shown = Math.min(visible, s.duration - s.scrollOffset);
  return {
    x: (f: number) => box.x + (box.width * f * shown) / visible,
    y: (f: number) => box.y + box.height * f,
  };
}

/** Draws a selection with a frequency range from fraction t0 to t1 of the view. */
async function select(page: Page, t0: number, t1: number): Promise<Range> {
  await page.locator(".tool-combo-btn").click();
  await page.locator(".layer-panel button", { hasText: "Selection" }).click();
  const p = await canvasPoint(page);
  await page.mouse.move(p.x(t0), p.y(0.3));
  await page.mouse.down();
  await page.mouse.move(p.x(t1), p.y(0.6), { steps: 5 });
  await page.mouse.up();
  const sel = (await snap(page)).selection;
  expect(sel).not.toBeNull();
  return sel!;
}

/** Closes the "..." menu by clicking its button again (a click on the
 * spectrogram would also deselect the annotation). */
const closeMenu = (page: Page) =>
  page.locator('.canvas-overflow-btn[title="Annotation options"]').click({ force: true });

const playButton = (page: Page) => page.locator(".canvas-item-play");

/** Clicks the play button and checks the stream plays `range` with `mode`. */
async function playAndCheck(page: Page, range: Range, mode: string) {
  await playButton(page).click();
  await expect.poll(async () => (await snap(page)).lastStream?.mode).toBe(mode);
  const st = (await snap(page)).lastStream!;
  expect(st.startSecs).toBeCloseTo(range.timeStart, 3);
  expect(st.endSecs).toBeCloseTo(range.timeEnd, 3);
  return st;
}

test.describe("Play button on selections and annotations", () => {
  test("plays just the selection, and an annotation with its own play setting", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
    await open(page);
    expect((await snap(page)).playbackMode).toBe("Normal");

    // A selection: plays its time range, band-passed to its frequency range.
    const sel = await select(page, 0.3, 0.5);
    await expect(playButton(page)).toBeVisible();
    await expect(playButton(page)).toHaveAttribute("title", "Play");
    const st = await playAndCheck(page, sel, "Normal");
    expect(st.bandLow).toBeCloseTo(sel.freqLow!, 0);
    expect(st.bandHigh).toBeCloseTo(sel.freqHigh!, 0);
    // While it plays the button stops it.
    await expect.poll(async () => (await snap(page)).isPlaying).toBe(true);
    await expect(playButton(page)).toHaveAttribute("title", "Stop");
    await playButton(page).click();
    await expect.poll(async () => (await snap(page)).isPlaying).toBe(false);

    // Make it an annotation.
    await page.locator('.canvas-overflow-btn[title="Selection options"]').click();
    await page.locator(".canvas-overflow-item", { hasText: "Annotate Region" }).click();
    await expect(page.locator(".annotation-inline-label-editor")).toBeVisible();
    await page.keyboard.press("Enter");
    await expect.poll(async () => (await snap(page)).regions.length).toBe(1);
    const region = (await snap(page)).regions[0];

    // Without its own setting it plays with the app's.
    await expect(playButton(page)).toHaveAttribute("title", "Play");
    await playAndCheck(page, region, "Normal");
    await playButton(page).click();

    // Give it HET at 40 kHz from its "..." menu.
    await page.locator('.canvas-overflow-btn[title="Annotation options"]').click();
    const mode = page.locator("select.play-setting-mode");
    await expect(mode).toHaveValue("");
    await mode.selectOption("het");
    const value = page.locator("input.play-setting-value");
    // HET starts at the centre of the annotation's frequency range.
    const centre = (region.freqLow! + region.freqHigh!) / 2000;
    expect(Number(await value.inputValue())).toBeCloseTo(centre, 0);
    await value.fill("40");
    await value.press("Enter");
    await expect(playButton(page)).toHaveText("HET 40 kHz");
    await closeMenu(page);

    const het = await playAndCheck(page, region, "Heterodyne");
    expect(het.hetFreq).toBe(40_000);
    // The app's own mode is unchanged.
    expect((await snap(page)).playbackMode).toBe("Normal");
    await playButton(page).click();

    // Saved with the annotation.
    await expect
      .poll(async () => (await savedTexts(page)).some((t) => /play:\s*\n\s*mode: heterodyne\s*\n\s*freq_hz: 40000/.test(t)))
      .toBe(true);

    // After a reload, the same file's annotation still plays with it.
    await page.reload();
    await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
    await open(page);
    await expect.poll(async () => (await snap(page)).regions.length).toBe(1);
    const p = await canvasPoint(page);
    await page.mouse.click(p.x(0.4), p.y(0.45));
    await expect(playButton(page)).toHaveText("HET 40 kHz");
    const again = await playAndCheck(page, region, "Heterodyne");
    expect(again.hetFreq).toBe(40_000);
    await playButton(page).click();

    // Cleared: back to the app's settings, and no longer saved.
    await page.locator('.canvas-overflow-btn[title="Annotation options"]').click();
    await page.locator("select.play-setting-mode").selectOption("");
    await expect(playButton(page)).toHaveText("");
    await expect(page.locator("input.play-setting-value")).toHaveCount(0);
    await closeMenu(page);
    await playAndCheck(page, region, "Normal");
    await expect.poll(async () => (await savedTexts(page)).some((t) => t.includes("freq_hz"))).toBe(false);
  });
});
