import { test, expect, type Page, type Route } from "@playwright/test";
import * as fs from "node:fs";
import * as path from "node:path";
import { MockDrive, mockServer } from "./drive_mock";

/**
 * The x10 time-expansion correction in the Info panel (src/te_correction.rs).
 *
 * A file at a x10 header rate that is too low for its species' calls (case T
 * in src/species_check.rs) gets the species note "Probably time-expanded
 * x10" with a "Correct for x10 time expansion" button. Pressing it reads the
 * file at ten times its rate; the time-expansion note that replaces the
 * species note has a "Remove x10 correction" button that goes back.
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
  passBandLo: number;
  passBandHi: number;
  hfrOn: boolean;
  hfrBandLo: number;
  hfrBandHi: number;
  hfrNudge: boolean;
}

async function snap(page: Page): Promise<Snap> {
  return page.evaluate(() =>
    (window as unknown as { __oversample_test: () => Snap }).__oversample_test(),
  );
}

const SPECIES_NOTE = '.rate-note[data-note="species"]';
const TE_NOTE = '.rate-note[data-note="time-expansion"]';
const CORRECT = 'button.rate-note-action[data-action="correct-tenfold"]';
const REMOVE = 'button.rate-note-action[data-action="remove-correction"]';
/** On the note of a correction from the file's metadata that the user removed. */
const APPLY = 'button.rate-note-action[data-action="apply-correction"]';
/** The heading's and file rows' mark for a rate corrected for time expansion. */
const MARK = ".file-badge-corrected";

/** Picks PS (pitch shift) in the HEAR bar, which turns HFR on. */
async function choosePitchShift(page: Page) {
  await page.locator('button[title^="PS"]').first().click();
  await expect.poll(async () => (await snap(page)).hfrOn).toBe(true);
}

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

/** Presses "Export for XC", saves the Annota-JSON file and closes the form. */
async function exportXc(page: Page, name: string) {
  await page.locator(".export-section-body button", { hasText: "Export for XC" }).click();
  const [dl] = await Promise.all([
    page.waitForEvent("download"),
    page.locator("button.annota-btn", { hasText: "Save file" }).click(),
  ]);
  const file = test.info().outputPath(name);
  await dl.saveAs(file);
  await page.locator("button.annota-btn", { hasText: "Close" }).click();
  return { file, json: JSON.parse(fs.readFileSync(file, "utf8")) };
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

    // The button comes after the amber claim and the species evidence.
    const treat = page.locator(`${SPECIES_NOTE} ${CORRECT}`);
    await expect(treat).toHaveText("Correct for ×10 time expansion");
    await expect(treat).toHaveAttribute(
      "title",
      "Sets the sample rate to 256 kHz and saves this choice in the recording's .batm file. The audio file is not changed.",
    );
    const order = await page
      .locator(SPECIES_NOTE)
      .evaluate((n) => Array.from(n.children).map((c) => c.className));
    expect(order).toEqual([
      "rate-note-warning",
      "rate-note-muted",
      "hash-calc-btn rate-note-action",
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
      /^Corrected for ×10 time expansion\. You set this factor, and it is saved in the recording's \.batm file\. Sample rate in the file header: 25\.6 kHz\.Remove ×10 correction$/,
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
    await expect(page.locator(CORRECT)).toHaveCount(0);

    // The corrected file plays, and going back to the file's rate stops it.
    // Play starts at "Auto", which plays the selection: about 30 ms at the
    // corrected rate, often over before the next check. "All" plays 2 s.
    await page.locator('[title="Play start position"]').click();
    await page.locator(".layer-panel button", { hasText: "Play from start" }).click();
    await page.locator(".play-main-btn").click();
    await expect.poll(async () => (await snap(page)).isPlaying).toBe(true);

    const useFile = page.locator(`${TE_NOTE} ${REMOVE}`);
    await expect(useFile).toHaveText("Remove ×10 correction");
    expect(await useFile.getAttribute("title")).toBeNull();
    await useFile.click();
    expect((await snap(page)).isPlaying).toBe(false);

    await expect(row(page, "Sample rate")).toHaveText("25.6 kHz");
    await expect(row(page, "Duration")).toHaveText(durationBefore!);
    expect(durationAfter).not.toBe(durationBefore);
    await expect(page.locator(TE_NOTE)).toHaveCount(0);
    await expect(page.locator(`${SPECIES_NOTE} ${CORRECT}`)).toBeVisible();
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

    // The pressed button becomes "Remove ×10 correction" in place. A
    // double-click applies the correction once, and Enter held down on the
    // focused button acts once.
    await page.locator(CORRECT).dblclick();
    await expect(row(page, "Sample rate")).toHaveText("256 kHz");
    await page.waitForTimeout(300);
    await expect(row(page, "Sample rate")).toHaveText("256 kHz");
    await page.locator(REMOVE).focus();
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
    await page.locator(CORRECT).click();
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

    await page.locator(REMOVE).click();
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
    await page.locator(CORRECT).click();
    await expect(row(page, "Sample rate")).toHaveText("256 kHz");
    await page.locator(".tool-combo-btn").click();
    await page.locator(".layer-panel button", { hasText: "Selection" }).click();
    await select(page, 0.3, 0.5, 0.3, 0.6);
    await page.locator(".canvas-overflow-btn").click();
    await page.locator(".canvas-overflow-menu button", { hasText: "Annotate" }).click();
    await expect.poll(async () => (await snap(page)).regions.length).toBe(1);
    const made = (await snap(page)).regions[0];

    // The annotation is saved in the file's own seconds and Hz, with the
    // factor. Opened again, the file is corrected again and the annotation
    // is on the same sounds.
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
    await expect.poll(async () => (await snap(page)).sampleRate, { timeout: 15_000 }).toBe(256_000);
    const reopened = await snap(page);
    expectRange(reopened.regions[0], made);
    // The saved set is in the file's own units and names the factor.
    const yaml = await page.evaluate(async () => {
      const root = await navigator.storage.getDirectory();
      const dir = await root.getDirectoryHandle("oversample-annotations");
      for await (const [, handle] of (dir as any).entries()) {
        const text = await (await (handle as FileSystemFileHandle).getFile()).text();
        if (text.includes("Region")) return text;
      }
      return "";
    });
    expect(yaml).toMatch(/time_expansion:\s+set_by_user: 10(\.0)?\b/);
    expect(yaml).toMatch(/sample_rate: 25600\b/);
  });

  test("the correction is made again when the file is opened again", async ({ page }) => {
    await openCase(page, CASE_T);
    await showInfo(page);
    await page.locator(CORRECT).click();
    await expect(row(page, "Sample rate")).toHaveText("256 kHz");
    await expect(page.locator(TE_NOTE)).toContainText("saved in the recording's .batm file");
    await page.locator(".file-item-close").first().click();
    await expect(page.locator(".file-item")).toHaveCount(0);
    // Opened again from Today's bats: corrected again from the saved factor.
    await page.getByText("Test bat", { exact: true }).first().click();
    await expect(page.locator(".file-item")).toHaveCount(1, { timeout: 30_000 });
    await showInfo(page);
    await expect(row(page, "Sample rate")).toHaveText("256 kHz", { timeout: 15_000 });
    await expect(page.locator(TE_NOTE)).toContainText("You set this factor");
    // Removing it is saved too: the next time, the file opens at its own rate.
    await page.locator(REMOVE).click();
    await expect(row(page, "Sample rate")).toHaveText("25.6 kHz");
    await page.locator(".file-item-close").first().click();
    await expect(page.locator(".file-item")).toHaveCount(0);
    await page.getByText("Test bat", { exact: true }).first().click();
    await expect(page.locator(".file-item")).toHaveCount(1, { timeout: 30_000 });
    await showInfo(page);
    await expect(page.locator(`${SPECIES_NOTE} ${CORRECT}`)).toBeVisible();
    await expect(row(page, "Sample rate")).toHaveText("25.6 kHz");
    await expect(page.locator(TE_NOTE)).toHaveCount(0);
  });

  test("a factor typed in the Time expansion row is applied, and a bad one says why", async ({
    page,
  }) => {
    await openCase(page, CASE_T);
    await showInfo(page);
    const field = page.locator(".te-factor-input");
    await expect(field).toHaveValue("1");
    expect((await field.boundingBox())!.height).toBeGreaterThanOrEqual(24);
    await field.fill("20");
    await field.press("Enter");
    await expect(row(page, "Sample rate")).toHaveText("512 kHz");
    await expect(page.locator(".te-factor-input")).toHaveValue("20");
    await expect(page.locator(TE_NOTE)).toContainText("Corrected for ×20 time expansion.");
    await page.locator(".te-factor-input").fill("abc");
    await page.locator(".te-factor-input").press("Enter");
    await expect(page.locator(".te-factor-error")).toHaveText(
      "Factor must be a number from 0.01 to 1000.",
    );
    await expect(row(page, "Sample rate")).toHaveText("512 kHz");
    await page.locator(".te-factor-input").fill("1");
    await page.locator(".te-factor-input").press("Enter");
    await expect(row(page, "Sample rate")).toHaveText("25.6 kHz");
    await expect(page.locator(".te-factor-error")).toHaveCount(0);
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
    await page.locator(CORRECT).click();
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

  test("Annota-JSON imported while corrected lands on the file's seconds and Hz", async ({
    page,
  }) => {
    await openCase(page, CASE_T);
    await showInfo(page);
    await page.locator(CORRECT).click();
    await expect(row(page, "Sample rate")).toHaveText("256 kHz");
    const file = test.info().outputPath("annota.json");
    fs.writeFileSync(
      file,
      JSON.stringify({
        set_name: "Test set",
        annotations: [
          {
            xc_nr: "815138",
            scientific_name: "Eptesicus nilssonii",
            start_time: 10,
            end_time: 12,
            frequency_low: 3000,
            frequency_high: 5000,
          },
        ],
      }),
    );
    await openAnnotationFiles(page);
    await importAnnotations(page, file);
    await expect.poll(async () => (await snap(page)).regions.length).toBe(1);
    expectRange((await snap(page)).regions[0], {
      timeStart: 1,
      timeEnd: 1.2,
      freqLow: 30_000,
      freqHigh: 50_000,
    });
  });

  test("Export for XC while corrected uses the file's seconds and Hz", async ({ page }) => {
    await openCase(page, CASE_T);
    await showInfo(page);
    await annotate(page, 0.3, 0.5, 0.3, 0.6);
    const region = (await snap(page)).regions[0];
    await openAnnotationFiles(page);
    const before = (await exportXc(page, "xc-before.json")).json.annotations[0];

    await openTab(page, "Info");
    await page.locator(CORRECT).click();
    await expect(row(page, "Sample rate")).toHaveText("256 kHz");
    await openAnnotationFiles(page);
    const exported = await exportXc(page, "xc-corrected.json");
    const during = exported.json.annotations[0];
    // Times are rounded to 6 decimals and frequencies to whole Hz.
    expect(during.start_time).toBeCloseTo(before.start_time, 5);
    expect(during.end_time).toBeCloseTo(before.end_time, 5);
    expect(Math.abs(during.frequency_low - before.frequency_low)).toBeLessThanOrEqual(1);
    expect(Math.abs(during.frequency_high - before.frequency_high)).toBeLessThanOrEqual(1);
    expect(during.start_time).toBeCloseTo(region.timeStart, 5);

    // Imported back while corrected, it lands on the region it came from.
    await importAnnotations(page, exported.file);
    await expect.poll(async () => (await snap(page)).regions.length).toBe(2);
    const [, back] = (await snap(page)).regions;
    const corrected = scaled(region, 10);
    expect(back.timeStart).toBeCloseTo(corrected.timeStart, 5);
    expect(back.timeEnd).toBeCloseTo(corrected.timeEnd, 5);
    expect(Math.abs(back.freqLow! - corrected.freqLow!)).toBeLessThanOrEqual(10);
    expect(Math.abs(back.freqHigh! - corrected.freqHigh!)).toBeLessThanOrEqual(10);
  });

  test("the PASS band that follows the HFR band is not converted", async ({ page }) => {
    await openCase(page, CASE_T);
    await showInfo(page);
    const pass = (s: Snap) => [s.passBandLo, s.passBandHi];
    expect(pass(await snap(page))).toEqual([20_000, 60_000]);
    await page.locator(CORRECT).click();
    await expect(row(page, "Sample rate")).toHaveText("256 kHz");
    // The app's default band, as a file loaded at 256 kHz gets, inside the
    // corrected file's 128 kHz.
    expect(pass(await snap(page))).toEqual([20_000, 60_000]);
    // PASS is a switch button: the top row turns it on and the bottom row
    // shows the range in use.
    const passSwitch = page.locator(".switch-btn", {
      has: page.locator(".switch-btn-name", { hasText: /^PASS$/ }),
    });
    await passSwitch.locator(".switch-btn-top").click();
    const caption = passSwitch.locator(".switch-btn-value");
    await expect(caption).toContainText("20\u201360 kHz");
    await expect(caption).not.toContainText("200");
    await page.locator(REMOVE).click();
    await expect(row(page, "Sample rate")).toHaveText("25.6 kHz");
    expect(pass(await snap(page))).toEqual([20_000, 60_000]);
  });

  test('the "Playing 1:1" card closes when the rate changes', async ({ page }) => {
    await openCase(page, CASE_T);
    await showInfo(page);
    await page.locator(CORRECT).click();
    await expect(row(page, "Sample rate")).toHaveText("256 kHz");
    // The corrected file holds sound above hearing range, so playing it at
    // 1:1 shows the card.
    expect((await snap(page)).hfrOn).toBe(false);
    await page.locator(".play-main-btn").click();
    await expect(page.locator(".hfr-nudge")).toBeVisible();
    await page.locator(REMOVE).click();
    await expect(row(page, "Sample rate")).toHaveText("25.6 kHz");
    await expect(page.locator(".hfr-nudge")).toHaveCount(0);
    expect((await snap(page)).hfrNudge).toBe(false);
  });

  test("Remove turns HFR off when it was off before the correction", async ({ page }) => {
    await openCase(page, CASE_T);
    await showInfo(page);
    expect((await snap(page)).hfrOn).toBe(false);
    await page.locator(CORRECT).click();
    await expect(row(page, "Sample rate")).toHaveText("256 kHz");
    await choosePitchShift(page);
    await page.locator(REMOVE).click();
    await expect(row(page, "Sample rate")).toHaveText("25.6 kHz");
    await expect.poll(async () => (await snap(page)).hfrOn).toBe(false);
  });

  test("HFR that was on before the correction stays on after Remove", async ({ page }) => {
    await openCase(page, CASE_T);
    await showInfo(page);
    await choosePitchShift(page);
    const before = await snap(page);
    await page.locator(CORRECT).click();
    await expect(row(page, "Sample rate")).toHaveText("256 kHz");
    expect((await snap(page)).hfrOn).toBe(true);
    await page.locator(REMOVE).click();
    await expect(row(page, "Sample rate")).toHaveText("25.6 kHz");
    const after = await snap(page);
    expect(after.hfrOn).toBe(true);
    // The band is converted there and back.
    expect(after.hfrBandLo).toBeCloseTo(before.hfrBandLo, 6);
    expect(after.hfrBandHi).toBeCloseTo(before.hfrBandHi, 6);
  });
});

/** XC844459's species at its 38.4 kHz rate, with GUANO that says TE 10, so
 * the app corrects it to 384 kHz when it opens it. 10 seconds, so the
 * selection drawn in `annotate` is long enough at either rate. */
const GUANO_CASE: Case = {
  species: "Pipistrellus pipistrellus",
  type: "echolocation",
  rate: 38_400,
  seconds: 10,
  guano: "GUANO|Version: 1.0\nTE: 10\n",
};

const GUANO_CORRECTED =
  /^Corrected for ×10 time expansion\. Sample rate in the file header: 38\.4\u00a0kHz\.Remove ×10 correction$/;
const GUANO_REMOVED =
  "Correction for ×10 time expansion removed. This choice is saved in the recording's .batm file. " +
  "Time expansion in the file's GUANO metadata: ×10.";

test.describe("a correction from the file's GUANO", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("can be removed and applied again", async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await openCase(page, GUANO_CASE);
    await showInfo(page);
    await expect(row(page, "Sample rate")).toHaveText("384 kHz");
    await expect(page.locator(TE_NOTE)).toHaveText(GUANO_CORRECTED);
    // Only a factor the user set is described as theirs.
    await expect(page.locator(TE_NOTE)).not.toContainText("You set this factor");
    const remove = page.locator(`${TE_NOTE} ${REMOVE}`);
    await expect(remove).toHaveAttribute(
      "title",
      "Sets the sample rate to 38.4\u00a0kHz and saves this choice in the recording's .batm file. The audio file is not changed.",
    );
    await annotate(page, 0.3, 0.5, 0.3, 0.6);
    const loaded = await snap(page);
    const region = loaded.regions[0];
    await settle(page);
    await shot(page, "guano-1440-corrected.png");

    await remove.click();
    await expect(row(page, "Sample rate")).toHaveText("38.4 kHz");
    const removed = await snap(page);
    expect(removed.sampleRate).toBe(38_400);
    expect(removed.duration).toBeCloseTo(10, 9);
    expectRange(removed.regions[0], scaled(region, 0.1));
    expect(removed.zoom).toBe(loaded.zoom);
    await expect(page.locator(`${TE_NOTE} > div`)).toHaveText([GUANO_REMOVED]);
    const apply = page.locator(`${TE_NOTE} ${APPLY}`);
    await expect(apply).toHaveText("Correct for ×10 time expansion");
    await expect(apply).toHaveAttribute("title", "Sets the sample rate to 384\u00a0kHz.");
    // At 38.4 kHz the file is too low for its species' calls. The species
    // note gives only the species' calls: the time-expansion note above it
    // gives the factor in the GUANO, and its button is the only one.
    await expect(page.locator(SPECIES_NOTE)).toContainText("Pipistrellus pipistrellus:");
    await expect(page.locator(SPECIES_NOTE)).not.toContainText("Probably time-expanded");
    await expect(page.locator(".rate-note-action")).toHaveCount(1);
    await settle(page);
    await shot(page, "guano-1440-removed.png");

    await apply.click();
    await expect(row(page, "Sample rate")).toHaveText("384 kHz");
    await expect(page.locator(TE_NOTE)).toHaveText(GUANO_CORRECTED);
    await expect(page.locator(SPECIES_NOTE)).toHaveCount(0);
    const again = await snap(page);
    expect(again.sampleRate).toBe(384_000);
    expect(again.duration).toBeCloseTo(1, 9);
    expectRange(again.regions[0], region);
    expect(errors).toEqual([]);
  });

  test("Remove after applying it again turns HFR off when it was off", async ({ page }) => {
    await openCase(page, GUANO_CASE);
    await showInfo(page);
    await page.locator(REMOVE).click();
    await expect(row(page, "Sample rate")).toHaveText("38.4 kHz");
    await page.locator(APPLY).click();
    await expect(row(page, "Sample rate")).toHaveText("384 kHz");
    await choosePitchShift(page);
    await page.locator(REMOVE).click();
    await expect(row(page, "Sample rate")).toHaveText("38.4 kHz");
    await expect.poll(async () => (await snap(page)).hfrOn).toBe(false);
  });

  test("the first Remove leaves HFR on", async ({ page }) => {
    await openCase(page, GUANO_CASE);
    await showInfo(page);
    await choosePitchShift(page);
    await page.locator(REMOVE).click();
    await expect(row(page, "Sample rate")).toHaveText("38.4 kHz");
    expect((await snap(page)).hfrOn).toBe(true);
  });

  test("annotations saved while it is removed open on the same sounds", async ({ page }) => {
    await openCase(page, GUANO_CASE);
    await showInfo(page);
    await page.locator(REMOVE).click();
    await expect(row(page, "Sample rate")).toHaveText("38.4 kHz");
    await annotate(page, 0.3, 0.5, 0.3, 0.6);
    const made = (await snap(page)).regions[0];
    // Drawn on the file at 38.4 kHz (10 seconds), in its seconds and Hz.
    expect(made.timeEnd).toBeGreaterThan(1);
    expect(made.freqHigh!).toBeLessThan(19_200);

    // Saved in the units the file opens in: corrected for time expansion.
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
    // The removal is saved with the annotations, so the file is read at the
    // rate in its header again, with the annotation where it was drawn.
    await expect.poll(async () => (await snap(page)).sampleRate, { timeout: 15_000 }).toBe(38_400);
    const reopened = await snap(page);
    expectRange(reopened.regions[0], made);
    // "Correct for x10" takes it back to the units it was saved in.
    await showInfo(page);
    await page.locator(APPLY).click();
    await expect(row(page, "Sample rate")).toHaveText("384 kHz");
    expectRange((await snap(page)).regions[0], scaled(made, 10));
  });

  test(".batm exported while it is removed uses the units the file opens in", async ({
    page,
  }) => {
    await openCase(page, GUANO_CASE);
    await showInfo(page);
    await annotate(page, 0.3, 0.5, 0.3, 0.6);
    const region = (await snap(page)).regions[0];
    await openTab(page, "Info");
    await page.locator(REMOVE).click();
    await expect(row(page, "Sample rate")).toHaveText("38.4 kHz");
    await openAnnotationFiles(page);
    const exported = batmFields((await exportBatm(page, "removed.batm")).text);
    expect(exported.sampleRate).toBe(384_000);
    expect(exported.duration).toBeCloseTo(1, 9);
    expectRange(exported.region, region);
  });
});

/** Opens the Files sidebar, where the file rows are. */
async function showFiles(page: Page) {
  const tab = page.locator("button.toolbar-sidebar-tab-left");
  if (!(await tab.evaluate((e) => e.classList.contains("active")))) await tab.click();
  await expect(page.locator(".sidebar:not(.right-sidebar)")).not.toHaveClass(/collapsed/);
}

test.describe('the "Corrected ×10" badge', () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("marks a rate the user corrected, in the heading and the file row", async ({ page }) => {
    await openCase(page, CASE_T);
    await showInfo(page);
    await showFiles(page);
    await expect(page.locator(MARK)).toHaveCount(0);
    await page.locator(CORRECT).click();
    await expect(row(page, "Sample rate")).toHaveText("256 kHz");

    const heading = page.locator(`.toolbar-badge-row ${MARK}`);
    const fileRow = page.locator(`.file-item-info ${MARK}`);
    await expect(heading).toHaveText("Corrected ×10");
    // A file row has no room for the words: its sample rate is the box.
    await expect(fileRow).toHaveText("256kHz");
    expect(await fileRow.evaluate((e) => e.parentElement!.classList.contains("badge-sample-rate"))).toBe(
      true,
    );
    for (const mark of [heading, fileRow]) {
      // The tooltip is the time-expansion note.
      await expect(mark).toHaveAttribute(
        "title",
        "Corrected for ×10 time expansion. You set this factor, and it is saved in the recording's .batm file. " +
          "Sample rate in the file header: 25.6\u00a0kHz.",
      );
      // A control: a grey box, after the facts, which are unchanged.
      expect(await mark.evaluate((e) => e.tagName)).toBe("BUTTON");
      await expect(mark).toHaveClass(/badge-btn/);
      await expect(mark).not.toHaveClass(/badge-warn/);
    }
    await expect(page.locator(".toolbar-badge-row .badge-sample-rate")).toHaveText("256kHz");
    expect(
      await heading.evaluate((e) => e.previousElementSibling!.classList.contains("badge-duration")),
    ).toBe(true);

    // It opens the Info panel at the time-expansion note.
    await page.locator('button[title="Hide info panel"]').click();
    await expect(page.locator(".right-sidebar")).toHaveClass(/collapsed/);
    await heading.click();
    await expect(page.locator(".right-sidebar")).not.toHaveClass(/collapsed/);
    await expect(page.locator(TE_NOTE)).toBeInViewport();

    // Gone with the correction.
    await page.locator(REMOVE).click();
    await expect(row(page, "Sample rate")).toHaveText("25.6 kHz");
    await expect(page.locator(MARK)).toHaveCount(0);
  });

  test("marks a rate corrected from the GUANO until the correction is removed", async ({
    page,
  }) => {
    await openCase(page, GUANO_CASE);
    await showInfo(page);
    const heading = page.locator(`.toolbar-badge-row ${MARK}`);
    await expect(heading).toHaveText("Corrected ×10");
    await expect(heading).toHaveAttribute(
      "title",
      "Corrected for ×10 time expansion. Sample rate in the file header: 38.4\u00a0kHz.",
    );
    await page.locator(REMOVE).click();
    await expect(page.locator(MARK)).toHaveCount(0);
    await page.locator(APPLY).click();
    await expect(heading).toHaveText("Corrected ×10");
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
        // Only a factor the user set is described as theirs.
        await expect(page.locator(TE_NOTE)).not.toContainText("You set this factor");
        // Its only button removes the correction made from the GUANO.
        await expect(page.locator(".rate-note-action")).toHaveCount(1);
        await expect(page.locator(`${TE_NOTE} ${REMOVE}`)).toHaveCount(1);
      } else {
        await expect(page.locator(".rate-note-action")).toHaveCount(0);
      }
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
    const treat = page.locator(CORRECT);
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
    const useFile = page.locator(REMOVE);
    await expect(useFile).toBeVisible();
    expect((await useFile.boundingBox())!.height).toBeGreaterThanOrEqual(36);
    expect((await snap(page)).duration).toBeCloseTo(2, 9);
    await settle(page);
    await shot(page, "phone-375-apply-after.png");

    await useFile.tap();
    await expect(row(page, "Sample rate")).toHaveText("25.6 kHz");
    await expect(page.locator(CORRECT)).toBeVisible();
    expect((await snap(page)).duration).toBeCloseTo(20, 9);
    await settle(page);
    await shot(page, "phone-375-undo-after.png");
    // No horizontal page scroll.
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
      375,
    );
  });

  test('the "Corrected ×10" badge opens the note when tapped', async ({ page }) => {
    await openCase(page, GUANO_CASE);
    const mark = page.locator(`.toolbar-badge-row ${MARK}`);
    await expect(mark).toHaveText("Corrected ×10");
    // The heading keeps one line of badges.
    expect((await page.locator(".toolbar").boundingBox())!.height).toBe(49);
    await expect(page.locator(".right-sidebar")).toHaveClass(/collapsed/);
    await mark.tap();
    await expect(page.locator(".right-sidebar")).not.toHaveClass(/collapsed/);
    await expect(page.locator(TE_NOTE)).toBeInViewport();
    await settle(page);
    await shot(page, "phone-375-badge-note.png");
    const remove = page.locator(REMOVE);
    expect((await remove.boundingBox())!.height).toBeGreaterThanOrEqual(36);
    await remove.tap();
    await expect(row(page, "Sample rate")).toHaveText("38.4 kHz");
    const apply = page.locator(APPLY);
    expect((await apply.boundingBox())!.height).toBeGreaterThanOrEqual(36);
    await settle(page);
    await shot(page, "phone-375-guano-removed.png");
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
      375,
    );
  });
});

test.describe("x10 correction and Google Drive", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  const regionCount = (yaml: string) => (yaml.match(/time_start:/g) ?? []).length;

  /** The set without the annotation `id`, as another device would save it. */
  function withoutAnnotation(yaml: string, id: string): string {
    const lines = yaml.split("\n");
    const start = lines.indexOf(`- id: ${id}`);
    let end = start + 1;
    while (end < lines.length && lines[end].startsWith("  ")) end++;
    lines.splice(start, end - start);
    return lines.join("\n");
  }

  test("a corrected open file merged with a Drive copy changed elsewhere: only the real changes count", async ({
    page,
  }) => {
    const drive = new MockDrive();
    // Saved by another device: four regions in the file's own seconds and
    // Hz, with all their digits.
    const at = "2026-11-01T10:00:00.000Z";
    const region = (n: number) =>
      `- id: other-${n}\n  type: Region\n  time_start: ${0.1234567890123 + n * 2.718281828459}\n` +
      `  time_end: ${0.9876543210987 + n * 2.718281828459}\n  freq_low: ${30123.456789012 + n * 777.7}\n` +
      `  freq_high: ${41234.567890123 + n * 333.3}\n  created_at: ${at}\n  modified_at: ${at}\n`;
    const size = wav(CASE_T.rate, CASE_T.seconds ?? 1).length;
    const root = drive.add({
      name: "Oversample",
      mimeType: "application/vnd.google-apps.folder",
      appProperties: { osFolder: "root" },
    });
    drive.onKeyQuery = (kind, key) => {
      if (kind !== "batm" || [...drive.files.values()].some((f) => f.appProperties.osKey === key)) return;
      drive.add({
        name: "XC815138 demo.wav.batm",
        parents: [root.id],
        appProperties: { osKind: "batm", osKey: key },
        content:
          `version: 3\nid: other-set\napp_version: 0.5.200\nmodified_at: ${at}\n` +
          `file_identity:\n  filename: XC815138 demo.wav\n  file_size: ${size}\nannotations:\n` +
          [0, 1, 2, 3].map(region).join(""),
      });
    };
    await mockServer(page, drive);
    await openCase(page, CASE_T);
    await expect.poll(async () => (await snap(page)).regions.length, { timeout: 20_000 }).toBe(4);
    const batm = () => [...drive.files.values()].find((f) => f.appProperties.osKind === "batm");
    await page.waitForTimeout(1_000);

    // Opened again and corrected: the saved regions are converted from the
    // file's seconds and Hz to the corrected rate, and back when the set is
    // compared with Drive.
    await page.reload();
    await expect(page.locator(".toolbar-badge-row .badge-sample-rate")).toBeVisible({
      timeout: 60_000,
    });
    await expect.poll(async () => (await snap(page)).regions.length, { timeout: 15_000 }).toBe(4);
    await showInfo(page);
    await page.locator(CORRECT).click();
    await expect(row(page, "Sample rate")).toHaveText("256 kHz");
    const file = batm()!;
    const [kept, ...gone] = [...file.content.matchAll(/^- id: (\S+)$/gm)].map((m) => m[1]);
    const uploadsBefore = drive.uploads.length;

    // Another device deletes three regions; this browser adds one.
    file.revisions[file.headRevisionId] = file.content;
    file.content = gone.reduce(withoutAnnotation, file.content);
    file.headRevisionId = `${file.headRevisionId}~`;
    file.modifiedTime = new Date().toISOString();
    await annotate(page, 0.85, 0.95, 0.3, 0.6);

    // The deletions are applied: the converted times and frequencies of the
    // regions this browser did not touch do not count as changes made here.
    await expect
      .poll(() => regionCount(drive.uploads.at(-1)?.content ?? ""), { timeout: 30_000 })
      .toBe(2);
    expect(file.content).toContain(kept);
    for (const id of gone) expect(file.content).not.toContain(id);
    await expect.poll(async () => (await snap(page)).regions.length).toBe(2);
    expect(drive.uploads.length).toBeGreaterThan(uploadsBefore);
    await page.locator(".sidebar-tab-dropdown").click();
    await page.locator(".sidebar-tab-option", { hasText: "Annotations" }).dispatchEvent("mousedown");
    const notice = page.locator(".right-sidebar .merge-notice");
    await expect(notice).toContainText("From Google Drive: 3 annotations removed");
    await expect(notice).not.toContainText("Changed in both copies");
  });
});
