import { test, expect, type Page } from "@playwright/test";

/**
 * A Pettersson D500X recording: the recorder writes a metadata block at the
 * front of the audio data, with the position its owner typed in as LAT and
 * LON rows (degrees, minutes, seconds).
 *
 * - Info shows the block's rows, then a Position row with the map
 *   (recorder_section in src/components/file_sidebar/metadata_panel.rs).
 * - The notes about the file's structure are in Data analysis, under File
 *   structure, and not on Info (oversample-core/src/audio/wav.rs `notes`).
 * - A note that changes which audio is read is on Info as well.
 */

const BLOCK_LEN = 980;
const RATE = 500_000;

/** 16-bit mono PCM: quiet noise, so the analysis has something to measure. */
function pcm(samples: number): Buffer {
  const data = Buffer.alloc(samples * 2);
  let seed = 4321;
  for (let i = 0; i < samples; i++) {
    seed = (seed * 1664525 + 1013904223) >>> 0;
    data.writeInt16LE(Math.round((seed / 0xffffffff - 0.5) * 0.05 * 32767), i * 2);
  }
  return data;
}

function header(rate: number, riffSize: number, dataSize: number): Buffer {
  const h = Buffer.alloc(44);
  h.write("RIFF", 0);
  h.writeUInt32LE(riffSize, 4);
  h.write("WAVEfmt ", 8);
  h.writeUInt32LE(16, 16);
  h.writeUInt16LE(1, 20);
  h.writeUInt16LE(1, 22);
  h.writeUInt32LE(rate, 24);
  h.writeUInt32LE(rate * 2, 28);
  h.writeUInt16LE(2, 32);
  h.writeUInt16LE(16, 34);
  h.write("data", 36);
  h.writeUInt32LE(dataSize, 40);
  return h;
}

/**
 * A D500X file as the recorder writes it: the data size leaves the block out,
 * so the last `BLOCK_LEN` bytes of audio are past the size the header gives.
 * `lat` and `lon` are the LAT and LON rows; leave them out for a recorder
 * with no position set.
 */
function d500xWav(position?: { lat: string; lon: string }): Buffer {
  const block = Buffer.alloc(BLOCK_LEN);
  block.writeUInt32LE(BLOCK_LEN, 0);
  block.write("M000001.WAV", 0xa4, "latin1");
  block.write("D500X V2.2.8 160421, 19:37:34", 0xc4, "latin1");
  block.write("f=500 PRE=OFF LEN=1", 0xf4, "latin1");
  block.write("USER0", 0x12c, "latin1");
  const rows = [
    "File Name:         M000001.WAV",
    "File Time:         200102 21:30:00",
    "S/N:               00001",
    ...(position
      ? [`LAT:               ${position.lat}`, `LON:               ${position.lon}`]
      : []),
    "TIMEZONE:          +10",
  ];
  block.write(rows.join("\r\n") + "\r\n", 0x1d4, "latin1");
  const audio = pcm(RATE / 2);
  return Buffer.concat([header(RATE, 36 + BLOCK_LEN + audio.length, audio.length), block, audio]);
}

/** A plain WAV whose header says twice the audio that is in the file. */
function cutShortWav(): Buffer {
  const audio = pcm(96_000);
  return Buffer.concat([header(96_000, 36 + audio.length * 2, audio.length * 2), audio]);
}

async function open(page: Page, name: string, buffer: Buffer) {
  await page.goto("/");
  await page
    .locator('input[type="file"][accept*=".wav"]')
    .first()
    .setInputFiles({ name, mimeType: "audio/wav", buffer });
  await expect(page.locator(".file-item", { hasText: name })).toBeVisible({ timeout: 30_000 });
  const btn = page.locator('button[title="Show info panel"]');
  if (await btn.count()) await btn.first().click();
  await expect(page.locator(".right-sidebar")).not.toHaveClass(/collapsed/);
}

async function showDataAnalysis(page: Page) {
  await page.locator(".right-sidebar .sidebar-tab-dropdown").click();
  await page.getByRole("button", { name: "Data analysis" }).click();
}

const sidebar = (page: Page) => page.locator(".right-sidebar");
/** A row of the Info panel, by its label. */
const row = (page: Page, label: string) =>
  sidebar(page).locator(".metadata-row-spacious", {
    has: page.locator(".metadata-key", { hasText: new RegExp(`^${label}$`, "i") }),
  });
const NOTES = ".metadata-note, .analysis-warning";

test.describe("Pettersson D500X recording", () => {
  test("Info shows the typed-in position on the map, after the LAT and LON rows", async ({
    page,
  }) => {
    // 12°30'36" S, 100°15'00" E: -12.51, 100.25.
    await open(page, "d500x.wav", d500xWav({ lat: "12 30 36 S", lon: "100 15 00 E" }));

    await expect(sidebar(page)).toContainText("Pettersson D500X metadata");
    await expect(row(page, "LAT")).toContainText("12 30 36 S");
    await expect(row(page, "LON")).toContainText("100 15 00 E");

    const position = row(page, "Position");
    await expect(position).toContainText("-12.51000, 100.25000");
    await expect(position).toContainText("Typed into the recorder");
    // The pin, as a share of the map: longitude across, latitude down.
    const pin = position.locator(".metadata-map-pin");
    await expect(pin).toHaveAttribute("style", /left: 77\.847%; top: 56\.950%/);
    await expect(position.getByRole("link", { name: "OpenStreetMap" })).toHaveAttribute(
      "href",
      "https://www.openstreetmap.org/?mlat=-12.51&mlon=100.25#map=10/-12.51/100.25",
    );

    // The Position row is the row after LON.
    const labels = await sidebar(page).locator(".metadata-key").allInnerTexts();
    const lon = labels.findIndex((l) => /^lon$/i.test(l));
    expect(labels[lon + 1]).toMatch(/^position$/i);
  });

  test("a recorder with no position set gets no Position row", async ({ page }) => {
    await open(page, "d500x-unset.wav", d500xWav({ lat: "00 00 00 N", lon: "000 00 00 E" }));
    await expect(row(page, "LAT")).toContainText("00 00 00 N");
    await expect(row(page, "Position")).toHaveCount(0);
    await expect(sidebar(page).locator(".metadata-map-pin")).toHaveCount(0);

    await open(page, "d500x-none.wav", d500xWav());
    await expect(sidebar(page)).toContainText("Pettersson D500X metadata");
    await expect(row(page, "Position")).toHaveCount(0);
  });

  test("the block's note is in Data analysis under File structure, and not on Info", async ({
    page,
  }) => {
    await open(page, "d500x.wav", d500xWav({ lat: "12 30 36 S", lon: "100 15 00 E" }));
    await expect(sidebar(page)).toContainText("Pettersson D500X metadata");
    await expect(sidebar(page).locator(NOTES)).toHaveCount(0);

    await showDataAnalysis(page);
    const group = sidebar(page).locator(".setting-group", {
      has: page.locator(".setting-group-title", { hasText: "File structure" }),
    });
    const note = group.locator(".metadata-note", { hasText: "Pettersson D500X metadata block" });
    await expect(note).toContainText(
      "980-byte Pettersson D500X metadata block before the audio. Oversample skips the block.",
    );
    // The rest of the note opens on a click.
    const detail = note.locator(".note-detail");
    await expect(detail).toBeHidden();
    await note.locator("summary").click();
    await expect(detail).toContainText("The Info tab lists the metadata");

    // The note and the lines round it keep clear of the panel's left edge.
    const panelLeft = (await sidebar(page).boundingBox())!.x;
    for (const text of [note.locator("summary"), sidebar(page).getByText("Estimated wSNR")]) {
      const box = await text.first().boundingBox();
      expect(box!.x - panelLeft).toBeGreaterThanOrEqual(8);
    }
  });
});

test("a file with less audio than its header says: the note is on Info and in Data analysis", async ({
  page,
}) => {
  await open(page, "cut-short.wav", cutShortWav());
  const text = "Less audio than the header says: 192,000 bytes, header says 384,000.";
  const onInfo = sidebar(page).locator(".analysis-warning", { hasText: "Less audio" });
  await expect(onInfo).toContainText(text);

  await showDataAnalysis(page);
  const group = sidebar(page).locator(".setting-group", {
    has: page.locator(".setting-group-title", { hasText: "File structure" }),
  });
  await expect(group.locator(".analysis-warning")).toContainText(text);
});
