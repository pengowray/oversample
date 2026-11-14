import { test, expect, type Page } from "@playwright/test";

/**
 * The XForm switch in the view bar and the rows of its Display Processing
 * menu (src/components/xform_button.rs).
 */

/** Mono 16-bit WAV of FM sweeps from 70 kHz down to 30 kHz. */
function sweepWav(rate: number, seconds: number): Buffer {
  const n = Math.round(rate * seconds);
  const data = Buffer.alloc(n * 2);
  const period = Math.round(0.06 * rate);
  const len = Math.round(0.008 * rate);
  let phase = 0;
  for (let i = 0; i < n; i++) {
    const k = i % period;
    let s = 0;
    if (k < len) {
      const u = k / len;
      phase += (2 * Math.PI * 70_000 * Math.pow(30 / 70, u)) / rate;
      s = Math.sin(phase) * 0.5 * Math.sin(Math.PI * u);
    } else {
      phase = 0;
    }
    data.writeInt16LE(Math.round(s * 32767), i * 2);
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

const ROWS = ["EQ", "Notch", "NR", "Xform", "Gain", "Resam"];

async function openRecording(page: Page) {
  await page.goto("/");
  await page
    .locator('input[type="file"][accept*=".wav"]')
    .first()
    .setInputFiles({ name: "sweeps.wav", mimeType: "audio/wav", buffer: sweepWav(192_000, 0.5) });
  await expect(page.locator("canvas").first()).toBeVisible();
}

const xformSwitch = (page: Page) => page.getByRole("switch", { name: "XForm" });

/** One row of the Display Processing menu, by its label. */
const row = (page: Page, label: string) =>
  page.locator(".dsp-filter-row", {
    has: page.locator(".dsp-filter-label", { hasText: new RegExp(`^${label}$`) }),
  });

/** The setting each row has selected, in ROWS order. */
async function selected(page: Page): Promise<string[]> {
  const out: string[] = [];
  for (const label of ROWS) {
    out.push((await row(page, label).locator("button.sel").innerText()).trim());
  }
  return out;
}

test("switching XForm on for the first time sets every row to sam", async ({ page }) => {
  await openRecording(page);
  await xformSwitch(page).click();
  await expect(xformSwitch(page)).toHaveAttribute("aria-checked", "true");
  await page.getByTitle("Display Processing settings").click();
  expect(await selected(page)).toEqual(["sam", "sam", "sam", "sam", "sam", "sam"]);
});

test("the rows keep their settings when XForm is switched off and on", async ({ page }) => {
  await openRecording(page);
  await xformSwitch(page).click();
  await page.getByTitle("Display Processing settings").click();

  await row(page, "NR").getByRole("button", { name: "off" }).click();
  await row(page, "Gain").getByRole("button", { name: "aut" }).click();
  await row(page, "Resam").getByRole("button", { name: "off" }).click();
  const chosen = ["sam", "sam", "off", "sam", "aut", "off"];
  expect(await selected(page)).toEqual(chosen);

  await xformSwitch(page).click();
  await expect(xformSwitch(page)).toHaveAttribute("aria-checked", "false");
  expect(await selected(page)).toEqual(chosen);

  await xformSwitch(page).click();
  await expect(xformSwitch(page)).toHaveAttribute("aria-checked", "true");
  expect(await selected(page)).toEqual(chosen);
});
