import { test, expect, type Page, type Route } from "@playwright/test";
import * as fs from "node:fs";
import * as path from "node:path";

/**
 * Time-expansion corrections the file was opened with, other than the GUANO
 * ×10 file that te_correction.spec.ts opens: a factor from Find recordings, a
 * Wildlife Acoustics `wamd` chunk, a GUANO factor other than 10, a streamed
 * WAV file; what HFR does when one is removed; and the marks of a corrected
 * rate in the heading and in a narrow Files sidebar.
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
  hfrOn: boolean;
  playbackMode: string;
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

/** Adds WAV files to the Files sidebar and waits for their rows. */
async function addFiles(page: Page, files: { name: string; buffer: Buffer }[]) {
  await page.goto("/");
  await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
  await page.waitForFunction(
    () => typeof (window as { __oversample_test?: unknown }).__oversample_test === "function",
  );
  await showFiles(page);
  await page
    .locator('input[type="file"]')
    .first()
    .setInputFiles(files.map((f) => ({ ...f, mimeType: "audio/wav" })));
  await expect(page.locator(".file-item:not(.loading)")).toHaveCount(files.length, {
    timeout: 30_000,
  });
  await expect(page.locator(".file-item.loading")).toHaveCount(0);
}

const SPECIES_NOTE = '.rate-note[data-note="species"]';

/** XC844459's species, Pipistrellus pipistrellus (42-51 kHz), at 38.4 kHz:
 * read at the rate in its header, too low for its calls. */
function pipistrelle(te: string): { name: string; buffer: Buffer } {
  return {
    name: `pipistrelle te${te}.wav`,
    buffer: wav({
      rate: 38_400,
      frames: 76_800,
      format: "16",
      guano: `GUANO|Version: 1.0\nSpecies|Manual: Pipistrellus pipistrellus\nTE: ${te}\n`,
    }),
  };
}

test.describe("after a correction from the file's metadata is removed", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  // The metadata states the factor, so the species note gives only the
  // species' call range: no "Probably time-expanded ×10" guess in amber, and
  // no second button. With TE 8 that guess would contradict the metadata.
  for (const te of ["10", "8"]) {
    test(`TE ${te}: the species note gives only the species' calls`, async ({ page }) => {
      await addFiles(page, [pipistrelle(te)]);
      await showInfo(page);
      await page.locator(REMOVE).click();
      await expect(row(page, "Sample rate")).toHaveText("38.4 kHz");
      await expect(page.locator(SPECIES_NOTE)).toHaveCount(1);
      await expect(page.locator(`${SPECIES_NOTE} > div`)).toHaveText([
        /^Pipistrellus pipistrellus: echolocation calls at 42.+51 kHz, but this file can only contain frequencies up to 19.2 kHz\.$/,
      ]);
      await expect(page.locator(".rate-note .rate-note-warning")).toHaveCount(0);
      await expect(page.locator(".rate-note-action")).toHaveText([
        `Correct for ×${te} time expansion`,
      ]);
    });
  }
});

/** Picks PS (pitch shift) in the HEAR bar, which turns HFR on. */
async function choosePitchShift(page: Page) {
  await page.locator('button[title^="PS"]').first().click();
  await expect.poll(async () => (await snap(page)).hfrOn).toBe(true);
}

async function selectFile(page: Page, index: number) {
  await page.locator(".file-item").nth(index).locator(".file-item-name").click();
  await expect(page.locator(".file-item").nth(index)).toHaveClass(/\bactive\b/);
}

test.describe("HFR when a correction from the file's metadata is removed", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  const files = [
    {
      name: "a corrected.wav",
      buffer: wav({ rate: 38_400, frames: 76_800, format: "16", guano: GUANO_TE10 }),
    },
    { name: "b native.wav", buffer: wav({ rate: 384_000, frames: 384_000, format: "16" }) },
  ];

  /** Opens both files, with the first current, its correction removed and
   * applied again with HFR off. */
  async function applyAgainWithHfrOff(page: Page) {
    await addFiles(page, files);
    await selectFile(page, 0);
    await showInfo(page);
    await page.locator(REMOVE).click();
    await expect(row(page, "Sample rate")).toHaveText("38.4 kHz");
    await page.locator(APPLY).click();
    await expect(row(page, "Sample rate")).toHaveText("384 kHz");
    expect((await snap(page)).hfrOn).toBe(false);
  }

  test("stays on when it was turned on while another file was current", async ({ page }) => {
    await applyAgainWithHfrOff(page);
    await selectFile(page, 1);
    await choosePitchShift(page);
    await selectFile(page, 0);
    expect((await snap(page)).hfrOn).toBe(true);
    await page.locator(REMOVE).click();
    await expect(row(page, "Sample rate")).toHaveText("38.4 kHz");
    const after = await snap(page);
    expect(after.hfrOn).toBe(true);
    expect(after.playbackMode).toBe("PitchShift");
  });

  test("turns off when it was turned on for the corrected file", async ({ page }) => {
    await applyAgainWithHfrOff(page);
    await choosePitchShift(page);
    // Switching away and back leaves HFR as it was.
    await selectFile(page, 1);
    await selectFile(page, 0);
    await page.locator(REMOVE).click();
    await expect(row(page, "Sample rate")).toHaveText("38.4 kHz");
    await expect.poll(async () => (await snap(page)).hfrOn).toBe(false);
  });
});

/** The local bat-demo-sounds checkout, if there is one: BAT_DEMO_SOUNDS_DIR,
 * else ../batchi-extras/bat-demo-sounds beside the main checkout, seen from
 * the main checkout or from a worktree under its .claude/worktrees/. */
const DEMO_DIR = [
  process.env.BAT_DEMO_SOUNDS_DIR,
  "../batchi-extras/bat-demo-sounds",
  "../../../../batchi-extras/bat-demo-sounds",
]
  .filter((d): d is string => !!d)
  .map((d) => path.resolve(process.cwd(), d))
  .find((d) => fs.existsSync(path.join(d, "index.json")));

/** Opens a demo recording, served from the local checkout. XC892810 is
 * stored at 38.4 kHz with GUANO that says TE 10, and has a CC BY-NC-SA 4.0
 * licence. */
async function openDemo(page: Page, xc: string) {
  const serve = (route: Route) => {
    const rel = decodeURIComponent(new URL(route.request().url()).pathname).replace(
      /^\/gh\/pengowray\/bat-demo-sounds@main/,
      "",
    );
    const file = path.join(DEMO_DIR!, rel);
    if (!file.startsWith(DEMO_DIR!) || !fs.existsSync(file)) {
      return route.fulfill({ status: 404, body: "" });
    }
    const type = file.endsWith(".json") ? "application/json" : "audio/wav";
    return route.fulfill({ contentType: type, body: fs.readFileSync(file) });
  };
  await page.route("https://archive.oversample.com/**", serve);
  await page.route("https://cdn.jsdelivr.net/gh/pengowray/bat-demo-sounds@main/**", serve);
  await page.goto(`/#${xc}`);
  await expect(page.locator(".toolbar-badge-row .badge-sample-rate")).toHaveText("384kHz", {
    timeout: 60_000,
  });
}

/** The number of lines the children of `selector` take. */
function badgeLines(page: Page, selector: string) {
  return page.locator(selector).evaluate((el) => {
    const mids = [...el.children]
      .filter((k) => getComputedStyle(k).display !== "none" && k.getBoundingClientRect().width)
      .map((k) => {
        const b = k.getBoundingClientRect();
        return (b.top + b.bottom) / 2;
      })
      .sort((a, b) => a - b);
    return mids.filter((m, i) => i === 0 || m - mids[i - 1] > 6).length;
  });
}

test.describe("a corrected rate in a phone's heading", () => {
  test.skip(!DEMO_DIR, "no local bat-demo-sounds checkout");
  test.use({
    viewport: { width: 375, height: 812 },
    isMobile: true,
    hasTouch: true,
    userAgent:
      "Mozilla/5.0 (Linux; Android 14; Pixel 7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36",
  });

  test("keeps the heading's height, and the licence label is whole or not shown", async ({
    page,
  }) => {
    await openDemo(page, "XC892810");
    await expect(page.locator(`.toolbar-badge-row ${MARK}`)).toBeVisible();
    expect((await page.locator(".toolbar").boundingBox())!.height).toBe(49);
    expect(await badgeLines(page, ".toolbar-badge-row")).toBe(1);
    const label = page.locator(".toolbar-cc-badge .toolbar-cc-label");
    const cut = await label.evaluate(
      (e) => getComputedStyle(e).display !== "none" && e.scrollWidth > e.clientWidth,
    );
    expect(cut).toBe(false);
    // The licence button stays, and opens the licence dialog.
    await expect(page.locator(".toolbar-cc-badge .toolbar-cc-icon")).toBeVisible();
  });
});

test.describe("a corrected rate in a narrow Files sidebar", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("a corrected row wraps no earlier than a row of a file stored at that rate", async ({
    page,
  }) => {
    // Each corrected file has a twin stored at its corrected rate, with the
    // same bit depth and length, so both rows give the same facts.
    await addFiles(page, [
      {
        name: "corrected float.wav",
        buffer: wav({ rate: 38_400, frames: 5_760, format: "float", guano: GUANO_TE10 }),
      },
      { name: "native float.wav", buffer: wav({ rate: 384_000, frames: 5_760, format: "float" }) },
      {
        name: "corrected 16-bit.wav",
        buffer: wav({ rate: 25_600, frames: 25_600, format: "16", guano: GUANO_TE10 }),
      },
      { name: "native 16-bit.wav", buffer: wav({ rate: 256_000, frames: 25_600, format: "16" }) },
    ]);
    const facts = (i: number) =>
      page.locator(".file-item").nth(i).locator(".file-item-info .badge-fact");
    for (const [c, n] of [
      [0, 1],
      [2, 3],
    ]) {
      expect(await facts(c).allInnerTexts()).toEqual(await facts(n).allInnerTexts());
    }
    for (const width of [140, 150, 160, 170, 180, 190, 200, 260]) {
      await setSidebarWidth(page, width);
      const heights = await page
        .locator(".file-item-info")
        .evaluateAll((infos) => infos.map((i) => Math.round(i.getBoundingClientRect().height)));
      expect(heights[0], `float at ${width}px`).toBe(heights[1]);
      expect(heights[2], `16-bit at ${width}px`).toBe(heights[3]);
    }
  });

  test("a corrected row with a licence icon stays on one line", async ({ page }) => {
    test.skip(!DEMO_DIR, "no local bat-demo-sounds checkout");
    await openDemo(page, "XC892810");
    await showFiles(page);
    const info = ".file-item:has-text('XC892810') .file-item-info";
    await expect(page.locator(`${info} .badge-cc-mini`)).toBeVisible();
    await expect(page.locator(`${info} ${MARK}`)).toHaveCount(1);
    for (const width of [140, 150, 160, 170, 180, 190, 200]) {
      await setSidebarWidth(page, width);
      expect(await badgeLines(page, info), `${width}px`).toBe(1);
    }
  });
});

test.describe("the boxed sample rate in a file row", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("opens the Info panel at the note of its file", async ({ page }) => {
    await addFiles(page, [
      {
        name: "corrected.wav",
        buffer: wav({ rate: 38_400, frames: 38_400, format: "16", guano: GUANO_TE10 }),
      },
      { name: "native.wav", buffer: wav({ rate: 384_000, frames: 38_400, format: "16" }) },
    ]);
    await selectFile(page, 1);
    const hideInfo = page.locator('button[title="Hide info panel"]');
    if (await hideInfo.count()) await hideInfo.click();
    await expect(page.locator(".right-sidebar")).toHaveClass(/collapsed/);
    const box = page.locator(".file-item").nth(0).locator(`.file-item-info ${MARK}`);
    await expect(box).toHaveText("384kHz");
    await expect(box).toHaveAttribute(
      "title",
      "Corrected for ×10 time expansion. Sample rate in the file header: 38.4 kHz.",
    );
    // Only the corrected file's rate is a box.
    await expect(page.locator(".file-item").nth(1).locator(MARK)).toHaveCount(0);
    await box.click();
    await expect(page.locator(".file-item").nth(0)).toHaveClass(/\bactive\b/);
    await expect(page.locator(".right-sidebar")).not.toHaveClass(/collapsed/);
    await expect(page.locator(TE_NOTE)).toBeInViewport();
    await expect(row(page, "Sample rate")).toHaveText("384 kHz");
  });
});

test.describe("a corrected rate in a desktop heading", () => {
  test.skip(!DEMO_DIR, "no local bat-demo-sounds checkout");
  test.use({ viewport: { width: 1440, height: 900 } });

  test("keeps the whole licence label", async ({ page }) => {
    await openDemo(page, "XC892810");
    await expect(page.locator(`.toolbar-badge-row ${MARK}`)).toHaveText("Corrected ×10");
    const label = page.locator(".toolbar-cc-badge .toolbar-cc-label");
    await expect(label).toBeVisible();
    await expect(label).toHaveText("BY-NC-SA 4.0");
    expect(await label.evaluate((e) => e.scrollWidth > e.clientWidth)).toBe(false);
  });
});
