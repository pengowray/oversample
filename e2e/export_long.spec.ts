import { test, expect, type Page } from "@playwright/test";
import { execFileSync } from "child_process";
import fs from "fs";
import os from "os";
import path from "path";

/**
 * Export from a recording that is read from disk as it is needed (a WAV of
 * more than 512 MB once decoded), not held in memory.
 *
 * - The whole file is too long to export: the Export button is disabled and
 *   the panel says why.
 * - A selection minutes into the file exports with its sound. (Export used to
 *   read audio that was not loaded yet, and wrote silence.)
 * - An MP4 export draws the annotations chosen in "Annotations".
 *
 * The recording is a 50 kHz tone (inside the selection that the tests draw) at 192 kHz, 16-bit mono, 730 s (280 MB),
 * written to a temporary folder.
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
    second.writeInt16LE(Math.round(0.3 * Math.sin((2 * Math.PI * 50_000 * i) / RATE) * 32767), i * 2);
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
async function selectFarFromStart(page: Page): Promise<Range> {
  const canvas = page.locator(".main-view .chart-stage > canvas");
  const box = (await canvas.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  for (let i = 0; i < 400 && (await snap(page)).scrollOffset < 200; i++) {
    await page.mouse.wheel(4000, 0);
  }
  expect((await snap(page)).scrollOffset).toBeGreaterThan(120);
  await page.locator(".tool-combo-btn").click();
  await page.locator(".layer-panel button", { hasText: "Selection" }).click();
  await page.mouse.move(box.x + box.width * 0.3, box.y + box.height * 0.3);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.6, box.y + box.height * 0.6, { steps: 5 });
  await page.mouse.up();
  const sel = (await snap(page)).selection;
  expect(sel).not.toBeNull();
  expect(sel!.timeStart).toBeGreaterThan(120);
  return sel!;
}

/** RMS of a WAV export's samples (16-bit or 32-bit float). */
function wavRms(buf: Buffer): { rms: number; frames: number } {
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
      for (let i = 0; i < frames; i++) {
        const at = pos + 8 + i * bytes;
        const v = isFloat ? buf.readFloatLE(at) : buf.readInt16LE(at) / 32768;
        sum += v * v;
      }
      return { rms: Math.sqrt(sum / Math.max(frames, 1)), frames };
    }
    pos += 8 + size + (size % 2);
  }
  throw new Error("no data chunk");
}

test.describe("export from a long recording", () => {
  test.setTimeout(300_000);

  test("the whole file is refused, and a selection far from the start has its sound", async ({
    page,
  }) => {
    await openRecording(page);
    await openExportPanel(page, "WAV");

    // 730 s at 192 kHz mono: over the 512 MB of samples one export holds.
    await expect(exportButton(page)).toHaveText("Export whole file (.wav)");
    await expect(exportButton(page)).toBeDisabled();
    const reason = page.locator(".export-too-long");
    await expect(reason).toContainText("Too long to export: 12m10s.");
    await expect(reason).toContainText("The longest export of this file is 11m39s.");
    await page.screenshot({ path: "test-results/export-too-long.png" });

    const sel = await selectFarFromStart(page);
    await expect(reason).toHaveCount(0);
    await expect(exportButton(page)).toBeEnabled();
    for (const depth of ["16-bit", "32-bit float"]) {
      await page.locator(".export-wav-options .export-radio", { hasText: depth }).first().locator("input").check();
      const downloadStarted = page.waitForEvent("download", { timeout: 120_000 });
      await exportButton(page).click();
      const download = await downloadStarted;
      const { rms, frames } = wavRms(fs.readFileSync(await download.path()));
      console.log(`${depth}: ${frames} frames, RMS ${rms.toFixed(4)}`);
      expect(frames).toBeGreaterThan((sel.timeEnd - sel.timeStart) * RATE * 0.99);
      // A 0.3 sine has an RMS of 0.212.
      expect(rms).toBeGreaterThan(0.18);
      expect(rms).toBeLessThan(0.24);
    }
  });

  test("MP4 export has sound and draws the chosen annotations", async ({ page }) => {
    await openRecording(page);
    await selectFarFromStart(page);
    await page.locator('.canvas-overflow-btn[title="Selection options"]').click();
    await page.locator(".canvas-overflow-item", { hasText: "Annotate Region" }).click();
    await expect(page.locator(".annotation-inline-label-editor")).toBeVisible();
    await page.keyboard.type("Feeding buzz");
    await page.keyboard.press("Enter");
    await expect.poll(async () => (await snap(page)).regions.length).toBe(1);

    await openExportPanel(page, "MP4");
    const annotations = page.locator(".export-annotations-select");
    await expect(annotations.locator("option")).toHaveText(["None", "All (1)"]);
    await page.screenshot({ path: "test-results/export-mp4-options.png" });

    const frames: Record<string, Buffer> = {};
    for (const choice of ["none", "all"]) {
      await annotations.selectOption(choice);
      const downloadStarted = page.waitForEvent("download", { timeout: 240_000 });
      await exportButton(page).click();
      const download = await downloadStarted;
      const mp4 = path.join(path.dirname(wavPath), `${choice}.mp4`);
      await download.saveAs(mp4);
      const png = `test-results/export-mp4-${choice}.png`;
      execFileSync("ffmpeg", ["-y", "-loglevel", "error", "-i", mp4, "-frames:v", "1", png]);
      frames[choice] = fs.readFileSync(png);
      // The loudest sample of the audio track, in dB: silence reads -91.
      const report = execFileSync(
        "sh",
        ["-c", `ffmpeg -hide_banner -i "${mp4}" -af volumedetect -vn -f null - 2>&1 | grep max_volume`],
        { encoding: "utf8" },
      );
      const maxDb = Number(report.match(/max_volume: (-?[\d.]+) dB/)![1]);
      console.log(`${choice}: max volume ${maxDb} dB`);
      expect(maxDb).toBeGreaterThan(-20);
    }
    expect(frames.none.equals(frames.all)).toBe(false);
  });
});
