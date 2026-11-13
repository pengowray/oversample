import { test, expect, type Page } from "@playwright/test";

/**
 * The "Playing 1:1" card and the HFR tutorial (src/components/hfr_tour.rs).
 *
 * Uses a generated 192 kHz WAV with 40 kHz tone bursts, so the card's
 * ultrasonic-recording rule applies, and a 48 kHz one where it must not.
 */

/** Mono 16-bit WAV: `seconds` of 5 ms bursts at `toneHz`, every 100 ms. */
function wav(rate: number, seconds: number, toneHz: number): Buffer {
  const n = Math.round(rate * seconds);
  const data = Buffer.alloc(n * 2);
  for (let i = 0; i < n; i++) {
    const t = i / rate;
    const inBurst = t % 0.1 < 0.005;
    const v = inBurst ? Math.sin(2 * Math.PI * toneHz * t) * 0.5 : 0;
    data.writeInt16LE(Math.round(v * 32767), i * 2);
  }
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

async function openWav(page: Page, rate: number, clearSetting = true) {
  await page.goto("/");
  await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
  if (clearSetting) {
    await page.evaluate(() => localStorage.removeItem("oversample_hfr_nudge_enabled"));
  }
  await page.locator('input[type="file"]').first().setInputFiles({
    name: `tone-${rate}.wav`,
    mimeType: "audio/wav",
    buffer: wav(rate, 2, rate > 48_000 ? 40_000 : 4_000),
  });
  await expect(page.locator(".band-gutter").first()).toBeVisible({ timeout: 30_000 });
}

async function dragOn(page: Page, selector: string, from: [number, number], to: [number, number]) {
  const b = (await page.locator(selector).first().boundingBox())!;
  await page.mouse.move(b.x + b.width * from[0], b.y + b.height * from[1]);
  await page.mouse.down();
  for (let i = 1; i <= 10; i++) {
    const k = i / 10;
    await page.mouse.move(
      b.x + b.width * (from[0] + (to[0] - from[0]) * k),
      b.y + b.height * (from[1] + (to[1] - from[1]) * k),
    );
  }
  await page.mouse.up();
}

const popoverTitle = (page: Page) => page.locator(".driver-popover-title");
const hfrOn = (page: Page) => page.locator(".hearing-bar .band-cell-hfr.hfr-on");

test.describe("HFR card and tutorial", () => {
  test("card shows once when an ultrasonic recording plays 1:1, and Turn on HFR works", async ({ page }) => {
    await openWav(page, 192_000);
    await page.locator(".play-main-btn").click();
    const card = page.locator(".hfr-nudge");
    await expect(card).toBeVisible();
    await expect(card).toContainText("Playing 1:1");
    await card.getByRole("button", { name: "Turn on HFR" }).click();
    await expect(card).toHaveCount(0);
    await expect(hfrOn(page)).toHaveCount(1);

    // Once per session: HFR off and play again shows nothing.
    await page.locator(".hearing-bar .band-cell-hfr").click();
    await page.locator(".play-main-btn").click(); // stop
    await page.locator(".play-main-btn").click(); // play
    await page.waitForTimeout(500);
    await expect(card).toHaveCount(0);
  });

  test("card shows during 1:1 live listening, without the tutorial", async ({ page }) => {
    await page.addInitScript(() => localStorage.removeItem("oversample_hfr_nudge_enabled"));
    // Synthetic mic at 256 kHz: the real listen pipeline, no hardware.
    await page.goto("/#synthtest-pulses-256");
    await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
    const card = page.locator(".hfr-nudge");
    await expect(card).toBeVisible({ timeout: 25_000 });
    await expect(card).toContainText("Playing 1:1");
    // The tutorial ends on pressing Play, so it is unavailable while listening.
    await expect(card.getByRole("button", { name: "HFR tutorial" })).toHaveCount(0);
    await page.getByTitle("Settings", { exact: true }).click();
    const tutorial = page.locator(".setting-button", { hasText: "HFR tutorial" });
    await expect(tutorial).toBeDisabled();
    await expect(tutorial).toHaveAttribute("title", "Stop listening first");

    await card.getByRole("button", { name: "Turn on HFR" }).click();
    await expect(card).toHaveCount(0);
    await expect(hfrOn(page)).toHaveCount(1);
  });

  test("live card closes when listening stops", async ({ page }) => {
    await page.addInitScript(() => localStorage.removeItem("oversample_hfr_nudge_enabled"));
    await page.goto("/#synthtest-pulses-256");
    const card = page.locator(".hfr-nudge");
    await expect(card).toBeVisible({ timeout: 30_000 });
    await page.evaluate(() => (window as unknown as { __synthStop: () => void }).__synthStop());
    await expect(card).toHaveCount(0);
  });

  test("no card for a 48 kHz recording", async ({ page }) => {
    await openWav(page, 48_000);
    await page.locator(".play-main-btn").click();
    await page.waitForTimeout(500);
    await expect(page.locator(".hfr-nudge")).toHaveCount(0);
  });

  test("Don't show again persists and Settings turns it back on", async ({ page }) => {
    await openWav(page, 192_000);
    await page.locator(".play-main-btn").click();
    await page.locator(".hfr-nudge").getByRole("button", { name: "Don't show again" }).click();
    expect(await page.evaluate(() => localStorage.getItem("oversample_hfr_nudge_enabled"))).toBe("false");

    await openWav(page, 192_000, false);
    await page.locator(".play-main-btn").click();
    await page.waitForTimeout(500);
    await expect(page.locator(".hfr-nudge")).toHaveCount(0);

    await page.getByTitle("Settings", { exact: true }).click();
    const box = page.locator("#hfr-nudge-enabled");
    await expect(box).not.toBeChecked();
    await box.check();
    expect(await page.evaluate(() => localStorage.getItem("oversample_hfr_nudge_enabled"))).toBe("true");
  });

  test("tutorial moves on as each step is done", async ({ page }) => {
    await openWav(page, 192_000);
    await page.locator(".play-main-btn").click();
    await page.locator(".hfr-nudge").getByRole("button", { name: "HFR tutorial" }).click();

    await expect(popoverTitle(page)).toHaveText("Turn on HFR", { timeout: 10_000 });
    await page.locator(".hearing-bar .band-cell-hfr").click();
    await expect(hfrOn(page)).toHaveCount(1);

    await expect(popoverTitle(page)).toHaveText("Choose frequencies");
    // Ends to the right of the strip: the release lands outside the cut-out.
    await dragOn(page, ".band-gutter", [0.7, 0.3], [3, 0.6]);

    await expect(popoverTitle(page)).toHaveText("Choose a time range");
    await dragOn(page, ".time-gutter", [0.1, 0.5], [0.5, 0.5]);

    await expect(popoverTitle(page)).toHaveText("Press Play");
    await page.locator(".play-main-btn").click();
    await expect(page.locator(".driver-popover")).toHaveCount(0);
  });

  test("Escape closes the tutorial; Band presets starts it", async ({ page }) => {
    await openWav(page, 192_000);
    await page.locator(".hearing-bar .range-btn").click();
    await page.getByRole("button", { name: "HFR tutorial" }).click();
    await expect(popoverTitle(page)).toHaveText("Turn on HFR", { timeout: 10_000 });
    await page.keyboard.press("Escape");
    await expect(page.locator(".driver-popover")).toHaveCount(0);
  });
});
