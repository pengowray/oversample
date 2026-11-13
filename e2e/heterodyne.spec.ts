import { test, expect, type Page } from "@playwright/test";

/**
 * The "Heterodyne recording" badge and the heterodyne note in the Info panel
 * (src/heterodyne.rs).
 *
 * Each test opens one demo recording with its Xeno-canto sidecar and audio
 * routed: the sidecar carries the remark or device text for the case, and the
 * audio is a generated WAV at the sample rate the case needs.
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
  /** Xeno-canto sidecar text fields (rmk, dvc, mic, method). */
  fields: Record<string, string>;
  rate: number;
  guano?: string;
}

/** Opens demo XC815138 with the case's sidecar text and audio. */
async function openCase(page: Page, c: Case) {
  const sidecar = {
    id: "815138",
    en: "Fringed Long-footed Myotis",
    gen: "Myotis",
    sp: "fimbriatus",
    rec: "Test",
    cnt: "Brazil",
    method: "field recording",
    rmk: "",
    url: "https://xeno-canto.org/815138",
    ...c.fields,
  };
  const audio = wav(c.rate, 1, c.guano);
  const handler = async (route: any) => {
    const url: string = route.request().url();
    if (url.endsWith("/index.json")) {
      return route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          version: 1,
          sounds: [
            {
              en: "Fringed Long-footed Myotis",
              filename: "XC815138 demo.wav",
              metadata: "XC815138 demo.xc.json",
              source: "xeno-canto",
              species: "Myotis fimbriatus",
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

async function showInfo(page: Page) {
  const btn = page.locator('button[title="Show info panel"]');
  if (await btn.count()) await btn.first().click();
  await expect(page.locator(".right-sidebar")).not.toHaveClass(/collapsed/);
}

/** The text and class of each line of the Info panel's heterodyne note. */
async function noteLines(page: Page) {
  return page
    .locator(".heterodyne-note > div")
    .evaluateAll((els) => els.map((e) => [e.className, e.textContent]));
}

const HEADLINE = "Recorded from a heterodyne bat detector's audio output.";
const CONSEQUENCE = "Frequencies are not the bat's call frequencies and can't be corrected.";
const WARN = "heterodyne-note-warning";
const DETAIL = "heterodyne-note-detail";

test.describe("heterodyne badge and note", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("level A: the remark says heterodyne", async ({ page }) => {
    await openCase(page, {
      fields: { rmk: "handheld hetrodyne detector set at 31,9 kHz" },
      rate: 44_100,
    });
    const lines = [HEADLINE, 'Remarks: "handheld hetrodyne detector set at 31,9 kHz"', CONSEQUENCE];

    const badge = page.locator(".toolbar-badge-row .file-badge-heterodyne");
    await expect(badge).toHaveText("Heterodyne recording");
    await expect(badge).toHaveAttribute("title", lines.join("\n"));
    // After the duration, so it doesn't split the rate, bit depth and duration.
    const prev = await badge.evaluate((e) => e.previousElementSibling?.className ?? "");
    expect(prev).toContain("badge-duration");
    await expect(page.locator(".file-item .file-badge-heterodyne")).toHaveText(
      "Heterodyne recording",
    );

    // Clicking the badge opens the Info panel on the note.
    await expect(page.locator(".right-sidebar")).toHaveClass(/collapsed/);
    await badge.click();
    await expect(page.locator(".right-sidebar")).not.toHaveClass(/collapsed/);
    await expect(page.locator(".right-sidebar .sidebar-tab-dropdown")).toContainText("Info");
    await expect(page.locator(".heterodyne-note")).toBeVisible();
    expect(await noteLines(page)).toEqual([
      [WARN, lines[0]],
      [DETAIL, lines[1]],
      [WARN, lines[2]],
    ]);
  });

  test("level B: the remark names a heterodyne-only detector", async ({ page }) => {
    await openCase(page, { fields: { rmk: "Recording with Magenta Bat5" }, rate: 48_000 });
    const lines = [
      HEADLINE,
      'Remarks: "Recording with Magenta Bat5"',
      "The Magenta Bat5 is a heterodyne-only detector.",
      CONSEQUENCE,
    ];
    const badge = page.locator(".toolbar-badge-row .file-badge-heterodyne");
    await expect(badge).toHaveText("Heterodyne recording");
    await expect(badge).toHaveAttribute("title", lines.join("\n"));
    await showInfo(page);
    expect(await noteLines(page)).toEqual([
      [WARN, lines[0]],
      [DETAIL, lines[1]],
      [DETAIL, lines[2]],
      [WARN, lines[3]],
    ]);
  });

  test("decomposed accents are normalised before matching", async ({ page }) => {
    // "hétérodyne" with each accent as a separate combining mark (NFD). The
    // word lists hold precomposed letters, so this only matches after NFC.
    const nfd = "détecteur hétérodyne";
    await openCase(page, { fields: { rmk: nfd }, rate: 44_100 });
    const badge = page.locator(".toolbar-badge-row .file-badge-heterodyne");
    await expect(badge).toHaveText("Heterodyne recording");
    await expect(badge).toHaveAttribute(
      "title",
      [HEADLINE, `Remarks: "${nfd.normalize("NFC")}"`, CONSEQUENCE].join("\n"),
    );
  });

  test("the badge on another file's row makes that file current", async ({ page }) => {
    await openCase(page, { fields: { rmk: "Recording with Magenta Bat5" }, rate: 48_000 });
    await page
      .locator('input[type="file"]')
      .first()
      .setInputFiles({ name: "plain-local.wav", mimeType: "audio/wav", buffer: wav(44_100, 1) });
    await expect(page.locator(".file-item.active .file-item-name")).toHaveText("plain-local.wav");
    await expect(page.locator(".toolbar-badge-row .file-badge-heterodyne")).toHaveCount(0);

    await page.locator(".file-item .file-badge-heterodyne").click();
    await expect(page.locator(".file-item.active .file-item-name")).toHaveText("XC815138 demo.wav");
    await expect(page.locator(".right-sidebar")).not.toHaveClass(/collapsed/);
    await expect(page.locator(".heterodyne-note")).toBeVisible();
  });

  test("the badge scrolls the Info panel back to the Sample rate row and note", async ({
    page,
  }) => {
    await openCase(page, { fields: { rmk: "Recording with Magenta Bat5" }, rate: 48_000 });
    await showInfo(page);
    const panel = page.locator(".right-sidebar .sidebar-panel");
    await panel.evaluate((el) => (el.scrollTop = el.scrollHeight));
    const note = page.locator(".heterodyne-note");
    const top = (sel: string) =>
      page.evaluate((s) => document.querySelector(s)!.getBoundingClientRect().top, sel);
    const panelBox = (await panel.boundingBox())!;
    expect(await top(".heterodyne-note")).toBeLessThan(panelBox.y);

    await page.locator(".toolbar-badge-row .file-badge-heterodyne").click();
    // The Sample rate row (just above the note) and the note are both in view.
    await expect
      .poll(() => note.evaluate((n) => n.previousElementSibling!.getBoundingClientRect().top), {
        timeout: 2_000,
      })
      .toBeGreaterThanOrEqual(panelBox.y - 1);
    const noteBox = (await note.boundingBox())!;
    expect(noteBox.y + noteBox.height).toBeLessThanOrEqual(panelBox.y + panelBox.height + 1);
    expect(await note.evaluate((n) => n.previousElementSibling!.textContent)).toContain(
      "Sample rate",
    );
  });

  test("a Ctrl-click on the badge only selects the row", async ({ page }) => {
    await openCase(page, { fields: { rmk: "Recording with Magenta Bat5" }, rate: 48_000 });
    await page
      .locator('input[type="file"]')
      .first()
      .setInputFiles({ name: "plain-local.wav", mimeType: "audio/wav", buffer: wav(44_100, 1) });
    await expect(page.locator(".file-item.active .file-item-name")).toHaveText("plain-local.wav");
    await expect(page.locator(".right-sidebar")).toHaveClass(/collapsed/);

    await page.locator(".file-item .file-badge-heterodyne").click({ modifiers: ["Control"] });
    await expect(page.locator(".file-item.active .file-item-name")).toHaveText("plain-local.wav");
    await expect(page.locator(".right-sidebar")).toHaveClass(/collapsed/);
  });

  test("Data analysis names Xeno-canto as the source of the device claim", async ({ page }) => {
    await openCase(page, {
      fields: { dvc: "Zoom H1n", mic: "Magenta Bat5 speaker" },
      rate: 48_000,
    });
    await showInfo(page);
    await page.locator(".right-sidebar .sidebar-tab-dropdown").click();
    await page.getByRole("button", { name: "Data analysis" }).click();
    // The generated tone looks like a 12-bit recorder, so the claim is shown
    // as a mismatch ("⚠ Xeno-canto says … — but analysis suggests: …").
    await expect(page.locator(".right-sidebar")).toContainText(
      /Xeno-canto says:? Zoom H1n \(Magenta Bat5 speaker\)/,
      { timeout: 30_000 },
    );
  });

  test("level C: a dual-mode detector gets a muted note and no badge", async ({ page }) => {
    await openCase(page, { fields: { dvc: "Pettersson D980" }, rate: 44_100 });
    await showInfo(page);
    expect(await noteLines(page)).toEqual([
      [
        DETAIL,
        "Possibly recorded from a bat detector's output " +
          "(heterodyne, time expansion or frequency division).",
      ],
      [DETAIL, 'Device: "Pettersson D980"'],
      [DETAIL, "The Pettersson D980 has a heterodyne mode and other modes."],
      [DETAIL, "If so, frequencies are not the bat's call frequencies."],
    ]);
    await expect(page.locator(".file-badge-heterodyne")).toHaveCount(0);
  });

  const NOTHING: [string, Case][] = [
    ["level B at 192 kHz", { fields: { rmk: "Recording with Magenta Bat5" }, rate: 192_000 }],
    [
      "a time-expanded file",
      {
        fields: { rmk: "handheld hetrodyne detector set at 31,9 kHz" },
        rate: 44_100,
        guano: "GUANO|Version: 1.0\nTE: 10\n",
      },
    ],
    ["plain remarks", { fields: { rmk: "Recorded at dusk near a pond" }, rate: 44_100 }],
  ];
  for (const [name, c] of NOTHING) {
    test(`no badge or note for ${name}`, async ({ page }) => {
      await openCase(page, c);
      await showInfo(page);
      await expect(page.locator(".metadata-row-inline .setting-label", { hasText: /^Sample rate$/ })).toBeVisible();
      await expect(page.locator(".file-badge-heterodyne")).toHaveCount(0);
      await expect(page.locator(".heterodyne-note")).toHaveCount(0);
      if (c.guano) {
        // The app applied the file's time expansion, which is why there is no hint.
        await expect(page.locator(".metadata-note", { hasText: /time expansion/ })).toBeVisible();
      }
    });
  }
});

test.describe("heterodyne badge on a phone", () => {
  test.use({
    viewport: { width: 375, height: 812 },
    isMobile: true,
    hasTouch: true,
    userAgent:
      "Mozilla/5.0 (Linux; Android 14; Pixel 7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36",
  });

  test("tapping the badge in the file list swaps the Files sidebar for Info", async ({ page }) => {
    await openCase(page, { fields: { rmk: "Recording with Magenta Bat5" }, rate: 48_000 });
    const left = page.locator(".sidebar:not(.right-sidebar)");
    await page.locator("button.toolbar-sidebar-tab-left").click();
    await expect(left).not.toHaveClass(/collapsed/);

    await page.locator(".file-item .file-badge-heterodyne").tap();
    await expect(left).toHaveClass(/collapsed/);
    await expect(page.locator(".right-sidebar")).not.toHaveClass(/collapsed/);
    const note = page.locator(".heterodyne-note");
    await expect(note).toBeVisible();
    // The note stays inside the panel.
    const panel = (await page.locator(".right-sidebar").boundingBox())!;
    const box = (await note.boundingBox())!;
    expect(box.x + box.width).toBeLessThanOrEqual(panel.x + panel.width + 0.5);
  });
});
