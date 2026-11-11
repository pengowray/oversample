import { test, expect, type Page } from "@playwright/test";
import fs from "fs";
import path from "path";

/**
 * Checks that left and right stay separate through the playback and live-listen
 * DSP, and in saved recordings. Both fixtures (generated in global-setup) alternate noise bursts between
 * the channels: L only, then R only. Noise is used instead of a tone so that
 * HET and PS output has energy whichever carrier or factor the app picks.
 *
 * A mono mixdown would put the same signal on both outputs, so the check is
 * that some output blocks are loud on L and quiet on R, and others the reverse.
 *
 * The init script records what the app sends to Web Audio: every
 * AudioBufferSourceNode buffer (file playback, native live listen) and every
 * ScriptProcessorNode output block (browser live listen).
 */
const fixtures = path.resolve(process.cwd(), "e2e", "fixtures");
const fileFixture = path.join(fixtures, "stereo-alternating.wav");
const micFixture = path.join(fixtures, "stereo-mic.wav");

test.use({
  launchOptions: {
    args: [
      "--autoplay-policy=no-user-gesture-required",
      "--use-fake-device-for-media-stream",
      "--use-fake-ui-for-media-stream",
      `--use-file-for-fake-audio-capture=${micFixture}`,
    ],
  },
});

type Block = { channels: number; left: number; right: number };
type Tap = { blocks: Block[]; windows: Block[]; processorChannels: number[] };

function installAudioTap() {
  const rms = (d: Float32Array) => {
    let sum = 0;
    for (let i = 0; i < d.length; i++) sum += d[i] * d[i];
    return Math.sqrt(sum / Math.max(1, d.length));
  };
  const record = (buf: AudioBuffer) => {
    const left = rms(buf.getChannelData(0));
    const right = buf.numberOfChannels > 1 ? rms(buf.getChannelData(1)) : left;
    tap.blocks.push({ channels: buf.numberOfChannels, left, right });
    // Playback buffers can span several L and R bursts of a low-rate file, so also
    // keep the level of each short section.
    const size = 2048;
    const l = buf.getChannelData(0);
    const r = buf.numberOfChannels > 1 ? buf.getChannelData(1) : l;
    for (let i = 0; i + size <= buf.length; i += size) {
      tap.windows.push({
        channels: buf.numberOfChannels,
        left: rms(l.subarray(i, i + size)),
        right: rms(r.subarray(i, i + size)),
      });
    }
  };
  const tap: Tap = { blocks: [], windows: [], processorChannels: [] };
  (window as unknown as { __audioTap: Tap }).__audioTap = tap;

  const start = AudioBufferSourceNode.prototype.start;
  AudioBufferSourceNode.prototype.start = function (
    this: AudioBufferSourceNode,
    ...args: Parameters<typeof start>
  ) {
    if (this.buffer) record(this.buffer);
    return start.apply(this, args);
  };

  const createProcessor = BaseAudioContext.prototype.createScriptProcessor;
  BaseAudioContext.prototype.createScriptProcessor = function (
    this: BaseAudioContext,
    ...args: Parameters<typeof createProcessor>
  ) {
    const node = createProcessor.apply(this, args);
    tap.processorChannels.push(args[1] ?? 2);
    let handler: ((ev: AudioProcessingEvent) => void) | null = null;
    Object.defineProperty(node, "onaudioprocess", {
      get: () => handler,
      set: (fn) => {
        handler = fn;
      },
    });
    node.addEventListener("audioprocess", (ev) => {
      if (!handler) return;
      handler(ev as AudioProcessingEvent);
      record((ev as AudioProcessingEvent).outputBuffer);
    });
    return node;
  } as typeof createProcessor;
}

function readTap(page: Page): Promise<Tap> {
  return page.evaluate(() => (window as unknown as { __audioTap: Tap }).__audioTap);
}

function clearTap(page: Page) {
  return page.evaluate(() => {
    const tap = (window as unknown as { __audioTap: Tap }).__audioTap;
    tap.blocks.length = 0;
    tap.windows.length = 0;
  });
}

/** Output blocks where one channel is at least 10x louder than the other. */
function separation(blocks: Block[]) {
  const loud = 1e-3;
  const leftOnly = blocks.filter((b) => b.left > loud && b.left > b.right * 10).length;
  const rightOnly = blocks.filter((b) => b.right > loud && b.right > b.left * 10).length;
  return { leftOnly, rightOnly, total: blocks.length };
}

async function waitForBlocks(page: Page, atLeast: number) {
  await page.waitForFunction(
    (n) => (window as unknown as { __audioTap: Tap }).__audioTap.blocks.length >= n,
    atLeast,
    { timeout: 30_000 },
  );
}

async function openApp(page: Page) {
  await page.addInitScript(installAudioTap);
  await page.goto("/");
  await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
}

async function selectMode(page: Page, titlePrefix: string) {
  await page.locator(`button[title^="${titlePrefix}"]`).first().click();
}

test.describe("file playback", () => {
  for (const [name, title] of [
    ["1:1", "1:1"],
    ["HET", "HET"],
    ["PS", "PS"],
  ]) {
    test(`${name} keeps left and right separate`, async ({ page }) => {
      await openApp(page);
      await page.locator('input[type="file"][accept*=".wav"]').first().setInputFiles(fileFixture);
      await expect(page.locator(".file-item, .file-row").first()).toBeVisible({ timeout: 30_000 });
      await selectMode(page, title);
      await clearTap(page);
      await page.keyboard.press("Space");
      await waitForBlocks(page, 8);
      const { blocks } = await readTap(page);
      console.log(`${name}:`, JSON.stringify(blocks.slice(0, 8)));
      expect(blocks.every((b) => b.channels === 2)).toBe(true);
      const s = separation(blocks);
      expect(s.leftOnly).toBeGreaterThan(0);
      expect(s.rightOnly).toBeGreaterThan(0);
    });
  }
});

test.describe("live listen", () => {
  async function listenInStereo(page: Page, stereo: boolean) {
    await openApp(page);
    if (stereo) {
      await page.locator('button[title^="Microphone options"]').first().click();
      await page.locator('select:has(option[value="stereo"])').first().selectOption("stereo");
      await page.keyboard.press("Escape");
    }
    await page.locator('button[title="Toggle live listening (L)"]').first().click();
  }

  for (const [name, title] of [
    ["HET", "HET"],
    ["PS", "PS"],
  ]) {
    test(`${name} keeps left and right separate when Channels is Stereo`, async ({ page }) => {
      await listenInStereo(page, true);
      await selectMode(page, title);
      await waitForBlocks(page, 5);
      await clearTap(page);
      await waitForBlocks(page, 30);
      const tap = await readTap(page);
      console.log(`live ${name}:`, JSON.stringify(tap.blocks.slice(0, 12)));
      expect(tap.processorChannels.at(-1)).toBe(2);
      expect(tap.blocks.every((b) => b.channels === 2)).toBe(true);
      const s = separation(tap.blocks);
      expect(s.leftOnly).toBeGreaterThan(0);
      expect(s.rightOnly).toBeGreaterThan(0);
    });
  }

  test("switching Channels to Stereo while listening takes effect", async ({ page }) => {
    await listenInStereo(page, false);
    await selectMode(page, "HET");
    await waitForBlocks(page, 5);
    await page.locator('button[title^="Microphone options"]').first().click();
    await page.locator('select:has(option[value="stereo"])').first().selectOption("stereo");
    await page.waitForFunction(
      () => (window as unknown as { __audioTap: Tap }).__audioTap.processorChannels.at(-1) === 2,
      undefined,
      { timeout: 15_000 },
    );
    await clearTap(page);
    await waitForBlocks(page, 30);
    const tap = await readTap(page);
    expect(tap.blocks.every((b) => b.channels === 2)).toBe(true);
    const s = separation(tap.blocks);
    expect(s.leftOnly).toBeGreaterThan(0);
    expect(s.rightOnly).toBeGreaterThan(0);
  });

  test("Channels setting is remembered after a reload", async ({ page }) => {
    await listenInStereo(page, true);
    await page.reload();
    await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
    await page.locator('button[title^="Microphone options"]').first().click();
    await expect(page.locator('select:has(option[value="stereo"])').first()).toHaveValue("stereo");
    await page.keyboard.press("Escape");
    await page.locator('button[title="Toggle live listening (L)"]').first().click();
    await selectMode(page, "HET");
    await waitForBlocks(page, 5);
    const tap = await readTap(page);
    expect(tap.processorChannels.at(-1)).toBe(2);
  });

  test("Channels set to Mono mixes the channels down", async ({ page }) => {
    await listenInStereo(page, false);
    await selectMode(page, "HET");
    await waitForBlocks(page, 30);
    const tap = await readTap(page);
    expect(tap.processorChannels.at(-1)).toBe(1);
    expect(tap.blocks.every((b) => b.channels === 1)).toBe(true);
  });
});

test.describe("recording", () => {
  type Wav = { channels: number; frames: number; left: Float32Array; right: Float32Array };

  /** Reads a browser recording, which is 32-bit float (the format Web Audio
   * delivers), by walking its RIFF chunks. */
  function parseWav(buf: Buffer): Wav {
    expect(buf.toString("latin1", 0, 4)).toBe("RIFF");
    let format = 0;
    let channels = 0;
    let bits = 0;
    let dataStart = -1;
    let dataSize = 0;
    let pos = 12;
    while (pos + 8 <= buf.length) {
      const id = buf.toString("latin1", pos, pos + 4);
      const size = buf.readUInt32LE(pos + 4);
      if (id === "fmt ") {
        format = buf.readUInt16LE(pos + 8);
        channels = buf.readUInt16LE(pos + 10);
        bits = buf.readUInt16LE(pos + 22);
      } else if (id === "data") {
        dataStart = pos + 8;
        dataSize = size;
      }
      pos += 8 + size + (size % 2);
    }
    // WAVE_FORMAT_IEEE_FLOAT (3), or WAVE_FORMAT_EXTENSIBLE (0xFFFE) for 3+ channels.
    expect([3, 0xfffe]).toContain(format);
    expect(bits).toBe(32);
    expect(dataStart).toBeGreaterThan(0);
    const frames = dataSize / (4 * channels);
    expect(Number.isInteger(frames)).toBe(true);
    const left = new Float32Array(frames);
    const right = new Float32Array(frames);
    for (let i = 0; i < frames; i++) {
      left[i] = buf.readFloatLE(dataStart + i * 4 * channels);
      right[i] = channels > 1 ? buf.readFloatLE(dataStart + i * 4 * channels + 4) : left[i];
    }
    return { channels, frames, left, right };
  }

  /** The same check as `separation`, on 4096-frame blocks of a saved file. */
  function wavSeparation(wav: Wav) {
    const size = 4096;
    const rms = (d: Float32Array) => Math.sqrt(d.reduce((sum, v) => sum + v * v, 0) / d.length);
    const blocks: Block[] = [];
    for (let i = 0; i + size <= wav.frames; i += size) {
      blocks.push({
        channels: wav.channels,
        left: rms(wav.left.subarray(i, i + size)),
        right: rms(wav.right.subarray(i, i + size)),
      });
    }
    return separation(blocks);
  }

  /** Sets options in the mic panel, given as [label, value] pairs. */
  async function setMicOptions(page: Page, options: [string, string][]) {
    const micOptions = page.locator('button[title^="Microphone options"]').first();
    await micOptions.click();
    for (const [label, value] of options) {
      await page
        .locator(`.layer-panel-slider-row:has(label:text-is("${label}")) select`)
        .first()
        .selectOption(value);
    }
    await micOptions.click();
  }

  /** Listens, records for `recordMs`, stops, and returns the file the app saves. */
  async function recordFromListening(page: Page, listenMs: number, recordMs: number) {
    await page.locator('button[title="Toggle live listening (L)"]').first().click();
    await page.waitForTimeout(listenMs);
    const record = page.locator('button[title="Record"]').first();
    await record.click();
    await page.waitForTimeout(recordMs);
    await record.click();
    return saveRecording(page);
  }

  async function saveRecording(page: Page) {
    const banner = page.locator(".unsaved-banner").first();
    await expect(banner).toBeVisible({ timeout: 30_000 });
    const downloadStarted = page.waitForEvent("download");
    await banner.click();
    const download = await downloadStarted;
    return parseWav(fs.readFileSync(await download.path()));
  }

  test("Stereo saves a 2-channel WAV that plays back with left and right separate", async ({
    page,
  }) => {
    await openApp(page);
    await setMicOptions(page, [["Channels", "stereo"]]);
    const wav = await recordFromListening(page, 500, 3_000);
    expect(wav.channels).toBe(2);
    expect(wav.frames).toBeGreaterThan(48_000);
    const saved = wavSeparation(wav);
    console.log("saved stereo:", JSON.stringify(saved));
    expect(saved.leftOnly).toBeGreaterThan(0);
    expect(saved.rightOnly).toBeGreaterThan(0);

    await selectMode(page, "1:1");
    await clearTap(page);
    await page.keyboard.press("Space");
    // 40 sections of 2048 frames is 1.7 s at 48 kHz, which spans L and R bursts.
    await page.waitForFunction(
      () => (window as unknown as { __audioTap: Tap }).__audioTap.windows.length >= 40,
      undefined,
      { timeout: 30_000 },
    );
    const { windows } = await readTap(page);
    expect(windows.every((b) => b.channels === 2)).toBe(true);
    const played = separation(windows);
    console.log("played stereo:", JSON.stringify(played));
    expect(played.leftOnly).toBeGreaterThan(0);
    expect(played.rightOnly).toBeGreaterThan(0);
  });

  test("Stereo stays 2-channel when the recording starts without listening first", async ({
    page,
  }) => {
    await openApp(page);
    await setMicOptions(page, [["Channels", "stereo"]]);
    const record = page.locator('button[title="Record"]').first();
    await record.click();
    // The "Ready to record" dialog appears unless the browser just asked for
    // mic permission.
    const confirm = page.locator('.xc-modal button:text-is("Record")').first();
    await confirm.click({ timeout: 2_000 }).catch(() => {});
    await page.waitForTimeout(3_000);
    await record.click();
    const wav = await saveRecording(page);
    expect(wav.channels).toBe(2);
    const saved = wavSeparation(wav);
    expect(saved.leftOnly).toBeGreaterThan(0);
    expect(saved.rightOnly).toBeGreaterThan(0);
  });

  test("Stereo pre-roll stays aligned after the listening buffer is trimmed", async ({ page }) => {
    await openApp(page);
    await setMicOptions(page, [
      ["Channels", "stereo"],
      ["Pre-roll buffer", "2"],
    ]);
    // Listening is trimmed to pre-roll + 2 s headroom = 4 s, so 6 s of listening
    // has been trimmed by the time the recording starts. The saved file is
    // 2-channel only if both live buffers were trimmed together.
    const wav = await recordFromListening(page, 6_000, 1_500);
    expect(wav.channels).toBe(2);
    expect(wav.frames).toBeGreaterThan(48_000);
    expect(wav.frames).toBeLessThan(48_000 * 7);
    const saved = wavSeparation(wav);
    expect(saved.leftOnly).toBeGreaterThan(0);
    expect(saved.rightOnly).toBeGreaterThan(0);
  });

  test("Mono saves a 1-channel WAV", async ({ page }) => {
    await openApp(page);
    await setMicOptions(page, [["Channels", "mono"]]);
    const wav = await recordFromListening(page, 500, 2_000);
    expect(wav.channels).toBe(1);
    expect(wav.frames).toBeGreaterThan(48_000);
  });
});

test.describe("export", () => {
  /** Reads the channel count and sample rate from a WAV's fmt chunk, plus its data. */
  function wavInfo(buf: Buffer) {
    expect(buf.toString("latin1", 0, 4)).toBe("RIFF");
    let channels = 0;
    let rate = 0;
    let dataStart = -1;
    let dataSize = 0;
    let pos = 12;
    while (pos + 8 <= buf.length) {
      const id = buf.toString("latin1", pos, pos + 4);
      const size = buf.readUInt32LE(pos + 4);
      if (id === "fmt ") {
        channels = buf.readUInt16LE(pos + 10);
        rate = buf.readUInt32LE(pos + 12);
      } else if (id === "data") {
        dataStart = pos + 8;
        dataSize = size;
      }
      pos += 8 + size + (size % 2);
    }
    const frames = dataSize / (2 * channels);
    const left = new Float32Array(frames);
    const right = new Float32Array(frames);
    for (let i = 0; i < frames; i++) {
      left[i] = buf.readInt16LE(dataStart + i * 2 * channels) / 32768;
      right[i] = channels > 1 ? buf.readInt16LE(dataStart + i * 2 * channels + 2) / 32768 : left[i];
    }
    return { channels, rate, frames, left, right };
  }

  async function exportFile(page: Page, format: "WAV" | "MP4") {
    const showInfo = page.locator('button[title="Show info panel"]').first();
    if (await showInfo.count()) await showInfo.click();
    await page.locator(".sidebar-tab-dropdown").first().click();
    await page.locator(".sidebar-tab-option", { hasText: /^Export$/ }).first().click();
    await page.locator(".export-radio", { hasText: format }).locator("input").check();
    const downloadStarted = page.waitForEvent("download", { timeout: 60_000 });
    const shownName = (await page.locator(".export-file-name").first().textContent())!;
    await page.locator(".export-panel button", { hasText: /^Export .*\(\./ }).first().click();
    const download = await downloadStarted;
    expect(download.suggestedFilename()).toBe(shownName);
    return { name: download.suggestedFilename(), buf: fs.readFileSync(await download.path()) };
  }

  test("WAV export of a stereo file keeps left and right, and names the TE factor", async ({
    page,
  }) => {
    await openApp(page);
    await page.locator('input[type="file"][accept*=".wav"]').first().setInputFiles(fileFixture);
    await expect(page.locator(".file-item, .file-row").first()).toBeVisible({ timeout: 30_000 });
    await selectMode(page, "TE");

    const { name, buf } = await exportFile(page, "WAV");
    console.log("exported:", name);
    expect(name).toMatch(/_TE\d+(\.\d+)?\.wav$/);
    const wav = wavInfo(buf);
    expect(wav.channels).toBe(2);
    const te = Number(name.match(/_TE(\d+)/)![1]);
    expect(wav.rate).toBe(192_000 / te);
    const size = 4096;
    const rms = (d: Float32Array) => Math.sqrt(d.reduce((sum, v) => sum + v * v, 0) / d.length);
    const blocks: Block[] = [];
    for (let i = 0; i + size <= wav.frames; i += size) {
      blocks.push({
        channels: 2,
        left: rms(wav.left.subarray(i, i + size)),
        right: rms(wav.right.subarray(i, i + size)),
      });
    }
    const s = separation(blocks);
    expect(s.leftOnly).toBeGreaterThan(0);
    expect(s.rightOnly).toBeGreaterThan(0);
  });

  test("WAV export in 1:1 has no transform suffix", async ({ page }) => {
    await openApp(page);
    await page.locator('input[type="file"][accept*=".wav"]').first().setInputFiles(fileFixture);
    await expect(page.locator(".file-item, .file-row").first()).toBeVisible({ timeout: 30_000 });
    await selectMode(page, "1:1");
    const { name, buf } = await exportFile(page, "WAV");
    expect(name).toBe("stereo-alternating_export.wav");
    expect(wavInfo(buf).channels).toBe(2);
  });

  test("MP4 export of a stereo file has 2 audio channels and names the TE factor", async ({
    page,
  }) => {
    await openApp(page);
    await page.locator('input[type="file"][accept*=".wav"]').first().setInputFiles(fileFixture);
    await expect(page.locator(".file-item, .file-row").first()).toBeVisible({ timeout: 30_000 });
    await selectMode(page, "TE");
    const { name, buf } = await exportFile(page, "MP4");
    console.log("exported:", name);
    expect(name).toMatch(/^stereo-alternating_TE\d+\.mp4$/);
    // AudioSampleEntry ('mp4a' or 'Opus'): channelcount is 24 bytes after the box start.
    const entry = [buf.indexOf("mp4a"), buf.indexOf("Opus")].filter((i) => i >= 4)[0];
    expect(entry).toBeGreaterThan(0);
    expect(buf.readUInt16BE(entry - 4 + 24)).toBe(2);
  });
});
