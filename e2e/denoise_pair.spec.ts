import { test, expect, type Page } from "@playwright/test";
import fs from "fs";
import os from "os";
import path from "path";

/**
 * A recording and its denoised copy behave as a pair: the Denoised switch (and
 * the D key) flips between them, and the scroll position, frequency range,
 * gain and playhead stay the same. The noise settings stay with each file.
 *
 * Makes a real denoised copy, so it needs the model file (see denoise.spec.ts):
 * DENOISE_MODEL, or the parity folder of batchi-noisier2noise. Without that
 * file the tests are skipped.
 */
const modelPath =
  process.env.DENOISE_MODEL ??
  path.join(os.homedir(), "projects/batchi-noisier2noise/runs/parity/model.onnx");

// The pinch gestures that set the zoom and the frequency range need touch.
test.use({ hasTouch: true });

const ORIGINAL = "pass.wav";
const COPY = "pass (denoised).wav";

type Snap = {
  currentFile: string | null;
  scrollOffset: number;
  zoom: number;
  minFreq: number | null;
  maxFreq: number | null;
  gainDb: number;
  gainMode: string;
  noiseReduceEnabled: boolean;
  notchEnabled: boolean;
  focusRange: string;
  isPlaying: boolean;
  playheadTime: number;
  streamFile: string | null;
};

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

async function serveModel(page: Page) {
  await page.route("**/oversample-bat-denoise-v1.onnx", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/octet-stream",
      headers: { "access-control-allow-origin": "*" },
      body: fs.readFileSync(modelPath),
    }),
  );
}

function snapshot(page: Page): Promise<Snap> {
  return page.evaluate(() =>
    (window as unknown as { __oversample_test: () => Snap }).__oversample_test(),
  );
}

/** The parts of a snapshot that the pair shares. */
function shared(s: Snap) {
  return {
    scrollOffset: s.scrollOffset,
    zoom: s.zoom,
    minFreq: s.minFreq,
    maxFreq: s.maxFreq,
    gainDb: s.gainDb,
    gainMode: s.gainMode,
    focusRange: s.focusRange,
  };
}

/** A two-finger pinch on the spectrogram: "x" zooms time, "y" zooms frequency. */
async function pinch(page: Page, axis: "x" | "y", startGap: number, endGap: number) {
  await page.evaluate(
    ({ axis, startGap, endGap }) => {
      const canvas = document.querySelector(".chart-stage canvas") as HTMLElement;
      const r = canvas.getBoundingClientRect();
      const cx = r.left + r.width / 2;
      const cy = r.top + r.height / 2;
      const mk = (id: number, x: number, y: number) =>
        new Touch({
          identifier: id,
          target: canvas,
          clientX: x,
          clientY: y,
          pageX: x,
          pageY: y,
          screenX: x,
          screenY: y,
          radiusX: 1,
          radiusY: 1,
          rotationAngle: 0,
          force: 1,
        });
      const fire = (type: string, touches: Touch[]) =>
        canvas.dispatchEvent(
          new TouchEvent(type, {
            cancelable: true,
            bubbles: true,
            composed: true,
            touches,
            targetTouches: touches,
            changedTouches: touches,
          }),
        );
      const pts = (gap: number): Touch[] =>
        axis === "x"
          ? [mk(0, cx - gap / 2, cy), mk(1, cx + gap / 2, cy)]
          : [mk(0, cx, cy - gap / 2), mk(1, cx, cy + gap / 2)];
      fire("touchstart", pts(startGap));
      for (let i = 1; i <= 6; i++) {
        fire("touchmove", pts(startGap + ((endGap - startGap) * i) / 6));
      }
      fire("touchend", []);
    },
    { axis, startGap, endGap },
  );
}

/** Set the manual gain through the Gain menu. */
async function setManualGain(page: Page, db: number) {
  await page.locator('button[title="Gain settings"]').click();
  await page.locator('.layer-panel input[type="range"][max="60"]').fill(String(db));
  await page.getByRole("button", { name: /^Manual/ }).click();
}

const denoisedSwitch = (page: Page) => page.getByRole("switch", { name: "Denoised" });

/** Open a recording, zoom in, and set a scroll position, frequency range and gain. */
async function openAndSetView(page: Page, seconds: number) {
  await page.goto("/");
  await page
    .locator('input[type="file"][accept*=".wav"]')
    .first()
    .setInputFiles({ name: ORIGINAL, mimeType: "audio/wav", buffer: batLikeWav(384_000, seconds) });
  await expect(page.locator(".chart-stage canvas").first()).toBeVisible({ timeout: 30_000 });
  await page.waitForFunction(
    () => typeof (window as { __oversample_test?: unknown }).__oversample_test === "function",
  );
  await pinch(page, "x", 60, 300);
  await pinch(page, "y", 120, 280);
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("ArrowRight");
  await setManualGain(page, 7);
}

async function makeDenoisedCopy(page: Page) {
  await page.locator('button[title="Noise filter settings"]').click();
  await page.getByRole("button", { name: "Make Denoised Copy" }).click();
  await expect(page.locator(".file-item", { hasText: COPY })).toBeVisible({ timeout: 200_000 });
  // Close the Noise menu.
  await page.locator('button[title="Noise filter settings"]').click();
  await expect(page.getByRole("button", { name: "Make Denoised Copy" })).toHaveCount(0);
  await expect.poll(async () => (await snapshot(page)).currentFile).toBe(COPY);
}

test("a saved copy opened beside its original pairs with it by name", async ({ page }) => {
  await page.goto("/");
  const input = page.locator('input[type="file"][accept*=".wav"]').first();
  const open = async (name: string) => {
    await input.setInputFiles({ name, mimeType: "audio/wav", buffer: batLikeWav(384_000, 2) });
    await expect(page.locator(".file-item", { hasText: name })).toBeVisible({ timeout: 30_000 });
    await expect.poll(async () => (await snapshot(page)).currentFile).toBe(name);
  };
  // The copy first, as an ordinary file. No model, no project.
  await open(COPY);
  await expect(page.locator(".chart-stage canvas").first()).toBeVisible({ timeout: 30_000 });
  await expect(denoisedSwitch(page)).toHaveCount(0);
  // Set a frequency focus (the "All" band preset), a view and a gain on the copy.
  await page.locator('button[title="Band range / presets"]').click();
  await page.getByRole("button", { name: "All", exact: true }).click();
  await pinch(page, "x", 60, 300);
  await pinch(page, "y", 120, 280);
  await page.keyboard.press("ArrowRight");
  await setManualGain(page, 5);
  const copyView = await snapshot(page);
  expect(copyView.scrollOffset).toBeGreaterThan(0);
  expect(copyView.maxFreq).not.toBeNull();

  // Another file, then the original: it opens on the copy's view, gain and
  // focus although the copy is not the open file at that moment.
  await open("other.wav");
  const otherView = await snapshot(page);
  expect(otherView.scrollOffset).toBe(0);
  expect(otherView.focusRange).not.toBe(copyView.focusRange);
  await open(ORIGINAL);
  expect(shared(await snapshot(page))).toEqual(shared(copyView));
  await expect(denoisedSwitch(page)).toHaveAttribute("aria-checked", "false");

  // The copy is listed under its original, though it was opened first.
  const rows = page.locator(".file-item");
  await expect(rows.nth(0)).toContainText("other.wav");
  await expect(rows.nth(1)).toContainText(ORIGINAL);
  await expect(rows.nth(2)).toContainText(COPY);
  await expect(rows.nth(2)).toHaveClass(/denoised-copy/);

  await page.keyboard.press("ArrowRight");
  await setManualGain(page, 9);
  const onOriginal = await snapshot(page);
  expect(onOriginal.scrollOffset).toBeGreaterThan(copyView.scrollOffset);

  await page.keyboard.press("d");
  await expect.poll(async () => (await snapshot(page)).currentFile).toBe(COPY);
  expect(shared(await snapshot(page))).toEqual(shared(onOriginal));

  // A file outside the pair has no switch, and D does nothing there.
  await page.locator(".file-item", { hasText: "other.wav" }).click();
  await expect.poll(async () => (await snapshot(page)).currentFile).toBe("other.wav");
  await expect(denoisedSwitch(page)).toHaveCount(0);
  await page.keyboard.press("d");
  expect((await snapshot(page)).currentFile).toBe("other.wav");
  expect((await snapshot(page)).scrollOffset).toBe(0);
});

test.describe("a recording and its denoised copy", () => {
  test.skip(!fs.existsSync(modelPath), `no model file at ${modelPath}`);

  test("the Denoised switch flips between them and keeps the view and gain", async ({ page }) => {
    test.setTimeout(300_000);
    await serveModel(page);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));

    await openAndSetView(page, 2.5);
    const before = await snapshot(page);
    expect(before.scrollOffset).toBeGreaterThan(0);
    expect(before.maxFreq).not.toBeNull();
    expect(before.gainMode).toBe("Manual");
    expect(before.gainDb).toBe(7);
    // No copy yet, so no switch.
    await expect(denoisedSwitch(page)).toHaveCount(0);

    await makeDenoisedCopy(page);

    // The copy opens on the original's view and gain.
    const onCopy = await snapshot(page);
    expect(shared(onCopy)).toEqual(shared(before));
    await expect(denoisedSwitch(page)).toBeVisible();
    await expect(denoisedSwitch(page)).toHaveAttribute("aria-checked", "true");

    // Making the copy did not reset the original: flip to it and back.
    await page.keyboard.press("d");
    await expect.poll(async () => (await snapshot(page)).currentFile).toBe(ORIGINAL);
    expect(shared(await snapshot(page))).toEqual(shared(before));
    await page.keyboard.press("d");
    await expect.poll(async () => (await snapshot(page)).currentFile).toBe(COPY);
    expect(shared(await snapshot(page))).toEqual(shared(before));

    // In the file list the copy sits under its original, indented.
    const rows = page.locator(".file-item");
    await expect(rows).toHaveCount(2);
    await expect(rows.nth(0)).toContainText(ORIGINAL);
    await expect(rows.nth(1)).toContainText(COPY);
    await expect(rows.nth(1)).toHaveClass(/denoised-copy/);
    await expect(rows.nth(0)).not.toHaveClass(/denoised-copy/);
    await page.screenshot({ path: "test-results/denoise-pair-copy.png" });

    // Move the view and turn noise filtering on while on the copy, then flip.
    await page.keyboard.press("ArrowRight");
    await pinch(page, "y", 120, 200);
    await page.locator('button[title="Turn noise filtering on or off"]').click();
    const copyChanged = await snapshot(page);
    expect(copyChanged.scrollOffset).not.toBe(onCopy.scrollOffset);
    expect(copyChanged.noiseReduceEnabled || copyChanged.notchEnabled).toBe(true);

    await denoisedSwitch(page).click();
    await expect.poll(async () => (await snapshot(page)).currentFile).toBe(ORIGINAL);
    await expect(denoisedSwitch(page)).toHaveAttribute("aria-checked", "false");
    const onOriginal = await snapshot(page);
    expect(shared(onOriginal)).toEqual(shared(copyChanged));
    // The noise filtering turned on for the copy is still off for the original.
    expect(onOriginal.noiseReduceEnabled).toBe(false);
    expect(onOriginal.notchEnabled).toBe(false);
    await page.screenshot({ path: "test-results/denoise-pair-original.png" });

    // Change the gain on the original, then flip back with the D key.
    await setManualGain(page, 12);
    await page.keyboard.press("d");
    await expect.poll(async () => (await snapshot(page)).currentFile).toBe(COPY);
    const backOnCopy = await snapshot(page);
    expect(backOnCopy.gainDb).toBe(12);
    expect(shared(backOnCopy)).toEqual({ ...shared(onOriginal), gainDb: 12 });
    // The copy kept its own noise filtering.
    expect(backOnCopy.noiseReduceEnabled).toBe(copyChanged.noiseReduceEnabled);
    expect(backOnCopy.notchEnabled).toBe(copyChanged.notchEnabled);

    expect(errors).toEqual([]);
  });

  test("flipping during playback keeps playing from the same position", async ({ page }) => {
    test.setTimeout(300_000);
    await serveModel(page);
    await openAndSetView(page, 8);
    await makeDenoisedCopy(page);

    await page.keyboard.press("Space");
    await expect.poll(async () => (await snapshot(page)).isPlaying).toBe(true);
    await expect.poll(async () => (await snapshot(page)).playheadTime).toBeGreaterThan(1);
    const playing = await snapshot(page);
    expect(playing.streamFile).toBe(COPY);

    await page.keyboard.press("d");
    // The audio stream restarts on the original's samples.
    await expect.poll(async () => (await snapshot(page)).streamFile).toBe(ORIGINAL);
    const flipped = await snapshot(page);
    expect(flipped.currentFile).toBe(ORIGINAL);
    expect(flipped.isPlaying).toBe(true);
    // The playhead did not go back to the start, and it keeps moving.
    expect(flipped.playheadTime).toBeGreaterThanOrEqual(playing.playheadTime);
    await expect
      .poll(async () => (await snapshot(page)).playheadTime)
      .toBeGreaterThan(flipped.playheadTime + 0.3);
    expect((await snapshot(page)).isPlaying).toBe(true);

    // And back again with the switch.
    await denoisedSwitch(page).click();
    await expect.poll(async () => (await snapshot(page)).streamFile).toBe(COPY);
    const flippedBack = await snapshot(page);
    expect(flippedBack.isPlaying).toBe(true);
    expect(flippedBack.playheadTime).toBeGreaterThan(flipped.playheadTime);

    await page.keyboard.press("Space");
    await expect.poll(async () => (await snapshot(page)).isPlaying).toBe(false);
  });
});
