import { test, expect, type Page } from "@playwright/test";
import { sweepWav } from "./xform_util";

/**
 * "Fine output adjustment" in the HET, TE, PS and ZC settings panels
 * (src/components/fine_output.rs), which replaced the Hear bar's OUT button.
 */

async function openSweep(page: Page) {
  await page.goto("/");
  await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
  await page.locator('input[type="file"]').first().setInputFiles({
    name: "sweep-384.wav",
    mimeType: "audio/wav",
    buffer: sweepWav(384_000, 2),
  });
  await expect(page.locator(".band-gutter").first()).toBeVisible({ timeout: 30_000 });
}

/** Selects a playback mode, opens its settings panel and the section in it. */
async function openSection(page: Page, mode: string) {
  const button = page.locator(".mode-radio-btn", { hasText: mode }).first();
  const head = page.locator(".fine-out-head");
  await button.click();
  if (!(await head.isVisible())) await button.click();
  await expect(head).toBeVisible();
  if ((await head.getAttribute("aria-expanded")) !== "true") await head.click();
  await expect(page.locator(".fine-out-svg")).toBeVisible();
}

/** Drags the nth handle of the heard band by `dy` pixels, in steps. */
async function dragHandle(page: Page, nth: number, dy: number) {
  const box = (await page.locator(".fine-out-handle .fine-out-knob").nth(nth).boundingBox())!;
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  for (let i = 1; i <= 8; i++) await page.mouse.move(x, y + (dy * i) / 8);
  return { release: () => page.mouse.up() };
}

const factorField = (page: Page) => page.locator(".layer-panel .factor-input").first();

test("the OUT button is gone and each mode's panel has the section", async ({ page }) => {
  await openSweep(page);
  await expect(page.locator(".out-range-btn")).toHaveCount(0);
  for (const mode of ["HET", "TE", "PS", "ZC"]) {
    await openSection(page, mode);
    await expect(page.locator(".fine-out-head")).toHaveText("Fine output adjustment");
    await expect(page.locator(".fine-out-col")).toHaveText(["RECORDING", "HEARD"]);
    await page.keyboard.press("Escape");
  }
});

test("PS: the high handle changes the factor, and Snap starts at Free", async ({ page }) => {
  await openSweep(page);
  await openSection(page, "PS");
  await expect(page.locator(".fine-out-snap select")).toHaveValue("Free");
  const before = await factorField(page).inputValue();
  const label = page.locator(".fine-out-value").first();
  const heardBefore = await label.textContent();
  const drag = await dragHandle(page, 0, 40);
  // The label follows the pointer before the pointer is released.
  await expect(label).not.toHaveText(heardBefore!);
  await drag.release();
  await expect(factorField(page)).not.toHaveValue(before);
});

test("PS: Divide and shift lets the low handle move alone", async ({ page }) => {
  await openSweep(page);
  await openSection(page, "PS");
  await page.getByRole("button", { name: "Divide and shift" }).click();
  const high = page.locator(".fine-out-value").first();
  const highBefore = await high.textContent();
  await expect(page.locator(".fine-out-shift")).toHaveText("");
  const drag = await dragHandle(page, 1, 12);
  await drag.release();
  await expect(page.locator(".fine-out-shift")).toHaveText(/^−\d/);
  await expect(high).toHaveText(highBefore!);
  await expect(page.locator(".fine-out-row", { hasText: "then lower by" })).toBeVisible();
  // "Divide only" takes the shift off again.
  await page.getByRole("button", { name: "Divide only" }).click();
  await expect(page.locator(".fine-out-shift")).toHaveText("");
});

test("a drag shows the XForm view and leaves it afterwards", async ({ page }) => {
  await openSweep(page);
  await openSection(page, "PS");
  const xform = page.locator(".switch-btn", { hasText: "XForm" }).locator('[role="switch"]');
  await expect(xform).toHaveAttribute("aria-checked", "false");
  const drag = await dragHandle(page, 0, 30);
  await expect(xform).toHaveAttribute("aria-checked", "true");
  await drag.release();
  await expect(xform).toHaveAttribute("aria-checked", "false", { timeout: 5_000 });
});

test("HET draws one band per carrier and the handle sets the LP cutoff", async ({ page }) => {
  await openSweep(page);
  await openSection(page, "HET");
  const carriers = await page.locator(".layer-panel .factor-preset.auto-derived, .layer-panel .factor-preset.sel").first().textContent();
  await expect(page.locator(".fine-out-svg text.fine-out-in")).toHaveCount(Number(carriers));
  const cutoff = page.locator(".het-value").first();
  const before = await cutoff.textContent();
  const drag = await dragHandle(page, 0, 30);
  await drag.release();
  await expect(cutoff).not.toHaveText(before!);
});

test("TE and ZC: the handle changes the factor", async ({ page }) => {
  await openSweep(page);
  await openSection(page, "TE");
  const before = await factorField(page).inputValue();
  await (await dragHandle(page, 0, 40)).release();
  await expect(factorField(page)).not.toHaveValue(before);
  await page.keyboard.press("Escape");

  await openSection(page, "ZC");
  const division = page.locator(".layer-panel .layer-panel-slider-row span").first();
  const zcBefore = await division.textContent();
  await (await dragHandle(page, 0, 40)).release();
  await expect(division).not.toHaveText(zcBefore!);
  await expect(division).toHaveText(/^÷\d+$/);
});

test("under XForm, the old picture stays up until the redrawn one arrives", async ({ page }) => {
  await openSweep(page);
  await openSection(page, "PS");
  const freeze = page.locator(".spectrogram-container .fine-out-freeze");
  const knob = (await page.locator(".fine-out-handle .fine-out-knob").first().boundingBox())!;
  const x = knob.x + knob.width / 2;
  await page.mouse.move(x, knob.y + 4);
  await page.mouse.down();
  // The first rest writes the factor with XForm just turned on; the second
  // changes it again, which redraws the XForm picture.
  await page.mouse.move(x, knob.y + 20, { steps: 4 });
  await page.waitForTimeout(400);
  await page.mouse.move(x, knob.y + 40, { steps: 4 });
  await expect(freeze).toHaveCount(1);
  await page.mouse.up();
  await expect(freeze).toHaveCount(0, { timeout: 6_000 });
});
