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
  await page.locator(".cd-method-opt", { hasText: name }).click();
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

test("a pin shows the species in a row above the spectrogram", async ({ page }) => {
  await openFile(page);
  await pickMethod(page, "BirdNET V2.4");
  await seed(page);
  const row = page.locator(".cd-list tr", { hasText: "Common Blackbird" });
  await expect(row).toBeVisible();
  await expect(page.locator(".cd-recording .cd-head-fact")).toHaveText("2 species, 4 windows");
  const pin = row.locator(".cd-pin");
  await expect(pin).toHaveAttribute("aria-pressed", "false");
  await expect(pin).toHaveAttribute("aria-label", "Show Common Blackbird above the spectrogram");
  const strips = page.locator(".score-strips-row");
  await expect(strips).toBeHidden();
  await pin.click();
  await expect(pin).toHaveAttribute("aria-pressed", "true");
  await expect(pin).toHaveAttribute("aria-label", "Remove Common Blackbird from above the spectrogram");
  await expect(strips).toBeVisible();
  expect(await page.evaluate(() => localStorage.getItem("oversample_score_strips"))).toContain("Turdus merula");
  // A species added by the search is listed too, and its pin removes it.
  await page.locator(".strip-search-input").fill("song thr");
  await expect(page.locator(".strip-search-empty")).toHaveText("No species found");
  await page.locator(".strip-search-input").fill("engine");
  await page.locator(".strip-search-option", { hasText: "Engine" }).dispatchEvent("mousedown");
  await expect(page.locator(".cd-list tr", { hasText: "Engine" }).locator(".cd-pin")).toHaveAttribute("aria-pressed", "true");
  await page.locator(".cd-list tr", { hasText: "Engine" }).locator(".cd-pin").click();
  await expect(page.locator(".cd-list tr", { hasText: "Engine" })).toHaveCount(0);
  // Unpin: the row above the spectrogram goes.
  await pin.click();
  await expect(pin).toHaveAttribute("aria-pressed", "false");
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
