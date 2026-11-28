import { test, expect, type Page } from "@playwright/test";
import { execFileSync } from "child_process";
import fs from "fs";
import os from "os";
import path from "path";

/**
 * Export from a recording that is read from disk as it is needed (a WAV of
 * more than 512 MB once decoded), not held in memory.
 *
 * - The whole file is too long to hold in memory. As WAV it is written to the
 *   browser's storage a piece at a time and downloaded from there. As MP4 its
 *   audio is encoded a piece at a time.
 * - A selection minutes into the file exports with its sound. (Export used to
 *   read audio that was not loaded yet, and wrote silence.)
 * - An MP4 export draws the annotations chosen in "Annotations", and its
 *   sound has no ultrasound folded down into it.
 *
 * The recording is a 5 kHz tone and a 50 kHz tone (each 0.15) at 192 kHz,
 * 16-bit mono, 730 s (280 MB), written to a temporary folder.
 */
const RATE = 192_000;
const SECONDS = 730;
const NAME = "export-long.wav";

type Range = { timeStart: number; timeEnd: number };
type Snap = {
  currentFile: string | null;
  scrollOffset: number;
  zoom: number;
  sampleRate: number;
  duration: number;
  regions: Range[];
  selection: Range | null;
};
const snap = (page: Page): Promise<Snap> =>
  page.evaluate(() => (window as unknown as { __oversample_test: () => Snap }).__oversample_test());

let wavPath = "";

test.beforeAll(() => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "oversample-export-"));
  wavPath = path.join(dir, NAME);
  const second = Buffer.alloc(RATE * 2);
  for (let i = 0; i < RATE; i++) {
    const t = (2 * Math.PI * i) / RATE;
    second.writeInt16LE(Math.round(0.15 * (Math.sin(5000 * t) + Math.sin(50_000 * t)) * 32767), i * 2);
  }
  const dataBytes = RATE * 2 * SECONDS;
  const header = Buffer.alloc(44);
  header.write("RIFF", 0);
  header.writeUInt32LE(36 + dataBytes, 4);
  header.write("WAVEfmt ", 8);
  header.writeUInt32LE(16, 16);
  header.writeUInt16LE(1, 20);
  header.writeUInt16LE(1, 22);
  header.writeUInt32LE(RATE, 24);
  header.writeUInt32LE(RATE * 2, 28);
  header.writeUInt16LE(2, 32);
  header.writeUInt16LE(16, 34);
  header.write("data", 36);
  header.writeUInt32LE(dataBytes, 40);
  const fd = fs.openSync(wavPath, "w");
  fs.writeSync(fd, header);
  for (let s = 0; s < SECONDS; s++) fs.writeSync(fd, second);
  fs.closeSync(fd);
});

test.afterAll(() => {
  if (wavPath) fs.rmSync(path.dirname(wavPath), { recursive: true, force: true });
});

async function openRecording(page: Page) {
  await page.goto("/");
  await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
  await page.locator('input[type="file"][accept*=".wav"]').first().setInputFiles(wavPath);
  await expect.poll(async () => (await snap(page)).currentFile, { timeout: 60_000 }).toBe(NAME);
  await expect(page.locator(".chart-stage canvas").first()).toBeVisible({ timeout: 30_000 });
}

async function openExportPanel(page: Page, format: "WAV" | "MP4") {
  const showInfo = page.locator('button[title="Show info panel"]').first();
  if (await showInfo.count()) await showInfo.click();
  await page.locator(".sidebar-tab-dropdown").first().click();
  await page.locator(".sidebar-tab-option", { hasText: /^Export$/ }).first().click();
  await page.locator(".export-radio", { hasText: format }).locator("input").check();
}

const exportButton = (page: Page) =>
  page.locator(".export-panel button", { hasText: /^Export .*\(\./ }).first();

/** Scrolls the view to minutes into the file and draws a selection there. */
async function selectFarFromStart(page: Page, fullHeight = false): Promise<Range> {
  const canvas = page.locator(".main-view .chart-stage > canvas");
  const box = (await canvas.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  for (let i = 0; i < 400 && (await snap(page)).scrollOffset < 200; i++) {
    await page.mouse.wheel(4000, 0);
  }
  expect((await snap(page)).scrollOffset).toBeGreaterThan(120);
  await page.locator(".tool-combo-btn").click();
  await page.locator(".layer-panel button", { hasText: "Selection" }).click();
  // 0.3 to 0.6 of the height is about 38 to 67 kHz, and the export keeps only
  // that band. The full height keeps everything.
  const [top, bottom] = fullHeight ? [2, box.height - 2] : [box.height * 0.3, box.height * 0.6];
  await page.mouse.move(box.x + box.width * 0.3, box.y + top);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.6, box.y + bottom, { steps: 5 });
  await page.mouse.up();
  const sel = (await snap(page)).selection;
  expect(sel).not.toBeNull();
  expect(sel!.timeStart).toBeGreaterThan(120);
  return sel!;
}

/** RMS and peak of a WAV export's samples (16-bit or 32-bit float). */
function wavRms(buf: Buffer): { rms: number; frames: number; peak: number } {
  let pos = 12;
  let bits = 16;
  let isFloat = false;
  while (pos + 8 <= buf.length) {
    const id = buf.toString("latin1", pos, pos + 4);
    const size = buf.readUInt32LE(pos + 4);
    if (id === "fmt ") {
      isFloat = buf.readUInt16LE(pos + 8) === 3;
      bits = buf.readUInt16LE(pos + 22);
    } else if (id === "data") {
      const bytes = bits / 8;
      const frames = Math.floor(size / bytes);
      let sum = 0;
      let peak = 0;
      for (let i = 0; i < frames; i++) {
        const at = pos + 8 + i * bytes;
        const v = isFloat ? buf.readFloatLE(at) : buf.readInt16LE(at) / 32768;
        sum += v * v;
        peak = Math.max(peak, Math.abs(v));
      }
      return { rms: Math.sqrt(sum / Math.max(frames, 1)), frames, peak };
    }
    pos += 8 + size + (size % 2);
  }
  throw new Error("no data chunk");
}

async function download(page: Page, timeout = 240_000): Promise<Buffer> {
  const started = page.waitForEvent("download", { timeout });
  await exportButton(page).click();
  return fs.readFileSync(await (await started).path());
}

test.describe("export from a long recording", () => {
  test.setTimeout(300_000);

  test("the whole file exports as WAV, and a selection far from the start has its sound", async ({
    page,
  }) => {
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await openRecording(page);

    // 730 s at 192 kHz mono is over the 512 MB of samples an export holds in
    // memory. It is written to the browser's storage and downloaded from there.
    await openExportPanel(page, "WAV");
    await expect(page.locator(".export-too-long")).toHaveCount(0);
    await expect(exportButton(page)).toHaveText("Export whole file (.wav)");
    await expect(exportButton(page)).toBeEnabled();
    const whole = wavRms(await download(page));
    console.log(`whole file: ${whole.frames} frames, RMS ${whole.rms.toFixed(4)}`);
    expect(whole.frames).toBe(RATE * SECONDS);
    // Two 0.15 sines have an RMS of 0.15.
    expect(whole.rms).toBeGreaterThan(0.145);
    expect(whole.rms).toBeLessThan(0.155);

    // Normalized: processed twice, and its peak is -3 dBFS (0.708).
    const normalize = page.locator(".export-panel .export-radio", { hasText: "Normalize" }).locator("input");
    await normalize.check();
    const loud = wavRms(await download(page));
    console.log(`whole file, normalized: peak ${loud.peak.toFixed(4)}`);
    expect(loud.frames).toBe(RATE * SECONDS);
    expect(loud.peak).toBeGreaterThan(0.7);
    expect(loud.peak).toBeLessThan(0.715);
    await normalize.uncheck();

    const sel = await selectFarFromStart(page);
    await expect(exportButton(page)).toBeEnabled();
    for (const depth of ["16-bit", "32-bit float"]) {
      await page.locator(".export-wav-options .export-radio", { hasText: depth }).first().locator("input").check();
      const { rms, frames } = wavRms(await download(page));
      console.log(`${depth}: ${frames} frames, RMS ${rms.toFixed(4)}`);
      expect(frames).toBeGreaterThan((sel.timeEnd - sel.timeStart) * RATE * 0.99);
      // The selection's band keeps the 50 kHz tone: a 0.15 sine, RMS 0.106.
      expect(rms).toBeGreaterThan(0.09);
      expect(rms).toBeLessThan(0.12);
    }
    expect(errors).toEqual([]);
  });

  test("the whole file exports as MP4, with sound to the end", async ({ page }) => {
    test.setTimeout(1_200_000);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await openRecording(page);
    await openExportPanel(page, "MP4");
    await expect(exportButton(page)).toHaveText("Export whole file (.mp4)");
    await expect(exportButton(page)).toBeEnabled();
    const t0 = Date.now();
    const started = page.waitForEvent("download", { timeout: 1_100_000 });
    await exportButton(page).click();
    const mp4 = path.join(path.dirname(wavPath), "whole.mp4");
    await (await started).saveAs(mp4);
    console.log(`whole-file MP4: ${fs.statSync(mp4).size} bytes in ${((Date.now() - t0) / 1000).toFixed(0)} s`);
    const seconds = Number(
      execFileSync("ffprobe", ["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", mp4], {
        encoding: "utf8",
      }),
    );
    expect(Math.abs(seconds - SECONDS)).toBeLessThan(0.5);
    // The last 10 s still have the 5 kHz tone, and a frame near the end is
    // not black (its tiles were computed for the export).
    const tail = execFileSync(
      "sh",
      ["-c", `ffmpeg -hide_banner -sseof -10 -i "${mp4}" -af volumedetect -vn -f null - 2>&1 | grep max_volume`],
      { encoding: "utf8" },
    );
    const tailDb = Number(tail.match(/max_volume: (-?[\d.]+) dB/)![1]);
    console.log(`last 10 s: max volume ${tailDb} dB`);
    expect(tailDb).toBeGreaterThan(-20);
    execFileSync("ffmpeg", ["-y", "-loglevel", "error", "-sseof", "-5", "-i", mp4, "-frames:v", "1", "test-results/export-mp4-whole-end.png"]);
    expect(errors).toEqual([]);
  });

  test("MP4 export has sound and draws the chosen annotations", async ({ page }) => {
    await openRecording(page);
    await selectFarFromStart(page, true);
    await page.locator('.canvas-overflow-btn[title="Selection options"]').click();
    await page.locator(".canvas-overflow-item", { hasText: "Annotate Region" }).click();
    await expect(page.locator(".annotation-inline-label-editor")).toBeVisible();
    await page.keyboard.type("Feeding buzz");
    await page.keyboard.press("Enter");
    await expect.poll(async () => (await snap(page)).regions.length).toBe(1);

    // Give it the tag "call" in the Annotations tab.
    const showInfo = page.locator('button[title="Show info panel"]').first();
    if (await showInfo.count()) await showInfo.click();
    await page.locator(".sidebar-tab-dropdown").first().click();
    await page.locator(".sidebar-tab-option", { hasText: /^Annotations$/ }).first().click();
    await page.locator(".annotation-edit").first().click();
    await page.locator(".annotation-tags-input").fill("#call");
    await page.locator(".annotation-tags-input").press("Enter");

    await openExportPanel(page, "MP4");
    const annotations = page.locator(".export-annotations-select");
    await expect(annotations.locator("option")).toHaveText(["None", "All (1)", "Tagged #call (1)"]);
    await expect(annotations).toHaveValue("all");
    await page.screenshot({ path: "test-results/export-mp4-options.png" });

    const frames: Record<string, Buffer> = {};
    for (const [choice, view] of [
      ["none", "static"],
      ["all", "static"],
      ["tag:call", "static"],
      ["all", "scroll"],
    ]) {
      await annotations.selectOption(choice);
      await page.locator(".export-mp4-options select").first().selectOption(view);
      const key = `${choice.replace(":", "-")}-${view}`;
      const downloadStarted = page.waitForEvent("download", { timeout: 240_000 });
      await exportButton(page).click();
      const mp4 = path.join(path.dirname(wavPath), `${key}.mp4`);
      await (await downloadStarted).saveAs(mp4);
      const png = `test-results/export-mp4-${key}.png`;
      execFileSync("ffmpeg", ["-y", "-loglevel", "error", "-i", mp4, "-frames:v", "1", png]);
      frames[key] = fs.readFileSync(png);
      // The loudest sample of the audio track, in dB. Silence reads -91. The
      // 5 kHz tone (0.15) is -16.5; the 50 kHz tone is filtered out, where it
      // used to fold down to 2 kHz and add to the level.
      const report = execFileSync(
        "sh",
        ["-c", `ffmpeg -hide_banner -i "${mp4}" -af volumedetect -vn -f null - 2>&1 | grep max_volume`],
        { encoding: "utf8" },
      );
      const maxDb = Number(report.match(/max_volume: (-?[\d.]+) dB/)![1]);
      console.log(`${key}: max volume ${maxDb} dB`);
      expect(maxDb).toBeGreaterThan(-20);
      expect(maxDb).toBeLessThan(-14);
    }
    expect(frames["none-static"].equals(frames["all-static"])).toBe(false);
    expect(frames["tag-call-static"].equals(frames["all-static"])).toBe(true);

  });
});

/**
 * The desktop app's side of a long WAV export. The page runs as if inside
 * the desktop app: `__TAURI_INTERNALS__` is a stand-in that answers the save
 * commands (`src-tauri/src/cmd_save.rs`) the way the app does, checks that
 * each part starts where the last one ended, and keeps what it was sent.
 */
type SaveStub = {
  dialogAnswer: "save" | "cancel";
  dialogs: string[];
  parts: number;
  bytes: number;
  head: number[];
  closedSize: number | null;
  discarded: number;
  errors: string[];
};
const saveStub = (page: Page) =>
  page.evaluate(() => (window as unknown as { __saveStub: SaveStub }).__saveStub);

async function asDesktopApp(page: Page, dialogAnswer: "save" | "cancel") {
  await page.addInitScript((answer) => {
    const st = {
      dialogAnswer: answer,
      dialogs: [] as string[],
      parts: 0,
      bytes: 0,
      head: [] as number[],
      closedSize: null as number | null,
      discarded: 0,
      errors: [] as string[],
    };
    (window as unknown as { __saveStub: typeof st }).__saveStub = st;
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {
      transformCallback: () => 0,
      metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
      invoke: (cmd: string, args: unknown, options?: { headers?: Record<string, string> }) => {
        switch (cmd) {
          case "save_open_dialog": {
            const { filename } = args as { filename: string };
            st.dialogs.push(filename);
            return Promise.resolve(st.dialogAnswer === "save" ? { id: 7, path: `/home/me/${filename}` } : null);
          }
          case "save_write": {
            const body = args as Uint8Array;
            const offset = Number(options?.headers?.["x-save-offset"]);
            if (options?.headers?.["x-save-id"] !== "7") st.errors.push("wrong id");
            if (offset !== st.bytes) {
              st.errors.push(`expected offset ${st.bytes}, got ${offset}`);
              return Promise.reject(`expected offset ${st.bytes}, got ${offset}`);
            }
            if (body.length > 8 * 1024 * 1024) st.errors.push(`part of ${body.length} bytes`);
            if (st.head.length < 80) st.head.push(...Array.from(body.slice(0, 80 - st.head.length)));
            st.parts++;
            st.bytes += body.length;
            return Promise.resolve(null);
          }
          case "save_close": {
            const { size } = args as { size: number };
            st.closedSize = size;
            return size === st.bytes ? Promise.resolve(null) : Promise.reject("size mismatch");
          }
          case "save_discard":
            st.discarded++;
            return Promise.resolve(null);
          default:
            return Promise.resolve(null);
        }
      },
    };
  }, dialogAnswer);
}

test.describe("long WAV export in the desktop app", () => {
  test.setTimeout(300_000);

  test("asks where to save, then writes the file in parts", async ({ page }) => {
    await asDesktopApp(page, "save");
    await openRecording(page);
    await openExportPanel(page, "WAV");
    await expect(page.locator(".export-too-long")).toHaveCount(0);
    await expect(exportButton(page)).toBeEnabled();
    await exportButton(page).click();
    await expect.poll(async () => (await saveStub(page)).closedSize, { timeout: 240_000 }).not.toBeNull();
    const st = await saveStub(page);
    expect(st.errors).toEqual([]);
    expect(st.dialogs).toEqual(["export-long_export.wav"]);
    expect(st.discarded).toBe(0);
    expect(st.closedSize).toBe(st.bytes);
    // An 80-byte header, 730 s of 16-bit mono samples, then the GUANO chunk.
    const dataBytes = RATE * 2 * SECONDS;
    expect(st.bytes).toBeGreaterThan(80 + dataBytes);
    expect(st.bytes).toBeLessThan(80 + dataBytes + 4096);
    const head = Buffer.from(st.head);
    expect(head.toString("latin1", 0, 4)).toBe("RIFF");
    expect(head.readUInt32LE(4)).toBe(st.bytes - 8);
    expect(head.toString("latin1", 72, 76)).toBe("data");
    expect(head.readUInt32LE(76)).toBe(dataBytes);
    console.log(`desktop: ${st.bytes} bytes in ${st.parts} parts`);
    await expect(page.getByText("Exported: export-long_export.wav")).toBeVisible();
    await expect(exportButton(page)).toBeEnabled();
  });

  test("closing the Save As dialog writes nothing", async ({ page }) => {
    await asDesktopApp(page, "cancel");
    await openRecording(page);
    await openExportPanel(page, "WAV");
    await exportButton(page).click();
    await expect.poll(async () => (await saveStub(page)).dialogs.length).toBe(1);
    await expect(exportButton(page)).toBeEnabled();
    const st = await saveStub(page);
    expect(st.parts).toBe(0);
    expect(st.errors).toEqual([]);
    await expect(page.locator(".export-error")).toHaveCount(0);
  });
});
