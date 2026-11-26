import { test, expect, type Page } from "@playwright/test";

/**
 * The "quiet playback" card (src/components/gain_nudge.rs): shows above
 * Play when what plays with gain off is very quiet, and turns on Gain.
 *
 * Uses generated 48 kHz WAVs with a 1 kHz tone, so the HFR card's
 * ultrasonic rule never applies.
 */

/** Mono 16-bit WAV: `seconds` of a 1 kHz tone at `amp` (0 to 1). */
function wav(seconds: number, amp: number): Buffer {
  return wavOf(seconds, (i, rate) => Math.sin((2 * Math.PI * 1000 * i) / rate) * amp);
}

/** Mono 16-bit 48 kHz WAV of `sample(i, rate)` (-1 to 1). */
function wavOf(seconds: number, sample: (i: number, rate: number) => number): Buffer {
  const rate = 48_000;
  const n = Math.round(rate * seconds);
  const data = Buffer.alloc(n * 2);
  for (let i = 0; i < n; i++) {
    data.writeInt16LE(Math.round(sample(i, rate) * 32767), i * 2);
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

/** About −46 dBFS. */
const QUIET = 0.005;

async function openWav(page: Page, amp: number, clearSetting = true, buffer = wav(8, amp)) {
  await page.goto("/");
  await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
  if (clearSetting) {
    await page.evaluate(() => localStorage.removeItem("oversample_gain_nudge_enabled"));
  }
  await page.locator('input[type="file"]').first().setInputFiles({
    name: `tone-${amp}.wav`,
    mimeType: "audio/wav",
    buffer,
  });
  await expect(page.locator(".band-gutter").first()).toBeVisible({ timeout: 30_000 });
}

const card = (page: Page) => page.locator(".gain-nudge");
async function testState(page: Page): Promise<Record<string, unknown>> {
  return page.evaluate(() =>
    (window as unknown as { __oversample_test: () => Record<string, unknown> }).__oversample_test(),
  );
}

test.describe("Quiet playback card", () => {
  test("shows for quiet playback with the peak level, and Turn on Gain works", async ({ page }) => {
    await openWav(page, QUIET);
    await page.locator(".play-main-btn").click();
    await expect(card(page)).toBeVisible({ timeout: 10_000 });
    await expect(card(page)).toContainText("Playback is very quiet (peak level −46 dB). Gain is off.");
    await card(page).getByRole("button", { name: "Turn on Gain" }).click();
    await expect(card(page)).toHaveCount(0);
    await expect(page.locator(".hearing-bar")).toContainText(/Peak \+4\d dB/);

    // Once per session: with gain off again, playing again shows nothing.
    await page.locator(".play-main-btn").click(); // stop
    await page.locator(".hearing-bar").getByTitle("Turn playback gain on or off").click();
    await page.locator(".play-main-btn").click(); // play
    await page.waitForTimeout(4_000);
    await expect(card(page)).toHaveCount(0);
  });

  test("no card for loud playback", async ({ page }) => {
    await openWav(page, 0.5);
    await page.locator(".play-main-btn").click();
    await expect.poll(async () => (await testState(page)).outputPeakDb, { timeout: 10_000 }).not.toBeNull();
    expect((await testState(page)).outputPeakDb as number).toBeGreaterThan(-7);
    await expect(card(page)).toHaveCount(0);
  });

  test("the level is the waveform's peak between the samples", async ({ page }) => {
    // A 12 kHz tone sampled 45 degrees off its crests, scaled so the samples
    // reach full scale: the waveform through them peaks 3 dB higher.
    const tone = wavOf(8, (i) => Math.sin((Math.PI / 2) * i + Math.PI / 4) / Math.SQRT1_2);
    await openWav(page, 1, true, tone);
    await page.locator(".play-main-btn").click();
    await expect.poll(async () => (await testState(page)).outputPeakDb, { timeout: 10_000 }).not.toBeNull();
    expect((await testState(page)).outputPeakDb as number).toBeCloseTo(3.0, 0);
  });

  test("no card for silence", async ({ page }) => {
    await openWav(page, 0);
    await page.locator(".play-main-btn").click();
    await page.waitForTimeout(4_000);
    await expect(card(page)).toHaveCount(0);
  });

  test("Don't show again persists and Settings turns it back on", async ({ page }) => {
    await openWav(page, QUIET);
    await page.locator(".play-main-btn").click();
    await card(page).getByRole("button", { name: "Don't show again" }).click();
    await expect(card(page)).toHaveCount(0);
    expect(await page.evaluate(() => localStorage.getItem("oversample_gain_nudge_enabled"))).toBe("false");

    await openWav(page, QUIET, false);
    await page.locator(".play-main-btn").click();
    await page.waitForTimeout(4_000);
    await expect(card(page)).toHaveCount(0);

    await page.getByTitle("Settings", { exact: true }).click();
    const box = page.locator("#gain-nudge-enabled");
    await expect(box).not.toBeChecked();
    await box.check();
    expect(await page.evaluate(() => localStorage.getItem("oversample_gain_nudge_enabled"))).toBe("true");
  });
});
