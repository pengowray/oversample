import { test, expect, type Page } from "@playwright/test";
import fs from "fs";
import os from "os";
import path from "path";

/**
 * The Channel chip in timeline view. A multitrack recorder saves each channel
 * of a take as its own file (260401_0010_1.wav, 260401_0010_2.wav). A timeline
 * built from the channel 1 files can be switched to channel 2: every clip then
 * draws and plays the channel 2 file of its take. A take whose channel 2 file
 * is not loaded leaves its place on the timeline empty.
 */

const SR = 192_000;
const SECS = 2;
/** Channel 1 files hold a 20 kHz tone and channel 2 files a 60 kHz tone. */
const TONE = { 1: 20_000, 2: 60_000 } as const;
/** Take 0011 has no channel 2 file. */
const FILES = [
  { name: "260401_0010_1.wav", ch: 1, at: 0 },
  { name: "260401_0010_2.wav", ch: 2, at: 0 },
  { name: "260401_0011_1.wav", ch: 1, at: 10 },
  { name: "260401_0012_1.wav", ch: 1, at: 20 },
  { name: "260401_0012_2.wav", ch: 2, at: 20 },
] as const;
const TOTAL_SECS = 22;

type Snap = {
  isPlaying: boolean;
  timelineFiles: string[] | null;
  timelineMissing: string[] | null;
  timelineStreamFiles: string[] | null;
};

/** Mono 16-bit WAV: a steady tone over quiet noise. `seed` makes each file's bytes differ. */
function toneWav(toneHz: number, seed: number): Buffer {
  const n = SR * SECS;
  const data = Buffer.alloc(n * 2);
  const rand = () => {
    seed = (seed * 1664525 + 1013904223) >>> 0;
    return seed / 0xffffffff - 0.5;
  };
  for (let i = 0; i < n; i++) {
    const s = rand() * 0.01 + 0.3 * Math.sin((2 * Math.PI * toneHz * i) / SR);
    data.writeInt16LE(Math.round(s * 32767), i * 2);
  }
  const header = Buffer.alloc(44);
  header.write("RIFF", 0);
  header.writeUInt32LE(36 + data.length, 4);
  header.write("WAVEfmt ", 8);
  header.writeUInt32LE(16, 16);
  header.writeUInt16LE(1, 20);
  header.writeUInt16LE(1, 22);
  header.writeUInt32LE(SR, 24);
  header.writeUInt32LE(SR * 2, 28);
  header.writeUInt16LE(2, 32);
  header.writeUInt16LE(16, 34);
  header.write("data", 36);
  header.writeUInt32LE(data.length, 40);
  return Buffer.concat([header, data]);
}

/** Writes the files to disk, because the app reads each recording's time from the file's modified time. */
function writeFixtures(): string[] {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "oversample-tracks-"));
  const t0 = Date.UTC(2026, 3, 1, 20, 0, 0) / 1000;
  return FILES.map((f, i) => {
    const p = path.join(dir, f.name);
    fs.writeFileSync(p, toneWav(TONE[f.ch], 1000 + i));
    const end = t0 + f.at + SECS;
    fs.utimesSync(p, end, end);
    return p;
  });
}

function snapshot(page: Page): Promise<Snap> {
  return page.evaluate(() =>
    (window as unknown as { __oversample_test: () => Snap }).__oversample_test(),
  );
}

/**
 * Which tone the spectrogram draws in the middle of the clip that starts at
 * `atSecs`: 1 (20 kHz), 2 (60 kHz), or 0 for neither.
 */
function drawnChannel(page: Page, atSecs: number): Promise<number> {
  return page
    .locator(".spectrogram-container .chart-stage canvas")
    .first()
    .evaluate(
      (c: HTMLCanvasElement, a: { frac: number; nyquist: number }) => {
        const ctx = c.getContext("2d")!;
        const x = Math.round(c.width * a.frac);
        // Brightest pixel of a thin column around a frequency's row.
        const peak = (hz: number) => {
          const y = Math.round(c.height * (1 - hz / a.nyquist));
          const d = ctx.getImageData(x - 2, Math.max(0, y - 6), 5, 12).data;
          let max = 0;
          for (let i = 0; i < d.length; i += 4) max = Math.max(max, d[i] + d[i + 1] + d[i + 2]);
          return max;
        };
        const quiet = peak(40_000);
        const ch1 = peak(20_000);
        const ch2 = peak(60_000);
        if (ch1 > quiet + 120 && ch1 > ch2) return 1;
        if (ch2 > quiet + 120 && ch2 > ch1) return 2;
        return 0;
      },
      { frac: (atSecs + SECS / 2) / TOTAL_SECS, nyquist: SR / 2 },
    );
}

const drawn = async (page: Page) => [
  await drawnChannel(page, 0),
  await drawnChannel(page, 10),
  await drawnChannel(page, 20),
];

const chip = (page: Page) =>
  page.locator(".layer-btn", { has: page.locator(".layer-btn-category", { hasText: /^Ch$/ }) });

async function openTimelineOfChannel1(page: Page) {
  await page.goto("/");
  await page.locator('input[type="file"][accept*=".wav"]').first().setInputFiles(writeFixtures());
  await expect(page.locator(".file-item:not(.loading)")).toHaveCount(FILES.length, {
    timeout: 60_000,
  });
  await page.getByRole("button", { name: /^Project/ }).first().click();
  await page.getByRole("button", { name: "Create project" }).click();
  const row = (name: string) => page.locator(".project-file-item", { hasText: name });
  await expect(row("260401_0012_2.wav")).toBeVisible({ timeout: 30_000 });
  await row("260401_0010_1.wav").click();
  await row("260401_0011_1.wav").click({ modifiers: ["Control"] });
  await row("260401_0012_1.wav").click({ modifiers: ["Control"] });
  await page.getByRole("button", { name: "Create Timeline" }).click();
  await expect.poll(() => drawn(page), { timeout: 30_000 }).toEqual([1, 1, 1]);
}

test.describe("timeline channel switch", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("choosing channel 2 draws and plays the channel 2 files", async ({ page }) => {
    test.setTimeout(180_000);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await openTimelineOfChannel1(page);

    // The chip names the channel the timeline shows.
    await expect(chip(page).locator(".layer-btn-value")).toHaveText("1");

    await chip(page).click();
    const menu = page.locator(".layer-panel");
    await expect(menu.getByRole("button", { name: /^Channel 1/ })).toHaveClass(/sel/);
    await menu.getByRole("button", { name: /^Channel 2/ }).click();

    // Takes 0010 and 0012 draw their channel 2 file. Take 0011 has none, so
    // its place is empty.
    await expect.poll(() => drawn(page), { timeout: 30_000 }).toEqual([2, 0, 2]);
    await expect(chip(page).locator(".layer-btn-value")).toHaveText("2");
    const on2 = await snapshot(page);
    expect(on2.timelineFiles).toEqual(["260401_0010_2.wav", "260401_0012_2.wav"]);
    expect(on2.timelineMissing).toEqual(["260401_0011"]);

    // Playback reads the channel 2 files.
    await page.keyboard.press("Space");
    await expect.poll(async () => (await snapshot(page)).isPlaying).toBe(true);
    expect((await snapshot(page)).timelineStreamFiles).toEqual([
      "260401_0010_2.wav",
      "260401_0012_2.wav",
    ]);

    // Switching back while playing moves the audio to the channel 1 files.
    await chip(page).click();
    await menu.getByRole("button", { name: /^Channel 1/ }).click();
    await expect.poll(() => drawn(page), { timeout: 30_000 }).toEqual([1, 1, 1]);
    await expect
      .poll(async () => (await snapshot(page)).timelineStreamFiles)
      .toEqual(["260401_0010_1.wav", "260401_0011_1.wav", "260401_0012_1.wav"]);
    expect((await snapshot(page)).isPlaying).toBe(true);
    await page.keyboard.press("Space");

    expect(errors).toEqual([]);
  });

  test("opening a take's missing channel file fills its empty place", async ({ page }) => {
    test.setTimeout(180_000);
    await openTimelineOfChannel1(page);
    await chip(page).click();
    await page.locator(".layer-panel").getByRole("button", { name: /^Channel 2/ }).click();
    await expect.poll(() => drawn(page), { timeout: 30_000 }).toEqual([2, 0, 2]);

    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "oversample-tracks-"));
    const late = path.join(dir, "260401_0011_2.wav");
    fs.writeFileSync(late, toneWav(TONE[2], 2000));
    const end = Date.UTC(2026, 3, 1, 20, 0, 0) / 1000 + 10 + SECS;
    fs.utimesSync(late, end, end);
    await page.locator(".sidebar-header-label", { hasText: "Files" }).click();
    await page.locator('input[type="file"][accept*=".wav"]').first().setInputFiles(late);

    await expect.poll(() => drawn(page), { timeout: 30_000 }).toEqual([2, 2, 2]);
    const snap = await snapshot(page);
    expect(snap.timelineFiles).toEqual([
      "260401_0010_2.wav",
      "260401_0011_2.wav",
      "260401_0012_2.wav",
    ]);
    expect(snap.timelineMissing).toEqual([]);
  });
});
