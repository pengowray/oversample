import { test, expect, type Page } from "@playwright/test";
import { sweepWav } from "./xform_util";

/**
 * The TE, PS and ZC settings panels (src/components/mode_button.rs): the
 * Auto button, the factor presets, the typed factor's limit, and the Play
 * tones, Play noise and Reset buttons under "Fine output adjustment".
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

const factorField = (page: Page) => page.locator(".layer-panel .factor-input").first();
const auto = (page: Page) => page.locator(".layer-panel .auto-toggle").first();
const preset = (page: Page, label: string) =>
  page.locator(".layer-panel .factor-preset", { hasText: new RegExp(`^${label}$`) });
const footer = (page: Page, name: string) =>
  page.locator(".fine-out-btn", { hasText: new RegExp(`^${name}$`) });

for (const mode of ["TE", "PS"]) {
  test(`${mode}: Auto sets the factor as soon as it is pressed`, async ({ page }) => {
    await openSweep(page);
    await openSection(page, mode);
    await expect(page.locator(".layer-panel .factor-preset")).toHaveText([
      "4x", "8x", "10x", "16x", "32x",
    ]);
    await expect(auto(page)).toHaveText("Auto");
    await expect(auto(page)).toHaveClass(/\bon\b/);
    const autoFactor = await factorField(page).inputValue();

    // A preset the Auto factor is not: Auto goes off and the factor is the preset's.
    const other = autoFactor === "32" ? "4" : "32";
    await preset(page, `${other}x`).click();
    await expect(factorField(page)).toHaveValue(other);
    await expect(auto(page)).not.toHaveClass(/\bon\b/);
    await expect(preset(page, `${other}x`)).toHaveClass(/\bsel\b/);

    await auto(page).click();
    await expect(auto(page)).toHaveClass(/\bon\b/);
    await expect(factorField(page)).toHaveValue(autoFactor);
  });

  test(`${mode}: Reset returns to Auto and is off when nothing is changed`, async ({ page }) => {
    await openSweep(page);
    await openSection(page, mode);
    const autoFactor = await factorField(page).inputValue();
    await expect(footer(page, "Reset")).toBeDisabled();
    await preset(page, autoFactor === "32" ? "4x" : "32x").click();
    await expect(footer(page, "Reset")).toBeEnabled();
    await footer(page, "Reset").click();
    await expect(factorField(page)).toHaveValue(autoFactor);
    await expect(auto(page)).toHaveClass(/\bon\b/);
    await expect(footer(page, "Reset")).toBeDisabled();
  });
}

test("TE: a typed factor goes up to 256", async ({ page }) => {
  await openSweep(page);
  await openSection(page, "TE");
  await factorField(page).fill("256");
  await factorField(page).press("Enter");
  await expect(factorField(page)).toHaveValue("256");
  await expect(page.locator(".fine-out-factor")).toHaveText("÷256");
  await factorField(page).fill("999");
  await factorField(page).press("Enter");
  await expect(factorField(page)).toHaveValue("256");
});

test("ZC: Reset returns the division to 8", async ({ page }) => {
  await openSweep(page);
  await openSection(page, "ZC");
  await expect(footer(page, "Reset")).toBeDisabled();
  await page.locator(".layer-panel .layer-panel-slider-row input[type=range]").first().fill("20");
  await expect(page.locator(".fine-out-factor")).toHaveText("÷20");
  await footer(page, "Reset").click();
  await expect(page.locator(".fine-out-factor")).toHaveText("÷8");
});

test("Play tones and Play noise each play a short sound in the heard band", async ({ page }) => {
  // Records every buffer that is started: its length and its loudest sample.
  await page.addInitScript(() => {
    const started: { seconds: number; peak: number }[] = [];
    (window as unknown as { __started: typeof started }).__started = started;
    const start = AudioBufferSourceNode.prototype.start;
    AudioBufferSourceNode.prototype.start = function (...args) {
      if (this.buffer) {
        const data = this.buffer.getChannelData(0);
        let peak = 0;
        for (const v of data) peak = Math.max(peak, Math.abs(v));
        started.push({ seconds: this.buffer.duration, peak });
      }
      return start.apply(this, args as []);
    };
  });
  await openSweep(page);
  await openSection(page, "TE");
  const started = () =>
    page.evaluate(() => (window as unknown as { __started: { seconds: number; peak: number }[] }).__started);

  await footer(page, "Play tones").click();
  await expect.poll(async () => (await started()).length).toBe(1);
  await footer(page, "Play noise").click();
  await expect.poll(async () => (await started()).length).toBe(2);
  for (const sound of await started()) {
    expect(sound.seconds).toBeGreaterThan(1);
    expect(sound.seconds).toBeLessThan(2);
    expect(sound.peak).toBeGreaterThan(0.15);
    expect(sound.peak).toBeLessThanOrEqual(0.2001);
  }
});
