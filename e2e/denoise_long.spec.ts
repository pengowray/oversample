import { test, expect, chromium, type Page } from "@playwright/test";
import fs from "fs";
import os from "os";
import path from "path";

/**
 * Make Denoised Copy on a recording too long to denoise in memory (more than
 * 24 million samples). The app sends the recording to the worker a piece at
 * a time and writes the copy to the browser's storage (OPFS) as it comes
 * back.
 *
 * The model is served from a local file, as in denoise.spec.ts. Without that
 * file the tests are skipped.
 *
 * The last test measures memory and time on recordings of 10 and 30 minutes.
 * It takes a long time and needs several GB of disk, so it runs only with
 * DENOISE_MEASURE=1. It uses a browser profile on disk. Playwright's usual
 * page is like a private window, where Chromium keeps the site's storage in
 * memory, so there the copy's file adds its whole size to the browser's
 * memory. DENOISE_MEASURE=private measures that case.
 */
const modelPath =
  process.env.DENOISE_MODEL ??
  path.join(os.homedir(), "projects/batchi-noisier2noise/runs/parity/model.onnx");

/** The folder in the browser's storage that the copies are written to. */
const STORAGE_DIR = "oversample-audio";

type Shape = {
  sampleRate: number;
  seconds: number;
  channels: 1 | 2;
  /** No calls from this second to `quietTo`. */
  quietFrom: number;
  quietTo: number;
};

/** Level of the noise in every sample, before the calls are added. */
const NOISE = 0.02;

/**
 * 16-bit samples, channels interleaved: noise with a 4 ms downward sweep
 * every 100 ms, and a stretch without sweeps. The right channel of a stereo
 * recording has its own noise and the sweeps at half the level.
 */
function samples(shape: Shape): Int16Array {
  const { sampleRate, seconds, channels } = shape;
  const n = Math.floor(sampleRate * seconds);
  const out = new Int16Array(n * channels);
  let seed = 12345;
  const rand = () => {
    seed = (seed * 1664525 + 1013904223) >>> 0;
    return seed / 0xffffffff - 0.5;
  };
  const period = Math.floor(sampleRate * 0.1);
  const callLen = Math.floor(sampleRate * 0.004);
  const quietFrom = shape.quietFrom * sampleRate;
  const quietTo = shape.quietTo * sampleRate;
  let phase = 0;
  for (let i = 0; i < n; i++) {
    let call = 0;
    const inCall = i % period;
    if (inCall < callLen && (i < quietFrom || i >= quietTo)) {
      const u = inCall / callLen;
      phase += 2 * Math.PI * (0.3 - 0.18 * u);
      call = 0.3 * Math.sin(Math.PI * u) ** 2 * Math.sin(phase);
    }
    for (let c = 0; c < channels; c++) {
      const s = rand() * NOISE + (c === 0 ? call : 0.5 * call);
      out[i * channels + c] = Math.round(Math.max(-1, Math.min(1, s)) * 32767);
    }
  }
  return out;
}

function wavHeader(shape: Shape, dataBytes: number): Buffer {
  const header = Buffer.alloc(44);
  header.write("RIFF", 0);
  header.writeUInt32LE(36 + dataBytes, 4);
  header.write("WAVEfmt ", 8);
  header.writeUInt32LE(16, 16);
  header.writeUInt16LE(1, 20);
  header.writeUInt16LE(shape.channels, 22);
  header.writeUInt32LE(shape.sampleRate, 24);
  header.writeUInt32LE(shape.sampleRate * 2 * shape.channels, 28);
  header.writeUInt16LE(2 * shape.channels, 32);
  header.writeUInt16LE(16, 34);
  header.write("data", 36);
  header.writeUInt32LE(dataBytes, 40);
  return header;
}

/** Writes the recording to a file in a new temporary folder; returns its path. */
function writeWav(name: string, shape: Shape): string {
  const data = samples(shape);
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "oversample-denoise-"));
  const file = path.join(dir, name);
  const bytes = Buffer.from(data.buffer, data.byteOffset, data.byteLength);
  fs.writeFileSync(file, Buffer.concat([wavHeader(shape, bytes.length), bytes]));
  return file;
}

/** RMS of the stretch without calls, in one channel of the original. */
function quietLevel(shape: Shape, channel: number): number {
  const data = samples({ ...shape, seconds: shape.quietTo });
  const from = Math.floor((shape.quietFrom + 0.2) * shape.sampleRate);
  const to = Math.floor((shape.quietTo - 0.2) * shape.sampleRate);
  let sum = 0;
  for (let i = from; i < to; i++) {
    const s = data[i * shape.channels + channel] / 32768;
    sum += s * s;
  }
  return Math.sqrt(sum / (to - from));
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

/**
 * Keeps what the denoiser's worker sends back, in `window.__copy`: the whole
 * recording of a short one (`whole`), and the pieces of a long one
 * (`pieces`), all with their channels interleaved.
 */
async function keepWorkerOutput(page: Page) {
  await page.addInitScript(() => {
    const kept = { whole: null as Float32Array | null, pieces: [] as Float32Array[] };
    (window as unknown as { __copy: typeof kept }).__copy = kept;
    const Original = window.Worker;
    window.Worker = class extends Original {
      constructor(url: string | URL, options?: WorkerOptions) {
        super(url, options);
        this.addEventListener("message", (e: MessageEvent) => {
          const type = e.data?.msg?.type;
          if (type === "Denoised") kept.whole = (e.data.samples as Float32Array).slice();
          if (type === "DenoisedPiece") kept.pieces.push((e.data.samples as Float32Array).slice());
        });
      }
    };
  });
}

async function openRecording(page: Page, file: string | { name: string; buffer: Buffer }) {
  await page.goto("/");
  const input = page.locator('input[type="file"][accept*=".wav"]').first();
  if (typeof file === "string") await input.setInputFiles(file);
  else await input.setInputFiles({ ...file, mimeType: "audio/wav" });
  await expect(page.locator("canvas").first()).toBeVisible({ timeout: 60_000 });
  await page.waitForFunction(
    () => typeof (window as { __oversample_test?: unknown }).__oversample_test === "function",
  );
  await page.locator('button[title="Noise filter settings"]').click();
  await expect(page.getByText("Denoised copy", { exact: true })).toBeVisible();
}

type Snap = { currentFile: string; channels: number; sampleRate: number; duration: number };
const snapshot = (page: Page) =>
  page.evaluate(() => (window as unknown as { __oversample_test: () => Snap }).__oversample_test());

/** Every file in the copies' folder of the browser's storage, with its size. */
function storedFiles(page: Page): Promise<{ path: string; size: number }[]> {
  return page.evaluate(async (dirName) => {
    const root = await navigator.storage.getDirectory();
    let dir: FileSystemDirectoryHandle;
    try {
      dir = await root.getDirectoryHandle(dirName);
    } catch {
      return [];
    }
    const out: { path: string; size: number }[] = [];
    const walk = async (d: FileSystemDirectoryHandle, prefix: string) => {
      // @ts-expect-error: entries() is missing from the DOM types in use.
      for await (const [name, handle] of d.entries()) {
        if (handle.kind === "directory") await walk(handle, `${prefix}${name}/`);
        else out.push({ path: prefix + name, size: (await handle.getFile()).size });
      }
    };
    await walk(dir, "");
    return out;
  }, STORAGE_DIR);
}

/** Length of the pieces kept, and the RMS of one channel from `from` to `to` seconds. */
function piecesLevel(page: Page, shape: Shape, channel: number, from: number, to: number) {
  return page.evaluate(
    ({ channels, sampleRate, channel, from, to }) => {
      const { pieces } = (window as unknown as { __copy: { pieces: Float32Array[] } }).__copy;
      const a = Math.floor(from * sampleRate) * channels + channel;
      const b = Math.floor(to * sampleRate) * channels + channel;
      let at = 0;
      let sum = 0;
      let count = 0;
      let finite = true;
      for (const piece of pieces) {
        for (let i = 0; i < piece.length; i++, at++) {
          if (!Number.isFinite(piece[i])) finite = false;
          if (at >= a && at < b && (at - a) % channels === 0) {
            sum += piece[i] * piece[i];
            count++;
          }
        }
      }
      return { samples: at, rms: Math.sqrt(sum / count), finite };
    },
    { channels: shape.channels, sampleRate: shape.sampleRate, channel, from, to },
  );
}

const dB = (ratio: number) => 20 * Math.log10(ratio);

test.beforeEach(() => {
  test.skip(!fs.existsSync(modelPath), `no model file at ${modelPath}`);
});

test("a 70 s recording at 384 kHz, longer than fits in memory, is denoised", async ({ page }) => {
  test.setTimeout(900_000);
  // 26.9 million samples: over the 24 million that are denoised in memory.
  const shape: Shape = { sampleRate: 384_000, seconds: 70, channels: 1, quietFrom: 30, quietTo: 32 };
  const file = writeWav("long.wav", shape);
  await serveModel(page);
  await keepWorkerOutput(page);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await openRecording(page, file);
  fs.rmSync(path.dirname(file), { recursive: true });
  const button = page.getByRole("button", { name: "Make Denoised Copy" });
  await expect(button).toBeEnabled();
  await expect(page.getByText(/too long/i)).toHaveCount(0);
  await button.click();
  await expect(page.getByText(/Denoising: \d+%/)).toBeVisible({ timeout: 120_000 });
  // A run this long says how much time is left.
  await expect(page.getByText(/Denoising: \d+%, about \d+ min left/)).toBeVisible({
    timeout: 120_000,
  });
  await page.screenshot({ path: "test-results/denoise-long-running.png" });
  await expect(page.locator(".file-item", { hasText: "long (denoised).wav" })).toBeVisible({
    timeout: 800_000,
  });

  // The copy is the open file: one channel, the original's rate and length.
  await expect.poll(async () => (await snapshot(page)).currentFile).toBe("long (denoised).wav");
  const copy = await snapshot(page);
  expect(copy.channels).toBe(1);
  expect(copy.sampleRate).toBe(384_000);
  expect(copy.duration).toBeCloseTo(70, 3);

  // It came from the worker in pieces, every sample of it.
  const kept = await page.evaluate(() => {
    const c = (window as unknown as { __copy: { whole: unknown; pieces: Float32Array[] } }).__copy;
    return { whole: c.whole !== null, pieces: c.pieces.length };
  });
  expect(kept.whole).toBe(false);
  expect(kept.pieces).toBeGreaterThan(200);
  const quiet = await piecesLevel(page, shape, 0, shape.quietFrom + 0.2, shape.quietTo - 0.2);
  expect(quiet.samples).toBe(shape.sampleRate * shape.seconds);
  expect(quiet.finite).toBe(true);
  // The stretch without calls is quieter than in the original.
  expect(dB(quiet.rms / quietLevel(shape, 0))).toBeLessThan(-6);

  // A copy of this size is decoded into memory, so its file in the browser's
  // storage is deleted once it is open.
  await expect.poll(() => storedFiles(page)).toEqual([]);

  // The D key goes to the original and back to the copy.
  await page.locator('button[title="Noise filter settings"]').click();
  await expect(button).toHaveCount(0);
  await page.keyboard.press("d");
  await expect.poll(async () => (await snapshot(page)).currentFile).toBe("long.wav");
  await page.keyboard.press("d");
  await expect.poll(async () => (await snapshot(page)).currentFile).toBe("long (denoised).wav");
  expect(errors).toEqual([]);
});

test("Cancel part-way through a long recording leaves no file behind", async ({ page }) => {
  test.setTimeout(300_000);
  const shape: Shape = { sampleRate: 384_000, seconds: 20, channels: 1, quietFrom: 8, quietTo: 9 };
  const data = samples(shape);
  const bytes = Buffer.from(data.buffer, data.byteOffset, data.byteLength);
  await serveModel(page);
  await openRecording(page, {
    name: "cancel.wav",
    buffer: Buffer.concat([wavHeader(shape, bytes.length), bytes]),
  });
  // Recordings over 1 million samples go a piece at a time in this test.
  await page.evaluate(() => {
    (window as unknown as Record<string, number>).__oversample_denoise_in_memory_max = 1_000_000;
  });
  await page.getByRole("button", { name: "Make Denoised Copy" }).click();
  // Wait until some of the copy is written.
  await expect(page.getByText(/Denoising: ([5-9]|\d\d)%/)).toBeVisible({ timeout: 200_000 });
  expect((await storedFiles(page)).length).toBeGreaterThan(0);

  await page.getByRole("button", { name: "Cancel" }).click();
  await expect(page.getByRole("button", { name: "Make Denoised Copy" })).toBeEnabled();
  await expect.poll(() => storedFiles(page)).toEqual([]);
  await expect(page.getByText("cancel (denoised).wav")).toHaveCount(0);

  // The button works again, and the second copy is complete.
  await page.getByRole("button", { name: "Make Denoised Copy" }).click();
  await expect(page.locator(".file-item", { hasText: "cancel (denoised).wav" })).toBeVisible({
    timeout: 250_000,
  });
  await expect.poll(async () => (await snapshot(page)).duration).toBeCloseTo(20, 3);
});

/** A 20 s recording at 384 kHz, and a page where over 1 million samples go a piece at a time. */
async function openTwentySeconds(page: Page, name: string) {
  const shape: Shape = { sampleRate: 384_000, seconds: 20, channels: 1, quietFrom: 8, quietTo: 9 };
  const data = samples(shape);
  const bytes = Buffer.from(data.buffer, data.byteOffset, data.byteLength);
  await page.addInitScript(() => {
    (window as unknown as Record<string, number>).__oversample_denoise_in_memory_max = 1_000_000;
  });
  await openRecording(page, { name, buffer: Buffer.concat([wavHeader(shape, bytes.length), bytes]) });
}

test("a browser that cannot write the copy to its storage refuses a long recording", async ({
  page,
}) => {
  await page.addInitScript(() => {
    delete (FileSystemFileHandle.prototype as unknown as Record<string, unknown>).createWritable;
  });
  await openTwentySeconds(page, "old-browser.wav");
  await expect(page.getByRole("button", { name: "Make Denoised Copy" })).toBeDisabled();
  await expect(page.getByText(/Recording too long for this browser/)).toBeVisible();
  // 1 million samples at 384 kHz is 2.6 s, rounded down.
  await expect(page.getByText(/Maximum at 384\skHz: 2 s\. This recording: 20 s\./)).toBeVisible();
  await page.screenshot({ path: "test-results/denoise-long-refused.png" });
});

test("too little browser storage for the copy is reported, with the sizes", async ({ page }) => {
  test.setTimeout(120_000);
  await serveModel(page);
  await page.addInitScript(() => {
    navigator.storage.estimate = async () => ({ quota: 5_000_000, usage: 1_000_000 });
  });
  await openTwentySeconds(page, "full.wav");
  await page.getByRole("button", { name: "Make Denoised Copy" }).click();
  await expect(page.getByText(/Not enough browser storage/)).toBeVisible({ timeout: 60_000 });
  // 7.68 million 4-byte samples and the header: 30.72 MB, rounded up.
  await expect(page.getByText(/Copy needs 31 MB\. Free: 4 MB\./)).toBeVisible();
  await expect(page.getByRole("button", { name: "Make Denoised Copy" })).toBeEnabled();
  expect(await storedFiles(page)).toEqual([]);
  await page.screenshot({ path: "test-results/denoise-long-no-storage.png" });
});

/**
 * A 480 MB WAV of quiet noise, 10 min 25 s at 384 kHz. With it open, the app
 * has over 500 MB of files open and reads every file opened after it from
 * the file a part at a time, as it does a file of 512 MB or more.
 */
function writeBallast(): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "oversample-denoise-"));
  const file = path.join(dir, "ballast.wav");
  const shape: Shape = { sampleRate: 384_000, seconds: 1, channels: 1, quietFrom: 0, quietTo: 1 };
  const second = samples(shape);
  const bytes = Buffer.from(second.buffer, second.byteOffset, second.byteLength);
  const seconds = 625;
  const out = fs.openSync(file, "w");
  fs.writeSync(out, wavHeader(shape, bytes.length * seconds));
  for (let i = 0; i < seconds; i++) fs.writeSync(out, bytes);
  fs.closeSync(out);
  return file;
}

type Shared = Snap & {
  scrollOffset: number;
  zoom: number;
  minFreq: number | null;
  maxFreq: number | null;
  gainDb: number;
  gainMode: string;
};

/** The view and gain that a copy shares with its original. */
async function sharedSettings(page: Page) {
  const s = (await snapshot(page)) as Shared;
  return {
    scrollOffset: s.scrollOffset,
    zoom: s.zoom,
    minFreq: s.minFreq,
    maxFreq: s.maxFreq,
    gainDb: s.gainDb,
    gainMode: s.gainMode,
  };
}

test("a copy that is read from its file pairs with its original, and closing it deletes the file", async ({
  page,
}) => {
  test.setTimeout(600_000);
  const ballast = writeBallast();
  const shape: Shape = { sampleRate: 384_000, seconds: 20, channels: 1, quietFrom: 8, quietTo: 9 };
  const data = samples(shape);
  const bytes = Buffer.from(data.buffer, data.byteOffset, data.byteLength);
  await serveModel(page);
  await page.addInitScript(() => {
    (window as unknown as Record<string, number>).__oversample_denoise_in_memory_max = 1_000_000;
  });
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await page.goto("/");
  const input = page.locator('input[type="file"][accept*=".wav"]').first();
  await input.setInputFiles(ballast);
  await expect(page.locator(".file-item", { hasText: "ballast.wav" })).toBeVisible({
    timeout: 120_000,
  });
  await input.setInputFiles({
    name: "paired.wav",
    mimeType: "audio/wav",
    buffer: Buffer.concat([wavHeader(shape, bytes.length), bytes]),
  });
  await expect(page.locator(".file-item", { hasText: "paired.wav" })).toBeVisible({
    timeout: 60_000,
  });
  await page.waitForFunction(
    () => typeof (window as { __oversample_test?: unknown }).__oversample_test === "function",
  );
  await expect.poll(async () => (await snapshot(page)).currentFile).toBe("paired.wav");

  // A manual gain and a scroll position for the copy to take over.
  await page.locator('button[title="Gain settings"]').click();
  await page.locator('.layer-panel input[type="range"][max="60"]').fill("7");
  await page.getByRole("button", { name: /^Manual/ }).click();
  await page.locator('button[title="Gain settings"]').click();
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("ArrowRight");
  const before = await sharedSettings(page);
  expect(before.gainDb).toBe(7);
  expect(before.scrollOffset).toBeGreaterThan(0);

  await page.locator('button[title="Noise filter settings"]').click();
  await page.getByRole("button", { name: "Make Denoised Copy" }).click();
  const copy = page.locator(".file-item", { hasText: "paired (denoised).wav" });
  await expect(copy).toBeVisible({ timeout: 400_000 });
  await page.locator('button[title="Noise filter settings"]').click();
  await expect.poll(async () => (await snapshot(page)).currentFile).toBe("paired (denoised).wav");

  // The copy is read from its file in the browser's storage: the file is
  // still there, with every sample.
  await expect
    .poll(() => storedFiles(page))
    .toEqual([{ path: expect.stringMatching(/\/paired \(denoised\)\.wav$/), size: 80 + data.length * 4 }]);
  const opened = await snapshot(page);
  expect(opened.channels).toBe(1);
  expect(opened.duration).toBeCloseTo(20, 3);

  // It opens on its original's view and gain, and the Denoised switch and
  // the D key go between the two.
  expect(await sharedSettings(page)).toEqual(before);
  const denoised = page.getByRole("switch", { name: "Denoised" });
  await expect(denoised).toHaveAttribute("aria-checked", "true");
  await denoised.click();
  await expect.poll(async () => (await snapshot(page)).currentFile).toBe("paired.wav");
  await expect(denoised).toHaveAttribute("aria-checked", "false");
  await page.keyboard.press("d");
  await expect.poll(async () => (await snapshot(page)).currentFile).toBe("paired (denoised).wav");

  // Closing the copy deletes its file.
  await copy.locator(".file-item-close").click();
  await expect(copy).toHaveCount(0);
  await expect.poll(() => storedFiles(page)).toEqual([]);
  fs.rmSync(path.dirname(ballast), { recursive: true });
  expect(errors).toEqual([]);
});

test("a stereo recording gives the same samples in pieces as in memory", async ({ page }) => {
  test.setTimeout(400_000);
  // 7 s and a bit: several blocks, and not a whole number of them.
  const shape: Shape = { sampleRate: 44_100, seconds: 7.3, channels: 2, quietFrom: 2, quietTo: 3 };
  const data = samples(shape);
  const bytes = Buffer.from(data.buffer, data.byteOffset, data.byteLength);
  const wav = { name: "pair.wav", buffer: Buffer.concat([wavHeader(shape, bytes.length), bytes]) };
  await serveModel(page);
  await keepWorkerOutput(page);
  const copied = page.locator(".file-item", { hasText: "pair (denoised).wav" });

  // In memory.
  await openRecording(page, wav);
  await page.getByRole("button", { name: "Make Denoised Copy" }).click();
  await expect(copied).toBeVisible({ timeout: 200_000 });
  const whole = await page.evaluate(() =>
    Array.from((window as unknown as { __copy: { whole: Float32Array } }).__copy.whole),
  );
  expect(whole.length).toBe(data.length);

  // In pieces: every recording goes that way when the most that is denoised
  // in memory is 0 samples.
  await openRecording(page, wav);
  await page.evaluate(() => {
    (window as unknown as Record<string, number>).__oversample_denoise_in_memory_max = 0;
  });
  await page.getByRole("button", { name: "Make Denoised Copy" }).click();
  await expect(copied).toBeVisible({ timeout: 200_000 });
  const pieces = await page.evaluate(() =>
    (window as unknown as { __copy: { pieces: Float32Array[] } }).__copy.pieces.flatMap((p) =>
      Array.from(p),
    ),
  );
  expect(pieces.length).toBe(whole.length);
  // Both are 32-bit floats, so equal numbers are equal bit for bit.
  const differing = pieces.filter((s, i) => s !== whole[i]).length;
  expect(differing).toBe(0);
  expect((await snapshot(page)).channels).toBe(2);
});

/**
 * Resident memory of the browser's processes, in MB, from /proc (Linux). The
 * browser is started by this test's own process, so its processes are the
 * ones that descend from it.
 */
function browserRssMb(): number {
  const children = new Map<number, number[]>();
  const rss = new Map<number, number>();
  for (const entry of fs.readdirSync("/proc")) {
    if (!/^\d+$/.test(entry)) continue;
    try {
      const status = fs.readFileSync(`/proc/${entry}/status`, "utf8");
      const parent = Number(/^PPid:\s+(\d+)/m.exec(status)?.[1]);
      rss.set(Number(entry), Number(/^VmRSS:\s+(\d+)/m.exec(status)?.[1] ?? 0) / 1024);
      children.set(parent, [...(children.get(parent) ?? []), Number(entry)]);
    } catch {
      // The process ended while this read it.
    }
  }
  let total = 0;
  const add = (pid: number) => {
    for (const child of children.get(pid) ?? []) {
      total += rss.get(child) ?? 0;
      add(child);
    }
  };
  add(process.pid);
  return total;
}

for (const shape of [
  { sampleRate: 384_000, seconds: 600, channels: 1, quietFrom: 300, quietTo: 302 },
  { sampleRate: 44_100, seconds: 1800, channels: 2, quietFrom: 900, quietTo: 902 },
] satisfies Shape[]) {
  const label = `${shape.seconds / 60} min at ${shape.sampleRate / 1000} kHz, ${shape.channels} ch`;
  test(`memory and time: ${label}`, async ({ page: privatePage, baseURL }) => {
    test.skip(!process.env.DENOISE_MEASURE, "set DENOISE_MEASURE=1 to run the measurements");
    test.setTimeout(3 * 3600_000);
    const inPrivate = process.env.DENOISE_MEASURE === "private";
    const profile = fs.mkdtempSync(path.join(os.tmpdir(), "oversample-profile-"));
    const context = inPrivate
      ? null
      : await chromium.launchPersistentContext(profile, {
          baseURL,
          viewport: { width: 1280, height: 720 },
        });
    const page = context ? await context.newPage() : privatePage;
    const file = writeWav("measure.wav", shape);
    await serveModel(page);
    // The app reads a recording of this size from its file for as long as
    // it is open, so the file stays until the test is over.
    await openRecording(page, file);
    // Let the opened recording settle before the baseline.
    await page.waitForTimeout(15_000);
    const baseline = browserRssMb();
    console.log(`${label}: ${Math.round(baseline)} MB before the copy is started`);
    let peak = baseline;
    const readings: number[] = [];
    // A reading every 2 s for the peak, and one in 15 kept for the report.
    let tick = 0;
    const timer = setInterval(() => {
      const now = browserRssMb();
      if (tick++ % 15 === 0) readings.push(Math.round(now));
      peak = Math.max(peak, now);
    }, 2000);

    const started = Date.now();
    await page.getByRole("button", { name: "Make Denoised Copy" }).click();
    await expect(page.getByText(/Denoising: \d+%/)).toBeVisible({ timeout: 600_000 });
    const firstBlock = Date.now();
    await expect(page.locator(".file-item", { hasText: "measure (denoised).wav" })).toBeVisible({
      timeout: 3 * 3600_000,
    });
    const finished = Date.now();
    clearInterval(timer);
    await expect.poll(async () => (await snapshot(page)).currentFile).toBe("measure (denoised).wav");
    const copy = await snapshot(page);
    expect(copy.channels).toBe(shape.channels);
    expect(copy.duration).toBeCloseTo(shape.seconds, 2);
    const stored = await storedFiles(page);

    // The D key goes to the original and back.
    await page.locator('button[title="Noise filter settings"]').click();
    await page.keyboard.press("d");
    await expect.poll(async () => (await snapshot(page)).currentFile).toBe("measure.wav");
    await page.keyboard.press("d");
    await expect.poll(async () => (await snapshot(page)).currentFile).toBe("measure (denoised).wav");
    await page.screenshot({ path: `test-results/denoise-measure-${shape.sampleRate}.png` });

    await context?.close();
    fs.rmSync(profile, { recursive: true });
    fs.rmSync(path.dirname(file), { recursive: true });
    console.log(
      JSON.stringify({
        label,
        storage: inPrivate ? "in memory (private window)" : "on disk",
        baselineMb: Math.round(baseline),
        peakMb: Math.round(peak),
        aboveBaselineMb: Math.round(peak - baseline),
        untilFirstBlockS: (firstBlock - started) / 1000,
        totalS: (finished - started) / 1000,
        stored,
        readings,
      }),
    );
  });
}
