import { test, expect, type Page, type Route } from "@playwright/test";
import * as fs from "node:fs";
import * as path from "node:path";

/**
 * Time-expansion corrections the file was opened with, other than GUANO in
 * a file loaded in memory (te_correction.spec.ts has those): a factor from
 * Find recordings, a streamed WAV file, and the "Corrected ×10" badge in a
 * narrow Files sidebar.
 */

const TE_NOTE = '.rate-note[data-note="time-expansion"]';
const REMOVE = 'button.rate-note-action[data-action="remove-correction"]';
const APPLY = 'button.rate-note-action[data-action="apply-correction"]';
const MARK = ".file-badge-corrected";
/** Set TE_SHOT_DIR to save screenshots there. */
const SHOT_DIR = process.env.TE_SHOT_DIR;

/** A WAV file: mono 8-bit, 16-bit or 32-bit float, with a GUANO chunk after
 * the data when given. The samples are a quiet 4 kHz tone, in the frame
 * ranges `tones` when given, else throughout, and silence elsewhere. */
function wav(opts: {
  rate: number;
  frames: number;
  format: "u8" | "16" | "float";
  guano?: string;
  tones?: [number, number][];
}): Buffer {
  const { rate, frames, format } = opts;
  const bps = format === "u8" ? 1 : format === "16" ? 2 : 4;
  const data = Buffer.alloc(frames * bps, format === "u8" ? 0x80 : 0);
  for (const [start, end] of opts.tones ?? [[0, frames]]) {
    for (let i = start; i < Math.min(end, frames); i++) {
      const s = Math.sin((2 * Math.PI * 4000 * i) / rate) * 0.25;
      if (format === "float") data.writeFloatLE(s, i * 4);
      else if (format === "16") data.writeInt16LE(Math.round(s * 32767), i * 2);
      else data.writeUInt8(128 + Math.round(s * 127), i);
    }
  }
  const chunks: Buffer[] = [data];
  if (opts.guano) {
    let text = Buffer.from(opts.guano, "utf8");
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
  h.writeUInt16LE(format === "float" ? 3 : 1, 20);
  h.writeUInt16LE(1, 22);
  h.writeUInt32LE(rate, 24);
  h.writeUInt32LE(rate * bps, 28);
  h.writeUInt16LE(bps, 32);
  h.writeUInt16LE(bps * 8, 34);
  h.write("data", 36);
  h.writeUInt32LE(data.length, 40);
  return Buffer.concat([h, body]);
}

const GUANO_TE10 = "GUANO|Version: 1.0\nTE: 10\n";

/** The value of the Info panel's File row labelled `label`. */
function row(page: Page, label: string) {
  return page
    .locator(".metadata-row-inline", {
      has: page.locator(".setting-label", { hasText: new RegExp(`^${label}$`) }),
    })
    .locator(".metadata-value");
}

async function showInfo(page: Page) {
  const btn = page.locator('button[title="Show info panel"]');
  if (await btn.count()) await btn.first().click();
  await expect(page.locator(".right-sidebar")).not.toHaveClass(/collapsed/);
  await expect(row(page, "Sample rate")).toBeVisible();
}

async function showFiles(page: Page) {
  const tab = page.locator("button.toolbar-sidebar-tab-left");
  if (!(await tab.evaluate((e) => e.classList.contains("active")))) await tab.click();
  await expect(page.locator(".sidebar:not(.right-sidebar)")).not.toHaveClass(/collapsed/);
}

interface Snap {
  sampleRate: number;
  duration: number;
  scrollOffset: number;
  isPlaying: boolean;
}

async function snap(page: Page): Promise<Snap> {
  return page.evaluate(() =>
    (window as unknown as { __oversample_test: () => Snap }).__oversample_test(),
  );
}

function json(route: Route, body: unknown) {
  return route.fulfill({
    contentType: "application/json",
    headers: { "Access-Control-Allow-Origin": "*" },
    body: JSON.stringify(body),
  });
}

test.describe("a correction from Find recordings", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("can be removed and applied again", async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    // One iNaturalist recording at 384 kHz, stored time-expanded x10 in a
    // 38.4 kHz file that has no GUANO.
    const audio = wav({ rate: 38_400, frames: 38_400, format: "16" });
    await page.route("https://audio.example.test/**", (route) =>
      route.fulfill({
        contentType: "audio/wav",
        headers: { "Access-Control-Allow-Origin": "*" },
        body: audio,
      }),
    );
    await page.route("**/search/**", (route) => {
      const path = new URL(route.request().url()).pathname;
      if (path.endsWith("/search/facets")) {
        return json(route, {
          groups: [{ grp: "bats", n: 1, species: 1 }],
          countries: [],
          sources: [{ source: "inat", n: 1 }],
          status: { inat: "ready" },
        });
      }
      if (path.endsWith("/search/species")) {
        return json(route, {
          total: 1,
          cursor: null,
          species: [
            {
              key: "b:Pipistrellus pipistrellus",
              gen: "Pipistrellus",
              sp: "pipistrellus",
              rank: 0,
              grp: "bats",
              en: "Common Pipistrelle",
              n: 1,
              sources: { inat: 1 },
            },
          ],
          status: { inat: "ready" },
          approx: false,
        });
      }
      if (path.endsWith("/search/recordings")) {
        return json(route, {
          total: 1,
          total_exact: true,
          by_source: { inat: 1 },
          cursor: null,
          truncated: false,
          items: [
            {
              id: "inat:1",
              source: "inat",
              gen: "Pipistrellus",
              sp: "pipistrellus",
              en: "Common Pipistrelle",
              cc: "GB",
              country: "United Kingdom",
              place: "Kew",
              smp: 384_000,
              len_s: 0.1,
              date: "2024-06-01",
              recordist: "A Recordist",
              licence: "//creativecommons.org/licenses/by/4.0/",
              quality: "",
              demo: false,
              time_expansion: 10,
              page_url: "https://www.inaturalist.org/observations/1",
              audio_url: "https://audio.example.test/rec.wav",
              audio_via: "direct",
            },
          ],
          status: { inat: "ready" },
        });
      }
      return json(route, { error: "not stubbed" });
    });
    await page.goto("/");
    await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
    await page.locator("button.add-files-btn", { hasText: "Find recordings" }).click();
    await page.locator(".find-fixed-row", { hasText: "All recordings" }).click();
    await page.locator(".find-rec-row .find-rec-load").click();
    await expect(page.locator(".toolbar-badge-row .badge-sample-rate")).toHaveText("384kHz", {
      timeout: 30_000,
    });
    const close = page.locator(".find-modal .xc-modal-close, .find-modal button[title='Close']");
    if (await page.locator(".find-modal").count()) {
      if (await close.count()) await close.first().click();
      else await page.keyboard.press("Escape");
    }
    await showInfo(page);
    await expect(page.locator(TE_NOTE)).toHaveText(
      "Corrected for ×10 time expansion. Sample rate in the file header: 38.4 kHz." +
        "Remove ×10 correction",
    );
    await expect(page.locator(`.toolbar-badge-row ${MARK}`)).toHaveText("Corrected ×10");

    await page.locator(REMOVE).click();
    await expect(row(page, "Sample rate")).toHaveText("38.4 kHz");
    await expect(page.locator(`${TE_NOTE} > div`)).toHaveText([
      "×10 correction removed until you close the file or reload the page. " +
        "Time expansion from Find recordings: ×10.",
    ]);
    await expect(page.locator(MARK)).toHaveCount(0);
    expect((await snap(page)).duration).toBeCloseTo(1, 9);

    await page.locator(APPLY).click();
    await expect(row(page, "Sample rate")).toHaveText("384 kHz");
    expect((await snap(page)).duration).toBeCloseTo(0.1, 9);
    await expect(page.locator(TE_NOTE)).toContainText("Corrected for ×10 time expansion.");
    expect(errors).toEqual([]);
  });
});

test.describe("a streamed WAV file corrected from its GUANO", () => {
  test.use({ viewport: { width: 1440, height: 900 } });
  test.setTimeout(240_000);

  test("can be removed and applied again", async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    // 8-bit, so 135 million samples are a 135 MB file that the app streams:
    // decoded to 32-bit floats they would take more than 512 MiB. The tone
    // is in the first 2 seconds and from 60 to 70 seconds at 38.4 kHz, past
    // the first 30 seconds, which the app decodes when it opens the file.
    const frames = 135_000_000;
    const file = test.info().outputPath("streamed-te10.wav");
    fs.writeFileSync(
      file,
      wav({
        rate: 38_400,
        frames,
        format: "u8",
        guano: GUANO_TE10,
        tones: [
          [0, 76_800],
          [2_304_000, 2_688_000],
        ],
      }),
    );
    await page.goto("/");
    await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
    await showFiles(page);
    await page.locator('input[type="file"]').first().setInputFiles(file);
    await expect(page.locator(".file-item")).toHaveCount(1, { timeout: 60_000 });
    await expect(page.locator(".toolbar-badge-row .file-badge-streaming")).toBeVisible({
      timeout: 60_000,
    });
    await showInfo(page);
    await expect(row(page, "Sample rate")).toHaveText("384 kHz");
    await expect(page.locator(TE_NOTE)).toContainText(
      "Corrected for ×10 time expansion. Sample rate in the file header: 38.4 kHz.",
    );
    expect((await snap(page)).duration).toBeCloseTo(frames / 384_000, 6);

    await page.locator(REMOVE).click({ timeout: 60_000 });
    await expect(row(page, "Sample rate")).toHaveText("38.4 kHz");
    expect((await snap(page)).duration).toBeCloseTo(frames / 38_400, 6);
    await expect(page.locator(`${TE_NOTE} > div`)).toHaveText([
      "×10 correction removed until you close the file or reload the page. " +
        "Time expansion in the file's GUANO metadata: ×10.",
    ]);
    await expect(page.locator(".toolbar-badge-row .file-badge-streaming")).toBeVisible();
    if (SHOT_DIR) {
      await page.waitForTimeout(1500);
      await page.screenshot({ path: path.join(SHOT_DIR, "streamed-1440-removed.png") });
    }
    // Past the first 30 seconds the samples are read from the file as they
    // are needed, by the source made for the new rate. Scroll there, and
    // play from there.
    const canvas = page.locator(".main-view .chart-stage > canvas");
    const box = (await canvas.boundingBox())!;
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    for (let i = 0; i < 80 && (await snap(page)).scrollOffset < 58; i++) {
      await page.mouse.wheel(0, 400);
    }
    expect((await snap(page)).scrollOffset).toBeGreaterThan(58);
    await page.locator(".play-main-btn").click();
    await expect.poll(async () => (await snap(page)).isPlaying).toBe(true);
    await page.waitForTimeout(1500);
    expect((await snap(page)).isPlaying).toBe(true);
    if (SHOT_DIR) {
      await page.screenshot({ path: path.join(SHOT_DIR, "streamed-1440-removed-60s.png") });
    }
    await page.locator(".play-main-btn").click();
    await expect.poll(async () => (await snap(page)).isPlaying).toBe(false);

    await page.locator(APPLY).click();
    await expect(row(page, "Sample rate")).toHaveText("384 kHz");
    expect((await snap(page)).duration).toBeCloseTo(frames / 384_000, 6);
    expect(errors).toEqual([]);
    fs.rmSync(file);
  });
});

/** Drags the Files sidebar's resize handle so the handle is at x = `width`. */
async function setSidebarWidth(page: Page, width: number) {
  const handle = page.locator(".sidebar:not(.right-sidebar) .sidebar-resize-handle");
  const box = (await handle.boundingBox())!;
  const x = box.x + 1.5;
  const y = box.y + box.height / 2;
  const centre = box.x + box.width / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + (width - centre), y, { steps: 8 });
  await page.mouse.up();
  await expect
    .poll(async () => {
      const b = (await handle.boundingBox())!;
      return Math.round(b.x + b.width / 2);
    })
    .toBe(width);
}

test.describe('the "Corrected ×10" badge in a narrow Files sidebar', () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("moves to the next line and leaves the facts where they were", async ({ page }) => {
    // The longest facts line: 384 kHz, 32-bit float.
    const files = [
      {
        name: "corrected float.wav",
        buffer: wav({ rate: 38_400, frames: 5_760, format: "float", guano: GUANO_TE10 }),
      },
      {
        name: "corrected 16-bit.wav",
        buffer: wav({ rate: 25_600, frames: 25_600, format: "16", guano: GUANO_TE10 }),
      },
    ];
    await page.goto("/");
    await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
    await showFiles(page);
    await page
      .locator('input[type="file"]')
      .first()
      .setInputFiles(files.map((f) => ({ ...f, mimeType: "audio/wav" })));
    await expect(page.locator(`.file-item-info ${MARK}`)).toHaveCount(2, { timeout: 30_000 });
    await expect(page.locator(".file-item-info .badge-sample-rate")).toHaveText([
      "384kHz",
      "256kHz",
    ]);

    for (const width of [140, 150, 160, 170, 180, 190, 200]) {
      await setSidebarWidth(page, width);
      const rows = await page.locator(".file-item-info").evaluateAll((infos) =>
        infos.map((info) => {
          const facts = [...info.querySelectorAll(".badge-fact")] as HTMLElement[];
          const tops = () => facts.map((f) => Math.round(f.getBoundingClientRect().top));
          const mark = info.querySelector(".file-badge-corrected") as HTMLElement;
          const left = info.getBoundingClientRect().left;
          const lineStarts = [...info.children]
            .filter((k) => getComputedStyle(k).display !== "none")
            .filter((k) => Math.abs(k.getBoundingClientRect().left - left) < 1)
            .map((k) => getComputedStyle(k, "::before").content);
          const withMark = tops();
          const height = info.getBoundingClientRect().height;
          mark.style.display = "none";
          const without = tops();
          const heightWithout = info.getBoundingClientRect().height;
          mark.style.display = "";
          return { withMark, without, height, heightWithout, lineStarts };
        }),
      );
      for (const r of rows) {
        // The facts are on the same lines with the badge as without it.
        expect(r.withMark, `${width}px`).toEqual(r.without);
        // At most one more 14px line, for the badge.
        expect(r.height - r.heightWithout, `${width}px`).toBeLessThanOrEqual(14 + 4);
        for (const content of r.lineStarts) expect(content).not.toContain("·");
      }
    }
  });
});
