import { test, expect, type Page } from "@playwright/test";

/**
 * The Call detection tab (src/components/file_sidebar/call_detection_panel*):
 * the method drop-down, the settings opened from the gear, the pin that
 * shows a species in a row above the spectrogram, and folds that stay open
 * across a reload. Results come from the classifier test hook's `seed`, so
 * no model is downloaded or run.
 */

function noiseWav(sampleRate: number, seconds: number, seed = 1234): Buffer {
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

/** Two species over four 3 s windows: Blackbird in three, Robin in one. */
const SEED = JSON.stringify({
  method: "birdnet",
  labels: ["Turdus merula_Common Blackbird", "Erithacus rubecula_European Robin", "Engine_Engine"],
  windows: [
    [0, 3, [[0, 0.91], [1, 0.2]]],
    [3, 6, [[0, 0.62]]],
    [6, 9, [[1, 0.55], [0, 0.1]]],
    [9, 12, [[0, 0.33]]],
  ],
});

async function openFile(page: Page) {
  await page.goto("/");
  await page.waitForFunction(() => typeof (window as { __oversample_classifier?: unknown }).__oversample_classifier === "object");
  const name = "call-detection-test.wav";
  await page
    .locator('input[type="file"][accept*=".wav"]')
    .first()
    .setInputFiles({ name, mimeType: "audio/wav", buffer: noiseWav(48_000, 12) });
  await expect(page.locator(".file-item", { hasText: name })).toBeVisible({ timeout: 30_000 });
  // The right sidebar's tab menu: Call detection.
  const tabs = page.locator(".sidebar-tab-dropdown").last();
  if (!(await tabs.isVisible())) await page.locator(".toolbar-sidebar-tab-right").click();
  if (!((await tabs.textContent()) ?? "").includes("Call detection")) {
    await tabs.click();
    await page.locator(".sidebar-tab-option", { hasText: "Call detection" }).dispatchEvent("mousedown");
  }
  await expect(page.locator(".cd-method-btn")).toBeVisible();
}

async function pickMethod(page: Page, name: string) {
  await page.locator(".cd-method-btn").click();
  await expect(page.locator(".cd-method-menu")).toBeVisible();
  // By the option's name only: a description can name another method.
  await page.locator(".cd-method-opt .preset-main").getByText(name, { exact: true }).click();
  await expect(page.locator(".cd-method-menu")).toBeHidden();
  await expect(page.locator(".cd-method-name")).toHaveText(name);
}

async function seed(page: Page) {
  const err = await page.evaluate(
    (s) => (window as unknown as { __oversample_classifier: { seed: (j: string) => string } }).__oversample_classifier.seed(s),
    SEED,
  );
  expect(err).toBe("");
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    if (!sessionStorage.getItem("cd-test-started")) {
      sessionStorage.setItem("cd-test-started", "1");
      localStorage.removeItem("oversample_score_strips");
      localStorage.removeItem("oversample_call_detection_folds");
    }
  });
});

test("the method drop-down switches methods and closes on Escape", async ({ page }) => {
  await openFile(page);
  await pickMethod(page, "BirdNET V2.4");
  await expect(page.locator(".cd-method-help")).toContainText("Bird species");
  await expect(page.locator(".cd-recording .cd-head-title")).toHaveText("In this recording");
  await pickMethod(page, "In-built bat call detection");
  await expect(page.locator(".cd-method-help")).toHaveText("Finds bat calls. Runs automatically.");
  await page.locator(".cd-method-btn").click();
  await expect(page.locator(".cd-method-menu")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".cd-method-menu")).toBeHidden();
  // The menu is on top where it is drawn.
  await page.locator(".cd-method-btn").click();
  const onTop = await page.evaluate(() => {
    const opt = document.querySelectorAll(".cd-method-opt")[3] as HTMLElement;
    const r = opt.getBoundingClientRect();
    const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
    return !!hit && opt.contains(hit);
  });
  expect(onTop).toBe(true);
});

test("the settings open beside Analyse, inside the sidebar", async ({ page }) => {
  await openFile(page);
  await pickMethod(page, "BirdNET V2.4");
  await expect(page.locator(".cd-in-force")).toContainText("Min confidence 0.05");
  await page.locator(".cd-gear").click();
  const settings = page.locator(".cd-settings");
  await expect(settings).toBeVisible();
  await expect(settings).toContainText("Minimum confidence");
  await expect(settings).toContainText("Boxes on the spectrogram");
  // Not cut off: inside the sidebar and the window, and drawn on top.
  const fits = await page.evaluate(() => {
    const s = document.querySelector(".cd-settings")!.getBoundingClientRect();
    const panel = document.querySelector(".cd-panel")!.getBoundingClientRect();
    const hit = document.elementFromPoint(s.left + s.width / 2, s.top + 10);
    return {
      inside: s.left >= panel.left - 0.5 && s.right <= panel.right + 0.5 && s.right <= window.innerWidth,
      onTop: !!hit && document.querySelector(".cd-settings")!.contains(hit),
    };
  });
  expect(fits).toEqual({ inside: true, onTop: true });
  // With results, turning the boxes off shows in the line of settings in force.
  await seed(page);
  await settings.getByLabel("Boxes on the spectrogram").check();
  await expect(page.locator(".cd-in-force")).not.toContainText("Boxes off");
  await settings.getByLabel("Boxes on the spectrogram").uncheck();
  await expect(page.locator(".cd-in-force")).toContainText("Boxes off");
  await page.locator(".cd-in-force").click();
  await expect(settings).toBeHidden();
});

test("species found get rows above the spectrogram, and a pin removes or adds one", async ({ page }) => {
  await openFile(page);
  await pickMethod(page, "BirdNET V2.4");
  const strips = page.locator(".score-strips-row");
  await expect(strips).toBeHidden();
  await seed(page);
  const row = page.locator(".cd-list tr", { hasText: "Common Blackbird" });
  await expect(row).toBeVisible();
  await expect(page.locator(".cd-recording .cd-head-fact")).toHaveText("2 species, 4 windows");
  // Both species found have a row without a pin being pressed first.
  const pin = row.locator(".cd-pin");
  const robinPin = page.locator(".cd-list tr", { hasText: "European Robin" }).locator(".cd-pin");
  await expect(strips).toBeVisible();
  await expect(pin).toHaveAttribute("aria-pressed", "true");
  await expect(pin).toHaveAttribute("aria-label", "Remove Common Blackbird from above the spectrogram");
  await expect(robinPin).toHaveAttribute("aria-pressed", "true");
  const height = async () => (await strips.boundingBox())!.height;
  const two = await height();
  // The pin removes the row, and that is saved.
  await pin.click();
  await expect(pin).toHaveAttribute("aria-pressed", "false");
  await expect(pin).toHaveAttribute("aria-label", "Show Common Blackbird above the spectrogram");
  await expect.poll(height).toBeLessThan(two);
  const saved = () => page.evaluate(() => JSON.parse(localStorage.getItem("oversample_score_strips") ?? "{}"));
  expect(JSON.stringify((await saved()).hidden)).toContain("Turdus merula");
  // And puts it back.
  await pin.click();
  await expect(pin).toHaveAttribute("aria-pressed", "true");
  await expect.poll(height).toBe(two);
  expect((await saved()).hidden).toEqual([]);
  // A class added by the search is listed too, and its pin removes it.
  await page.locator(".strip-search-input").fill("song thr");
  await expect(page.locator(".strip-search-empty")).toHaveText("No species found");
  await page.locator(".strip-search-input").fill("engine");
  await page.locator(".strip-search-option", { hasText: "Engine" }).dispatchEvent("mousedown");
  await expect(page.locator(".cd-list tr", { hasText: "Engine" }).locator(".cd-pin")).toHaveAttribute("aria-pressed", "true");
  expect(JSON.stringify((await saved()).species)).toContain("Engine");
  await page.locator(".cd-list tr", { hasText: "Engine" }).locator(".cd-pin").click();
  await expect(page.locator(".cd-list tr", { hasText: "Engine" })).toHaveCount(0);
  // Every row removed: nothing above the spectrogram.
  await pin.click();
  await robinPin.click();
  await expect(strips).toBeHidden();
});

test("folds stay open across a reload", async ({ page }) => {
  await openFile(page);
  await pickMethod(page, "BirdNET V2.4");
  await seed(page);
  const fold = page.locator('.cd-fold[data-fold="windows"]');
  await expect(fold.locator(".cd-fold-head")).toHaveAttribute("aria-expanded", "false");
  await fold.locator(".cd-fold-head").click();
  await expect(fold.locator(".cd-fold-body")).toBeVisible();
  await expect(fold.locator(".cd-fold-body tr", { hasText: "European Robin" })).toBeVisible();
  const annotate = page.locator('.cd-fold[data-fold="annotate"] .cd-fold-head');
  await expect(annotate).toHaveText(/Add 4 annotations/);
  await openFile(page);
  await pickMethod(page, "BirdNET V2.4");
  await seed(page);
  await expect(page.locator('.cd-fold[data-fold="windows"] .cd-fold-head')).toHaveAttribute("aria-expanded", "true");
  await expect(page.locator('.cd-fold[data-fold="annotate"] .cd-fold-head')).toHaveAttribute("aria-expanded", "false");
});

/** BirdNET+ V3.0 windows every 1.5 s: a robin in the one at 1.5 s. */
const V3_SEED = JSON.stringify({
  method: "birdnet-v3",
  labels: ["Turdus merula_Common Blackbird", "Erithacus rubecula_European Robin"],
  windows: [
    [0, 3, [[0, 0.8]]],
    [1.5, 4.5, [[1, 0.6], [0, 0.2]]],
    [3, 6, [[0, 0.1]]],
    [4.5, 7.5, [[1, 0.2]]],
    [6, 9, [[0, 0.4]]],
  ],
});

/**
 * Window boxes, as BirdBox's model gives them: [window start, t0, t1, f0, f1,
 * confidence]. The robin's window names the first; the second's window
 * scores only a blackbird at 0.10, so it has no name; the third is below the
 * default minimum box confidence of 0.30.
 */
const BOX_SEED = JSON.stringify({
  raw: [
    [1.5, 2.4, 2.8, 2000, 4000, 0.7],
    [3.0, 3.9, 4.2, 5000, 7000, 0.5],
    [6.0, 7.0, 7.4, 3000, 6000, 0.2],
  ],
  samples: 12 * 32000,
});

type Hook = { seed: (j: string) => string; seedBoxes: (j: string) => string; boxes: () => string };

/** Pixels of the spectrogram in the bird boxes' lime. */
async function limePixels(page: Page): Promise<number> {
  return page.evaluate(() => {
    const c = document.querySelector(".chart-stage canvas") as HTMLCanvasElement;
    const d = c.getContext("2d")!.getImageData(0, 0, c.width, c.height).data;
    let n = 0;
    for (let i = 0; i < d.length; i += 4) {
      if (Math.abs(d[i] - 180) < 12 && Math.abs(d[i + 1] - 240) < 12 && Math.abs(d[i + 2] - 60) < 16) n++;
    }
    return n;
  });
}

test("bird sound boxes are drawn and named from BirdNET+ V3.0", async ({ page }) => {
  await page.addInitScript(() => localStorage.removeItem("oversample_bird_boxes"));
  await openFile(page);
  await pickMethod(page, "BirdNET+ V3.0");
  const errs = await page.evaluate(
    ([a, b]) => {
      const h = (window as unknown as { __oversample_classifier: Hook }).__oversample_classifier;
      return [h.seed(a), h.seedBoxes(b)];
    },
    [V3_SEED, BOX_SEED],
  );
  expect(errs).toEqual(["", ""]);
  const hook = () =>
    page.evaluate(() => JSON.parse((window as unknown as { __oversample_classifier: Hook }).__oversample_classifier.boxes()));
  // Off by default: nothing drawn, nothing listed.
  expect(await hook()).toEqual([]);
  await expect(page.locator(".cd-list tr", { hasText: "Bird sounds" })).toHaveCount(0);
  await page.locator(".cd-gear").click();
  const settings = page.locator(".cd-settings");
  await settings.getByLabel("Draw a box round each bird sound").check();
  await settings.getByLabel("Boxes on the spectrogram").check();
  await expect(page.locator(".cd-in-force")).toContainText("One box per phrase");
  await expect(page.locator(".cd-in-force")).toContainText("Min box confidence 0.30");
  await expect(settings).toContainText("Minimum box confidence");
  const row = page.locator(".cd-list tr", { hasText: "Bird sounds" });
  await expect(row).toContainText("Bird sounds: 2 boxes");
  await expect(row).toContainText("0.70");
  const boxes = await hook();
  expect(boxes.map((b: { name: string | null }) => b.name)).toEqual(["European Robin", null]);
  expect(boxes[0].species.map((s: [string, number]) => s[0])).toEqual(["European Robin", "Common Blackbird"]);
  expect(boxes[1].species.map((s: [string, number]) => s[0])).toEqual(["Common Blackbird"]);
  await expect.poll(() => limePixels(page)).toBeGreaterThan(50);
  // A lower minimum keeps the third box.
  await settings.getByLabel("Minimum box confidence").fill("0.15");
  await expect(row).toContainText("Bird sounds: 3 boxes");
  await settings.getByRole("button", { name: "Call", exact: true }).click();
  await expect(page.locator(".cd-in-force")).toContainText("One box per call");
  await expect(page.locator(".cd-in-force")).toContainText("Min box confidence 0.15");
  // The row shows the best box and opens its card.
  await row.click();
  await expect.poll(() => limePixels(page)).toBeGreaterThan(50);
  // The setting is saved; turning it off hides the boxes.
  expect(await page.evaluate(() => localStorage.getItem("oversample_bird_boxes"))).toBe("true");
  await settings.getByLabel("Draw a box round each bird sound").uncheck();
  await expect(row).toHaveCount(0);
  await expect.poll(() => limePixels(page)).toBe(0);
});

test("a file analysed without bird boxes asks for another Analyse", async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem("oversample_bird_boxes", "true"));
  await openFile(page);
  await pickMethod(page, "BirdNET+ V3.0");
  // With the box model missing, the download includes it: 72 MB + 21 MB.
  await expect(page.locator(".detection-primary-btn")).toHaveText("Download model (93 MB)");
  const seedErr = await page.evaluate(
    (a) => (window as unknown as { __oversample_classifier: Hook }).__oversample_classifier.seed(a),
    V3_SEED,
  );
  expect(seedErr).toBe("");
  await expect(page.locator(".cd-recording")).toContainText("Analyse again to add bird sound boxes to this file.");
  // No boxes found: the row says so and has no best confidence.
  const err = await page.evaluate(
    () => (window as unknown as { __oversample_classifier: Hook }).__oversample_classifier.seedBoxes('{"raw":[],"samples":384000}'),
  );
  expect(err).toBe("");
  await expect(page.locator(".cd-recording")).not.toContainText("Analyse again to add bird sound boxes");
  const row = page.locator(".cd-list tr", { hasText: "Bird sounds" });
  await expect(row).toContainText("Bird sounds: 0 boxes");
  await expect(row.locator(".detection-col-num")).toHaveText("–");
});

test("Birds and bats is the default and lists bird boxes by species, whatever Find bird boxes is set to", async ({
  page,
}) => {
  await page.addInitScript(() => localStorage.removeItem("oversample_bird_boxes"));
  await openFile(page);
  await expect(page.locator(".cd-method-name")).toHaveText("Birds and bats");
  // A 48 kHz file: BirdNET+ V3.0 and BirdBox only, no BattyBirdNET.
  await expect(page.locator(".cd-in-force")).toContainText("BirdNET+ V3.0 with BirdBox");
  await expect(page.locator(".cd-in-force")).not.toContainText("BattyBirdNET");
  await expect(page.locator(".detection-region")).toHaveCount(0);
  await expect(page.locator(".detection-primary-btn")).toHaveText("Download models (93 MB)");
  const errs = await page.evaluate(
    ([a, b]) => {
      const h = (window as unknown as { __oversample_classifier: Hook }).__oversample_classifier;
      return [h.seed(a), h.seedBoxes(b)];
    },
    [V3_SEED, BOX_SEED],
  );
  expect(errs).toEqual(["", ""]);
  const list = page.locator(".cd-recording .cd-list");
  await expect(list.locator("tr.cd-group-row").first()).toHaveText("Birds");
  const robin = list.locator("tr", { hasText: "European Robin" });
  await expect(robin).toContainText("1 box");
  await expect(robin).toContainText("0.60");
  await expect(list.locator("tr", { hasText: "Bird sp." })).toContainText("1 box");
  await expect(page.locator(".cd-recording .cd-head-fact")).toHaveText("1 species");
  await expect.poll(() => limePixels(page)).toBeGreaterThan(50);
  // The same results in full under BirdNET+ V3.0.
  await pickMethod(page, "BirdNET+ V3.0");
  await expect(page.locator(".cd-list tr", { hasText: "European Robin" })).toBeVisible();
});

test("Birds and bats lists BirdNET+ V3.0's birds by window when no boxes were looked for", async ({ page }) => {
  await openFile(page);
  const err = await page.evaluate(
    (a) => (window as unknown as { __oversample_classifier: Hook }).__oversample_classifier.seed(a),
    V3_SEED,
  );
  expect(err).toBe("");
  const list = page.locator(".cd-recording .cd-list");
  await expect(list.locator("tr", { hasText: "Common Blackbird" })).toContainText("2 windows");
  await expect(list.locator("tr", { hasText: "European Robin" })).toContainText("1 window");
  // Not analysed for boxes yet, so Analyse is still the main action.
  await expect(page.locator(".detection-primary-btn")).toHaveText("Download models (93 MB)");
});
