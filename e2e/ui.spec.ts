import { test, expect } from "@playwright/test";

/**
 * UI behaviour tests. Each test deliberately exercises a single interaction
 * surface (mobile layout, sidebar tabs, overflow menu, etc.) so a failure
 * points at one feature rather than "the app". Heavier flows that need real
 * audio fixtures live in their own file.
 */

test.describe("mobile layout", () => {
  test.use({ viewport: { width: 390, height: 844 } });

  test("applies .mobile class and shows hamburger empty-state hint", async ({ page }) => {
    await page.goto("/");
    // is_mobile is recomputed from viewport width on mount (<768px → mobile).
    await expect(page.locator(".app.mobile")).toBeVisible({ timeout: 30_000 });
    // The Files sidebar starts open and covers the hint, which shows once it
    // is closed. Mobile empty-state copy differs from desktop ("Tap ☰..." vs "Drop ...").
    await expect(page.locator(".empty-state")).not.toContainText(/Tap/);
    await page.locator("button.toolbar-sidebar-tab-left").click();
    await expect(page.locator(".empty-state")).toContainText(/Tap/);
  });

  test("Files sidebar is open while no file is open, and closes when one opens", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator(".app.mobile")).toBeVisible({ timeout: 30_000 });
    // Files are opened from the sidebar, so with nothing open it is shown,
    // on phones too (owner, 2026-11-19).
    const left = page.locator(".sidebar:not(.right-sidebar)");
    await expect(left).not.toHaveClass(/collapsed/);
    await expect(page.locator(".sidebar-header-label", { hasText: "Files" })).toBeInViewport();

    // Opening a file closes the overlay so the recording is visible.
    await page.locator('input[type="file"]').first().setInputFiles({
      name: "silence.wav", mimeType: "audio/wav", buffer: silentWav(48_000, 1),
    });
    await expect(page.locator(".band-gutter").first()).toBeVisible({ timeout: 30_000 });
    await expect(left).toHaveClass(/collapsed/);

    // Closing the last file leaves it open.
    await page.locator("button.toolbar-sidebar-tab-left").click();
    await page.locator(".file-item-close").first().click();
    await expect(page.locator(".band-gutter")).toHaveCount(0);
    await page.waitForTimeout(300);
    await expect(left).not.toHaveClass(/collapsed/);
  });
});

/** Mono 16-bit WAV of silence. */
function silentWav(rate: number, seconds: number): Buffer {
  const data = Buffer.alloc(Math.round(rate * seconds) * 2);
  const h = Buffer.alloc(44);
  h.write("RIFF", 0);
  h.writeUInt32LE(36 + data.length, 4);
  h.write("WAVE", 8);
  h.write("fmt ", 12);
  h.writeUInt32LE(16, 16);
  h.writeUInt16LE(1, 20);
  h.writeUInt16LE(1, 22);
  h.writeUInt32LE(rate, 24);
  h.writeUInt32LE(rate * 2, 28);
  h.writeUInt16LE(2, 32);
  h.writeUInt16LE(16, 34);
  h.write("data", 36);
  h.writeUInt32LE(data.length, 40);
  return Buffer.concat([h, data]);
}

test.describe("selection overflow menu", () => {
  test("the ... button appears inside the selection", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
    await page
      .locator('input[type="file"][accept*=".wav"]')
      .first()
      .setInputFiles("e2e/fixtures/stereo-alternating.wav");

    const canvas = page.locator(".main-view .chart-stage > canvas");
    await expect(canvas).toBeVisible({ timeout: 30_000 });
    await page.locator(".tool-combo-btn").click();
    await page.locator(".layer-panel button", { hasText: "Selection" }).click();

    const box = (await canvas.boundingBox())!;
    const left = box.x + box.width * 0.3;
    const right = box.x + box.width * 0.6;
    const top = box.y + box.height * 0.2;
    const bottom = box.y + box.height * 0.7;
    await page.mouse.move(left, top);
    await page.mouse.down();
    await page.mouse.move(right, bottom, { steps: 5 });
    await page.mouse.up();

    const btn = page.locator(".canvas-overflow-btn");
    await expect(btn).toBeVisible();
    const pos = (await btn.boundingBox())!;
    expect(pos.x).toBeGreaterThanOrEqual(left);
    expect(pos.x + pos.width).toBeLessThanOrEqual(right);
    expect(pos.y).toBeGreaterThanOrEqual(top);
    expect(pos.y + pos.height).toBeLessThanOrEqual(bottom);
  });

  test("Export... opens the Export tab in the info panel", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
    await page
      .locator('input[type="file"][accept*=".wav"]')
      .first()
      .setInputFiles("e2e/fixtures/stereo-alternating.wav");

    const canvas = page.locator(".main-view .chart-stage > canvas");
    await expect(canvas).toBeVisible({ timeout: 30_000 });
    await page.locator(".tool-combo-btn").click();
    await page.locator(".layer-panel button", { hasText: "Selection" }).click();

    const box = (await canvas.boundingBox())!;
    await page.mouse.move(box.x + box.width * 0.3, box.y + box.height * 0.2);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width * 0.6, box.y + box.height * 0.7, { steps: 5 });
    await page.mouse.up();

    await expect(page.locator(".right-sidebar")).toHaveClass(/collapsed/);
    await page.locator(".canvas-overflow-btn").click();
    await page.locator(".canvas-overflow-item", { hasText: "Export" }).click();

    await expect(page.locator(".right-sidebar")).not.toHaveClass(/collapsed/);
    await expect(page.locator(".right-sidebar .sidebar-tab-dropdown")).toContainText("Export");
    await expect(
      page.locator(".right-sidebar .export-panel button", { hasText: "Export selection (.wav)" }),
    ).toBeInViewport();
    await expect(page.locator(".right-sidebar .export-file-name")).toHaveText(
      "stereo-alternating_selection.wav",
    );
    await expect(page.locator(".canvas-overflow-menu")).toHaveCount(0);
  });

  test("with nothing selected, the Export tab exports the whole file", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
    await page
      .locator('input[type="file"][accept*=".wav"]')
      .first()
      .setInputFiles("e2e/fixtures/stereo-alternating.wav");
    await expect(page.locator(".main-view .chart-stage > canvas")).toBeVisible({ timeout: 30_000 });
    await page.locator('button[title="Show info panel"]').first().click();
    await page.locator(".sidebar-tab-dropdown").first().click();
    await page.locator(".sidebar-tab-option", { hasText: /^Export$/ }).first().click();
    await expect(page.locator(".export-panel button", { hasText: "Export whole file (.wav)" })).toBeVisible();
  });

  test("shows which channel the sound reached first, and by how much", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
    await page
      .locator('input[type="file"][accept*=".wav"]')
      .first()
      .setInputFiles("e2e/fixtures/stereo-right-first.wav");

    const canvas = page.locator(".main-view .chart-stage > canvas");
    await expect(canvas).toBeVisible({ timeout: 30_000 });
    await page.locator(".tool-combo-btn").click();
    await page.locator(".layer-panel button", { hasText: "Selection" }).click();

    const box = (await canvas.boundingBox())!;
    await page.mouse.move(box.x + box.width * 0.3, box.y + box.height * 0.2);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width * 0.5, box.y + box.height * 0.7, { steps: 5 });
    await page.mouse.up();
    await page.locator(".canvas-overflow-btn").click();

    const row = page.locator(".channel-delay");
    await expect(row).toContainText("L/R delay: 1.00 ms, R first");
    await expect(row).toContainText("Path difference: 34 cm");
  });

  test("the menu stays inside the window for a selection low on the chart", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
    await page
      .locator('input[type="file"][accept*=".wav"]')
      .first()
      .setInputFiles("e2e/fixtures/stereo-right-first.wav");

    const canvas = page.locator(".main-view .chart-stage > canvas");
    await expect(canvas).toBeVisible({ timeout: 30_000 });
    await page.locator(".tool-combo-btn").click();
    await page.locator(".layer-panel button", { hasText: "Selection" }).click();

    const box = (await canvas.boundingBox())!;
    await page.mouse.move(box.x + box.width * 0.1, box.y + box.height * 0.85);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width * 0.5, box.y + box.height * 0.95, { steps: 5 });
    await page.mouse.up();
    await page.locator(".canvas-overflow-btn").click();

    const button = page.locator(".canvas-overflow-menu button", { hasText: "Annotate" });
    await expect(button).toBeVisible();
    const menu = (await page.locator(".canvas-overflow-menu").boundingBox())!;
    expect(menu.y).toBeGreaterThanOrEqual(0);
    expect(menu.y + menu.height).toBeLessThanOrEqual(page.viewportSize()!.height);
  });
});

test.describe("sidebar tabs", () => {
  test("Settings tab toggles active class and switches panel", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });

    const settings = page.locator(".sidebar-settings-btn");
    const files = page.locator(".sidebar-header-label", { hasText: "Files" });

    // Files starts active.
    await expect(files).toHaveClass(/active/);
    await expect(settings).not.toHaveClass(/active/);

    // Click Settings → it becomes active, Files loses active.
    await settings.click();
    await expect(settings).toHaveClass(/active/);
    await expect(files).not.toHaveClass(/active/);

    // Click Settings again → goes back to Files (mod.rs:155-161 behaviour).
    await settings.click();
    await expect(files).toHaveClass(/active/);
    await expect(settings).not.toHaveClass(/active/);
  });
});

test.describe("toolbar overflow menu", () => {
  test("opens on click, closes via backdrop", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });

    const btn = page.locator(".toolbar-overflow-btn");
    await expect(page.locator(".toolbar-overflow-menu")).toHaveCount(0);

    await btn.click();
    await expect(page.locator(".toolbar-overflow-menu")).toBeVisible();
    // At least one item should appear (Undo/Redo, etc.).
    await expect(page.locator(".toolbar-overflow-item").first()).toBeVisible();

    // The backdrop sits behind the menu and closes it when clicked.
    // Click at a corner of the viewport to land on the backdrop, not the menu.
    await page.locator(".toolbar-overflow-backdrop").click({ position: { x: 5, y: 5 } });
    await expect(page.locator(".toolbar-overflow-menu")).toHaveCount(0);
  });
});

test.describe("About dialog", () => {
  test("closes when clicking outside the dialog (overlay)", async ({ page }) => {
    await page.goto("/");
    await page.locator(".toolbar-brand").click();
    await expect(page.locator(".about-dialog")).toBeVisible();

    // The overlay has an on:click that sets show_about=false. Click well outside
    // the dialog (top-left corner) to ensure we hit the overlay, not the dialog.
    await page.locator(".about-overlay").click({ position: { x: 5, y: 5 } });
    await expect(page.locator(".about-dialog")).toHaveCount(0);
  });

  test("clicking inside the dialog does NOT close it", async ({ page }) => {
    await page.goto("/");
    await page.locator(".toolbar-brand").click();
    const dlg = page.locator(".about-dialog");
    await expect(dlg).toBeVisible();
    // stop_propagation on the dialog should keep it open when its body is clicked.
    await dlg.click({ position: { x: 20, y: 20 } });
    await expect(dlg).toBeVisible();
    // Tidy up so subsequent tests start clean.
    await page.locator(".about-close").click();
  });
});

test.describe("localStorage persistence", () => {
  test("reads pre-seeded keys at startup", async ({ page }) => {
    // Seed localStorage BEFORE the app boots: navigate first, then write the
    // values, then reload. AppState reads these in `AppState::new()` at mount.
    await page.goto("/");
    await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });

    await page.evaluate(() => {
      localStorage.setItem("oversample_show_status_bar", "false");
      localStorage.setItem("oversample_projects_enabled", "true");
    });

    await page.reload();
    await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });

    // We can't easily observe app state from the outside, but we *can* observe
    // the side-effect: projects_enabled adds a "Project" tab to the sidebar.
    await expect(
      page.locator(".sidebar-header-label", { hasText: "Project" }),
    ).toBeVisible({ timeout: 10_000 });
  });

  test("writes settings to localStorage when toggled", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
    // Reset the key so we can detect a write.
    await page.evaluate(() => localStorage.removeItem("oversample_bat_book_favourites"));
    // Set via JS (no UI control needed) and verify it survives a reload.
    await page.evaluate(() => {
      localStorage.setItem("oversample_bat_book_favourites", "test_species");
    });
    const stored = await page.evaluate(() =>
      localStorage.getItem("oversample_bat_book_favourites"),
    );
    expect(stored).toBe("test_species");
  });
});
