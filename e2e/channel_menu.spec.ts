import { test, expect, type Page } from "@playwright/test";

/**
 * The Channels list of the mic menu, shown for a device with more than 2
 * channels. No browser has such a device, so the tests call
 * `window.__oversample_fake_wide_mic` (src/test_hook.rs), which makes the mic
 * a USB device with the given channel count, channel names and X8 input
 * report. No audio is opened.
 */

const X8_NAMES = [
  "Mix L", "Mix R",
  "Input 1: INT MIC", "Input 2: INT MIC",
  "Input 3: EXT MIC", "Input 4: EXT MIC",
  "Input 5: LINE IN", "Input 6: EXT MIC",
];

// A version 1 report from a patched X8: INT MIC on inputs 1 and 2, EXT MIC on
// 3, 4 and 6, LINE IN on 5, phantom power on for inputs 3, 4 and 6, 48 V.
const X8_REPORT = [
  0x58, 0x38, 1, 28, 0x21, 0x01, 0x01, 0,
  1, 1, 3, 3, 4, 3,
  1, 1, 0, 1,
  1, 1, 2, 2, 3, 3,
  0x3f, 3, 3, 0,
];

async function wideMic(page: Page, name: string, channels: number, names: string[], report: number[] | null) {
  await page.goto("/");
  await page.waitForFunction(() => typeof (window as any).__oversample_fake_wide_mic === "function", null, { timeout: 30_000 });
  await page.evaluate(
    ([n, c, ns, r]) => (window as any).__oversample_fake_wide_mic(n, c, ns, r),
    [name, channels, names, report] as const,
  );
  // On a phone the Files sidebar is open over the toolbar until it is closed.
  if ((page.viewportSize()?.width ?? 1280) < 768) {
    await expect(page.locator(".app.mobile")).toBeVisible({ timeout: 30_000 });
    await page.locator("button.toolbar-sidebar-tab-left").click();
  }
  await page.locator('button[title^="Microphone options"]').click();
  await expect(page.locator(".channel-menu-btn")).toBeVisible();
}

const rows = (page: Page) =>
  page.locator(".channel-menu .channel-opt").evaluateAll((els) =>
    els.map((e) => (e as HTMLElement).innerText.replace(/\n/g, " | ")),
  );

test("a patched X8 lists each input with its source, phantom voltage and USB numbers", async ({ page }) => {
  await wideMic(page, "Portacapture X8", 8, X8_NAMES, X8_REPORT);
  const button = page.locator(".channel-menu-btn");
  await expect(button).toContainText("Mix L and R (stereo)");
  await button.click();
  const menu = page.locator(".channel-menu");
  await expect(menu).toBeVisible();
  await expect(menu.locator(".layer-panel-title")).toHaveText(["Stereo", "Mono"]);
  expect(await rows(page)).toEqual([
    "Mix L and R | USB 1, 2",
    "Input 1 and 2 | INT MIC | USB 3, 4",
    "Input 3 and 4 | EXT MIC, 48 V | USB 5, 6",
    "Input 5 and 6 | 5: LINE IN; 6: EXT MIC, 48 V | USB 7, 8",
    "Mix L and R averaged | USB 1, 2",
    "Mix L | USB 1",
    "Mix R | USB 2",
    "Input 1 | INT MIC | USB 3",
    "Input 2 | INT MIC | USB 4",
    "Input 3 | EXT MIC, 48 V | USB 5",
    "Input 4 | EXT MIC, 48 V | USB 6",
    "Input 5 | LINE IN | USB 7",
    "Input 6 | EXT MIC, 48 V | USB 8",
  ]);

  // Choosing a row closes the list, keeps the mic menu open and saves the
  // choice for this device.
  await menu.locator(".channel-opt", { hasText: "Input 3 and 4" }).click();
  await expect(menu).toHaveCount(0);
  await expect(button).toContainText("Input 3 and 4 (stereo)");
  await expect(page.locator(".mic-combo-row .layer-panel")).toBeVisible();
  expect(await page.evaluate(() => localStorage.getItem("oversample_mic_channel_picks")))
    .toBe('{"Portacapture X8":"5,6"}');

  // The chosen row is marked, and a press elsewhere in the mic menu closes
  // the list only.
  await button.click();
  await expect(menu.locator(".channel-opt.sel")).toContainText("Input 3 and 4");
  await page.locator(".mic-combo-row .layer-panel label", { hasText: "Pre-roll buffer" }).click();
  await expect(menu).toHaveCount(0);
  await expect(page.locator(".mic-combo-row .layer-panel")).toBeVisible();
});

test("a device without channel names lists channel numbers", async ({ page }) => {
  await wideMic(page, "Some interface", 5, [], null);
  const button = page.locator(".channel-menu-btn");
  await expect(button).toContainText("Channels 1 and 2 (stereo)");
  await button.click();
  await expect(page.locator(".channel-menu")).toBeVisible();
  expect(await rows(page)).toEqual([
    "Channels 1 and 2",
    "Channels 3 and 4",
    "Channels 1 and 2 averaged",
    "Channel 1", "Channel 2", "Channel 3", "Channel 4", "Channel 5",
  ]);
});

test.describe("on a phone", () => {
  test.use({ viewport: { width: 375, height: 812 } });

  test("the list is drawn over the mic menu, inside the screen", async ({ page }) => {
    await wideMic(page, "Portacapture X8", 8, X8_NAMES, X8_REPORT);
    await page.locator(".channel-menu-btn").click();
    const menu = page.locator(".channel-menu");
    await expect(menu).toBeVisible();
    await expect(menu.locator(".channel-opt").first()).toBeInViewport();
    const box = (await menu.boundingBox())!;
    expect(box.x).toBeGreaterThanOrEqual(0);
    expect(box.x + box.width).toBeLessThanOrEqual(375);
    expect(box.y).toBeGreaterThanOrEqual(0);
    expect(box.y + box.height).toBeLessThanOrEqual(812);
    // The row under the middle of the list is the list's own, not the mic
    // menu behind it.
    const onTop = await page.evaluate(([x, y]) => {
      const el = document.elementFromPoint(x, y);
      return !!el?.closest(".channel-menu");
    }, [box.x + box.width / 2, box.y + box.height / 2] as const);
    expect(onTop).toBe(true);
    // Rows are tall enough for a finger.
    const rowBox = (await menu.locator(".channel-opt").first().boundingBox())!;
    expect(rowBox.height).toBeGreaterThanOrEqual(40);
  });
});
