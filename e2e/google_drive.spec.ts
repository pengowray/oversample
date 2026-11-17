import { test, expect, type Page } from "@playwright/test";
import { EMAIL, MockDrive, mockServer } from "./drive_mock";

/**
 * Google Drive copies of annotation files (src/cloud/, the "Google Drive"
 * group in Settings).
 *
 * The Oversample API (`/google/*`) and the Drive REST API are mocked. The
 * real sign-in pop-up cannot be mocked, so the signed-in tests start with
 * the status route already saying "signed in". The API base is set to the
 * page's own origin so the cookie-carrying `/google/*` requests are
 * same-origin.
 */

function noiseWav(sampleRate: number, seconds: number, seed = 4321): Buffer {
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

type Range = { timeStart: number; timeEnd: number; freqLow: number | null; freqHigh: number | null };
type Snap = {
  currentFile: string | null;
  scrollOffset: number;
  zoom: number;
  sampleRate: number;
  duration: number;
  regions: Range[];
  selection: Range | null;
};

const snap = (page: Page): Promise<Snap> =>
  page.evaluate(() => (window as unknown as { __oversample_test: () => Snap }).__oversample_test());

async function open(page: Page, name: string, wav: Buffer) {
  // The file input is in the Files tab.
  await page.getByRole("button", { name: "Files", exact: true }).click();
  const files = page.locator(".file-item", { hasText: name });
  await page
    .locator('input[type="file"][accept*=".wav"]')
    .first()
    .setInputFiles({ name, mimeType: "audio/wav", buffer: wav });
  await expect(files).toHaveCount(1, { timeout: 30_000 });
  await expect.poll(async () => (await snap(page)).currentFile).toBe(name);
  await expect(page.locator(".chart-stage canvas").first()).toBeVisible({ timeout: 30_000 });
}

async function annotate(page: Page, t0: number, t1: number) {
  const before = (await snap(page)).regions.length;
  await page.locator(".tool-combo-btn").click();
  await page.locator(".layer-panel button", { hasText: "Selection" }).click();
  const canvas = page.locator(".main-view .chart-stage > canvas");
  const box = (await canvas.boundingBox())!;
  const s = await snap(page);
  const visible = (box.width / s.zoom) * (512 / s.sampleRate);
  const shown = Math.min(visible, s.duration - s.scrollOffset);
  const x = (f: number) => box.x + (box.width * f * shown) / visible;
  const y = (f: number) => box.y + box.height * f;
  await page.mouse.move(x(t0), y(0.3));
  await page.mouse.down();
  await page.mouse.move(x(t1), y(0.6), { steps: 5 });
  await page.mouse.up();
  await page.locator('.canvas-overflow-btn[title="Selection options"]').click();
  await page.locator(".canvas-overflow-item", { hasText: "Annotate Region" }).click();
  await expect(page.locator(".annotation-inline-label-editor")).toBeVisible();
  await page.keyboard.press("Enter");
  await expect.poll(async () => (await snap(page)).regions.length).toBe(before + 1);
}

async function openSettings(page: Page) {
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await expect(page.locator(".setting-group-title", { hasText: "Beta" })).toBeVisible();
}

const savedFiles = (page: Page): Promise<Record<string, string>> =>
  page.evaluate(async () => {
    const root = await navigator.storage.getDirectory();
    const out: Record<string, string> = {};
    let dir: FileSystemDirectoryHandle;
    try {
      dir = await root.getDirectoryHandle("oversample-annotations");
    } catch {
      return out;
    }
    const entries = (dir as unknown as { entries: () => AsyncIterable<[string, FileSystemFileHandle]> }).entries();
    for await (const [name, handle] of entries) {
      try {
        out[name] = await (await handle.getFile()).text();
      } catch {}
    }
    return out;
  });

// ── Tests ───────────────────────────────────────────────────────────────────

test.describe("Google Drive copies", () => {
  test("the Google Drive group is hidden when the server does not offer sign-in", async ({ page }) => {
    const drive = new MockDrive();
    drive.configured = false;
    let statusAsked = false;
    page.on("request", (r) => {
      if (r.url().endsWith("/google/status")) statusAsked = true;
    });
    await mockServer(page, drive);
    await page.goto("/");
    await openSettings(page);
    await expect.poll(() => statusAsked).toBe(true);
    await page.waitForTimeout(500);
    await expect(page.locator(".setting-group-title", { hasText: "Google Drive" })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Sign in with Google" })).toHaveCount(0);
  });

  test("signed out: the group offers sign-in", async ({ page }) => {
    const drive = new MockDrive();
    drive.signedIn = false;
    await mockServer(page, drive);
    await page.goto("/");
    await openSettings(page);
    await expect(page.getByRole("button", { name: "Sign in with Google" })).toBeVisible();
    await expect(page.locator(".drive-settings")).toContainText('"Oversample" folder in your Google Drive');
  });

  test("signed in: the account is shown, and an annotation is uploaded once after the delay", async ({ page }) => {
    const drive = new MockDrive();
    await mockServer(page, drive);
    await page.goto("/");
    await openSettings(page);
    await expect(page.locator(".drive-account")).toHaveText(EMAIL);
    await expect(page.locator(".drive-status")).toHaveText("Saved to Google Drive");

    const name = "drive-upload.wav";
    await open(page, name, noiseWav(96_000, 8, 1111));
    await annotate(page, 0.2, 0.4);
    const annotatedAt = Date.now();

    await expect.poll(() => drive.uploads.length, { timeout: 20_000 }).toBe(1);
    const up = drive.uploads[0];
    // Uploads wait 5 s after the last save.
    expect(up.at - annotatedAt).toBeGreaterThanOrEqual(4_000);
    expect(up.method).toBe("POST");
    expect(up.name).toBe(`${name}.batm`);
    expect(up.appProperties.osKind).toBe("batm");
    expect(up.appProperties.osKey).toMatch(/^b3_[0-9a-f]+\.batm$/);
    expect(up.content).toContain("time_start:");
    expect(up.content).not.toContain("file_path");
    // The new file went into Oversample/annotations.
    const folder = [...drive.files.values()].find((f) => f.appProperties.osFolder === "annotations");
    expect(folder).toBeTruthy();
    const uploaded = [...drive.files.values()].find((f) => f.appProperties.osKey === up.appProperties.osKey)!;
    expect(uploaded.parents).toEqual([folder!.id]);

    // No second upload of the same content.
    await page.waitForTimeout(7_000);
    expect(drive.uploads.length).toBe(1);
    await openSettings(page);
    await expect(page.locator(".drive-status")).toHaveText("Saved to Google Drive");
  });

  test("a Drive copy of a file with no local copy is used, and no empty set is uploaded", async ({ page }) => {
    const drive = new MockDrive();
    const name = "drive-download.wav";
    const wav = noiseWav(96_000, 8, 2222);
    const region = (id: string, start: number) =>
      `- id: ${id}\n  type: Region\n  time_start: ${start}\n  time_end: ${start + 0.5}\n  freq_low: 20000.0\n  freq_high: 40000.0\n` +
      `  created_at: 2026-11-01T10:00:00.000Z\n  modified_at: 2026-11-01T10:00:00.000Z\n`;
    const yaml =
      `version: 3\nid: drive-set\napp_version: 0.5.200\ncreated_at: 2026-11-01T10:00:00.000Z\n` +
      `modified_at: 2026-11-01T10:00:00.000Z\nfile_identity:\n  filename: ${name}\n  file_size: ${wav.length}\n` +
      `annotations:\n${region("from-drive-1", 1.0)}${region("from-drive-2", 3.0)}`;
    // Answer the lookup by key with a file saved by "another device".
    drive.onKeyQuery = (kind, key) => {
      if (kind !== "batm" || [...drive.files.values()].some((f) => f.appProperties.osKey === key)) return;
      drive.add({ name: `${name}.batm`, appProperties: { osKind: "batm", osKey: key }, content: yaml });
    };
    await mockServer(page, drive);
    await page.goto("/");
    await openSettings(page);
    await expect(page.locator(".drive-account")).toHaveText(EMAIL);

    await open(page, name, wav);
    await expect
      .poll(async () => (await snap(page)).regions.map((r) => r.timeStart).sort((a, b) => a - b), { timeout: 20_000 })
      .toEqual([1.0, 3.0]);
    // Saved on this device too.
    await expect
      .poll(async () => Object.values(await savedFiles(page)).some((t) => t.includes("from-drive-2")))
      .toBe(true);
    // Nothing goes back up.
    await page.waitForTimeout(7_000);
    expect(drive.uploads).toEqual([]);
  });

  test("sign out clears the Drive state and keeps the local copy", async ({ page }) => {
    const drive = new MockDrive();
    await mockServer(page, drive);
    await page.goto("/");
    await openSettings(page);
    await expect(page.locator(".drive-account")).toHaveText(EMAIL);

    await page.getByRole("button", { name: "Sign out" }).click();
    await expect(page.getByRole("button", { name: "Sign in with Google" })).toBeVisible();
    await expect(page.locator(".drive-account")).toHaveCount(0);
    await expect.poll(() => drive.logouts).toBe(1);
    // Drive ids and revisions are forgotten; content fingerprints stay.
    const seen = await page.evaluate(() => localStorage.getItem("oversample_drive_seen"));
    for (const f of Object.values((seen ? JSON.parse(seen) : { files: {} }).files) as { id: string; rev: string | null }[]) {
      expect(f.id).toBe("");
      expect(f.rev).toBeNull();
    }

    // A save while signed out stays local and is not uploaded.
    await open(page, "signed-out.wav", noiseWav(96_000, 8, 3333));
    await annotate(page, 0.2, 0.4);
    await expect
      .poll(async () => Object.keys(await savedFiles(page)).length)
      .toBeGreaterThan(0);
    await page.waitForTimeout(6_000);
    expect(drive.uploads).toEqual([]);
  });

  test("a change made while signed out is uploaded at the next sign-in, not merged away", async ({ page }) => {
    const drive = new MockDrive();
    await mockServer(page, drive);
    await page.goto("/");
    await openSettings(page);
    await expect(page.locator(".drive-account")).toHaveText(EMAIL);
    await open(page, "offline-edit.wav", noiseWav(96_000, 8, 4444));
    await annotate(page, 0.1, 0.2);
    await annotate(page, 0.5, 0.6);
    const regionsIn = (text: string) => (text.match(/time_start:/g) ?? []).length;
    await expect.poll(() => drive.uploads.length, { timeout: 20_000 }).toBe(1);
    expect(regionsIn(drive.uploads[0].content)).toBe(2);

    await openSettings(page);
    await page.getByRole("button", { name: "Sign out" }).click();
    await expect(page.getByRole("button", { name: "Sign in with Google" })).toBeVisible();
    // Remove the second annotation while signed out (undo).
    // (A label edit is an undo step of its own, so press until one is left.)
    for (let i = 0; i < 4 && (await snap(page)).regions.length > 1; i++) {
      await page.keyboard.press("Control+z");
      await page.waitForTimeout(200);
    }
    expect((await snap(page)).regions.length).toBe(1);
    await expect
      .poll(async () => Object.values(await savedFiles(page)).some((t) => regionsIn(t) === 1))
      .toBe(true);

    drive.signedIn = true;
    await page.reload();
    await expect.poll(() => drive.uploads.length, { timeout: 30_000 }).toBe(2);
    expect(drive.uploads[1].method).toBe("PATCH");
    expect(regionsIn(drive.uploads[1].content)).toBe(1);
  });

  test("first sign-in with two local sets makes one set of folders", async ({ page }) => {
    const drive = new MockDrive();
    drive.signedIn = false;
    await mockServer(page, drive);
    await page.goto("/");
    await openSettings(page);
    await expect(page.getByRole("button", { name: "Sign in with Google" })).toBeVisible();
    const set = (file: string) =>
      `version: 3\nid: set-${file}\napp_version: 0.5.200\nmodified_at: 2026-11-01T10:00:00.000Z\n` +
      `file_identity:\n  filename: ${file}.wav\n  file_size: 100\nannotations:\n` +
      `- id: r-${file}\n  type: Region\n  time_start: 1.0\n  time_end: 2.0\n` +
      `  created_at: 2026-11-01T10:00:00.000Z\n  modified_at: 2026-11-01T10:00:00.000Z\n`;
    await page.evaluate(
      async (files) => {
        const root = await navigator.storage.getDirectory();
        const dir = await root.getDirectoryHandle("oversample-annotations", { create: true });
        for (const [name, text] of files) {
          const w = await (await dir.getFileHandle(name, { create: true })).createWritable();
          await w.write(text);
          await w.close();
        }
      },
      [
        ["first.wav_100.batm", set("first")],
        ["second.wav_100.batm", set("second")],
      ],
    );
    drive.signedIn = true;
    await page.reload();
    await expect.poll(() => drive.uploads.length, { timeout: 30_000 }).toBe(2);
    expect(drive.uploads.map((u) => u.name).sort()).toEqual(["first.wav.batm", "second.wav.batm"]);
    const folders = (marker: string) =>
      [...drive.files.values()].filter((f) => f.appProperties.osFolder === marker).length;
    expect(folders("root")).toBe(1);
    expect(folders("annotations")).toBe(1);
  });

  test("Retry uploads a change whose upload failed", async ({ page }) => {
    const drive = new MockDrive();
    drive.failUploads = true;
    await mockServer(page, drive);
    await page.goto("/");
    await openSettings(page);
    await expect(page.locator(".drive-account")).toHaveText(EMAIL);
    await open(page, "retry.wav", noiseWav(96_000, 8, 5555));
    await annotate(page, 0.2, 0.4);
    await openSettings(page);
    await expect(page.locator(".drive-settings")).toContainText("Can't reach Google Drive", { timeout: 20_000 });
    expect(drive.uploads).toEqual([]);

    drive.failUploads = false;
    await page.getByRole("button", { name: "Retry" }).click();
    await expect.poll(() => drive.uploads.length, { timeout: 20_000 }).toBe(1);
    await expect(page.locator(".drive-status")).toHaveText("Saved to Google Drive");
  });

  // ── Where the open file's annotations are saved ──────────────────────────

  async function showAnnotations(page: Page) {
    const show = page.locator('button[title="Show info panel"]');
    if (await show.isVisible()) await show.click();
    await page.locator(".sidebar-tab-dropdown").click();
    await page.locator(".sidebar-tab-option", { hasText: "Annotations" }).dispatchEvent("mousedown");
  }

  test("signed out, the Annotations panel says the annotations are saved in this browser", async ({ page }) => {
    const drive = new MockDrive();
    drive.signedIn = false;
    await mockServer(page, drive);
    await page.goto("/");
    await open(page, "where-out.wav", noiseWav(96_000, 8, 6666));
    await annotate(page, 0.2, 0.4);
    await showAnnotations(page);
    const line = page.locator(".right-sidebar .annotation-storage");
    await expect(line).toHaveText("Saved in this browser");
    await expect(line.locator("button")).toHaveCount(0);
  });

  test("signed in, the line shows this file's upload: waiting, then saved in Google Drive", async ({ page }) => {
    const drive = new MockDrive();
    await mockServer(page, drive);
    await page.goto("/");
    await open(page, "where-in.wav", noiseWav(96_000, 8, 7777));
    await annotate(page, 0.2, 0.4);
    await showAnnotations(page);
    const line = page.locator(".right-sidebar .annotation-storage");
    await expect(line).toContainText("Saved in this browser");
    const driveButton = line.getByRole("button");
    await expect(driveButton).toHaveText("Waiting to upload to Google Drive");
    await expect.poll(() => drive.uploads.length, { timeout: 20_000 }).toBe(1);
    await expect(driveButton).toHaveText("Saved in Google Drive");

    // The same line is in the annotation's menu.
    const menuButton = page.locator('.canvas-overflow-btn[title="Annotation options"]').first();
    await menuButton.click();
    await expect(page.locator(".canvas-overflow-menu .annotation-storage")).toContainText("Saved in Google Drive");
    // The menu closes on a click outside it (its backdrop covers the page).
    await page.locator(".canvas-overflow-backdrop").click({ position: { x: 5, y: 5 } });
    await expect(page.locator(".canvas-overflow-menu")).toHaveCount(0);

    // The button opens the Google Drive group in Settings.
    await driveButton.click();
    await expect(page.locator(".drive-settings")).toBeInViewport();
  });

  // ── Undoing a file's only annotation ─────────────────────────────────────

  async function undoUntilNone(page: Page) {
    // (A label edit is an undo step of its own.)
    for (let i = 0; i < 4 && (await snap(page)).regions.length > 0; i++) {
      await page.keyboard.press("Control+z");
      await page.waitForTimeout(200);
    }
    expect((await snap(page)).regions.length).toBe(0);
  }

  test("undoing the only annotation is saved: after a reload the file has no annotations", async ({ page }) => {
    const drive = new MockDrive();
    drive.signedIn = false;
    await mockServer(page, drive);
    await page.goto("/");
    const wav = noiseWav(96_000, 8, 8888);
    await open(page, "undo-only.wav", wav);
    await annotate(page, 0.2, 0.4);
    await expect
      .poll(async () => Object.values(await savedFiles(page)).some((t) => t.includes("time_start:")))
      .toBe(true);
    await undoUntilNone(page);
    await expect
      .poll(async () => Object.values(await savedFiles(page)).every((t) => !t.includes("time_start:")))
      .toBe(true);

    await page.reload();
    await open(page, "undo-only.wav", wav);
    await page.waitForTimeout(1500);
    expect((await snap(page)).regions.length).toBe(0);
  });

  test("undoing the only annotation uploads the empty set to Google Drive", async ({ page }) => {
    const drive = new MockDrive();
    await mockServer(page, drive);
    await page.goto("/");
    await open(page, "undo-drive.wav", noiseWav(96_000, 8, 9999));
    await annotate(page, 0.2, 0.4);
    await expect.poll(() => drive.uploads.length, { timeout: 20_000 }).toBe(1);
    expect(drive.uploads[0].content).toContain("time_start:");

    await undoUntilNone(page);
    await expect.poll(() => drive.uploads.length, { timeout: 20_000 }).toBe(2);
    expect(drive.uploads[1].method).toBe("PATCH");
    expect(drive.uploads[1].content).not.toContain("time_start:");
  });
});

