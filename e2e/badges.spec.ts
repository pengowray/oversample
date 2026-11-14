import { test, expect, type Page } from "@playwright/test";

/**
 * File badges: the heading's badge row (.toolbar-badge-row) and the file list
 * rows (.file-item-info). Facts are plain text, controls are boxes
 * (style.css, "File badges").
 */

/** Mono WAV of a quiet 4 kHz tone: 16-bit, 24-bit or 32-bit float. */
function wav(rate: number, seconds: number, format: "16" | "24" | "float" = "16"): Buffer {
  const n = Math.round(rate * seconds);
  const bps = format === "16" ? 2 : format === "24" ? 3 : 4;
  const data = Buffer.alloc(n * bps);
  for (let i = 0; i < n; i++) {
    const s = Math.sin((2 * Math.PI * 4000 * i) / rate) * 0.25;
    if (format === "float") data.writeFloatLE(s, i * 4);
    else if (format === "24") data.writeIntLE(Math.round(s * 8388607), i * 3, 3);
    else data.writeInt16LE(Math.round(s * 32767), i * 2);
  }
  const h = Buffer.alloc(44);
  h.write("RIFF", 0);
  h.writeUInt32LE(36 + data.length, 4);
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
  return Buffer.concat([h, data]);
}

/** Two recordings of a sequence, each in two track files, and one 384 kHz
 *  32-bit float file. */
const LOCAL_FILES = [
  { name: "260305_0058_1-2.wav", buffer: wav(192_000, 1.2, "24") },
  { name: "260305_0058_3.wav", buffer: wav(192_000, 1.2, "24") },
  { name: "260305_0059_1-2.wav", buffer: wav(192_000, 0.9, "24") },
  { name: "260305_0059_3.wav", buffer: wav(192_000, 0.9, "24") },
  { name: "Myotis daubentonii, long name test.wav", buffer: wav(384_000, 1.5, "float") },
];

async function openLocalFiles(page: Page) {
  await page.goto("/");
  await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
  await showFilesSidebar(page);
  await page
    .locator('input[type="file"]')
    .first()
    .setInputFiles(LOCAL_FILES.map((f) => ({ ...f, mimeType: "audio/wav" })));
  await expect(page.locator(".file-item")).toHaveCount(LOCAL_FILES.length, { timeout: 30_000 });
}

async function showFilesSidebar(page: Page) {
  const tab = page.locator("button.toolbar-sidebar-tab-left");
  if (!(await tab.evaluate((e) => e.classList.contains("active")))) await tab.click();
  await expect(page.locator(".sidebar:not(.right-sidebar)")).not.toHaveClass(/collapsed/);
}

function row(page: Page, name: string) {
  return page.locator(".file-item", {
    has: page.locator(".file-item-name", { hasText: name }),
  });
}

test.describe("sequence and track menus in the heading", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test.beforeEach(async ({ page }) => {
    await openLocalFiles(page);
    await row(page, "260305_0059_3").locator(".file-item-name").click();
    await expect(page.locator(".toolbar-title-filename")).toHaveText("260305_0059_3.wav");
  });

  const seqButton = (page: Page) => page.locator(".toolbar-badge-row > button.file-badge-seq");
  const menu = (page: Page) => page.locator(".badge-dropdown-panel");

  test("Escape closes the menu and goes no further", async ({ page }) => {
    // Escape also stops listening or recording (app.rs), so while a menu is
    // open, Escape must not reach the window.
    await page.evaluate(() => {
      (window as any).escapesAtWindow = 0;
      window.addEventListener("keydown", (e) => {
        if (e.key === "Escape") (window as any).escapesAtWindow++;
      });
    });
    await seqButton(page).focus();
    await page.keyboard.press("Enter");
    await expect(menu(page)).toHaveCount(1);
    await page.keyboard.press("Escape");
    await expect(menu(page)).toHaveCount(0);
    expect(await page.evaluate(() => (window as any).escapesAtWindow)).toBe(0);
    // With no menu open, Escape reaches the window as before.
    await page.keyboard.press("Escape");
    expect(await page.evaluate(() => (window as any).escapesAtWindow)).toBe(1);
  });

  test("a press outside the menu closes it", async ({ page }) => {
    await seqButton(page).click();
    await expect(menu(page)).toHaveCount(1);
    await page.locator(".main-view").click({ position: { x: 200, y: 100 } });
    await expect(menu(page)).toHaveCount(0);

    // The track button swaps the sequence menu for the track menu.
    await seqButton(page).click();
    await expect(menu(page)).toHaveCount(1);
    await page.locator(".toolbar-badge-row > button.file-badge-track").click();
    await expect(page.locator(".badge-dropdown-panel .badge-dropdown-item")).toHaveText([
      /^\[1-2\]/,
      /^\[3\]/,
    ]);
  });

  test("the menu's own button still opens and closes it, and its items still work", async ({
    page,
  }) => {
    await seqButton(page).click();
    await expect(menu(page)).toHaveCount(1);
    await seqButton(page).click();
    await expect(menu(page)).toHaveCount(0);

    await seqButton(page).click();
    await page.locator(".badge-dropdown-item", { hasText: "#58" }).click();
    await expect(menu(page)).toHaveCount(0);
    await expect(page.locator(".toolbar-title-filename")).toHaveText("260305_0058_3.wav");
  });

  test("opening a sidebar closes the menu", async ({ page }) => {
    const hideInfo = page.locator('button[title="Hide info panel"]');
    if (await hideInfo.count()) await hideInfo.click();
    await expect(page.locator(".right-sidebar")).toHaveClass(/collapsed/);
    await seqButton(page).click();
    await expect(menu(page)).toHaveCount(1);
    await page.locator('button[title="Show info panel"]').click();
    await expect(page.locator(".right-sidebar")).not.toHaveClass(/collapsed/);
    await expect(menu(page)).toHaveCount(0);
  });
});
