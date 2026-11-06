import { test, expect, type Page } from "@playwright/test";
import path from "path";

/**
 * Checks that left and right stay separate through the playback and live-listen
 * DSP. Both fixtures (generated in global-setup) alternate noise bursts between
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
type Tap = { blocks: Block[]; processorChannels: number[] };

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
  };
  const tap: Tap = { blocks: [], processorChannels: [] };
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
    (window as unknown as { __audioTap: Tap }).__audioTap.blocks.length = 0;
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

  test("Channels set to Mono mixes the channels down", async ({ page }) => {
    await listenInStereo(page, false);
    await selectMode(page, "HET");
    await waitForBlocks(page, 30);
    const tap = await readTap(page);
    expect(tap.processorChannels.at(-1)).toBe(1);
    expect(tap.blocks.every((b) => b.channels === 1)).toBe(true);
  });
});
