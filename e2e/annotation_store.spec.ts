import { test, expect, type Page } from "@playwright/test";

/**
 * Saved annotations (browser: one YAML file per recording in the
 * `oversample-annotations` folder of the origin's private file system).
 *
 * - A Xeno-canto recording that arrives with its own annotation set keeps the
 *   annotations the user saved for it earlier.
 * - A saved set is only given to the file it was saved for, also when two
 *   files have the same name and size.
 *
 * (src/opfs.rs, src/annota.rs, src/file_identity.rs.)
 */

type Range = { timeStart: number; timeEnd: number; freqLow: number | null; freqHigh: number | null };
type Snap = {
  currentFile: string | null;
  scrollOffset: number;
  zoom: number;
  sampleRate: number;
  duration: number;
  regions: Range[];
  selection: Range | null;
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

/** Every saved annotation file: name and text. */
const savedFiles = (page: Page): Promise<Record<string, string>> =>
  page.evaluate(async () => {
    const root = await navigator.storage.getDirectory();
    const out: Record<string, string> = {};
    let dir: FileSystemDirectoryHandle;
    try {
      dir = await root.getDirectoryHandle("oversample-annotations");
    } catch {
      return out;
    }
    const entries = (dir as unknown as { entries: () => AsyncIterable<[string, FileSystemFileHandle]> }).entries();
    for await (const [name, handle] of entries) {
      // A file the app is writing at this moment cannot be read; the callers
      // poll, and read it on a later call.
      try {
        out[name] = await (await handle.getFile()).text();
      } catch {}
    }
    return out;
  });

const writeSavedFile = (page: Page, name: string, text: string) =>
  page.evaluate(
    async ([name, text]) => {
      const root = await navigator.storage.getDirectory();
      const dir = await root.getDirectoryHandle("oversample-annotations", { create: true });
      const handle = await dir.getFileHandle(name, { create: true });
      const w = await handle.createWritable();
      await w.write(text);
      await w.close();
    },
    [name, text],
  );

/** Start times of the regions stored in a saved file's text. */
const savedStarts = (text: string): number[] =>
  [...text.matchAll(/time_start: ([0-9.eE+-]+)/g)].map((m) => Number(m[1]));

const starts = async (page: Page) => (await snap(page)).regions.map((r) => r.timeStart).sort((a, b) => a - b);

async function open(page: Page, name: string, seconds = 8, seed = 4321, rate = 96_000) {
  const files = page.locator(".file-item", { hasText: name });
  const before = await files.count();
  await page
    .locator('input[type="file"][accept*=".wav"]')
    .first()
    .setInputFiles({ name, mimeType: "audio/wav", buffer: noiseWav(rate, seconds, seed) });
  await expect(files).toHaveCount(before + 1, { timeout: 30_000 });
  await expect.poll(async () => (await snap(page)).currentFile).toBe(name);
  await expect(page.locator(".chart-stage canvas").first()).toBeVisible({ timeout: 30_000 });
}

async function selectTool(page: Page) {
  await page.locator(".tool-combo-btn").click();
  await page.locator(".layer-panel button", { hasText: "Selection" }).click();
}

/**
 * Draws a selection from fraction t0 to t1 of the file and makes it an
 * annotation. Returns the new region's start time.
 */
async function annotate(page: Page, t0: number, t1: number): Promise<number> {
  const before = (await snap(page)).regions.length;
  await selectTool(page);
  const canvas = page.locator(".main-view .chart-stage > canvas");
  const box = (await canvas.boundingBox())!;
  const s = await snap(page);
  const visible = (box.width / s.zoom) * (512 / s.sampleRate);
  const shown = Math.min(visible, s.duration - s.scrollOffset);
  const x = (f: number) => box.x + (box.width * f * shown) / visible;
  const y = (f: number) => box.y + box.height * f;
  await page.mouse.move(x(t0), y(0.3));
  await page.mouse.down();
  await page.mouse.move(x(t1), y(0.6), { steps: 5 });
  await page.mouse.up();
  const sel = (await snap(page)).selection;
  expect(sel).not.toBeNull();
  // "Annotate Region" in the selection's menu makes it an annotation and
  // opens the label box; Enter keeps the default label.
  await page.locator('.canvas-overflow-btn[title="Selection options"]').click();
  await page.locator(".canvas-overflow-item", { hasText: "Annotate Region" }).click();
  await expect(page.locator(".annotation-inline-label-editor")).toBeVisible();
  await page.keyboard.press("Enter");
  await expect.poll(async () => (await snap(page)).regions.length).toBe(before + 1);
  return sel!.timeStart;
}

/** Waits until some saved file holds a region starting at `start`. */
async function waitSaved(page: Page, start: number) {
  await expect
    .poll(async () => Object.values(await savedFiles(page)).some((t) => savedStarts(t).some((s) => Math.abs(s - start) < 1e-6)))
    .toBe(true);
}

// ── Xeno-canto ──────────────────────────────────────────────────────────────

const XC_ANNOTATIONS = [
  { annotation_xc_id: 1, xc_nr: "111", scientific_name: "Myotis daubentonii", start_time: 1.0, end_time: 2.0, frequency_high: 40000, frequency_low: 20000, sound_type: "echolocation" },
  { annotation_xc_id: 2, xc_nr: "111", scientific_name: "Myotis daubentonii", start_time: 3.0, end_time: 3.5, frequency_high: 40000, frequency_low: 20000, sound_type: "echolocation" },
];

function xcRecord(id: string): Record<string, unknown> {
  const rec: Record<string, unknown> = {
    id,
    gen: "Myotis",
    sp: id === "111" ? "daubentonii" : "nattereri",
    en: id === "111" ? "Daubenton's Bat" : "Natterer's Bat",
    rec: "A Recordist",
    cnt: "Netherlands",
    loc: "Somewhere",
    lic: "//creativecommons.org/licenses/by-nc-sa/4.0/",
    q: "A",
    type: "echolocation",
    file: `https://xeno-canto.org/${id}/download`,
    "file-name": `XC${id}.wav`,
    smp: "96000",
    length: "0:08",
  };
  if (id === "111") {
    rec["annotation-set"] = { set_name: "Annotation set for XC111", annotations: XC_ANNOTATIONS };
  }
  return rec;
}

async function stubXc(page: Page) {
  const cors = { "access-control-allow-origin": "*" };
  await page.route("**/search/**", (r) => r.fulfill({ status: 404, headers: cors, body: "{}" }));
  await page.route("**/xc/recording/*", (r) => {
    const id = r.request().url().split("/").pop()!;
    return r.fulfill({ status: 200, contentType: "application/json", headers: cors, body: JSON.stringify(xcRecord(id)) });
  });
  await page.route("**/xc/api/3/recordings*", (r) =>
    r.fulfill({
      status: 200,
      contentType: "application/json",
      headers: cors,
      body: JSON.stringify({ numRecordings: "2", numSpecies: "2", page: 1, numPages: 1, recordings: [xcRecord("222"), xcRecord("111")] }),
    }),
  );
  await page.route("**/xc/audio/*", (r) => {
    const id = r.request().url().split("/").pop()!;
    return r.fulfill({ status: 200, contentType: "audio/wav", headers: cors, body: noiseWav(96_000, 8, Number(id)) });
  });
}

/** Opens XC`id` by its link in the Find recordings dialog. */
async function loadFromFind(page: Page, id: number) {
  const files = page.locator(".file-item", { hasText: `XC${id}` });
  const before = await files.count();
  await page.locator("button.add-files-btn", { hasText: "Find recordings" }).click();
  await expect(page.locator(".find-modal")).toBeVisible();
  await page.locator(".find-modal input").first().fill(`XC${id}`);
  await page.locator(".find-link-row").click();
  await expect(files).toHaveCount(before + 1, { timeout: 30_000 });
  await expect.poll(async () => (await snap(page)).currentFile ?? "").toContain(`XC${id}`);
  await expect(page.locator(".chart-stage canvas").first()).toBeVisible({ timeout: 30_000 });
}

async function begin(page: Page) {
  await stubXc(page);
  await page.goto("/");
  await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
}

/** The saved file of XC111 holds the user's region and one copy of each XC region. */
async function expectSavedXc111(page: Page, mine: number) {
  await expect
    .poll(async () => {
      const texts = Object.values(await savedFiles(page)).filter((t) => t.includes("XC111"));
      return texts.map((t) => savedStarts(t).sort((a, b) => a - b));
    })
    .toEqual([[1, 3, mine].sort((a, b) => a - b)]);
}

test.describe("Saved annotations on a Xeno-canto recording with its own annotation set", () => {
  test("are kept when the recording is opened again from Find recordings", async ({ page }) => {
    await begin(page);
    await loadFromFind(page, 111);
    await expect.poll(() => starts(page)).toEqual([1, 3]);
    const mine = await annotate(page, 0.75, 0.9);
    expect(mine).toBeGreaterThan(3.6);
    await waitSaved(page, mine);
    const all = [1, 3, mine];

    // After a page reload.
    await page.reload();
    await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
    await loadFromFind(page, 111);
    await expect.poll(() => starts(page)).toEqual(all);
    // Nothing later takes the user's region away or adds copies.
    await page.waitForTimeout(1500);
    expect(await starts(page)).toEqual(all);
    await expectSavedXc111(page, mine);
    expect(await page.locator(".file-item", { hasText: "XC111" }).count()).toBe(1);

    // And again in the same session, as a second copy in the list.
    await loadFromFind(page, 111);
    await expect.poll(() => starts(page)).toEqual(all);
    await page.waitForTimeout(1500);
    expect(await starts(page)).toEqual(all);
    await expectSavedXc111(page, mine);
  });

  test("are kept when the saved file is slow to read", async ({ page }) => {
    await begin(page);
    await loadFromFind(page, 111);
    await expect.poll(() => starts(page)).toEqual([1, 3]);
    const mine = await annotate(page, 0.75, 0.9);
    await waitSaved(page, mine);

    // From here on, reading a saved annotation file takes a second.
    await page.addInitScript(() => {
      const getFile = FileSystemFileHandle.prototype.getFile;
      FileSystemFileHandle.prototype.getFile = async function (this: FileSystemFileHandle) {
        if (this.name.endsWith(".batm")) await new Promise((r) => setTimeout(r, 1000));
        return getFile.call(this);
      };
    });
    await page.reload();
    await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
    await loadFromFind(page, 111);
    await expect.poll(() => starts(page)).toEqual([1, 3, mine]);
    await page.waitForTimeout(2500);
    expect(await starts(page)).toEqual([1, 3, mine]);
    await expectSavedXc111(page, mine);
  });

  test("are kept when the recording is opened again from the Xeno-canto browser", async ({ page }) => {
    await begin(page);
    await loadFromFind(page, 111);
    await expect.poll(() => starts(page)).toEqual([1, 3]);
    const mine = await annotate(page, 0.75, 0.9);
    await waitSaved(page, mine);

    await page.reload();
    await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
    await loadFromBrowser(page, 111);
    await expect.poll(() => starts(page)).toEqual([1, 3, mine]);
    await page.waitForTimeout(1500);
    expect(await starts(page)).toEqual([1, 3, mine]);
    await expectSavedXc111(page, mine);
  });

  test("the annotation set goes to the recording it came with", async ({ page }) => {
    await begin(page);
    // XC111's audio arrives late, after another file has been opened.
    let release: () => void = () => {};
    const held = new Promise<void>((r) => (release = r));
    await page.route("**/xc/audio/111", async (r) => {
      await held;
      return r.fulfill({ status: 200, contentType: "audio/wav", headers: { "access-control-allow-origin": "*" }, body: noiseWav(96_000, 8, 111) });
    });
    await page.locator("button.add-files-btn", { hasText: "Find recordings" }).click();
    await page.locator(".find-modal input").first().fill("XC111");
    await page.locator(".find-link-row").click();
    await page.keyboard.press("Escape");
    await open(page, "other.wav", 8, 77);
    release();
    await expect(page.locator(".file-item", { hasText: "XC111" })).toHaveCount(1, { timeout: 30_000 });
    await page.waitForTimeout(1000);

    await page.locator(".file-item", { hasText: "other.wav" }).click();
    await expect.poll(async () => (await snap(page)).currentFile).toBe("other.wav");
    expect(await starts(page)).toEqual([]);
    await page.locator(".file-item", { hasText: "XC111" }).click();
    await expect.poll(async () => (await snap(page)).currentFile ?? "").toContain("XC111");
    expect(await starts(page)).toEqual([1, 3]);
  });
});

/** Opens XC`id` from the search results of the Xeno-canto browser. */
async function loadFromBrowser(page: Page, id: number) {
  await page.locator("button.add-files-btn", { hasText: "Find recordings" }).click();
  await expect(page.locator(".find-modal")).toBeVisible();
  await page.locator(".find-modal button", { hasText: "Explore Xeno-canto" }).first().click();
  await expect(page.locator(".xc-modal")).toBeVisible();
  const search = page.locator(".xc-search-input");
  const noKey = page.locator(".xc-modal button", { hasText: "Continue without key" });
  await expect(search.or(noKey).first()).toBeVisible();
  if (!(await search.count())) await noKey.first().click();
  await search.fill("Myotis");
  await search.press("Enter");
  await page.locator(".xc-rec-row", { hasText: `XC${id}` }).locator(".xc-btn-load").click();
  await expect.poll(async () => (await snap(page)).currentFile ?? "", { timeout: 30_000 }).toContain(`XC${id}`);
  await expect(page.locator(".chart-stage canvas").first()).toBeVisible({ timeout: 30_000 });
}

// ── Any file ────────────────────────────────────────────────────────────────

test.describe("Saved annotations of a file", () => {
  test("the first annotation on a new file is still there after a reload", async ({ page }) => {
    await page.goto("/");
    await open(page, "first.wav", 8, 5);
    const mine = await annotate(page, 0.3, 0.4);
    await waitSaved(page, mine);

    await page.reload();
    await open(page, "first.wav", 8, 5);
    await expect.poll(() => starts(page)).toEqual([mine]);
  });

  test("an annotation drawn before the saved ones were read is added to them", async ({ page }) => {
    await page.goto("/");
    await open(page, "slow.wav", 8, 6);
    const saved = await annotate(page, 0.3, 0.4);
    await waitSaved(page, saved);

    // From here on, reading a saved annotation file takes four seconds.
    await page.addInitScript(() => {
      const getFile = FileSystemFileHandle.prototype.getFile;
      FileSystemFileHandle.prototype.getFile = async function (this: FileSystemFileHandle) {
        if (this.name.endsWith(".batm")) await new Promise((r) => setTimeout(r, 4000));
        return getFile.call(this);
      };
    });
    await page.reload();
    await open(page, "slow.wav", 8, 6);
    expect(await starts(page)).toEqual([]);
    const drawn = await annotate(page, 0.75, 0.9);
    expect(await starts(page)).toEqual([drawn]);

    const both = [saved, drawn].sort((a, b) => a - b);
    await expect.poll(() => starts(page)).toEqual(both);
    await expect
      .poll(async () => Object.values(await savedFiles(page)).map((t) => savedStarts(t).sort((a, b) => a - b)))
      .toEqual([both]);
  });
});

// ── Two files with the same name and size ───────────────────────────────────

test.describe("Two different files with the same name and size", () => {
  test("each keeps its own annotations", async ({ page }) => {
    await page.goto("/");
    await open(page, "same.wav", 8, 1);
    const mine = await annotate(page, 0.3, 0.4);
    await waitSaved(page, mine);

    await page.reload();
    await open(page, "same.wav", 8, 2);
    await page.waitForTimeout(1500);
    expect(await starts(page)).toEqual([]);
    const other = await annotate(page, 0.75, 0.9);
    await waitSaved(page, other);

    await page.reload();
    await open(page, "same.wav", 8, 1);
    await expect.poll(() => starts(page)).toEqual([mine]);
  });

  test("a set saved under the name and size goes only to the file whose hash it holds", async ({ page }) => {
    await page.goto("/");
    await open(page, "same.wav", 8, 1);
    const mine = await annotate(page, 0.3, 0.4);
    await waitSaved(page, mine);
    // Move the saved set to the key made of the name and size, where a set
    // saved before the file's hash was known is kept.
    const files = await savedFiles(page);
    const [hashKey, text] = Object.entries(files).find(([, t]) => savedStarts(t).length === 1)!;
    expect(hashKey).toMatch(/^b3_/);
    expect(text).toContain("spot_hash_b3");
    const size = 44 + 96_000 * 8 * 2;
    const nameKey = `same.wav_${size}.batm`;
    await writeSavedFile(page, nameKey, text);
    await page.evaluate(async (key) => {
      const root = await navigator.storage.getDirectory();
      const dir = await root.getDirectoryHandle("oversample-annotations");
      await dir.removeEntry(key);
    }, hashKey);

    // The other file with that name and size does not get the set.
    await page.reload();
    await open(page, "same.wav", 8, 2);
    await page.waitForTimeout(1500);
    expect(await starts(page)).toEqual([]);
    const other = await annotate(page, 0.75, 0.9);
    await waitSaved(page, other);
    // The first file's set is still stored, unchanged.
    expect(savedStarts((await savedFiles(page))[nameKey])).toEqual([mine]);

    // The file it belongs to still gets it.
    await page.reload();
    await open(page, "same.wav", 8, 1);
    await expect.poll(() => starts(page)).toEqual([mine]);
  });

  test("a set saved with no hash in it still loads", async ({ page }) => {
    await page.goto("/");
    await open(page, "same.wav", 8, 1);
    const mine = await annotate(page, 0.3, 0.4);
    await waitSaved(page, mine);
    const files = await savedFiles(page);
    const [hashKey, text] = Object.entries(files).find(([, t]) => savedStarts(t).length === 1)!;
    // As an old version saved it: under the name and size, with no hashes.
    const old = text
      .split("\n")
      .filter((line) => !/^\s*(spot_hash_b3|content_hash|full_blake3|full_sha256):/.test(line))
      .join("\n");
    expect(old).not.toContain("spot_hash_b3");
    const size = 44 + 96_000 * 8 * 2;
    await writeSavedFile(page, `same.wav_${size}.batm`, old);
    await page.evaluate(async (key) => {
      const root = await navigator.storage.getDirectory();
      const dir = await root.getDirectoryHandle("oversample-annotations");
      await dir.removeEntry(key);
    }, hashKey);

    // There is no hash to compare, so a different file of that name and
    // size is also given the set. It saves its copy under its own hash and
    // leaves the set as it was.
    const nameKey = `same.wav_${size}.batm`;
    await page.reload();
    await open(page, "same.wav", 8, 2);
    await expect.poll(() => starts(page)).toEqual([mine]);
    await expect
      .poll(async () => Object.keys(await savedFiles(page)).filter((k) => k.startsWith("b3_")).length)
      .toBe(1);
    expect((await savedFiles(page))[nameKey]).toBe(old);

    // The file the set was made for still gets it afterwards.
    await page.reload();
    await open(page, "same.wav", 8, 1);
    await expect.poll(() => starts(page)).toEqual([mine]);
    await expect
      .poll(async () => Object.keys(await savedFiles(page)).filter((k) => k.startsWith("b3_")).length)
      .toBe(2);
    expect((await savedFiles(page))[nameKey]).toBe(old);
  });
});
