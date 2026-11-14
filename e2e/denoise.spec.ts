import { test, expect, type Page } from "@playwright/test";
import fs from "fs";
import os from "os";
import path from "path";

/**
 * The Bat Call Model section of the NR menu: the button, the model download,
 * the progress, and the denoised copy in the file list.
 *
 * The model is not in the repository. The test serves it from a local file in
 * place of the download: DENOISE_MODEL, or the parity folder that
 * batchi-noisier2noise's reference.py writes. Without that file the download
 * test is skipped. The file must be the one the catalog's size and hash name.
 */
const modelPath =
  process.env.DENOISE_MODEL ??
  path.join(os.homedir(), "projects/batchi-noisier2noise/runs/parity/model.onnx");

/** Mono 16-bit WAV: noise with a downward sweep every 100 ms. */
function batLikeWav(sampleRate: number, seconds: number): Buffer {
  const n = Math.floor(sampleRate * seconds);
  const data = Buffer.alloc(n * 2);
  let seed = 12345;
  const rand = () => {
    seed = (seed * 1664525 + 1013904223) >>> 0;
    return seed / 0xffffffff - 0.5;
  };
  const callLen = Math.floor(sampleRate * 0.004);
  let phase = 0;
  for (let i = 0; i < n; i++) {
    let s = rand() * 0.02;
    const inCall = i % Math.floor(sampleRate * 0.1);
    if (inCall < callLen) {
      const u = inCall / callLen;
      const freq = sampleRate * (0.3 - 0.18 * u);
      phase += (2 * Math.PI * freq) / sampleRate;
      s += 0.3 * Math.sin(Math.PI * u) ** 2 * Math.sin(phase);
    }
    data.writeInt16LE(Math.round(Math.max(-1, Math.min(1, s)) * 32767), i * 2);
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

async function openRecording(page: Page, name: string, wav: Buffer) {
  await page.goto("/");
  await page
    .locator('input[type="file"][accept*=".wav"]')
    .first()
    .setInputFiles({ name, mimeType: "audio/wav", buffer: wav });
  await expect(page.locator("canvas").first()).toBeVisible();
  await page.getByRole("button", { name: /^NR/ }).click();
  await expect(page.getByText("Bat Call Model")).toBeVisible();
}

test("a 96 kHz recording cannot be denoised, and the menu says why", async ({ page }) => {
  await openRecording(page, "low-rate.wav", batLikeWav(96_000, 1));
  await expect(page.getByRole("button", { name: "Make Denoised Copy" })).toBeDisabled();
  await expect(
    page.getByText("Needs a sample rate of 192 kHz or more. This recording: 96 kHz."),
  ).toBeVisible();
  await page.screenshot({ path: "test-results/denoise-refused.png" });
});

test("Make Denoised Copy downloads the model and adds the copy to the file list", async ({
  page,
}) => {
  test.skip(!fs.existsSync(modelPath), `no model file at ${modelPath}`);
  test.setTimeout(240_000);
  await page.route("**/oversample-bat-denoise-v1.onnx", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/octet-stream",
      headers: { "access-control-allow-origin": "*" },
      body: fs.readFileSync(modelPath),
    }),
  );
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await openRecording(page, "pass.wav", batLikeWav(384_000, 2.5));
  await expect(page.getByText("Adds a new recording. Downloads a 0.3 MB model first.")).toBeVisible();
  await page.screenshot({ path: "test-results/denoise-idle.png" });

  await page.getByRole("button", { name: "Make Denoised Copy" }).click();
  await expect(page.getByRole("button", { name: "Cancel" })).toBeVisible();
  await expect(page.getByText(/Denoising: \d+%/)).toBeVisible({ timeout: 120_000 });
  await page.screenshot({ path: "test-results/denoise-running.png" });

  await expect(page.getByText("pass (denoised).wav").first()).toBeVisible({ timeout: 200_000 });
  await expect(page.getByRole("button", { name: "Make Denoised Copy" })).toBeVisible();
  // The model is stored now, so the help line no longer mentions the download.
  await expect(page.getByText("Adds a new recording and keeps the original.")).toBeVisible();
  await page.screenshot({ path: "test-results/denoise-done.png" });
  expect(errors).toEqual([]);
});

test("Cancel stops the work and brings the button back", async ({ page }) => {
  test.skip(!fs.existsSync(modelPath), `no model file at ${modelPath}`);
  test.setTimeout(120_000);
  await page.route("**/oversample-bat-denoise-v1.onnx", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/octet-stream",
      headers: { "access-control-allow-origin": "*" },
      body: fs.readFileSync(modelPath),
    }),
  );
  await openRecording(page, "long.wav", batLikeWav(384_000, 20));
  await page.getByRole("button", { name: "Make Denoised Copy" }).click();
  await expect(page.getByText(/Denoising: \d+%/)).toBeVisible({ timeout: 100_000 });
  await page.getByRole("button", { name: "Cancel" }).click();
  await expect(page.getByRole("button", { name: "Make Denoised Copy" })).toBeEnabled();
  await expect(page.getByText("long (denoised).wav")).toHaveCount(0);
});
