import { test, expect, type Page } from "@playwright/test";

/**
 * HET playback of files whose highest frequency (half the sample rate) is
 * below the usual bat range.
 *
 * Choosing HET turns on high-frequency mode (HFR), which picks a default band:
 * the file species' call range, or the lowest bat frequency (16 kHz) up to the
 * file's highest frequency. Before the fix, a band that lay wholly above the
 * file's highest frequency came out empty, and the effect that sets the
 * default band ran again on its own change forever, so the page stopped
 * responding. Myotis fimbriatus calls at 35-80 kHz, above 44.1 and 48 kHz
 * files; a 16 kHz file is below the lowest bat frequency even with no species.
 *
 * Each test opens one demo recording with a generated mono 4 kHz tone at the
 * case's sample rate, chooses HET, checks the page still answers and the band
 * fits the file, then presses Play and checks the app schedules finite audio.
 */

test.use({
  viewport: { width: 1440, height: 900 },
  launchOptions: { args: ["--autoplay-policy=no-user-gesture-required"] },
});

/** Mono 16-bit WAV of a quiet 4 kHz tone. */
function wav(rate: number, seconds: number): Buffer {
  const n = Math.round(rate * seconds);
  const data = Buffer.alloc(n * 2);
  for (let i = 0; i < n; i++) {
    data.writeInt16LE(Math.round(Math.sin((2 * Math.PI * 4000 * i) / rate) * 8000), i * 2);
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

interface Case {
  rate: number;
  seconds: number;
  /** Give the recording the species Myotis fimbriatus (calls at 35-80 kHz). */
  species: boolean;
}

/** Opens demo XC815138 with the case's audio and species. */
async function openCase(page: Page, c: Case) {
  const sidecar = {
    id: "815138",
    en: c.species ? "Fringed Long-footed Myotis" : "",
    gen: c.species ? "Myotis" : "",
    sp: c.species ? "fimbriatus" : "",
    rec: "Test",
    cnt: "Brazil",
    method: "field recording",
    rmk: "",
    url: "https://xeno-canto.org/815138",
  };
  const audio = wav(c.rate, c.seconds);
  const handler = async (route: any) => {
    const url: string = route.request().url();
    if (url.endsWith("/index.json")) {
      return route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          version: 1,
          sounds: [
            {
              en: sidecar.en,
              filename: "XC815138 demo.wav",
              metadata: "XC815138 demo.xc.json",
              source: "xeno-canto",
              species: c.species ? "Myotis fimbriatus" : "",
              xc_id: 815138,
            },
          ],
        }),
      });
    }
    if (url.endsWith(".xc.json")) {
      return route.fulfill({ contentType: "application/json", body: JSON.stringify(sidecar) });
    }
    if (url.endsWith(".wav")) return route.fulfill({ contentType: "audio/wav", body: audio });
    return route.fulfill({ status: 404, body: "" });
  };
  await page.route("https://archive.oversample.com/**", handler);
  await page.route("https://cdn.jsdelivr.net/gh/pengowray/bat-demo-sounds@main/**", handler);
  await page.goto("/#XC815138");
  await expect(page.locator(".toolbar-badge-row .badge-sample-rate")).toBeVisible({
    timeout: 30_000,
  });
}

type Tap = { buffers: { length: number; peak: number; finite: boolean }[] };

/** Records every buffer the app hands to Web Audio for playback. */
function installAudioTap() {
  const tap: Tap = { buffers: [] };
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
      tap.buffers.push({ length: this.buffer.length, peak, finite });
    }
    return start.apply(this, args);
  };
}

/**
 * Runs `fn` in the page and says whether it finished within `ms`. A page stuck
 * in a loop never finishes, and Playwright's own actions would wait for the
 * whole test timeout.
 */
async function finishes(page: Page, fn: () => unknown, ms = 5_000): Promise<boolean> {
  return Promise.race([
    page.evaluate(fn).then(() => true),
    new Promise<boolean>((res) => setTimeout(() => res(false), ms)),
  ]);
}

/** Whether the page runs a timer within 5 s. */
function answers(page: Page): Promise<boolean> {
  return finishes(page, () => new Promise((res) => setTimeout(res, 0)));
}

type Snapshot = {
  hfrEnabled: boolean;
  playbackMode: string;
  bandLo: number;
  bandHi: number;
  hetFreq: number;
  hetCutoff: number;
};

function snapshot(page: Page): Promise<Snapshot> {
  return page.evaluate(
    () => (window as unknown as { __oversample_test: () => Snapshot }).__oversample_test(),
  );
}

const cases: [string, Case][] = [
  ["44.1 kHz, 1 s, species above the file's range", { rate: 44_100, seconds: 1, species: true }],
  ["48 kHz, 3 s, species above the file's range", { rate: 48_000, seconds: 3, species: true }],
  ["16 kHz, 1 s, no species", { rate: 16_000, seconds: 1, species: false }],
];

test.describe("HET on files below the bat range", () => {
  for (const [name, c] of cases) {
    test(`${name}: choosing HET keeps the page responsive and plays`, async ({ page }) => {
      await page.addInitScript(installAudioTap);
      await openCase(page, c);
      const nyquist = c.rate / 2;

      const clickHet = () =>
        (document.querySelector('button[title^="HET"]') as HTMLButtonElement).click();
      expect(await finishes(page, clickHet), "choosing HET finishes").toBe(true);
      expect(await answers(page), "page answers after choosing HET").toBe(true);

      // HFR is on with a band inside the file's range, and HET mixes below
      // the file's highest frequency.
      const s = await snapshot(page);
      expect(s.hfrEnabled).toBe(true);
      expect(s.playbackMode).toBe("Heterodyne");
      expect(s.bandLo).toBeGreaterThanOrEqual(0);
      expect(s.bandHi).toBeGreaterThan(s.bandLo);
      expect(s.bandHi).toBeLessThanOrEqual(nyquist);
      expect(s.hetFreq).toBeGreaterThan(0);
      expect(s.hetFreq).toBeLessThan(nyquist);
      expect(s.hetCutoff).toBeGreaterThan(0);
      expect(s.hetCutoff).toBeLessThan(nyquist);

      await page.locator(".play-main-btn").click();
      await page.waitForFunction(
        () => (window as unknown as { __audioTap: Tap }).__audioTap.buffers.length >= 1,
        undefined,
        { timeout: 15_000 },
      );
      expect(await answers(page), "page answers while playing HET").toBe(true);
      const { buffers } = await page.evaluate(
        () => (window as unknown as { __audioTap: Tap }).__audioTap,
      );
      expect(buffers.every((b) => b.finite && b.length > 0)).toBe(true);
    });
  }
});
