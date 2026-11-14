import { test, expect, type Page, type Route } from "@playwright/test";
import * as fs from "node:fs";
import * as path from "node:path";

/**
 * The species call-range note under the Info panel's Sample rate row
 * (src/species_check.rs): a file whose sample rate is too low for the
 * echolocation calls of the species it is labelled with.
 *
 * The first group opens one demo recording with a generated WAV and a
 * Xeno-canto sidecar written for the case (the metadata of the Xeno-canto
 * record the case is named after). The second group opens real demo
 * recordings, served from a local bat-demo-sounds checkout
 * (../batchi-extras/bat-demo-sounds, or BAT_DEMO_SOUNDS_DIR), and is skipped
 * when there is none.
 */

/** Mono 16-bit WAV of a quiet 4 kHz tone, with a GUANO chunk when given. */
function wav(rate: number, seconds: number, guano?: string): Buffer {
  const n = Math.round(rate * seconds);
  const data = Buffer.alloc(n * 2);
  for (let i = 0; i < n; i++) {
    data.writeInt16LE(Math.round(Math.sin((2 * Math.PI * 4000 * i) / rate) * 8000), i * 2);
  }
  const chunks = [data];
  if (guano) {
    let text = Buffer.from(guano, "utf8");
    const head = Buffer.alloc(8);
    head.write("guan", 0);
    head.writeUInt32LE(text.length, 4);
    if (text.length % 2) text = Buffer.concat([text, Buffer.alloc(1)]);
    chunks.push(head, text);
  }
  const body = Buffer.concat(chunks);
  const h = Buffer.alloc(44);
  h.write("RIFF", 0);
  h.writeUInt32LE(36 + body.length, 4);
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
  return Buffer.concat([h, body]);
}

interface Case {
  /** Scientific name, "Genus species". */
  species: string;
  /** Xeno-canto sound type ("" for none). */
  type: string;
  rate: number;
  /** Other sidecar fields (cnt, rmk, dvc, ...). */
  fields?: Record<string, string>;
  guano?: string;
}

async function routeDemo(page: Page, handler: (route: Route, url: string) => unknown) {
  const h = (route: Route) => handler(route, route.request().url());
  await page.route("https://archive.oversample.com/**", h);
  await page.route("https://cdn.jsdelivr.net/gh/pengowray/bat-demo-sounds@main/**", h);
}

/** Opens demo XC815138 with the case's sidecar and a generated WAV. */
async function openCase(page: Page, c: Case) {
  const [gen, sp] = c.species.split(" ");
  const sidecar = {
    id: "815138",
    en: "Test bat",
    gen,
    sp,
    rec: "Test",
    cnt: "Sweden",
    method: "field recording",
    rmk: "",
    type: c.type,
    url: "https://xeno-canto.org/815138",
    ...c.fields,
  };
  const audio = wav(c.rate, 1, c.guano);
  await routeDemo(page, (route, url) => {
    if (url.endsWith("/index.json")) {
      return route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          version: 1,
          sounds: [
            {
              en: "Test bat",
              filename: "XC815138 demo.wav",
              metadata: "XC815138 demo.xc.json",
              source: "xeno-canto",
              species: c.species,
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
  });
  await page.goto("/#XC815138");
  await waitForFile(page);
}

async function waitForFile(page: Page) {
  await expect(page.locator(".toolbar-badge-row .badge-sample-rate")).toBeVisible({
    timeout: 60_000,
  });
}

async function showInfo(page: Page) {
  const btn = page.locator('button[title="Show info panel"]');
  if (await btn.count()) await btn.first().click();
  await expect(page.locator(".right-sidebar")).not.toHaveClass(/collapsed/);
  await expect(
    page.locator(".metadata-row-inline .setting-label", { hasText: /^Sample rate$/ }),
  ).toBeVisible();
}

/** The class and text of each line of a note. */
async function lines(page: Page, note: string) {
  return page
    .locator(`.${note} > div`)
    .evaluateAll((els) => els.map((e) => [e.className, e.textContent]));
}

/** `s` as the app writes it: a no-break space before each "kHz", and a word
 * joiner after each range's en dash. */
function spaced(s: string): string {
  return s.replaceAll(" kHz", " kHz").replaceAll("–", "–⁠");
}

const WARN = "species-range-note-warning";
const DETAIL = "species-range-note-detail";
const OTHER =
  "This file may be a bat detector's output (heterodyne, time expansion or frequency " +
  "division), or contain only lower-frequency sounds such as social calls.";
const tenfold = (real: string) =>
  spaced(
    `Probably time-expanded ×10 (recorded at ${real} kHz). Frequencies shown are one ` +
      "tenth of the real ones, and durations are ten times longer.",
  );
const fact = (species: string, range: string, limit: string, corrected = false) =>
  spaced(
    `${species}: echolocation calls at ${range} kHz, but ` +
      (corrected ? "after time-expansion correction, " : "") +
      `this file can only contain frequencies up to ${limit} kHz.`,
  );

/** The note sits right under the Sample rate row, or under the
 * time-expansion note when there is one. */
async function expectUnderSampleRate(page: Page, note: string, afterTimeExpansion = false) {
  const prev = await page
    .locator(`.${note}`)
    .evaluate((n) => [n.previousElementSibling!.className, n.previousElementSibling!.textContent]);
  if (afterTimeExpansion) {
    expect(prev[0]).toBe("metadata-note");
    expect(prev[1]).toContain("time expansion");
  } else {
    expect(prev[1]).toContain("Sample rate");
  }
}

/** Nothing new in the toolbar's badge row or the file list. */
async function expectNoBadge(page: Page) {
  for (const where of [".toolbar-badge-row", ".file-item"]) {
    await expect(page.locator(where).first()).not.toContainText(/time-expanded|echolocation/i);
  }
  await expect(page.locator(".file-badge-heterodyne")).toHaveCount(0);
}

test.describe("species call-range note", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("x10 header rate: probably time-expanded (XC1001172)", async ({ page }) => {
    await openCase(page, { species: "Eptesicus nilssonii", type: "echolocation", rate: 25_600 });
    await showInfo(page);
    expect(await lines(page, "species-range-note")).toEqual([
      [WARN, tenfold("256")],
      [DETAIL, fact("Eptesicus nilssonii", "25–45", "12.8")],
    ]);
    await expectUnderSampleRate(page, "species-range-note");
    await expect(page.locator(".heterodyne-note")).toHaveCount(0);
    await expectNoBadge(page);
  });

  test("x10 header rate (XC844459)", async ({ page }) => {
    await openCase(page, {
      species: "Pipistrellus pipistrellus",
      type: "echolocation",
      rate: 38_400,
      fields: { cnt: "Netherlands", rmk: "The recording has not been modified." },
    });
    await showInfo(page);
    expect(await lines(page, "species-range-note")).toEqual([
      [WARN, tenfold("384")],
      [DETAIL, fact("Pipistrellus pipistrellus", "42–51", "19.2")],
    ]);
  });

  test("other rate: possibilities only, all muted (XC1099113)", async ({ page }) => {
    await openCase(page, {
      species: "Pipistrellus kuhlii",
      type: "echolocation",
      rate: 48_000,
      fields: { cnt: "Italy", dvc: "Apodemus pippyg", rmk: "Rate slowed down by 8 times" },
    });
    await showInfo(page);
    expect(await lines(page, "species-range-note")).toEqual([
      [DETAIL, fact("Pipistrellus kuhlii", "35–45", "24")],
      [DETAIL, OTHER],
    ]);
    await expectUnderSampleRate(page, "species-range-note");
    await expectNoBadge(page);
  });

  test("no sound type is checked; 32 kHz is not a x10 rate (XC759031)", async ({ page }) => {
    await openCase(page, {
      species: "Pipistrellus nathusii",
      type: "",
      rate: 32_000,
      fields: {
        cnt: "Germany",
        dvc: "Pettersson D240x",
        // The time-expansion wording keeps the D240x from giving a level C
        // heterodyne hint.
        rmk: "Pettersson D240x original recording (=time expanded 10x, sampling rate 44.100 Hz)",
      },
    });
    await showInfo(page);
    expect(await lines(page, "species-range-note")).toEqual([
      [DETAIL, fact("Pipistrellus nathusii", "35–44", "16")],
      [DETAIL, OTHER],
    ]);
  });

  test("a synonym keeps the recording's name (Lasiurus cinereus)", async ({ page }) => {
    await openCase(page, {
      species: "Lasiurus cinereus",
      type: "echolocation",
      rate: 22_050,
      fields: { cnt: "United States" },
    });
    await showInfo(page);
    expect(await lines(page, "species-range-note")).toEqual([
      [DETAIL, fact("Lasiurus cinereus", "17–36", "11")],
      [DETAIL, OTHER],
    ]);
  });

  test("a subspecies with its own entry uses its own range", async ({ page }) => {
    // Rhinolophus ferrumequinum is 78-84 kHz; the nippon entry is only in
    // the Japan book, and the recording is from Sweden.
    await openCase(page, {
      species: "Rhinolophus ferrumequinum",
      type: "echolocation",
      rate: 44_100,
      fields: { sp: "ferrumequinum nippon" },
    });
    await showInfo(page);
    expect(await lines(page, "species-range-note")).toEqual([
      [DETAIL, fact("Rhinolophus ferrumequinum nippon", "58–77", "22.1")],
      [DETAIL, OTHER],
    ]);
  });

  test("above 96 kHz the note has only the species line", async ({ page }) => {
    await openCase(page, {
      species: "Rhinolophus hipposideros",
      type: "echolocation",
      rate: 192_000,
      fields: { dvc: "AudioMoth" },
    });
    await showInfo(page);
    expect(await lines(page, "species-range-note")).toEqual([
      [DETAIL, fact("Rhinolophus hipposideros", "105–115", "96")],
    ]);
  });

  test("corrected for time expansion and still too low", async ({ page }) => {
    await openCase(page, {
      species: "Pipistrellus pygmaeus",
      type: "echolocation",
      rate: 25_600,
      guano: "GUANO|Version: 1.0\nTE: 2\n",
    });
    await showInfo(page);
    expect(await lines(page, "species-range-note")).toEqual([
      [DETAIL, fact("Pipistrellus pygmaeus", "51–60", "25.6", true)],
      [DETAIL, OTHER],
    ]);
    await expectUnderSampleRate(page, "species-range-note", true);
  });

  test("heterodyne level C gets the species line last (XC933228)", async ({ page }) => {
    await openCase(page, {
      species: "Eptesicus nilssonii",
      type: "echolocation, feeding buzz",
      rate: 44_100,
      fields: {
        dvc: "ultrasonic detector and mobile phone",
        rmk: "recorded sound from ultrasonic detectors spaker with mobilphone",
      },
    });
    await showInfo(page);
    const d = "heterodyne-note-detail";
    expect(await lines(page, "heterodyne-note")).toEqual([
      [
        d,
        "Possibly recorded from a bat detector's output " +
          "(heterodyne, time expansion or frequency division).",
      ],
      [d, 'Remarks: "recorded sound from ultrasonic detectors spaker with mobilphone"'],
      [d, "If so, frequencies are not the bat's call frequencies."],
      [d, fact("Eptesicus nilssonii", "25–45", "22.1")],
    ]);
    await expect(page.locator(".species-range-note")).toHaveCount(0);
  });

  test("heterodyne level A keeps its note alone (XC1034529)", async ({ page }) => {
    await openCase(page, {
      species: "Eptesicus nilssonii",
      type: "echolocation",
      rate: 48_000,
      fields: { rmk: "hetrodyne detector set at 31,9 kHz" },
    });
    await showInfo(page);
    const note = await lines(page, "heterodyne-note");
    expect(note.length).toBe(4);
    expect(note.map((l) => l[1]).join("\n")).not.toContain("echolocation calls at");
    await expect(page.locator(".species-range-note")).toHaveCount(0);
  });

  test("heterodyne level C with the range reached: no species line (XC1062080)", async ({
    page,
  }) => {
    await openCase(page, {
      species: "Myotis blythii",
      type: "echolocation",
      rate: 44_100,
      fields: { dvc: "Pettersson D980" },
    });
    await showInfo(page);
    expect((await lines(page, "heterodyne-note")).length).toBe(4);
    await expect(page.locator(".species-range-note")).toHaveCount(0);
  });

  const NOTHING: [string, Case][] = [
    [
      "a social call (XC1100088)",
      { species: "Pipistrellus nathusii", type: "social call", rate: 38_400 },
    ],
    [
      "a feeding buzz alone (XC1193372)",
      { species: "Nyctalus leisleri", type: "feeding buzz", rate: 24_000 },
    ],
    [
      "a song at 192 kHz (XC1193786)",
      { species: "Vespertilio murinus", type: "song", rate: 192_000 },
    ],
    [
      "a limit exactly at the lowest call frequency",
      { species: "Eptesicus nilssonii", type: "echolocation", rate: 50_000 },
    ],
    [
      "Tadarida teniotis at 22.05 kHz (XC1154390)",
      { species: "Tadarida teniotis", type: "echolocation", rate: 22_050 },
    ],
    [
      "a x10 file the app corrected (GUANO TE 10)",
      {
        species: "Pipistrellus pipistrellus",
        type: "echolocation",
        rate: 38_400,
        guano: "GUANO|Version: 1.0\nTE: 10\n",
      },
    ],
    [
      "no species (Mystery mystery)",
      { species: "Mystery mystery", type: "echolocation", rate: 8_000 },
    ],
    [
      "a porpoise (Phocoena dioptrica)",
      { species: "Phocoena dioptrica", type: "echolocation", rate: 96_000 },
    ],
  ];
  for (const [name, c] of NOTHING) {
    test(`no note for ${name}`, async ({ page }) => {
      await openCase(page, c);
      await showInfo(page);
      await expect(page.locator(".species-range-note")).toHaveCount(0);
      await expect(page.locator(".heterodyne-note")).toHaveCount(0);
      if (c.guano) {
        await expect(page.locator(".metadata-note", { hasText: /time expansion/ })).toBeVisible();
      }
    });
  }

  test("a local file's GUANO species is checked", async ({ page }) => {
    await openCase(page, { species: "Myotis fimbriatus", type: "social call", rate: 192_000 });
    await page
      .locator('input[type="file"]')
      .first()
      .setInputFiles({
        name: "guano-kuhlii.wav",
        mimeType: "audio/wav",
        buffer: wav(48_000, 1, "GUANO|Version: 1.0\nSpecies|Manual: Pipistrellus kuhlii\n"),
      });
    await expect(page.locator(".file-item.active .file-item-name")).toHaveText(
      "guano-kuhlii.wav",
    );
    await showInfo(page);
    expect(await lines(page, "species-range-note")).toEqual([
      [DETAIL, fact("Pipistrellus kuhlii", "35–45", "24")],
      [DETAIL, OTHER],
    ]);
  });
});

test.describe("species call-range note on a phone", () => {
  test.use({
    viewport: { width: 375, height: 812 },
    isMobile: true,
    hasTouch: true,
    userAgent:
      "Mozilla/5.0 (Linux; Android 14; Pixel 7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36",
  });

  test("the note stays inside the Info panel and keeps ranges whole", async ({ page }) => {
    await openCase(page, { species: "Eptesicus nilssonii", type: "echolocation", rate: 25_600 });
    await page.locator('button[title="Show info panel"]').first().tap();
    const note = page.locator(".species-range-note");
    await expect(note).toBeVisible();
    const panel = (await page.locator(".right-sidebar").boundingBox())!;
    const box = (await note.boundingBox())!;
    expect(box.x + box.width).toBeLessThanOrEqual(panel.x + panel.width + 0.5);
    // No wrapped line ends with a range's dash or starts with "kHz".
    const visualLines = await note.evaluate((n) => {
      const out: string[] = [];
      for (const line of Array.from(n.children)) {
        const text = line.firstChild!;
        const chars = text.textContent!;
        const range = document.createRange();
        let top: number | null = null;
        for (let i = 0; i < chars.length; i++) {
          range.setStart(text, i);
          range.setEnd(text, i + 1);
          const rect = range.getBoundingClientRect();
          // A zero-width character (the word joiner) stays on the line before it.
          if (rect.width > 0 && Math.round(rect.top) !== top) {
            top = Math.round(rect.top);
            out.push("");
          }
          out[out.length - 1] += chars[i];
        }
      }
      return out;
    });
    expect(visualLines.length).toBeGreaterThan(2);
    for (const visual of visualLines) {
      expect(visual.trimEnd()).not.toMatch(/[\u2013\u2060]$/);
      expect(visual.trimStart()).not.toMatch(/^kHz/);
    }
  });
});

/** The local bat-demo-sounds checkout, if there is one: BAT_DEMO_SOUNDS_DIR,
 * else ../batchi-extras/bat-demo-sounds beside the main checkout, seen from
 * the main checkout or from a worktree under its .claude/worktrees/. */
const DEMO_DIR = [
  process.env.BAT_DEMO_SOUNDS_DIR,
  "../batchi-extras/bat-demo-sounds",
  "../../../../batchi-extras/bat-demo-sounds",
]
  .filter((d): d is string => !!d)
  .map((d) => path.resolve(process.cwd(), d))
  .find((d) => fs.existsSync(path.join(d, "index.json")));

/** Serves the demo archive from the local checkout, with the recording's
 * sidecar changed by `edit` when given. */
async function openDemo(page: Page, xc: string, edit?: (sidecar: any) => any) {
  await routeDemo(page, (route, url) => {
    const rel = decodeURIComponent(new URL(url).pathname).replace(
      /^\/gh\/pengowray\/bat-demo-sounds@main/,
      "",
    );
    const file = path.join(DEMO_DIR!, rel);
    if (!file.startsWith(DEMO_DIR!) || !fs.existsSync(file)) {
      return route.fulfill({ status: 404, body: "" });
    }
    if (edit && file.endsWith(".xc.json")) {
      const sidecar = edit(JSON.parse(fs.readFileSync(file, "utf8")));
      return route.fulfill({ contentType: "application/json", body: JSON.stringify(sidecar) });
    }
    const type = file.endsWith(".json") ? "application/json" : "audio/wav";
    return route.fulfill({ contentType: type, body: fs.readFileSync(file) });
  });
  await page.goto(`/#${xc}`);
  await waitForFile(page);
  await showInfo(page);
}

test.describe("species call-range note on real demo recordings", () => {
  test.use({ viewport: { width: 1440, height: 900 } });
  test.skip(!DEMO_DIR, "no local bat-demo-sounds checkout");
  test.setTimeout(120_000);

  // The spec expected case T for these two Echo Meter 3 files, from
  // Xeno-canto's 38.4 kHz rate. Their GUANO says TE 10, so the app corrects
  // them to 384 kHz when it loads them, and they reach the species' calls.
  for (const xc of ["XC892810", "XC892815", "XC891469"]) {
    test(`${xc}: corrected for x10 time expansion, so no note`, async ({ page }) => {
      await openDemo(page, xc);
      await expect(
        page.locator(".metadata-note", {
          hasText: "Corrected for \u00d710 time expansion; the file says 38.4 kHz.",
        }),
      ).toBeVisible();
      await expect(page.locator(".species-range-note")).toHaveCount(0);
      await expect(page.locator(".heterodyne-note")).toHaveCount(0);
    });
  }

  for (const xc of ["XC924726", "XC924727"]) {
    test(`${xc}: the level B heterodyne note is unchanged`, async ({ page }) => {
      await openDemo(page, xc);
      expect(await lines(page, "heterodyne-note")).toEqual([
        ["heterodyne-note-warning", "Recorded from a heterodyne bat detector's audio output."],
        ["heterodyne-note-detail", 'Remarks: "Recording with Magenta Bat5"'],
        ["heterodyne-note-detail", "The Magenta Bat5 is a heterodyne-only detector."],
        [
          "heterodyne-note-warning",
          "Frequencies are not the bat's call frequencies and can't be corrected.",
        ],
      ]);
      await expect(page.locator(".species-range-note")).toHaveCount(0);
    });
  }

  // In the demo corpus only XC924726 and XC924727 fall below their species'
  // calls, and their heterodyne note replaces the species note. Without the
  // remark that names the detector, the species note shows.
  test("XC924726 without its remark: the species note", async ({ page }) => {
    await openDemo(page, "XC924726", (sidecar) => ({ ...sidecar, rmk: "" }));
    await expect(page.locator(".heterodyne-note")).toHaveCount(0);
    expect(await lines(page, "species-range-note")).toEqual([
      [DETAIL, fact("Rhinolophus pumilus", "104\u2013121", "24")],
      [DETAIL, OTHER],
    ]);
  });

  const NO_NOTE = [
    ["XC994474", "a social call"],
    ["XC1002662", "22.05 kHz against 22 kHz"],
    ["XC954775", "Nyctalus aviator"],
    ["XC840308", "Nyctalus aviator"],
    ["XC809882", "Austronomus australis"],
    ["XC1063198", "Scotophilus kuhlii at 96 kHz"],
    ["XC1033558", "a flying fox"],
    ["XC1025973", "a flying fox"],
    ["XC959788", "Cynopterus brachyotis, which doesn't echolocate"],
    ["XC1062211", "Pipistrellus hanaki at 441 kHz"],
    ["XC928094", "Myotis dasycneme at 256 kHz"],
  ];
  for (const [xc, what] of NO_NOTE) {
    test(`${xc}: no note (${what})`, async ({ page }) => {
      await openDemo(page, xc);
      await expect(page.locator(".species-range-note")).toHaveCount(0);
      await expect(page.locator(".heterodyne-note")).toHaveCount(0);
    });
  }
});
