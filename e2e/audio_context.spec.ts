import { test, expect, type Page } from "@playwright/test";

/**
 * Playback of a 500 kHz recording, and playback when an audio context made at
 * a sample rate never starts.
 *
 * Firefox makes a context at 500 kHz without an error, but the context never
 * starts: its state stays "suspended" and nothing plays. The app asks for at
 * most 384 kHz, and the buffers keep the file's 500 kHz. If a context still
 * does not start, the app plays through a context at the browser's default
 * rate.
 *
 * Chromium starts a context at any of these rates, so the second group of
 * tests replaces `AudioContext` with one whose contexts made at a named
 * sample rate report "suspended" for ever, as Firefox's 500 kHz one does.
 */

test.use({
  viewport: { width: 1440, height: 900 },
  launchOptions: { args: ["--autoplay-policy=no-user-gesture-required"] },
});

const RATE = 500_000;

/** Mono 16-bit WAV of one tone at `toneHz`. */
function wav(rate: number, seconds: number, toneHz: number): Buffer {
  const n = Math.round(rate * seconds);
  const data = Buffer.alloc(n * 2);
  for (let i = 0; i < n; i++) {
    data.writeInt16LE(Math.round(Math.sin((2 * Math.PI * toneHz * i) / rate) * 8000), i * 2);
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

type Played = {
  bufferRate: number;
  contextRate: number;
  /** The context's state as the page reads it. */
  contextState: string;
  peak: number;
  finite: boolean;
};
type Tap = { played: Played[]; askedRates: number[] };

/**
 * Records every buffer the app hands to Web Audio for playback, with the
 * context it is played on, and every sample rate a context is asked for.
 */
function installAudioTap() {
  const tap: Tap = { played: [], askedRates: [] };
  (window as unknown as { __audioTap: Tap }).__audioTap = tap;
  const start = AudioBufferSourceNode.prototype.start;
  AudioBufferSourceNode.prototype.start = function (
    this: AudioBufferSourceNode,
    ...args: Parameters<typeof start>
  ) {
    if (this.buffer) {
      let peak = 0;
      let finite = true;
      for (let c = 0; c < this.buffer.numberOfChannels; c++) {
        const d = this.buffer.getChannelData(c);
        for (let i = 0; i < d.length; i++) {
          if (!Number.isFinite(d[i])) finite = false;
          else peak = Math.max(peak, Math.abs(d[i]));
        }
      }
      tap.played.push({
        bufferRate: this.buffer.sampleRate,
        contextRate: this.context.sampleRate,
        contextState: this.context.state,
        peak,
        finite,
      });
    }
    return start.apply(this, args);
  };
  const Real = window.AudioContext;
  window.AudioContext = new Proxy(Real, {
    construct(target, args: [AudioContextOptions?]) {
      const ctx = new target(...args);
      const asked = args[0]?.sampleRate;
      if (asked) {
        tap.askedRates.push(asked);
        // Like Firefox's context at 500 kHz: made without an error, never starts.
        if ((window as unknown as { __stallRatedContexts?: boolean }).__stallRatedContexts) {
          Object.defineProperty(ctx, "state", { get: () => "suspended" });
          ctx.resume = () => new Promise<void>(() => {});
        }
      }
      return ctx;
    },
  });
}

function stallRatedContexts() {
  (window as unknown as { __stallRatedContexts: boolean }).__stallRatedContexts = true;
}

async function openTone(page: Page, toneHz: number) {
  await page.goto("/");
  await page
    .locator('input[type="file"][accept*=".wav"]')
    .first()
    .setInputFiles({ name: "tone.wav", mimeType: "audio/wav", buffer: wav(RATE, 1, toneHz) });
  await expect(page.locator(".chart-stage canvas").first()).toBeVisible({ timeout: 30_000 });
}

async function playAndCollect(page: Page): Promise<Tap> {
  await page.locator(".play-main-btn").click();
  await page.waitForFunction(
    () => (window as unknown as { __audioTap: Tap }).__audioTap.played.length >= 1,
    undefined,
    { timeout: 15_000 },
  );
  return page.evaluate(() => (window as unknown as { __audioTap: Tap }).__audioTap);
}

type Snapshot = { playbackMode: string; hetFreq: number };
function snapshot(page: Page): Promise<Snapshot> {
  return page.evaluate(() =>
    (window as unknown as { __oversample_test: () => Snapshot }).__oversample_test(),
  );
}

/** Opens a 500 kHz tone that the mode makes audible, and selects the mode. */
async function openInMode(page: Page, mode: "1:1" | "HET") {
  if (mode === "1:1") {
    await openTone(page, 5_000);
    expect((await snapshot(page)).playbackMode).toBe("Normal");
    return;
  }
  // HET's default carrier for a 500 kHz file with no species is 31 kHz
  // (band 16 to 250 kHz, cutoff 15 kHz), so a 30 kHz tone plays at 1 kHz.
  await openTone(page, 30_000);
  await page.locator('button[title^="HET"]').first().click();
  const s = await snapshot(page);
  expect(s.playbackMode).toBe("Heterodyne");
  expect(Math.abs(s.hetFreq - 30_000), "tone is within 2 kHz of the carrier").toBeLessThan(2_000);
}

for (const mode of ["1:1", "HET"] as const) {
  test(`a 500 kHz recording plays in ${mode} through a context at 384 kHz`, async ({ page }) => {
    await page.addInitScript(installAudioTap);
    await openInMode(page, mode);
    const tap = await playAndCollect(page);

    expect(Math.max(...tap.askedRates), "highest context rate asked for").toBe(384_000);
    for (const p of tap.played) {
      expect(p.bufferRate, "buffers keep the file's rate").toBe(RATE);
      expect(p.contextRate).toBe(384_000);
      expect(p.finite).toBe(true);
    }
    expect(Math.max(...tap.played.map((p) => p.peak)), "loudest sample").toBeGreaterThan(0.01);
  });

  test(`${mode} plays through a default-rate context when the 384 kHz one never starts`, async ({
    page,
  }) => {
    const warnings: string[] = [];
    page.on("console", (m) => {
      if (m.type() === "warning") warnings.push(m.text());
    });
    await page.addInitScript(installAudioTap);
    await page.addInitScript(stallRatedContexts);
    await openInMode(page, mode);
    const defaultRate = await page.evaluate(() => {
      const c = new AudioContext();
      const r = c.sampleRate;
      void c.close();
      return r;
    });
    const tap = await playAndCollect(page);

    for (const p of tap.played) {
      expect(p.bufferRate, "buffers keep the file's rate").toBe(RATE);
      expect(p.contextRate, "played on the default-rate context").toBe(defaultRate);
      expect(p.contextState).toBe("running");
      expect(p.finite).toBe(true);
    }
    expect(Math.max(...tap.played.map((p) => p.peak)), "loudest sample").toBeGreaterThan(0.01);
    expect(warnings.join("\n")).toContain("An audio context at 384000 Hz did not start");

    // The rate that did not start is not asked for again.
    const askedBefore = tap.askedRates.length;
    await page.locator(".play-main-btn").click();
    await page.waitForTimeout(300);
    await page.locator(".play-main-btn").click();
    await page.waitForFunction(
      (n) => (window as unknown as { __audioTap: Tap }).__audioTap.played.length > n,
      tap.played.length,
      { timeout: 15_000 },
    );
    const again = await page.evaluate(() => (window as unknown as { __audioTap: Tap }).__audioTap);
    expect(again.askedRates.slice(askedBefore).every((r) => r === defaultRate)).toBe(true);
    expect(again.played[again.played.length - 1].contextState).toBe("running");
  });
}
