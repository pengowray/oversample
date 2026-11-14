import { test, expect, type Page } from "@playwright/test";

/**
 * HET playback of files whose species calls above the file's Nyquist
 * frequency (half the sample rate), or whose Nyquist is below the lowest bat
 * frequency (16 kHz).
 *
 * Turning on high-frequency reception (HFR) sets a default band from the
 * file's species or the lowest bat frequency (FocusRange::hfr_default, used
 * in mode_button.rs). An empty default band made the page stop responding
 * at any sample rate, for example Myotis fimbriatus (35-80 kHz) at 44.1 kHz
 * and Rhinolophus hipposideros (105-115 kHz) at 192 kHz. A default band cut
 * to a sliver at Nyquist, such as 22.00-22.05 kHz for Eptesicus serotinus at
 * 44.1 kHz, made HET play nothing audible.
 *
 * Each test opens one demo recording with a generated mono tone 1 kHz from
 * the expected HET carrier, turns HFR on, and checks that the page still
 * answers and the band is the expected one. Then it presses Play and checks
 * that the app schedules finite, audible audio.
 */

test.use({
  viewport: { width: 1440, height: 900 },
  launchOptions: { args: ["--autoplay-policy=no-user-gesture-required"] },
});

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

/** Xeno-canto name fields for each species the cases use. */
const SPECIES = {
  none: { en: "", gen: "", sp: "" },
  // Bat book: 35-80 kHz.
  fimbriatus: { en: "Fringed Long-footed Myotis", gen: "Myotis", sp: "fimbriatus" },
  // Bat book: 22-55 kHz.
  serotinus: { en: "Serotine", gen: "Eptesicus", sp: "serotinus" },
  // Bat book: 105-115 kHz.
  hipposideros: { en: "Lesser Horseshoe Bat", gen: "Rhinolophus", sp: "hipposideros" },
};

interface Case {
  rate: number;
  seconds: number;
  species: keyof typeof SPECIES;
  /** The default band HFR should pick, in Hz. */
  band: [number, number];
  /** A tone 1 kHz from the HET carrier that `band` gives. */
  toneHz: number;
  /** How HFR is turned on: by choosing HET, or by PASS > ON and then HET. */
  via?: "het" | "pass";
}

/** Opens demo XC815138 with the case's audio and species. */
async function openCase(page: Page, c: Case) {
  const s = SPECIES[c.species];
  const sidecar = {
    id: "815138",
    en: s.en,
    gen: s.gen,
    sp: s.sp,
    rec: "Test",
    cnt: "Brazil",
    method: "field recording",
    rmk: "",
    url: "https://xeno-canto.org/815138",
  };
  const audio = wav(c.rate, c.seconds, c.toneHz);
  const handler = async (route: any) => {
    const url: string = route.request().url();
    if (url.endsWith("/index.json")) {
      return route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          version: 1,
          sounds: [
            {
              en: s.en,
              filename: "XC815138 demo.wav",
              metadata: "XC815138 demo.xc.json",
              source: "xeno-canto",
              species: s.gen ? `${s.gen} ${s.sp}` : "",
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
async function finishes<A>(
  page: Page,
  fn: (arg: A) => unknown,
  arg: A,
  ms = 5_000,
): Promise<boolean> {
  return Promise.race([
    page.evaluate(fn, arg).then(() => true),
    new Promise<boolean>((res) => setTimeout(() => res(false), ms)),
  ]);
}

/** Whether the page runs a timer within 5 s. */
function answers(page: Page): Promise<boolean> {
  return finishes(page, () => new Promise((res) => setTimeout(res, 0)), undefined);
}

/** Clicks the first element matching `selector` from inside the page. */
function clickIn(selector: string) {
  (document.querySelector(selector) as HTMLElement).click();
}

/** Opens the PASS menu and clicks its ON button, which turns HFR on. */
async function passOn() {
  // PASS is a switch button: its bottom row opens the menu with ON.
  const pass = [...document.querySelectorAll(".switch-btn")].find(
    (b) => b.querySelector(".switch-btn-name")?.textContent?.trim() === "PASS",
  );
  (pass?.querySelector(".switch-btn-value") as HTMLElement).click();
  await new Promise((r) => setTimeout(r, 200));
  const on = [...document.querySelectorAll(".layer-panel button")].find(
    (b) => b.textContent?.trim() === "ON",
  );
  (on as HTMLElement).click();
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

// HET's automatic cutoff is half the band (at most 15 kHz) and its carrier is
// one cutoff above the band's low edge, so each tone is 1 kHz below that.
const cases: [string, Case][] = [
  [
    "44.1 kHz, species above the file's range",
    { rate: 44_100, seconds: 1, species: "fimbriatus", band: [16_000, 22_050], toneHz: 18_025 },
  ],
  [
    "48 kHz, 3 s, species above the file's range",
    { rate: 48_000, seconds: 3, species: "fimbriatus", band: [16_000, 24_000], toneHz: 19_000 },
  ],
  [
    "16 kHz, no species",
    { rate: 16_000, seconds: 1, species: "none", band: [0, 8_000], toneHz: 3_000 },
  ],
  [
    "44.1 kHz, species range cut to 50 Hz at Nyquist",
    { rate: 44_100, seconds: 1, species: "serotinus", band: [16_000, 22_050], toneHz: 18_025 },
  ],
  [
    "192 kHz, species above the file's range",
    {
      rate: 192_000,
      seconds: 1,
      species: "hipposideros",
      band: [16_000, 96_000],
      toneHz: 30_000,
    },
  ],
  [
    "192 kHz, species above the file's range, HFR on from PASS > ON",
    {
      rate: 192_000,
      seconds: 1,
      species: "hipposideros",
      band: [16_000, 96_000],
      toneHz: 30_000,
      via: "pass",
    },
  ],
  // The species range fits this file, so it is the band. This shows the
  // species above resolves from the recording's metadata.
  [
    "384 kHz, species inside the file's range",
    {
      rate: 384_000,
      seconds: 1,
      species: "hipposideros",
      band: [105_000, 115_000],
      toneHz: 109_000,
    },
  ],
];

test.describe("HET on files with a species above Nyquist or below the bat range", () => {
  for (const [name, c] of cases) {
    test(`${name}: turning HFR on keeps the page responsive and HET is audible`, async ({
      page,
    }) => {
      await page.addInitScript(installAudioTap);
      await openCase(page, c);
      const nyquist = c.rate / 2;

      if (c.via === "pass") {
        expect(await finishes(page, passOn, undefined), "PASS > ON finishes").toBe(true);
        expect(await answers(page), "page answers after PASS > ON").toBe(true);
        expect((await snapshot(page)).hfrEnabled).toBe(true);
      }
      expect(
        await finishes(page, clickIn, 'button[title^="HET"]'),
        "choosing HET finishes",
      ).toBe(true);
      expect(await answers(page), "page answers after choosing HET").toBe(true);

      // HFR is on with the expected band, and HET mixes below the file's
      // highest frequency.
      const s = await snapshot(page);
      expect(s.hfrEnabled).toBe(true);
      expect(s.playbackMode).toBe("Heterodyne");
      expect(s.bandLo).toBeCloseTo(c.band[0], 0);
      expect(s.bandHi).toBeCloseTo(c.band[1], 0);
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
      // The tone is 1 kHz from the carrier, so HET plays it at 1 kHz.
      expect(Math.max(...buffers.map((b) => b.peak)), "loudest sample").toBeGreaterThan(0.01);
    });
  }
});

test.describe("Band presets on a file below the species' range", () => {
  test("the bat book's current species preset applies a band that fits the file", async ({
    page,
  }) => {
    // Myotis fimbriatus (35-80 kHz) in a 44.1 kHz file. The preset applied
    // 35-80 kHz as it was, so the band shown and played was 22.05-22.05 kHz.
    const c: Case = {
      rate: 44_100,
      seconds: 1,
      species: "fimbriatus",
      band: [16_000, 22_050],
      toneHz: 18_025,
    };
    await page.addInitScript(installAudioTap);
    await openCase(page, c);
    expect(
      await finishes(page, clickIn, 'button[title^="HET"]'),
      "choosing HET finishes",
    ).toBe(true);

    // Opening the bat book selects the recording's species on its current page.
    await page.getByText("Bat book", { exact: true }).first().click();
    await expect(page.locator(".bat-book-chip.selected")).toContainText("Fringed Long-footed");
    await page.locator('button[title="Band range / presets"]').filter({ visible: true }).click();
    const preset = page.locator(".layer-panel button", { hasText: "Bat book: current page" });
    await expect(preset).toContainText("(35–80 kHz)");
    await preset.click();
    expect(await answers(page), "page answers after the preset").toBe(true);

    const s = await snapshot(page);
    expect(s.hfrEnabled).toBe(true);
    expect(s.bandLo).toBeCloseTo(c.band[0], 0);
    expect(s.bandHi).toBeCloseTo(c.band[1], 0);

    await page.locator(".play-main-btn").click();
    await page.waitForFunction(
      () => (window as unknown as { __audioTap: Tap }).__audioTap.buffers.length >= 1,
      undefined,
      { timeout: 15_000 },
    );
    const { buffers } = await page.evaluate(
      () => (window as unknown as { __audioTap: Tap }).__audioTap,
    );
    expect(buffers.every((b) => b.finite && b.length > 0)).toBe(true);
    expect(Math.max(...buffers.map((b) => b.peak)), "loudest sample").toBeGreaterThan(0.01);
  });
});
