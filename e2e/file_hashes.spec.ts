import { test, expect, type Page, type Locator } from "@playwright/test";
import { createHash } from "node:crypto";
import { mkdirSync } from "node:fs";

/**
 * The File Identity group of the Info panel (src/components/file_sidebar/
 * metadata_panel.rs, src/file_identity.rs).
 *
 * - One hash is shown for a file: the full BLAKE3 for a small file and the
 *   spot hash for a large one. The other hashes are in a list that opens.
 * - A hash is on one line, shortened in the middle. Selecting it, and its
 *   Copy button, give the whole hash.
 * - A hash read from saved annotations is compared with the file: by itself
 *   for a small file, and with the row's Check button for a large file.
 * - A hash that differs from the saved one is marked, and its list opens.
 *
 * Small and large are divided at 24 000 000 bytes
 * (`file_identity::SMALL_FILE_THRESHOLD`).
 */

const SHOTS = "test-results/file-hashes";
const WRONG_SHA = "0123456789abcdef".repeat(4);

function noiseWav(sampleRate: number, seconds: number, seed: number): Buffer {
  const n = Math.floor(sampleRate * seconds);
  const data = Buffer.alloc(n * 2);
  for (let i = 0; i < n; i++) {
    seed = (seed * 1664525 + 1013904223) >>> 0;
    data.writeInt16LE(Math.round((seed / 0xffffffff - 0.5) * 0.05 * 32767), i * 2);
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

const sha256 = (b: Buffer) => createHash("sha256").update(b).digest("hex");

/**
 * Saves an annotation file for a recording of this name and size, holding a
 * full SHA-256 and no other hash, as an earlier session would have left it.
 * It is stored under the name-and-size key, which the app reads when it finds
 * nothing under the file's spot hash.
 */
async function saveAnnotationsWithSha(page: Page, name: string, size: number, sha: string) {
  const yaml =
    `version: 3\nid: set-${name}\napp_version: 0.5.0\ncreated_at: 2026-01-01T00:00:00.000Z\n` +
    `modified_at: 2026-01-01T00:00:00.000Z\nfile_identity:\n  filename: ${name}\n  file_size: ${size}\n` +
    `  full_sha256: ${sha}\nannotations: []\n`;
  await page.evaluate(
    async ([key, text]) => {
      const root = await navigator.storage.getDirectory();
      const dir = await root.getDirectoryHandle("oversample-annotations", { create: true });
      const handle = await dir.getFileHandle(key, { create: true });
      const w = await handle.createWritable();
      await w.write(text);
      await w.close();
    },
    [`${name}_${size}.batm`, yaml],
  );
}

async function open(page: Page, name: string, buffer: Buffer) {
  await page
    .locator('input[type="file"][accept*=".wav"]')
    .first()
    .setInputFiles({ name, mimeType: "audio/wav", buffer });
  await expect(page.locator(".file-item", { hasText: name })).toHaveCount(1, { timeout: 60_000 });
  await expect(page.locator(".chart-stage canvas").first()).toBeVisible({ timeout: 60_000 });
}

async function identity(page: Page): Promise<Locator> {
  const show = page.locator('button[title="Show info panel"]');
  if (await show.count()) await show.click();
  const group = page.locator(".file-identity");
  await expect(group).toBeVisible();
  await group.scrollIntoViewIfNeeded();
  return group;
}

const row = (group: Locator, kind: string) => group.locator(`.hash-row[data-hash="${kind}"]`);

async function shot(page: Page, group: Locator, name: string) {
  mkdirSync(SHOTS, { recursive: true });
  await group.scrollIntoViewIfNeeded();
  await page.locator(".right-sidebar").screenshot({ path: `${SHOTS}/${name}.png` });
}

test.beforeEach(async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.goto("/");
  await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
});

test("a small file shows its full BLAKE3, with the other hashes in a list", async ({ page }) => {
  const wav = noiseWav(96_000, 4, 11);
  await open(page, "small.wav", wav);
  const group = await identity(page);

  // The main hash is the only hash row outside the list.
  const main = group.locator(":scope > .hash-row[data-hash]:not([data-hash='size'])");
  await expect(main).toHaveCount(1);
  await expect(main).toHaveAttribute("data-hash", "blake3");
  await expect(main).toHaveAttribute("data-state", "computed");
  await expect(main.locator(".hash-status")).toHaveText("Computed from this file");

  // One line, shortened in the middle, with the whole hash in the text.
  const value = main.locator(".hash-value");
  const full = (await value.getAttribute("data-full"))!;
  expect(full).toMatch(/^[0-9a-f]{64}$/);
  await expect(value).toHaveText(full);
  const box = (await value.boundingBox())!;
  const panel = (await group.boundingBox())!;
  expect(box.height).toBeLessThan(20);
  expect(box.x + box.width).toBeLessThanOrEqual(panel.x + panel.width);
  expect(await value.locator(".hash-mid").evaluate((e) => getComputedStyle(e).fontSize)).toBe("0px");

  // Selecting the line and copying gives the whole hash.
  await value.evaluate((e) => {
    const range = document.createRange();
    range.selectNodeContents(e);
    const sel = window.getSelection()!;
    sel.removeAllRanges();
    sel.addRange(range);
  });
  await page.keyboard.press("ControlOrMeta+c");
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(full);

  // So does the Copy button.
  await page.evaluate(() => navigator.clipboard.writeText(""));
  await main.locator('button[title="Copy full hash"]').click();
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(full);

  // The other three hashes are in a closed list.
  const others = group.locator("details.hash-others");
  await expect(others.locator("summary")).toHaveText("Other hashes (3)");
  expect(await others.evaluate((d: HTMLDetailsElement) => d.open)).toBe(false);
  await shot(page, group, "small-closed");
  await others.locator("summary").click();
  for (const kind of ["spot", "sha256", "content"]) {
    await expect(row(others, kind)).toBeVisible();
    await expect(row(others, kind)).toHaveAttribute("data-state", "computed");
  }
  await expect(row(others, "sha256").locator(".hash-value")).toHaveAttribute("data-full", sha256(wav));
  await shot(page, group, "small-open");
});

test("a small file's saved hash is compared with the file when it is shown", async ({ page }) => {
  const same = noiseWav(96_000, 4, 21);
  const changed = noiseWav(96_000, 4, 22);
  await saveAnnotationsWithSha(page, "same.wav", same.length, sha256(same));
  await saveAnnotationsWithSha(page, "changed.wav", changed.length, WRONG_SHA);

  await open(page, "same.wav", same);
  let group = await identity(page);
  const others = group.locator("details.hash-others");
  // A match leaves the list closed.
  await expect(row(others, "sha256")).toHaveAttribute("data-state", "match");
  expect(await others.evaluate((d: HTMLDetailsElement) => d.open)).toBe(false);
  await others.locator("summary").click();
  await expect(row(others, "sha256").locator(".hash-status")).toHaveText(/Matches the hash in saved annotations/);
  await expect(row(others, "sha256").locator("button", { hasText: "Check" })).toHaveCount(0);
  await shot(page, group, "small-match");

  await open(page, "changed.wav", changed);
  group = await identity(page);
  const bad = row(group, "sha256");
  // The mismatch opens the list by itself.
  await expect(bad).toHaveAttribute("data-state", "mismatch");
  await expect(bad).toBeVisible();
  await expect(bad.locator(".hash-status-mismatch")).toHaveText(/Does not match the hash in saved annotations/);
  const pair = bad.locator(".hash-pair .hash-value");
  await expect(pair).toHaveCount(2);
  await expect(pair.nth(0)).toHaveAttribute("data-full", sha256(changed));
  await expect(pair.nth(1)).toHaveAttribute("data-full", WRONG_SHA);
  // Marked in red, on a tinted row.
  const colour = await bad.locator(".hash-status-mismatch").evaluate((e) => getComputedStyle(e).color);
  expect(colour).toBe("rgb(255, 123, 114)");
  expect(await bad.evaluate((e) => getComputedStyle(e).backgroundColor)).not.toBe("rgba(0, 0, 0, 0)");
  await shot(page, group, "small-mismatch");
});

test("a large file shows its spot hash, and a saved hash has a Check button", async ({ page }) => {
  test.setTimeout(240_000);
  // 33 s at 384 kHz, 16-bit: 25.3 MB.
  const same = noiseWav(384_000, 33, 31);
  const changed = noiseWav(384_000, 33, 32);
  expect(same.length).toBeGreaterThan(24_000_000);
  await saveAnnotationsWithSha(page, "large-same.wav", same.length, sha256(same));
  await saveAnnotationsWithSha(page, "large-changed.wav", changed.length, WRONG_SHA);

  await open(page, "large-same.wav", same);
  let group = await identity(page);
  const main = group.locator(":scope > .hash-row[data-hash]:not([data-hash='size'])");
  await expect(main).toHaveCount(1);
  await expect(main).toHaveAttribute("data-hash", "spot");
  await expect(main).toHaveAttribute("data-state", "computed");

  let others = group.locator("details.hash-others");
  await expect(others.locator("summary")).toHaveText("Other hashes (3)");
  await others.locator("summary").click();
  let sha = row(others, "sha256");
  // Saved, and not compared until asked.
  await expect(sha).toHaveAttribute("data-state", "saved");
  await expect(sha.locator(".hash-status")).toHaveText("From saved annotations. Not compared yet.");
  // Hashes with no value can be computed.
  await expect(row(others, "blake3")).toHaveAttribute("data-state", "none");
  await expect(row(others, "blake3").locator(".hash-not-computed")).toHaveText("Not computed");
  await expect(row(others, "blake3").locator("button", { hasText: "Compute" })).toBeVisible();
  // Check is to the left of Copy.
  const check = sha.locator("button", { hasText: "Check" });
  const copy = sha.locator('button[title="Copy full hash"]');
  await expect(check).toBeVisible();
  await expect(copy).toBeVisible();
  expect((await check.boundingBox())!.x).toBeLessThan((await copy.boundingBox())!.x);
  await shot(page, group, "large-not-compared");

  await check.click();
  await expect(sha).toHaveAttribute("data-state", "match", { timeout: 60_000 });
  await expect(sha.locator(".hash-status")).toHaveText(/Matches the hash in saved annotations/);
  await expect(sha.locator("button", { hasText: "Check" })).toHaveCount(0);
  // The list the user opened is still open after the check.
  expect(await others.evaluate((d: HTMLDetailsElement) => d.open)).toBe(true);
  await shot(page, group, "large-match");

  // Compute fills in a hash that had no value (the content hash comes with
  // the full BLAKE3).
  await row(others, "blake3").locator("button", { hasText: "Compute" }).click();
  await expect(row(others, "blake3")).toHaveAttribute("data-state", "computed", { timeout: 60_000 });
  await expect(row(others, "content")).toHaveAttribute("data-state", "computed");

  await open(page, "large-changed.wav", changed);
  group = await identity(page);
  others = group.locator("details.hash-others");
  await others.locator("summary").click();
  sha = row(others, "sha256");
  await expect(sha).toHaveAttribute("data-state", "saved");
  await sha.locator("button", { hasText: "Check" }).click();
  await expect(sha).toHaveAttribute("data-state", "mismatch", { timeout: 60_000 });
  await expect(sha.locator(".hash-status-mismatch")).toHaveText(/Does not match the hash in saved annotations/);
  await expect(sha.locator(".hash-pair .hash-value").nth(0)).toHaveAttribute("data-full", sha256(changed));
  await shot(page, group, "large-mismatch");
});
