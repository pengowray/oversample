import { test, expect, type Page } from "@playwright/test";

/**
 * How a file opens when another file was open before it: stopped, with the
 * view and the playhead at the start, without the previous file's selection,
 * and with its own annotations. The previous file keeps its own scroll,
 * view history and bookmarks.
 * (The file-switch Effect in src/components/app.rs; the rules are in
 * src/scope.rs.)
 */

type Range = { timeStart: number; timeEnd: number; freqLow: number | null; freqHigh: number | null };
type Snap = {
  currentFile: string | null;
  scrollOffset: number;
  zoom: number;
  sampleRate: number;
  duration: number;
  isPlaying: boolean;
  playheadTime: number;
  streamFile: string | null;
  selection: Range | null;
  regions: Range[];
  navIndex: number;
  navLength: number;
  bookmarks: number[];
};

function noiseWav(sampleRate: number, seconds: number, seed = 4321): Buffer {
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

const snap = (page: Page): Promise<Snap> =>
  page.evaluate(() => (window as unknown as { __oversample_test: () => Snap }).__oversample_test());

async function open(page: Page, name: string, seconds = 8, seed = 4321, rate = 96_000) {
  await page
    .locator('input[type="file"][accept*=".wav"]')
    .first()
    .setInputFiles({ name, mimeType: "audio/wav", buffer: noiseWav(rate, seconds, seed) });
  await expect(page.locator(".file-item", { hasText: name })).toBeVisible({ timeout: 30_000 });
  await expect.poll(async () => (await snap(page)).currentFile).toBe(name);
}

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

async function select(page: Page, t0: number, t1: number, y0: number, y1: number) {
  const canvas = page.locator(".main-view .chart-stage > canvas");
  const box = (await canvas.boundingBox())!;
  const s = await snap(page);
  const visible = (box.width / s.zoom) * (512 / s.sampleRate);
  const shown = Math.min(visible, s.duration - s.scrollOffset);
  const x = (f: number) => box.x + (box.width * f * shown) / visible;
  await page.mouse.move(x(t0), box.y + box.height * y0);
  await page.mouse.down();
  await page.mouse.move(x(t1), box.y + box.height * y1, { steps: 5 });
  await page.mouse.up();
}

async function stubXc(page: Page) {
  await page.route("**/search/**", (r) =>
    r.fulfill({ status: 404, headers: { "access-control-allow-origin": "*" }, body: "{}" }),
  );
  await page.route("**/xc/recording/*", (r) => {
    const id = r.request().url().split("/").pop()!;
    const rec: Record<string, unknown> = {
      id,
      gen: "Myotis",
      sp: id === "111" ? "daubentonii" : "nattereri",
      en: id === "111" ? "Daubenton's Bat" : "Natterer's Bat",
      file: `https://xeno-canto.org/${id}/download`,
      "file-name": `XC${id}.wav`,
      smp: "96000",
      length: "0:08",
    };
    if (id === "111") {
      rec["annotation-set"] = {
        set_name: "Annotation set for XC111",
        annotations: [
          { annotation_xc_id: 1, xc_nr: "111", scientific_name: "Myotis daubentonii", start_time: 1.0, end_time: 2.0, frequency_high: 40000, frequency_low: 20000, sound_type: "echolocation" },
          { annotation_xc_id: 2, xc_nr: "111", scientific_name: "Myotis daubentonii", start_time: 3.0, end_time: 3.5, frequency_high: 40000, frequency_low: 20000, sound_type: "echolocation" },
        ],
      };
    }
    return r.fulfill({ status: 200, contentType: "application/json", headers: { "access-control-allow-origin": "*" }, body: JSON.stringify(rec) });
  });
  await page.route("**/xc/audio/*", async (r) => {
    const id = r.request().url().split("/").pop()!;
    return r.fulfill({ status: 200, contentType: "audio/wav", headers: { "access-control-allow-origin": "*" }, body: noiseWav(96_000, 8, Number(id)) });
  });
}

async function loadXc(page: Page, id: number, waitName = true) {
  await page.locator("button.add-files-btn", { hasText: "Find recordings" }).click();
  await expect(page.locator(".find-modal")).toBeVisible();
  await page.locator(".find-modal input").first().fill(`XC${id}`);
  await page.locator(".find-link-row").click();
  if (waitName) await expect.poll(async () => (await snap(page)).currentFile ?? "", { timeout: 30_000 }).toContain(`XC${id}`);
}


async function show(page: Page, name: string) {
  await page.locator(".file-item", { hasText: name }).click();
  await expect.poll(async () => (await snap(page)).currentFile ?? "").toContain(name);
}

async function start(page: Page, name: string, seconds = 8) {
  await page.goto("/");
  await open(page, name, seconds);
  await expect(page.locator(".chart-stage canvas").first()).toBeVisible({ timeout: 30_000 });
}

/** Zooms in, scrolls right, and returns the scroll position. */
async function scrollAway(page: Page, presses = 4): Promise<number> {
  await zoomIn(page);
  for (let i = 0; i < presses; i++) await page.keyboard.press("ArrowRight");
  const scroll = (await snap(page)).scrollOffset;
  expect(scroll).toBeGreaterThan(1);
  return scroll;
}

/** Starts playback and waits until the view has followed the playhead. */
async function playPast(page: Page, scroll: number) {
  await page.keyboard.press("Space");
  await expect.poll(async () => (await snap(page)).isPlaying).toBe(true);
  await expect.poll(async () => (await snap(page)).scrollOffset).toBeGreaterThan(scroll + 0.3);
}

async function expectStoppedAtStart(page: Page) {
  await expect.poll(async () => (await snap(page)).isPlaying).toBe(false);
  // The old stream used to end a few seconds later and move the view then.
  await page.waitForTimeout(1500);
  const s = await snap(page);
  expect(s.isPlaying).toBe(false);
  expect(s.playheadTime).toBe(0);
  expect(s.scrollOffset).toBe(0);
}

async function selectTool(page: Page) {
  await page.locator(".tool-combo-btn").click();
  await page.locator(".layer-panel button", { hasText: "Selection" }).click();
}

test.describe("Opening another file", () => {
  test("a file opened during playback opens stopped at the start", async ({ page }) => {
    await start(page, "alpha.wav");
    const before = await scrollAway(page);
    await playPast(page, before);

    await open(page, "beta.wav", 8, 99);
    await expectStoppedAtStart(page);

    // The first file is where it was before Play, as after Stop.
    await show(page, "alpha.wav");
    expect((await snap(page)).scrollOffset).toBeCloseTo(before, 6);
    expect((await snap(page)).playheadTime).toBe(0);
  });

  test("a file opened after Stop has its playhead at the start", async ({ page }) => {
    await start(page, "alpha.wav");
    const before = await scrollAway(page);
    await playPast(page, before);
    await page.keyboard.press("Space");
    expect((await snap(page)).playheadTime).toBeGreaterThan(before);

    await open(page, "beta.wav", 8, 99);
    await expectStoppedAtStart(page);
  });

  test("the selection is not carried to another recording", async ({ page }) => {
    await start(page, "alpha.wav");
    await selectTool(page);
    await select(page, 0.5, 0.7, 0.3, 0.6);
    const made = (await snap(page)).selection;
    expect(made).not.toBeNull();
    expect(made!.timeStart).toBeGreaterThan(2);

    await open(page, "beta.wav", 8, 99);
    expect((await snap(page)).selection).toBeNull();

    // Play starts at the start of the file, not at the old selection.
    await page.keyboard.press("Space");
    await expect.poll(async () => (await snap(page)).isPlaying).toBe(true);
    expect((await snap(page)).playheadTime).toBeLessThan(made!.timeStart - 0.5);
    await page.keyboard.press("Space");
  });

  test("another channel of the same recording keeps the selection", async ({ page }) => {
    await start(page, "rec_Ch1.wav");
    await open(page, "rec_Ch2.wav", 8, 99);
    await selectTool(page);
    await select(page, 0.5, 0.7, 0.3, 0.6);
    const made = (await snap(page)).selection;
    expect(made).not.toBeNull();

    await show(page, "rec_Ch1.wav");
    expect((await snap(page)).selection).toEqual(made);
  });

  test("a file keeps its scroll after a shorter file is opened", async ({ page }) => {
    await start(page, "alpha.wav", 20);
    const before = await scrollAway(page, 30);
    expect(before).toBeGreaterThan(5);

    await open(page, "beta.wav", 1, 99);
    expect((await snap(page)).scrollOffset).toBe(0);

    await show(page, "alpha.wav");
    expect((await snap(page)).scrollOffset).toBeCloseTo(before, 6);
  });

  test("closing the open file shows the next one with its own view", async ({
    page,
  }) => {
    await start(page, "alpha.wav", 20);
    await open(page, "beta.wav", 8, 99);
    await show(page, "alpha.wav");
    await scrollAway(page, 30);
    await selectTool(page);
    await select(page, 0.2, 0.4, 0.3, 0.6);
    expect((await snap(page)).selection).not.toBeNull();

    // alpha is first in the list, so beta takes its position.
    await page.locator(".file-item", { hasText: "alpha.wav" }).locator(".file-item-close").click();
    await expect.poll(async () => (await snap(page)).currentFile).toBe("beta.wav");
    const shown = await snap(page);
    expect(shown.selection).toBeNull();
    expect(shown.scrollOffset).toBe(0);
  });

  test("a Xeno-canto recording opened during playback opens stopped, with its own annotations", async ({
    page,
  }) => {
    await stubXc(page);
    await page.goto("/");
    await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
    await loadXc(page, 111);
    await expect(page.locator(".chart-stage canvas").first()).toBeVisible({ timeout: 30_000 });
    await expect.poll(async () => (await snap(page)).regions.length).toBe(2);
    const before = await scrollAway(page);
    await playPast(page, before);

    await loadXc(page, 222);
    await expectStoppedAtStart(page);
    expect((await snap(page)).regions).toEqual([]);
    expect((await snap(page)).selection).toBeNull();

    await show(page, "XC111");
    expect((await snap(page)).regions.length).toBe(2);
    expect((await snap(page)).scrollOffset).toBeCloseTo(before, 6);
    await show(page, "XC222");
    expect((await snap(page)).regions).toEqual([]);
    expect((await snap(page)).scrollOffset).toBe(0);
  });
});

// ── View history (Back and Forward) and bookmarks ───────────────────────────

/** The Back or Forward item of the top bar's menu; the menu is left open. */
async function historyItem(page: Page, label: "Back" | "Forward") {
  if (!(await page.locator(".toolbar-overflow-menu").count())) {
    await page.locator(".toolbar-overflow-btn").click();
  }
  return page.locator(".toolbar-overflow-item", { hasText: label });
}

async function closeMenu(page: Page) {
  if (await page.locator(".toolbar-overflow-backdrop").count()) {
    await page.locator(".toolbar-overflow-backdrop").click();
  }
}

async function canGoBack(page: Page): Promise<boolean> {
  const enabled = await (await historyItem(page, "Back")).isEnabled();
  await closeMenu(page);
  return enabled;
}

/** Clicks the overview strip at fraction `f` of its width: the view jumps there. */
async function jumpTo(page: Page, f: number): Promise<number> {
  const box = (await page.locator(".overview-strip canvas").last().boundingBox())!;
  await page.mouse.click(box.x + box.width * f, box.y + box.height / 2);
  return (await snap(page)).scrollOffset;
}

/**
 * Makes a view history in the open file by jumping to three places. Returns
 * the place Back goes to (the first of the three: each jump records the
 * place it left, and Back goes to the one recorded before the newest).
 */
async function makeHistory(page: Page): Promise<number> {
  await zoomIn(page);
  const first = await jumpTo(page, 0.3);
  const second = await jumpTo(page, 0.6);
  const third = await jumpTo(page, 0.8);
  expect(first).toBeGreaterThan(2);
  expect(second).toBeGreaterThan(first + 1);
  expect(third).toBeGreaterThan(second + 1);
  expect(await canGoBack(page)).toBe(true);
  return first;
}

async function goBack(page: Page) {
  await (await historyItem(page, "Back")).click();
}

test.describe("View history and bookmarks belong to a file", () => {
  test("Back in a newly opened file does not go to the previous file's position", async ({
    page,
  }) => {
    await start(page, "alpha.wav", 20);
    const place = await makeHistory(page);
    const left = (await snap(page)).scrollOffset;

    await open(page, "beta.wav", 20, 99);
    expect(await canGoBack(page)).toBe(false);
    expect((await snap(page)).navLength).toBe(0);
    expect((await snap(page)).scrollOffset).toBe(0);

    // The first file is where it was left, with its history.
    await show(page, "alpha.wav");
    expect((await snap(page)).scrollOffset).toBeCloseTo(left, 6);
    expect(await canGoBack(page)).toBe(true);
    await goBack(page);
    await expect.poll(async () => (await snap(page)).scrollOffset).toBeCloseTo(place, 6);
  });

  test("closing the open file does not leave its history to the next file", async ({ page }) => {
    await start(page, "alpha.wav", 20);
    await open(page, "beta.wav", 20, 99);
    await show(page, "alpha.wav");
    await makeHistory(page);

    await page.locator(".file-item", { hasText: "alpha.wav" }).locator(".file-item-close").click();
    await expect.poll(async () => (await snap(page)).currentFile).toBe("beta.wav");
    expect(await canGoBack(page)).toBe(false);
  });

  test("channels of one recording share the history", async ({ page }) => {
    await start(page, "rec_Ch1.wav", 20);
    await open(page, "rec_Ch2.wav", 20, 99);
    const place = await makeHistory(page);

    await show(page, "rec_Ch1.wav");
    expect(await canGoBack(page)).toBe(true);
    await goBack(page);
    await expect.poll(async () => (await snap(page)).scrollOffset).toBeCloseTo(place, 6);
  });

  test("a recording and its denoised copy share the history, also with the D key", async ({
    page,
  }) => {
    await start(page, "bat.wav", 20);
    await open(page, "bat (denoised).wav", 20, 99);
    await open(page, "other.wav", 20, 7);
    await show(page, "bat.wav");
    const place = await makeHistory(page);
    const history = (await snap(page)).navLength;

    // D shows the denoised copy: same recording, same history.
    await page.keyboard.press("d");
    await expect.poll(async () => (await snap(page)).currentFile).toBe("bat (denoised).wav");
    expect((await snap(page)).navLength).toBe(history);
    // An unrelated file has none, and the pair has it again afterwards.
    await show(page, "other.wav");
    expect((await snap(page)).navLength).toBe(0);
    await show(page, "bat (denoised).wav");
    expect((await snap(page)).navLength).toBe(history);
    await goBack(page);
    await expect.poll(async () => (await snap(page)).scrollOffset).toBeCloseTo(place, 6);
  });

  test("a bookmark made in one file is not shown in another", async ({ page }) => {
    // A click on the spectrogram during playback makes a bookmark. The list
    // of bookmarks shows for a few seconds when playback reaches the end.
    const playToEnd = async () => {
      await page.keyboard.press("Space");
      await expect.poll(async () => (await snap(page)).isPlaying).toBe(true);
      await expect.poll(async () => (await snap(page)).isPlaying).toBe(false);
    };
    await start(page, "alpha.wav", 2);
    await page.keyboard.press("Space");
    await expect.poll(async () => (await snap(page)).isPlaying).toBe(true);
    const box = (await page.locator(".main-view .chart-stage > canvas").boundingBox())!;
    await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
    await expect.poll(async () => (await snap(page)).isPlaying).toBe(false);
    await expect(page.locator(".bookmark-popup .bookmark-item")).toHaveCount(1);
    const made = (await snap(page)).bookmarks;
    expect(made.length).toBe(1);

    await open(page, "beta.wav", 2, 99);
    expect((await snap(page)).bookmarks).toEqual([]);
    await expect(page.locator(".bookmark-popup")).toHaveCount(0);
    await playToEnd();
    // Counted once, without waiting: the list goes away by itself after a
    // few seconds.
    await page.waitForTimeout(300);
    expect(await page.locator(".bookmark-popup").count()).toBe(0);

    // The first file has its bookmark again. (The list is not checked here:
    // the timer that hid the first list can hide a second one early.)
    await show(page, "alpha.wav");
    expect((await snap(page)).bookmarks).toEqual(made);
  });
});
