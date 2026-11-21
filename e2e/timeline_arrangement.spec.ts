import { test, expect, type Page } from "@playwright/test";
import fs from "fs";
import os from "os";
import path from "path";

/**
 * Where the files of a timeline go. Files that were copied or downloaded
 * together have the same file date, and files from different nights are days
 * apart, so a new timeline of either puts the files one after another. The
 * Order chip changes that, and adds a pause between files. Files with
 * different sample rates play on one timeline.
 */

type Snap = {
  isPlaying: boolean;
  playheadTime: number;
  timelineFiles: string[] | null;
  timelineStarts: number[] | null;
  timelineDuration: number | null;
  timelineStreamFiles: string[] | null;
  timelineRows: number[] | null;
  timelineOnlyRow: number | null;
  selection: { timeStart: number; timeEnd: number; freqLow: number | null; freqHigh: number | null } | null;
};

type Clip = { name: string; sr: number; secs: number; hz: number; mtime?: number };

/** Mono 16-bit WAV: a steady tone over quiet noise. */
function toneWav(sr: number, secs: number, toneHz: number, seed: number): Buffer {
  const n = Math.round(sr * secs);
  const data = Buffer.alloc(n * 2);
  const rand = () => {
    seed = (seed * 1664525 + 1013904223) >>> 0;
    return seed / 0xffffffff - 0.5;
  };
  for (let i = 0; i < n; i++) {
    const s = rand() * 0.01 + 0.3 * Math.sin((2 * Math.PI * toneHz * i) / sr);
    data.writeInt16LE(Math.round(s * 32767), i * 2);
  }
  const header = Buffer.alloc(44);
  header.write("RIFF", 0);
  header.writeUInt32LE(36 + data.length, 4);
  header.write("WAVEfmt ", 8);
  header.writeUInt32LE(16, 16);
  header.writeUInt16LE(1, 20);
  header.writeUInt16LE(1, 22);
  header.writeUInt32LE(sr, 24);
  header.writeUInt32LE(sr * 2, 28);
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

/**
 * The frequency in Hz of the brightest row of the spectrogram at `atSecs` on
 * a timeline `totalSecs` long that fills the view, with `nyquist` at the top.
 * 0 when that column has no bright row (a pause, or nothing drawn yet).
 */
function drawnTone(page: Page, atSecs: number, totalSecs: number, nyquist: number): Promise<number> {
  return page
    .locator(".spectrogram-container .chart-stage canvas")
    .first()
    .evaluate(
      (c: HTMLCanvasElement, a: { frac: number; nyquist: number }) => {
        const ctx = c.getContext("2d")!;
        const x = Math.round(c.width * a.frac);
        const d = ctx.getImageData(x, 0, 1, c.height).data;
        const sums: number[] = [];
        for (let y = 0; y < c.height; y++) sums.push(d[y * 4] + d[y * 4 + 1] + d[y * 4 + 2]);
        const sorted = [...sums].sort((p, q) => p - q);
        const median = sorted[sorted.length >> 1];
        let best = 0;
        for (let y = 1; y < c.height; y++) if (sums[y] > sums[best]) best = y;
        if (sums[best] < median + 120) return 0;
        return Math.round(a.nyquist * (1 - best / c.height));
      },
      { frac: atSecs / totalSecs, nyquist },
    );
}

/**
 * As `drawnTone`, for row `row` of `rows` equal rows: the brightest row of
 * pixels inside that row's band, as a frequency on the row's own axis.
 */
function drawnToneInRow(
  page: Page,
  atSecs: number,
  totalSecs: number,
  nyquist: number,
  row: number,
  rows: number,
): Promise<number> {
  return page
    .locator(".spectrogram-container .chart-stage canvas")
    .first()
    .evaluate(
      (c: HTMLCanvasElement, a: { frac: number; nyquist: number; row: number; rows: number }) => {
        const ctx = c.getContext("2d")!;
        const x = Math.round(c.width * a.frac);
        const rowH = c.height / a.rows;
        const top = Math.round(a.row * rowH) + 2;
        const h = Math.floor(rowH) - 4;
        const d = ctx.getImageData(x, top, 1, h).data;
        const sums: number[] = [];
        for (let y = 0; y < h; y++) sums.push(d[y * 4] + d[y * 4 + 1] + d[y * 4 + 2]);
        const sorted = [...sums].sort((p, q) => p - q);
        const median = sorted[sorted.length >> 1];
        let best = 0;
        for (let y = 1; y < h; y++) if (sums[y] > sums[best]) best = y;
        if (sums[best] < median + 120) return 0;
        return Math.round(a.nyquist * (1 - (best + 2) / rowH));
      },
      { frac: atSecs / totalSecs, nyquist, row, rows },
    );
}

async function openTimeline(page: Page, clips: Clip[]): Promise<string[]> {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "oversample-arrange-"));
  const paths = clips.map((f, i) => {
    const p = path.join(dir, f.name);
    fs.writeFileSync(p, toneWav(f.sr, f.secs, f.hz, 100 + i));
    if (f.mtime !== undefined) fs.utimesSync(p, f.mtime, f.mtime);
    return p;
  });
  await page.addInitScript(() => localStorage.setItem("oversample_projects_enabled", "true"));
  await page.goto("/");
  await page.locator('input[type="file"][accept*=".wav"]').first().setInputFiles(paths);
  await expect(page.locator(".file-item:not(.loading)")).toHaveCount(clips.length, {
    timeout: 60_000,
  });
  await page.getByRole("button", { name: /^Project/ }).first().click();
  await page.getByRole("button", { name: "Create project" }).click();
  const row = (name: string) => page.locator(".project-file-item", { hasText: name });
  await expect(row(clips[clips.length - 1].name)).toBeVisible({ timeout: 30_000 });
  // Chosen in reverse, to show that the order comes from the names.
  const reversed = [...clips].reverse();
  await row(reversed[0].name).click();
  for (const f of reversed.slice(1)) await row(f.name).click({ modifiers: ["Control"] });
  await page.getByRole("button", { name: "Create Timeline" }).click();
  await expect.poll(async () => (await snapshot(page)).timelineFiles?.length).toBe(clips.length);
  return paths;
}

const chip = (page: Page) =>
  page.locator(".layer-btn", { has: page.locator(".layer-btn-category", { hasText: /^Order$/ }) });

async function choose(page: Page, option: string) {
  await chip(page).click();
  await page.locator(".layer-panel").getByRole("button", { name: option, exact: true }).click();
}

const near = (hz: number) => ({ lo: hz - 1500, hi: hz + 1500 });
async function expectTones(
  page: Page,
  total: number,
  nyquist: number,
  want: { at: number; hz: number }[],
) {
  await expect
    .poll(
      async () => {
        const got = [];
        for (const w of want) got.push(await drawnTone(page, w.at, total, nyquist));
        return got.every((g, i) =>
          want[i].hz === 0 ? g === 0 : g >= near(want[i].hz).lo && g <= near(want[i].hz).hi,
        )
          ? "ok"
          : JSON.stringify(got);
      },
      { timeout: 30_000 },
    )
    .toBe("ok");
}

/**
 * For each of `rows` equal rows of the frequency strip: whether it has light
 * pixels, which are the kHz labels and tick marks.
 */
function gutterLabelPixels(page: Page, rows: number): Promise<boolean[]> {
  return page
    .locator(".band-gutter canvas")
    .first()
    .evaluate((c: HTMLCanvasElement, n: number) => {
      const ctx = c.getContext("2d")!;
      const rowH = c.height / n;
      const out: boolean[] = [];
      for (let r = 0; r < n; r++) {
        const top = Math.round(r * rowH) + 2;
        const d = ctx.getImageData(0, top, c.width, Math.floor(rowH) - 4).data;
        let light = 0;
        for (let i = 0; i < d.length; i += 4) if (d[i] + d[i + 1] + d[i + 2] > 380) light++;
        out.push(light > 20);
      }
      return out;
    }, rows);
}

const NAMES = ["alpha.wav", "bravo.wav", "charlie.wav"];
const card = (page: Page, name: string) => page.locator(".file-item", { hasText: name });
/** The test files named in the rows matching `selector`, top to bottom. */
function cardNames(page: Page, selector: string): Promise<(string | undefined)[]> {
  return page
    .locator(selector)
    .evaluateAll(
      (els, names) => els.map((e) => names.find((n) => (e.textContent ?? "").includes(n))),
      NAMES,
    );
}

const SAME_DATE: Clip[] = [
  { name: "alpha.wav", sr: 192_000, secs: 2, hz: 20_000 },
  { name: "bravo.wav", sr: 192_000, secs: 3, hz: 40_000 },
  { name: "charlie.wav", sr: 192_000, secs: 1.5, hz: 60_000 },
];

test.describe("timeline arrangement", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("files with the same date play one after another", async ({ page }) => {
    test.setTimeout(240_000);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await openTimeline(page, SAME_DATE);

    const snap = await snapshot(page);
    expect(snap.timelineFiles).toEqual(["alpha.wav", "bravo.wav", "charlie.wav"]);
    expect(snap.timelineStarts).toEqual([0, 2, 5]);
    expect(snap.timelineDuration).toBeCloseTo(6.5, 3);
    await expect(chip(page).locator(".layer-btn-value")).toHaveText("One after another");
    // The frequency strip has its kHz labels on a timeline.
    await expect.poll(() => gutterLabelPixels(page, 1), { timeout: 15_000 }).toEqual([true]);
    // Each file is drawn in its own place.
    await expectTones(page, 6.5, 96_000, [
      { at: 1, hz: 20_000 },
      { at: 3.5, hz: 40_000 },
      { at: 5.75, hz: 60_000 },
    ]);

    // Playback runs through all three files to the end of the timeline.
    await page.keyboard.press("Space");
    await expect.poll(async () => (await snapshot(page)).isPlaying).toBe(true);
    expect((await snapshot(page)).timelineStreamFiles).toEqual([
      "alpha.wav",
      "bravo.wav",
      "charlie.wav",
    ]);
    await expect
      .poll(async () => (await snapshot(page)).playheadTime, { timeout: 20_000 })
      .toBeGreaterThan(5.5);
    await expect.poll(async () => (await snapshot(page)).isPlaying, { timeout: 20_000 }).toBe(false);

    // A 1 s pause between the files.
    await choose(page, "One after another, 1 s pause");
    await expect.poll(async () => (await snapshot(page)).timelineStarts).toEqual([0, 3, 7]);
    expect((await snapshot(page)).timelineDuration).toBeCloseTo(8.5, 3);
    await expect(chip(page).locator(".layer-btn-value")).toHaveText("One after another, 1\u00a0s pause");
    await expectTones(page, 8.5, 96_000, [
      { at: 1, hz: 20_000 },
      { at: 2.5, hz: 0 },
      { at: 4.5, hz: 40_000 },
      { at: 6.5, hz: 0 },
      { at: 7.75, hz: 60_000 },
    ]);

    // The project keeps the arrangement: close the timeline and open it again.
    await page.locator(".sidebar-header-label", { hasText: "Files" }).click();
    await page.locator(".timeline-exit-btn").click();
    await expect.poll(async () => (await snapshot(page)).timelineFiles ?? null).toBeNull();
    await page.getByRole("button", { name: /^Project/ }).first().click();
    await page.locator(".project-timeline-item").click();
    await expect.poll(async () => (await snapshot(page)).timelineStarts).toEqual([0, 3, 7]);
    await expect(chip(page).locator(".layer-btn-value")).toHaveText("One after another, 1\u00a0s pause");

    // Recorded times are still on offer. These files have the same date, so
    // they all start within a few seconds of each other.
    await choose(page, "At their recorded times");
    await expect
      .poll(async () => (await snapshot(page)).timelineDuration ?? 99)
      .toBeLessThan(6);

    expect(errors).toEqual([]);
  });

  test("a saved project opens its timeline with the same arrangement after a reload", async ({
    page,
  }) => {
    test.setTimeout(240_000);
    const paths = await openTimeline(page, SAME_DATE);
    await choose(page, "One after another, 2\u00a0s pause");
    await expect.poll(async () => (await snapshot(page)).timelineStarts).toEqual([0, 4, 9]);
    // Save the project with the timeline in it. The project panel is open.
    const save = page.getByRole("button", { name: "Save", exact: true });
    if (await save.isEnabled()) await save.click();
    await expect(save).toBeDisabled({ timeout: 20_000 });
    await expect(page.locator(".project-unsaved")).toHaveCount(0);
    await page.waitForTimeout(1500);

    await page.reload();
    // The project is loaded before its files are opened.
    await page.getByRole("button", { name: /^Project/ }).first().click();
    await page.getByRole("button", { name: "Load project" }).click();
    await page.locator(".project-list-item").first().click();
    await expect(page.locator(".project-file-item.missing")).toHaveCount(3);
    await page.locator(".sidebar-header-label", { hasText: "Files" }).click();
    await page.locator('input[type="file"][accept*=".wav"]').first().setInputFiles(paths);
    await expect(page.locator(".file-item:not(.loading)")).toHaveCount(3, { timeout: 60_000 });
    await page.getByRole("button", { name: /^Project/ }).first().click();
    await expect(page.locator(".project-file-item.missing")).toHaveCount(0, { timeout: 30_000 });

    await page.locator(".project-timeline-item").click();
    await expect
      .poll(async () => (await snapshot(page)).timelineFiles)
      .toEqual(["alpha.wav", "bravo.wav", "charlie.wav"]);
    expect((await snapshot(page)).timelineStarts).toEqual([0, 4, 9]);
    await expect(chip(page).locator(".layer-btn-value")).toHaveText("One after another, 2\u00a0s pause");
  });

  test("the Files tab makes a timeline without a project, and its files can be reordered", async ({
    page,
  }) => {
    test.setTimeout(240_000);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "oversample-arrange-"));
    const paths = SAME_DATE.map((f, i) => {
      const p = path.join(dir, f.name);
      fs.writeFileSync(p, toneWav(f.sr, f.secs, f.hz, 100 + i));
      return p;
    });
    await page.goto("/");
    await page.locator('input[type="file"][accept*=".wav"]').first().setInputFiles(paths);
    await expect(page.locator(".file-item:not(.loading)")).toHaveCount(3, { timeout: 60_000 });
    for (const f of SAME_DATE) {
      await page.locator(".file-item", { hasText: f.name }).click({ modifiers: ["Control"] });
    }
    await page.getByRole("button", { name: "Create Timeline" }).click();
    await expect
      .poll(async () => (await snapshot(page)).timelineFiles)
      .toEqual(["alpha.wav", "bravo.wav", "charlie.wav"]);
    expect((await snapshot(page)).timelineStarts).toEqual([0, 2, 5]);

    // The timeline's files are listed first, in play order, and each card
    // has buttons that move it. Move alpha one place later.
    await expect.poll(() => cardNames(page, ".file-item")).toEqual(NAMES);
    await expect(card(page, "alpha.wav").getByRole("button", { name: "Play earlier" })).toBeDisabled();
    await expect(card(page, "charlie.wav").getByRole("button", { name: "Play later" })).toBeDisabled();
    await card(page, "alpha.wav").getByRole("button", { name: "Play later" }).click();
    await expect
      .poll(async () => (await snapshot(page)).timelineFiles)
      .toEqual(["bravo.wav", "alpha.wav", "charlie.wav"]);
    expect((await snapshot(page)).timelineStarts).toEqual([0, 3, 5]);
    await expect
      .poll(() => cardNames(page, ".file-item"))
      .toEqual(["bravo.wav", "alpha.wav", "charlie.wav"]);
    await expectTones(page, 6.5, 96_000, [
      { at: 1.5, hz: 40_000 },
      { at: 4, hz: 20_000 },
      { at: 5.75, hz: 60_000 },
    ]);
    // Playback follows the new order.
    await page.keyboard.press("Space");
    await expect.poll(async () => (await snapshot(page)).isPlaying).toBe(true);
    expect((await snapshot(page)).timelineStreamFiles).toEqual([
      "bravo.wav",
      "alpha.wav",
      "charlie.wav",
    ]);
    await page.keyboard.press("Space");

    // Placed at their recorded times, the files have no order to change.
    const moveButtons = page.locator(".file-item .timeline-order-btn");
    await choose(page, "At their recorded times");
    await expect(moveButtons).toHaveCount(0);
    await choose(page, "One after another, 0.5\u00a0s pause");
    await expect(moveButtons).toHaveCount(6);

    // Screenshots and the UI audit, at desktop and phone width.
    const out = process.env.ARRANGE_OUT;
    if (out) {
      const audit = fs.readFileSync(process.env.UI_AUDIT!, "utf8");
      await chip(page).click();
      await page.screenshot({ path: `${out}/order-1440.png` });
      fs.writeFileSync(`${out}/audit-1440.json`, JSON.stringify(await page.evaluate(audit), null, 1));
      await page.keyboard.press("Escape");
      await page.setViewportSize({ width: 375, height: 812 });
      await page.waitForTimeout(1500);
      await page.screenshot({ path: `${out}/order-375.png` });
      fs.writeFileSync(`${out}/audit-375.json`, JSON.stringify(await page.evaluate(audit), null, 1));
    }
    expect(errors).toEqual([]);
  });

  test("files that start together each get a row, and one row can play alone", async ({ page }) => {
    test.setTimeout(240_000);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await openTimeline(page, SAME_DATE);
    await choose(page, "All start together");
    await expect.poll(async () => (await snapshot(page)).timelineStarts).toEqual([0, 0, 0]);
    const snap = await snapshot(page);
    expect(snap.timelineRows).toEqual([0, 1, 2]);
    expect(snap.timelineDuration).toBeCloseTo(3, 3);
    await expect(chip(page).locator(".layer-btn-value")).toHaveText("All start together");

    // Each row draws its own file, on its own frequency axis. Charlie is
    // 1.5 s long, so its row is empty at 2.5 s.
    const rowTones = async () => [
      await drawnToneInRow(page, 1, 3, 96_000, 0, 3),
      await drawnToneInRow(page, 1, 3, 96_000, 1, 3),
      await drawnToneInRow(page, 1, 3, 96_000, 2, 3),
      await drawnToneInRow(page, 2.5, 3, 96_000, 2, 3),
    ];
    await expect
      .poll(
        async () => {
          const got = await rowTones();
          const want = [20_000, 40_000, 60_000, 0];
          return got.every((g, i) => Math.abs(g - want[i]) <= 2500) ? "ok" : JSON.stringify(got);
        },
        { timeout: 30_000 },
      )
      .toBe("ok");
    await expect(page.locator(".timeline-row-name")).toHaveText([
      "alpha.wav",
      "bravo.wav",
      "charlie.wav",
    ]);

    // The frequency strip beside the spectrogram has kHz labels in every row.
    await expect.poll(() => gutterLabelPixels(page, 3), { timeout: 15_000 }).toEqual([true, true, true]);

    // A selection dragged inside the middle row gets that row's frequencies:
    // from 3/4 of the way up the row (72 kHz) to 1/4 of the way up (24 kHz).
    await page.locator(".tool-combo-btn").click();
    await page.locator(".layer-panel button", { hasText: "Selection" }).click();
    const canvas = page.locator(".spectrogram-container .chart-stage canvas").first();
    const box = (await canvas.boundingBox())!;
    const rowH = box.height / 3;
    await page.mouse.move(box.x + box.width * 0.4, box.y + rowH * 1.25);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width * 0.6, box.y + rowH * 1.75, { steps: 6 });
    await page.mouse.up();
    const sel = (await snapshot(page)).selection!;
    expect(sel).not.toBeNull();
    expect(sel.freqHigh!).toBeGreaterThan(68_000);
    expect(sel.freqHigh!).toBeLessThan(76_000);
    expect(sel.freqLow!).toBeGreaterThan(20_000);
    expect(sel.freqLow!).toBeLessThan(28_000);
    expect(sel.timeStart).toBeGreaterThan(0.9);
    expect(sel.timeEnd).toBeLessThan(2.1);
    await page.keyboard.press("Escape");

    // All rows play together; Solo on the middle row plays only bravo.
    const out = process.env.ARRANGE_OUT;
    if (out) await page.screenshot({ path: `${out}/rows-3.png` });
    const solo = page.locator(".timeline-row-label").nth(1).getByRole("button", { name: "Solo" });
    await solo.click();
    await expect(solo).toHaveAttribute("aria-pressed", "true");
    expect((await snapshot(page)).timelineOnlyRow).toBe(1);
    await page.keyboard.press("Space");
    await expect.poll(async () => (await snapshot(page)).isPlaying).toBe(true);
    expect((await snapshot(page)).timelineStreamFiles).toEqual(["bravo.wav"]);
    // Solo off while playing: all three files again.
    await solo.click();
    await expect
      .poll(async () => (await snapshot(page)).timelineStreamFiles)
      .toEqual(["alpha.wav", "bravo.wav", "charlie.wav"]);
    await page.keyboard.press("Space");

    // Wave view has the same rows, with their names and Solo buttons.
    await page.getByRole("button", { name: "Wave", exact: true }).click();
    await expect(page.locator(".timeline-row-label:visible")).toHaveCount(3);
    await page.waitForTimeout(1500);
    if (out) await page.screenshot({ path: `${out}/rows-3-wave.png` });
    await page.getByRole("button", { name: "Spec", exact: true }).click();

    // The Project tab lists the rows from the top. Move alpha down one row.
    await expect.poll(() => cardNames(page, ".project-file-item")).toEqual(NAMES);
    const projRow = (name: string) => page.locator(".project-file-item", { hasText: name });
    await expect(projRow("alpha.wav").getByRole("button", { name: "Move up" })).toBeDisabled();
    await projRow("alpha.wav").getByRole("button", { name: "Move down" }).click();
    const moved = ["bravo.wav", "alpha.wav", "charlie.wav"];
    await expect.poll(async () => (await snapshot(page)).timelineFiles).toEqual(moved);
    expect((await snapshot(page)).timelineRows).toEqual([0, 1, 2]);
    await expect(page.locator(".timeline-row-name:visible")).toHaveText(moved);
    await expect.poll(() => cardNames(page, ".project-file-item")).toEqual(moved);
    if (out) await page.screenshot({ path: `${out}/rows-3-project-tab.png` });
    // The Files tab shows the same order, with the same buttons.
    await page.locator(".sidebar-header-label", { hasText: "Files" }).click();
    await expect.poll(() => cardNames(page, ".file-item")).toEqual(moved);
    await expect(card(page, "bravo.wav").getByRole("button", { name: "Move up" })).toBeDisabled();
    if (out) {
      await page.screenshot({ path: `${out}/rows-3-files-tab.png` });
      await page.setViewportSize({ width: 375, height: 812 });
      await page.waitForTimeout(2000);
      await page.screenshot({ path: `${out}/rows-3-phone.png` });
      const audit = fs.readFileSync(process.env.UI_AUDIT!, "utf8");
      const report = (await page.evaluate(audit)) as { findings: { where: string }[] };
      fs.writeFileSync(
        `${out}/audit-rows-phone.json`,
        JSON.stringify(report.findings.filter((f) => f.where.includes("timeline")), null, 1),
      );
    }
    expect(errors).toEqual([]);
  });

  test("eight files that start together fit the view", async ({ page }) => {
    test.setTimeout(240_000);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    const eight: Clip[] = Array.from({ length: 8 }, (_, i) => ({
      name: `260401_0010_${i + 1}.wav`,
      sr: 192_000,
      secs: 2,
      hz: 10_000 + i * 10_000,
    }));
    await openTimeline(page, eight);
    await choose(page, "All start together");
    await expect
      .poll(async () => (await snapshot(page)).timelineRows)
      .toEqual([0, 1, 2, 3, 4, 5, 6, 7]);
    await expect(page.locator(".timeline-row-label")).toHaveCount(8);
    await page.waitForTimeout(3000);
    const out = process.env.ARRANGE_OUT;
    if (out) {
      await page.screenshot({ path: `${out}/rows-8.png` });
      const audit = fs.readFileSync(process.env.UI_AUDIT!, "utf8");
      const report = (await page.evaluate(audit)) as { findings: { where: string }[] };
      fs.writeFileSync(
        `${out}/audit-rows-8.json`,
        JSON.stringify(report.findings.filter((f) => f.where.includes("timeline")), null, 1),
      );
    }
    expect(errors).toEqual([]);
  });

  test("files recorded days apart play one after another", async ({ page }) => {
    test.setTimeout(180_000);
    const t0 = Date.UTC(2026, 3, 1, 20, 0, 0) / 1000;
    await openTimeline(
      page,
      SAME_DATE.map((f, i) => ({ ...f, mtime: t0 + i * 86_400 })),
    );
    const snap = await snapshot(page);
    expect(snap.timelineStarts).toEqual([0, 2, 5]);
    expect(snap.timelineDuration).toBeCloseTo(6.5, 3);
  });

  test("files with different sample rates play on one timeline", async ({ page }) => {
    test.setTimeout(240_000);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await openTimeline(page, [
      { name: "alpha.wav", sr: 192_000, secs: 2, hz: 20_000 },
      { name: "bravo.wav", sr: 384_000, secs: 3, hz: 40_000 },
      { name: "charlie.wav", sr: 44_100, secs: 1.5, hz: 5_000 },
    ]);
    expect((await snapshot(page)).timelineStarts).toEqual([0, 2, 5]);
    // The view reaches the highest file's top frequency (192 kHz), and each
    // file's tone is drawn at its own frequency.
    await expectTones(page, 6.5, 192_000, [
      { at: 1, hz: 20_000 },
      { at: 3.5, hz: 40_000 },
      { at: 5.75, hz: 5_000 },
    ]);

    await page.keyboard.press("Space");
    await expect.poll(async () => (await snapshot(page)).isPlaying).toBe(true);
    await expect
      .poll(async () => (await snapshot(page)).playheadTime, { timeout: 20_000 })
      .toBeGreaterThan(5.5);
    await page.screenshot({ path: process.env.ARRANGE_SHOT ?? "test-results/arrange-mixed.png" });
    expect(errors).toEqual([]);
  });
});
