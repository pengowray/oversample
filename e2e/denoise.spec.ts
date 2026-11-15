import { test, expect, type Page } from "@playwright/test";
import fs from "fs";
import os from "os";
import path from "path";

/**
 * The Denoised copy group of the Noise menu: the button, the model download,
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

/** Serve the model from the local file. DENOISE_REAL_DOWNLOAD=1 leaves the
 * request alone, so the test downloads from the catalog's real URL. */
async function serveModel(page: Page) {
  if (process.env.DENOISE_REAL_DOWNLOAD) return;
  await page.route("**/oversample-bat-denoise-v1.onnx", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/octet-stream",
      headers: { "access-control-allow-origin": "*" },
      body: fs.readFileSync(modelPath),
    }),
  );
}

async function openRecording(page: Page, name: string, wav: Buffer) {
  await page.goto("/");
  await page
    .locator('input[type="file"][accept*=".wav"]')
    .first()
    .setInputFiles({ name, mimeType: "audio/wav", buffer: wav });
  await expect(page.locator("canvas").first()).toBeVisible();
  await page.locator('button[title="Noise filter settings"]').click();
  await expect(page.getByText("Denoised copy", { exact: true })).toBeVisible();
}

test("a 16 kHz recording cannot be denoised, and the menu says why", async ({ page }) => {
  await openRecording(page, "low-rate.wav", batLikeWav(16_000, 2));
  await expect(page.getByRole("button", { name: "Make denoised copy" })).toBeDisabled();
  // The space before "kHz" is a no-break space.
  await expect(page.getByText(/Sample rate too low/)).toBeVisible();
  await expect(page.getByText(/Needs 22\.05\skHz or more\. This recording: 16\skHz\./)).toBeVisible();
  await page.screenshot({ path: "test-results/denoise-refused.png" });
});

test("a 44.1 kHz recording is denoised and the copy is added to the file list", async ({
  page,
}) => {
  test.skip(
    !process.env.DENOISE_REAL_DOWNLOAD && !fs.existsSync(modelPath),
    `no model file at ${modelPath}`,
  );
  test.setTimeout(240_000);
  await serveModel(page);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await openRecording(page, "bird-rate.wav", batLikeWav(44_100, 6));
  await expect(page.getByText("Model: trained on bat recordings")).toBeVisible();
  const button = page.getByRole("button", { name: "Make denoised copy" });
  await expect(button).toBeEnabled();
  await button.click();
  await expect(page.getByText("bird-rate (denoised).wav").first()).toBeVisible({
    timeout: 200_000,
  });
  await expect(button).toBeEnabled();
  expect(errors).toEqual([]);
});

test("Make denoised copy downloads the model and adds the copy to the file list", async ({
  page,
}) => {
  test.skip(
    !process.env.DENOISE_REAL_DOWNLOAD && !fs.existsSync(modelPath),
    `no model file at ${modelPath}`,
  );
  test.setTimeout(240_000);
  await serveModel(page);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await openRecording(page, "pass.wav", batLikeWav(384_000, 2.5));
  await expect(page.getByText("Model: trained on bat recordings")).toBeVisible();
  await expect(page.getByText("Adds a copy and keeps the original.")).toBeVisible();
  await expect(page.getByText("Downloads the model first: 0.3 MB, once.")).toBeVisible();
  await page.screenshot({ path: "test-results/denoise-idle.png" });

  await page.getByRole("button", { name: "Make denoised copy" }).click();
  await expect(page.getByRole("button", { name: "Cancel" })).toBeVisible();
  await expect(page.getByText(/Denoising: \d+%/)).toBeVisible({ timeout: 120_000 });
  await page.screenshot({ path: "test-results/denoise-running.png" });

  await expect(page.getByText("pass (denoised).wav").first()).toBeVisible({ timeout: 200_000 });
  await expect(page.getByRole("button", { name: "Make denoised copy" })).toBeVisible();
  // The model is stored now, so the download line is gone.
  await expect(page.getByText("Adds a copy and keeps the original.")).toBeVisible();
  await expect(page.getByText(/Downloads the model first/)).toHaveCount(0);
  await page.screenshot({ path: "test-results/denoise-done.png" });
  expect(errors).toEqual([]);
});

test("Cancel stops the work and brings the button back", async ({ page }) => {
  test.skip(
    !process.env.DENOISE_REAL_DOWNLOAD && !fs.existsSync(modelPath),
    `no model file at ${modelPath}`,
  );
  test.setTimeout(120_000);
  await serveModel(page);
  await openRecording(page, "long.wav", batLikeWav(384_000, 20));
  await page.getByRole("button", { name: "Make denoised copy" }).click();
  await expect(page.getByText(/Denoising: \d+%/)).toBeVisible({ timeout: 100_000 });
  await page.getByRole("button", { name: "Cancel" }).click();
  await expect(page.getByRole("button", { name: "Make denoised copy" })).toBeEnabled();
  await expect(page.getByText("long (denoised).wav")).toHaveCount(0);
});

const STEREO_RATE = 44_100;
const STEREO_SECONDS = 6;

/**
 * Stereo samples, left and right. Both have quiet noise of their own and a
 * 50 ms whistle every 100 ms, at half the level on the right. The third
 * second has no whistles, and the last two have whistles on the left only.
 */
function stereoSamples(): [Float32Array, Float32Array] {
  const n = STEREO_RATE * STEREO_SECONDS;
  const left = new Float32Array(n);
  const right = new Float32Array(n);
  let seed = 4321;
  const rand = () => {
    seed = (seed * 1664525 + 1013904223) >>> 0;
    return seed / 0xffffffff - 0.5;
  };
  const period = Math.floor(STEREO_RATE * 0.1);
  const callLen = Math.floor(STEREO_RATE * 0.05);
  let phase = 0;
  for (let i = 0; i < n; i++) {
    const second = Math.floor(i / STEREO_RATE);
    left[i] = rand() * 0.004;
    right[i] = rand() * 0.004;
    const inCall = i % period;
    if (second !== 2 && inCall < callLen) {
      const u = inCall / callLen;
      phase += 2 * Math.PI * (0.3 - 0.18 * u);
      const call = 0.3 * Math.sin(Math.PI * u) ** 2 * Math.sin(phase);
      left[i] += call;
      if (second < 4) right[i] += 0.5 * call;
    }
  }
  return [left, right];
}

function stereoWav(left: Float32Array, right: Float32Array): Buffer {
  const data = Buffer.alloc(left.length * 4);
  for (let i = 0; i < left.length; i++) {
    data.writeInt16LE(Math.round(left[i] * 32767), i * 4);
    data.writeInt16LE(Math.round(right[i] * 32767), i * 4 + 2);
  }
  const header = Buffer.alloc(44);
  header.write("RIFF", 0);
  header.writeUInt32LE(36 + data.length, 4);
  header.write("WAVEfmt ", 8);
  header.writeUInt32LE(16, 16);
  header.writeUInt16LE(1, 20);
  header.writeUInt16LE(2, 22);
  header.writeUInt32LE(STEREO_RATE, 24);
  header.writeUInt32LE(STEREO_RATE * 4, 28);
  header.writeUInt16LE(4, 32);
  header.writeUInt16LE(16, 34);
  header.write("data", 36);
  header.writeUInt32LE(data.length, 40);
  return Buffer.concat([header, data]);
}

/** RMS level of `x` from `from` to `to` seconds. */
function level(x: ArrayLike<number>, from: number, to: number): number {
  let sum = 0;
  const a = Math.floor(from * STEREO_RATE);
  const b = Math.floor(to * STEREO_RATE);
  for (let i = a; i < b; i++) sum += x[i] * x[i];
  return Math.sqrt(sum / (b - a));
}

const dB = (ratio: number) => 20 * Math.log10(ratio);

test("a stereo recording gives a stereo copy with the same balance between its channels", async ({
  page,
}) => {
  test.skip(
    !process.env.DENOISE_REAL_DOWNLOAD && !fs.existsSync(modelPath),
    `no model file at ${modelPath}`,
  );
  test.setTimeout(240_000);
  await serveModel(page);
  // Keep the samples the worker sends back: the copy's channels, interleaved.
  await page.addInitScript(() => {
    const Original = window.Worker;
    window.Worker = class extends Original {
      constructor(url: string | URL, options?: WorkerOptions) {
        super(url, options);
        this.addEventListener("message", (e: MessageEvent) => {
          if (e.data?.msg?.type === "Denoised" && e.data.samples) {
            (window as unknown as { __denoised: number[] }).__denoised = Array.from(
              e.data.samples as Float32Array,
            );
          }
        });
      }
    };
  });
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  const [left, right] = stereoSamples();
  await openRecording(page, "stereo.wav", stereoWav(left, right));
  await expect(page.getByText("Adds a copy and keeps the original.")).toBeVisible();
  await page.getByRole("button", { name: "Make denoised copy" }).click();
  await expect(page.locator(".file-item", { hasText: "stereo (denoised).wav" })).toBeVisible({
    timeout: 200_000,
  });

  // The copy is the open file, and it has two channels.
  const snapshot = () =>
    page.evaluate(() =>
      (
        window as unknown as { __oversample_test: () => { currentFile: string; channels: number } }
      ).__oversample_test(),
    );
  await expect.poll(async () => (await snapshot()).currentFile).toBe("stereo (denoised).wav");
  expect((await snapshot()).channels).toBe(2);

  // The D key goes to the stereo original and back to the copy.
  await page.locator('button[title="Noise filter settings"]').click();
  await expect(page.getByRole("button", { name: "Make denoised copy" })).toHaveCount(0);
  await page.keyboard.press("d");
  await expect.poll(async () => (await snapshot()).currentFile).toBe("stereo.wav");
  expect((await snapshot()).channels).toBe(2);
  await page.keyboard.press("d");
  await expect.poll(async () => (await snapshot()).currentFile).toBe("stereo (denoised).wav");

  const copy = await page.evaluate(() => (window as unknown as { __denoised: number[] }).__denoised);
  expect(copy.length).toBe(left.length * 2);
  const copyLeft = copy.filter((_, i) => i % 2 === 0);
  const copyRight = copy.filter((_, i) => i % 2 === 1);

  // The channels differ: the fifth second has whistles on the left only.
  expect(dB(level(copyLeft, 4.2, 4.8) / level(copyRight, 4.2, 4.8))).toBeGreaterThan(20);

  // A stretch with whistles in both: left over right is the same in the copy
  // as in the original, within 0.1 dB.
  const before = dB(level(left, 0.5, 1.5) / level(right, 0.5, 1.5));
  const after = dB(level(copyLeft, 0.5, 1.5) / level(copyRight, 0.5, 1.5));
  expect(before).toBeGreaterThan(5.9);
  expect(Math.abs(after - before)).toBeLessThan(0.1);

  // The second without whistles is quieter in each channel of the copy.
  for (const [original, denoised] of [
    [left, copyLeft],
    [right, copyRight],
  ] as const) {
    expect(dB(level(denoised, 2.2, 2.8) / level(original, 2.2, 2.8))).toBeLessThan(-6);
  }
  expect(errors).toEqual([]);
});
