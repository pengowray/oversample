import { test, expect, type Page, type Route } from "@playwright/test";
import * as fs from "node:fs";
import * as path from "node:path";

/**
 * The x10 time-expansion correction in the Info panel (src/te_correction.rs).
 *
 * A file at a x10 header rate that is too low for its species' calls (case T
 * in src/species_check.rs) gets the species note "Probably time-expanded
 * x10" with a "Treat as time-expanded x10" button. Pressing it reads the file
 * at ten times its rate; the time-expansion note that replaces the species
 * note has a "Use the file's sample rate" button that goes back.
 *
 * The recording is demo XC815138 with a generated WAV and the metadata of
 * XC1001172 (Eptesicus nilssonii, 25-45 kHz, at 25.6 kHz), served as in
 * species_range.spec.ts.
 *
 * Set TE_SHOT_DIR to save screenshots of each state there.
 */

const SHOT_DIR = process.env.TE_SHOT_DIR;

async function shot(page: Page, name: string) {
  if (SHOT_DIR) await page.screenshot({ path: path.join(SHOT_DIR, name) });
}

/** Mono 16-bit WAV of a quiet 4 kHz tone, with a GUANO chunk when given. */
function wav(rate: number, seconds: number, guano?: string): Buffer {
  const n = Math.round(rate * seconds);
  const data = Buffer.alloc(n * 2);
  for (let i = 0; i < n; i++) {
    data.writeInt16LE(Math.round(Math.sin((2 * Math.PI * 4000 * i) / rate) * 8000), i * 2);
  }
  const chunks = [data];
  if (guano) {
    let text = Buffer.from(guano, "utf8");
    const head = Buffer.alloc(8);
    head.write("guan", 0);
    head.writeUInt32LE(text.length, 4);
    if (text.length % 2) text = Buffer.concat([text, Buffer.alloc(1)]);
    chunks.push(head, text);
  }
  const body = Buffer.concat(chunks);
  const h = Buffer.alloc(44);
  h.write("RIFF", 0);
  h.writeUInt32LE(36 + body.length, 4);
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
  return Buffer.concat([h, body]);
}

interface Case {
  species: string;
  type: string;
  rate: number;
  seconds?: number;
  fields?: Record<string, string>;
  guano?: string;
}

/** XC1001172: Eptesicus nilssonii (25-45 kHz) at 25.6 kHz. 20 seconds, so
 * the corrected file (2 seconds) still plays long enough to stop it. */
const CASE_T: Case = {
  species: "Eptesicus nilssonii",
  type: "echolocation",
  rate: 25_600,
  seconds: 20,
};

async function routeDemo(page: Page, handler: (route: Route, url: string) => unknown) {
  const h = (route: Route) => handler(route, route.request().url());
  await page.route("https://archive.oversample.com/**", h);
  await page.route("https://cdn.jsdelivr.net/gh/pengowray/bat-demo-sounds@main/**", h);
}

/** Opens demo XC815138 with the case's sidecar and a generated WAV. */
async function openCase(page: Page, c: Case) {
  const [gen, sp] = c.species.split(" ");
  const sidecar = {
    id: "815138",
    en: "Test bat",
    gen,
    sp,
    rec: "Test",
    cnt: "Sweden",
    method: "field recording",
    rmk: "",
    type: c.type,
    url: "https://xeno-canto.org/815138",
    ...c.fields,
  };
  const audio = wav(c.rate, c.seconds ?? 1, c.guano);
  await routeDemo(page, (route, url) => {
    if (url.endsWith("/index.json")) {
      return route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          version: 1,
          sounds: [
            {
              en: "Test bat",
              filename: "XC815138 demo.wav",
              metadata: "XC815138 demo.xc.json",
              source: "xeno-canto",
              species: c.species,
              xc_id: 815138,
            },
          ],
        }),
      });
    }
    if (url.endsWith(".xc.json")) {
      return route.fulfill({ contentType: "application/json", body: JSON.stringify(sidecar) });
    }
    if (url.endsWith(".wav")) return route.fulfill({ contentType: "audio/wav", body: audio });
    return route.fulfill({ status: 404, body: "" });
  });
  await page.goto("/#XC815138");
  await expect(page.locator(".toolbar-badge-row .badge-sample-rate")).toBeVisible({
    timeout: 60_000,
  });
  await page.waitForFunction(
    () => typeof (window as { __oversample_test?: unknown }).__oversample_test === "function",
  );
}

async function showInfo(page: Page) {
  const btn = page.locator('button[title="Show info panel"]');
  if (await btn.count()) await btn.first().click();
  await expect(page.locator(".right-sidebar")).not.toHaveClass(/collapsed/);
  await expect(row(page, "Sample rate")).toBeVisible();
}

/** The value of the Info panel's File row labelled `label`. */
function row(page: Page, label: string) {
  return page
    .locator(".metadata-row-inline", {
      has: page.locator(".setting-label", { hasText: new RegExp(`^${label}$`) }),
    })
    .locator(".metadata-value");
}

interface Range {
  timeStart: number;
  timeEnd: number;
  freqLow: number | null;
  freqHigh: number | null;
}

interface Snap {
  sampleRate: number;
  duration: number;
  fileMaxFreq: number;
  displayFreqLo: number;
  displayFreqHi: number;
  scrollOffset: number;
  zoom: number;
  selection: Range | null;
  regions: Range[];
  isPlaying: boolean;
}

async function snap(page: Page): Promise<Snap> {
  return page.evaluate(() =>
    (window as unknown as { __oversample_test: () => Snap }).__oversample_test(),
  );
}

const SPECIES_NOTE = '.rate-note[data-note="species"]';
const TE_NOTE = '.rate-note[data-note="time-expansion"]';
const TREAT = 'button.rate-note-action[data-action="treat-as-tenfold"]';
const USE_FILE = 'button.rate-note-action[data-action="use-file-rate"]';

/** `r` with its times divided and its frequencies multiplied by `k`. */
function scaled(r: Range, k: number): Range {
  return {
    timeStart: r.timeStart / k,
    timeEnd: r.timeEnd / k,
    freqLow: r.freqLow === null ? null : r.freqLow * k,
    freqHigh: r.freqHigh === null ? null : r.freqHigh * k,
  };
}

function expectRange(actual: Range | null | undefined, expected: Range) {
  expect(actual).toBeTruthy();
  const a = actual!;
  expect(a.timeStart).toBeCloseTo(expected.timeStart, 9);
  expect(a.timeEnd).toBeCloseTo(expected.timeEnd, 9);
  for (const k of ["freqLow", "freqHigh"] as const) {
    if (expected[k] === null) expect(a[k]).toBeNull();
    else expect(a[k]!).toBeCloseTo(expected[k]!, 4);
  }
}

/** Drags out a selection from `t0` to `t1`, given as fractions of the part of
 * the file in view, over the given fractions of the spectrogram's height
 * (from the top). The spectrogram's width in seconds is its width in columns
 * (pixels over zoom) times the seconds per column (512 samples). */
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

/** Draws a region annotation over the given fractions of the view (as in
 * `select`) and waits for it. */
async function annotate(page: Page, t0: number, t1: number, y0: number, y1: number) {
  const n = (await snap(page)).regions.length;
  await page.locator(".tool-combo-btn").click();
  await page.locator(".layer-panel button", { hasText: "Selection" }).click();
  await select(page, t0, t1, y0, y1);
  await page.locator(".canvas-overflow-btn").click();
  await page.locator(".canvas-overflow-menu button", { hasText: "Annotate" }).click();
  await expect.poll(async () => (await snap(page)).regions.length).toBe(n + 1);
}

/** Opens a right-sidebar tab by its name in the tab menu. */
async function openTab(page: Page, name: string) {
  await page.locator(".right-sidebar .sidebar-tab-dropdown").click();
  await page.locator(".sidebar-tab-option", { hasText: new RegExp(`^${name}$`) }).first().click();
}

/** Opens the Annotations tab with its "Annotation files" section expanded. */
async function openAnnotationFiles(page: Page) {
  await openTab(page, "Annotations");
  const header = page.locator(".export-section-header", { hasText: "Annotation files" });
  await expect(header).toBeVisible();
  const body = page.locator(".export-section-body");
  if (!(await body.evaluate((b) => b.classList.contains("open")))) await header.click();
}

/** Presses "Export .batm" and returns the downloaded file's path and text. */
async function exportBatm(page: Page, name: string) {
  const [dl] = await Promise.all([
    page.waitForEvent("download"),
    page.locator(".export-section-body button", { hasText: "Export .batm" }).click(),
  ]);
  const file = test.info().outputPath(name);
  await dl.saveAs(file);
  return { file, text: fs.readFileSync(file, "utf8") };
}

/** Imports an annotation file with "Import .batm/.json". */
async function importAnnotations(page: Page, file: string) {
  const [chooser] = await Promise.all([
    page.waitForEvent("filechooser"),
    page.locator(".export-section-body button", { hasText: "Import .batm/.json" }).click(),
  ]);
  await chooser.setFiles(file);
}

/** The rate, duration and first region of a .batm file. */
function batmFields(text: string) {
  const num = (key: string) => {
    const m = text.match(new RegExp(`^\\s*${key}: ([-0-9.e]+)$`, "m"));
    expect(m, key).toBeTruthy();
    return Number(m![1]);
  };
  return {
    sampleRate: num("sample_rate"),
    duration: num("duration_secs"),
    region: {
      timeStart: num("time_start"),
      timeEnd: num("time_end"),
      freqLow: num("freq_low"),
      freqHigh: num("freq_high"),
    } as Range,
  };
}

/** Lets hover and tap highlights fade before a screenshot: the pointer goes to
 * the window title, or a phone taps the Info panel's "File" heading. */
async function settle(page: Page) {
  if (page.viewportSize()!.width < 768) {
    await page.locator(".right-sidebar .setting-group-title", { hasText: "File" }).first().tap();
  } else {
    await page.mouse.move(785, 16);
  }
  await page.waitForTimeout(500);
}

test.describe("x10 correction on a probably time-expanded file", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("the button corrects the rate and the second button undoes it", async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await openCase(page, CASE_T);
    await showInfo(page);

    // Zoom in on time (Ctrl+wheel) and frequency (Shift+wheel), and scroll.
    await expect(page.locator(".main-view .chart-stage > canvas")).toBeVisible();
    const box = (await page.locator(".main-view .chart-stage > canvas").boundingBox())!;
    await page.mouse.move(box.x + box.width * 0.2, box.y + box.height * 0.5);
    await page.keyboard.down("Control");
    for (let i = 0; i < 25; i++) await page.mouse.wheel(0, -100);
    await page.keyboard.up("Control");
    await page.keyboard.down("Shift");
    for (let i = 0; i < 4; i++) await page.mouse.wheel(0, -100);
    await page.keyboard.up("Shift");
    for (let i = 0; i < 3; i++) await page.mouse.wheel(0, 100);
    await expect.poll(async () => (await snap(page)).scrollOffset).toBeGreaterThan(0.1);

    // An annotation and a selection in view, made before the correction.
    await page.locator(".tool-combo-btn").click();
    await page.locator(".layer-panel button", { hasText: "Selection" }).click();
    await select(page, 0.15, 0.4, 0.3, 0.6);
    await page.locator(".canvas-overflow-btn").click();
    await page.locator(".canvas-overflow-menu button", { hasText: "Annotate" }).click();
    await expect.poll(async () => (await snap(page)).regions.length).toBe(1);
    await select(page, 0.55, 0.8, 0.2, 0.5);
    await expect.poll(async () => (await snap(page)).selection !== null).toBe(true);

    const before = await snap(page);
    expect(before.sampleRate).toBe(25_600);
    expect(before.duration).toBeCloseTo(20, 9);
    expect(before.fileMaxFreq).toBe(12_800);
    expect(before.zoom).toBeGreaterThan(5);
    expect(before.displayFreqHi - before.displayFreqLo).toBeLessThan(12_800);
    const region = before.regions[0];
    const selection = before.selection!;
    await expect(row(page, "Sample rate")).toHaveText("25.6 kHz");
    const durationBefore = await row(page, "Duration").textContent();

    // The button sits under the amber line it acts on, above the species line.
    const treat = page.locator(`${SPECIES_NOTE} ${TREAT}`);
    await expect(treat).toHaveText("Treat as time-expanded ×10");
    await expect(treat).toHaveAttribute(
      "title",
      "Uses 256 kHz as the sample rate. The file is not changed.",
    );
    const order = await page
      .locator(SPECIES_NOTE)
      .evaluate((n) => Array.from(n.children).map((c) => c.className));
    expect(order).toEqual([
      "rate-note-warning",
      "hash-calc-btn rate-note-action",
      "rate-note-muted",
    ]);
    expect((await treat.boundingBox())!.height).toBeGreaterThanOrEqual(24);
    await settle(page);
    await shot(page, "desktop-1440-before.png");

    // Playback stops when the rate changes.
    await page.locator(".play-main-btn").click();
    await expect.poll(async () => (await snap(page)).isPlaying).toBe(true);

    await treat.click();
    await expect(row(page, "Sample rate")).toHaveText("256 kHz");
    await expect(page.locator(SPECIES_NOTE)).toHaveCount(0);
    await expect(page.locator(TE_NOTE)).toHaveText(
      /^Corrected for ×10 time expansion; the file says 25\.6 kHz\.Use the file's sample rate$/,
    );
    // The time-expansion note is now the first note under the Sample rate row.
    expect(
      await page.locator(TE_NOTE).evaluate((n) => n.previousElementSibling!.textContent),
    ).toContain("Sample rate");
    await expect(row(page, "Duration")).not.toHaveText(durationBefore!);

    const after = await snap(page);
    expect(after.isPlaying).toBe(false);
    expect(after.sampleRate).toBe(256_000);
    expect(after.duration).toBeCloseTo(2, 9);
    expect(after.fileMaxFreq).toBe(128_000);
    // The same frequency rows stay in view.
    expect(after.displayFreqLo).toBeCloseTo(before.displayFreqLo * 10, 6);
    expect(after.displayFreqHi).toBeCloseTo(before.displayFreqHi * 10, 6);
    // Zoom is in pixels per spectrogram column, so it stays.
    expect(after.zoom).toBe(before.zoom);
    expect(after.scrollOffset).toBeCloseTo(before.scrollOffset / 10, 9);
    // Annotations and the selection stay on the same samples.
    expect(after.regions.length).toBe(1);
    expectRange(after.regions[0], scaled(region, 10));
    expectRange(after.selection, scaled(selection, 10));
    const durationAfter = await row(page, "Duration").textContent();
    await settle(page);
    await shot(page, "desktop-1440-apply-after.png");

    // A second press is not possible: the button has gone.
    await expect(page.locator(TREAT)).toHaveCount(0);

    // The corrected file plays, and going back to the file's rate stops it.
    await page.locator(".play-main-btn").click();
    await expect.poll(async () => (await snap(page)).isPlaying).toBe(true);

    const useFile = page.locator(`${TE_NOTE} ${USE_FILE}`);
    await expect(useFile).toHaveText("Use the file's sample rate");
    expect(await useFile.getAttribute("title")).toBeNull();
    await useFile.click();
    expect((await snap(page)).isPlaying).toBe(false);

    await expect(row(page, "Sample rate")).toHaveText("25.6 kHz");
    await expect(row(page, "Duration")).toHaveText(durationBefore!);
    expect(durationAfter).not.toBe(durationBefore);
    await expect(page.locator(TE_NOTE)).toHaveCount(0);
    await expect(page.locator(`${SPECIES_NOTE} ${TREAT}`)).toBeVisible();
    const undone = await snap(page);
    expect(undone.sampleRate).toBe(25_600);
    expect(undone.duration).toBe(before.duration);
    expect(undone.fileMaxFreq).toBe(12_800);
    expect(undone.displayFreqLo).toBeCloseTo(before.displayFreqLo, 6);
    expect(undone.displayFreqHi).toBeCloseTo(before.displayFreqHi, 6);
    expect(undone.zoom).toBe(before.zoom);
    expect(undone.scrollOffset).toBeCloseTo(before.scrollOffset, 9);
    expectRange(undone.regions[0], region);
    expectRange(undone.selection, selection);
    await settle(page);
    await shot(page, "desktop-1440-undo-after.png");

    // The pressed button becomes "Use the file's sample rate" in place. A
    // double-click applies the correction once, and Enter held down on the
    // focused button acts once.
    await page.locator(TREAT).dblclick();
    await expect(row(page, "Sample rate")).toHaveText("256 kHz");
    await page.waitForTimeout(300);
    await expect(row(page, "Sample rate")).toHaveText("256 kHz");
    await page.locator(USE_FILE).focus();
    await page.keyboard.down("Enter");
    await expect(row(page, "Sample rate")).toHaveText("25.6 kHz");
    await page.keyboard.down("Enter");
    await page.keyboard.down("Enter");
    await page.keyboard.up("Enter");
    await page.waitForTimeout(300);
    await expect(row(page, "Sample rate")).toHaveText("25.6 kHz");
    expect(errors).toEqual([]);
  });

  test("an annotation made after the correction is kept by the undo", async ({ page }) => {
    await openCase(page, CASE_T);
    await showInfo(page);
    await page.locator(TREAT).click();
    await expect(row(page, "Sample rate")).toHaveText("256 kHz");

    await page.locator(".tool-combo-btn").click();
    await page.locator(".layer-panel button", { hasText: "Selection" }).click();
    await select(page, 0.3, 0.5, 0.3, 0.6);
    await page.locator(".canvas-overflow-btn").click();
    await page.locator(".canvas-overflow-menu button", { hasText: "Annotate" }).click();
    await expect.poll(async () => (await snap(page)).regions.length).toBe(1);
    const made = (await snap(page)).regions[0];
    // Drawn on the corrected file (2 seconds), in its seconds and Hz.
    expect(made.timeStart).toBeGreaterThan(0.5);
    expect(made.timeEnd).toBeLessThan(1.1);
    expect(made.freqHigh!).toBeGreaterThan(12_800);

    await page.locator(USE_FILE).click();
    await expect(row(page, "Sample rate")).toHaveText("25.6 kHz");
    const back = await snap(page);
    expect(back.regions.length).toBe(1);
    expectRange(back.regions[0], scaled(made, 0.1));
  });

  test("saved annotations stay on the same sounds when the file is opened again", async ({
    page,
  }) => {
    await openCase(page, CASE_T);
    await showInfo(page);
    await page.locator(TREAT).click();
    await expect(row(page, "Sample rate")).toHaveText("256 kHz");
    await page.locator(".tool-combo-btn").click();
    await page.locator(".layer-panel button", { hasText: "Selection" }).click();
    await select(page, 0.3, 0.5, 0.3, 0.6);
    await page.locator(".canvas-overflow-btn").click();
    await page.locator(".canvas-overflow-menu button", { hasText: "Annotate" }).click();
    await expect.poll(async () => (await snap(page)).regions.length).toBe(1);
    const made = (await snap(page)).regions[0];

    // The correction lasts for the session: the file opens at its own rate,
    // and the annotation saved while it was corrected is in the file's own
    // seconds and Hz.
    await expect
      .poll(
        () =>
          page.evaluate(async () => {
            const root = await navigator.storage.getDirectory();
            const dir = await root.getDirectoryHandle("oversample-annotations", { create: true });
            for await (const [, handle] of (dir as any).entries()) {
              const text = await (await (handle as FileSystemFileHandle).getFile()).text();
              if (text.includes("Region")) return true;
            }
            return false;
          }),
        { timeout: 15_000 },
      )
      .toBe(true);
    await page.reload();
    await expect(page.locator(".toolbar-badge-row .badge-sample-rate")).toBeVisible({
      timeout: 60_000,
    });
    await expect.poll(async () => (await snap(page)).regions.length, { timeout: 15_000 }).toBe(1);
    const reopened = await snap(page);
    expect(reopened.sampleRate).toBe(25_600);
    expectRange(reopened.regions[0], scaled(made, 0.1));
  });

  test(".batm files exported and imported while corrected use the file's seconds and Hz", async ({
    page,
  }) => {
    await openCase(page, CASE_T);
    await showInfo(page);
    await annotate(page, 0.3, 0.5, 0.3, 0.6);
    const region = (await snap(page)).regions[0];

    await openAnnotationFiles(page);
    const before = await exportBatm(page, "before.batm");
    const b = batmFields(before.text);
    expect(b.sampleRate).toBe(25_600);
    expect(b.duration).toBeCloseTo(20, 9);
    expectRange(b.region, region);

    await openTab(page, "Info");
    await page.locator(TREAT).click();
    await expect(row(page, "Sample rate")).toHaveText("256 kHz");
    const corrected = (await snap(page)).regions[0];
    expectRange(corrected, scaled(region, 10));

    // Exported while corrected: the same file as before the correction.
    await openAnnotationFiles(page);
    const during = batmFields((await exportBatm(page, "corrected.batm")).text);
    expect(during.sampleRate).toBe(25_600);
    expect(during.duration).toBeCloseTo(20, 9);
    expectRange(during.region, region);

    // Imported while corrected: lands on the same sounds as the annotation
    // already there.
    await importAnnotations(page, before.file);
    await expect.poll(async () => (await snap(page)).regions.length).toBe(2);
    for (const r of (await snap(page)).regions) expectRange(r, corrected);
  });

});

test.describe("no x10 button on other files", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  const OTHERS: [string, Case][] = [
    [
      "a rate that is not a x10 rate (XC1099113)",
      { species: "Pipistrellus kuhlii", type: "echolocation", rate: 48_000 },
    ],
    [
      "a file the app corrected from its GUANO",
      {
        species: "Pipistrellus pipistrellus",
        type: "echolocation",
        rate: 38_400,
        guano: "GUANO|Version: 1.0\nTE: 10\n",
      },
    ],
    [
      "a x10 header rate with a heterodyne remark",
      {
        species: "Eptesicus nilssonii",
        type: "echolocation",
        rate: 25_600,
        fields: { rmk: "hetrodyne detector set at 31,9 kHz" },
      },
    ],
    [
      "a social call at a x10 header rate",
      { species: "Pipistrellus nathusii", type: "social call", rate: 38_400 },
    ],
  ];
  for (const [name, c] of OTHERS) {
    test(`none for ${name}`, async ({ page }) => {
      await openCase(page, c);
      await showInfo(page);
      if (c.guano) {
        await expect(page.locator(TE_NOTE)).toContainText("Corrected for ×10 time expansion");
      }
      await expect(page.locator(".rate-note-action")).toHaveCount(0);
    });
  }
});

test.describe("x10 correction on a phone", () => {
  test.use({
    viewport: { width: 375, height: 812 },
    isMobile: true,
    hasTouch: true,
    userAgent:
      "Mozilla/5.0 (Linux; Android 14; Pixel 7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36",
  });

  test("the buttons fit the Info panel and are big enough to tap", async ({ page }) => {
    await openCase(page, CASE_T);
    await page.locator('button[title="Show info panel"]').first().tap();
    const treat = page.locator(TREAT);
    await expect(treat).toBeVisible();
    await treat.scrollIntoViewIfNeeded();
    const panel = (await page.locator(".right-sidebar").boundingBox())!;
    const box = (await treat.boundingBox())!;
    expect(box.height).toBeGreaterThanOrEqual(36);
    expect(box.x).toBeGreaterThanOrEqual(panel.x);
    expect(box.x + box.width).toBeLessThanOrEqual(panel.x + panel.width + 0.5);
    // The label stays on one line.
    const lineTops = await treat.evaluate((b) => {
      const r = document.createRange();
      r.selectNodeContents(b);
      return new Set(Array.from(r.getClientRects()).map((x) => Math.round(x.top))).size;
    });
    expect(lineTops).toBe(1);
    await expect(row(page, "Sample rate")).toHaveText("25.6 kHz");
    await settle(page);
    await shot(page, "phone-375-before.png");

    await treat.tap();
    await expect(row(page, "Sample rate")).toHaveText("256 kHz");
    const useFile = page.locator(USE_FILE);
    await expect(useFile).toBeVisible();
    expect((await useFile.boundingBox())!.height).toBeGreaterThanOrEqual(36);
    expect((await snap(page)).duration).toBeCloseTo(2, 9);
    await settle(page);
    await shot(page, "phone-375-apply-after.png");

    await useFile.tap();
    await expect(row(page, "Sample rate")).toHaveText("25.6 kHz");
    await expect(page.locator(TREAT)).toBeVisible();
    expect((await snap(page)).duration).toBeCloseTo(20, 9);
    await settle(page);
    await shot(page, "phone-375-undo-after.png");
    // No horizontal page scroll.
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
      375,
    );
  });
});
