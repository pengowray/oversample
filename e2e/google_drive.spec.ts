import { test, expect, type Page } from "@playwright/test";
import { ACCOUNT_ID, EMAIL, MockDrive, mockServer } from "./drive_mock";

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
  await page.locator(".sidebar-header-label", { hasText: "Files" }).click();
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

/** Every saved annotation file in every store, by path from the storage root. */
const savedFiles = (page: Page): Promise<Record<string, string>> =>
  page.evaluate(async () => {
    const out: Record<string, string> = {};
    const walk = async (dir: FileSystemDirectoryHandle, prefix: string) => {
      const entries = (dir as unknown as { entries: () => AsyncIterable<[string, FileSystemHandle]> }).entries();
      for await (const [name, handle] of entries) {
        if (handle.kind === "directory") await walk(handle as FileSystemDirectoryHandle, `${prefix}${name}/`);
        else if (name.endsWith(".batm") || name.endsWith(".batproj") || name === "sync.json") {
          try {
            out[prefix + name] = await (await (handle as FileSystemFileHandle).getFile()).text();
          } catch {}
        }
      }
    };
    await walk(await navigator.storage.getDirectory(), "");
    return out;
  });

const DEVICE = "oversample-annotations/";
const ACCOUNT = `oversample-accounts/${ACCOUNT_ID}/annotations/`;
const inStore = (files: Record<string, string>, prefix: string) =>
  Object.keys(files).filter((k) => k.startsWith(prefix));

/** Write files into the browser's storage, by path from its root. */
const writeFiles = (page: Page, files: [string, string][]) =>
  page.evaluate(async (files) => {
    for (const [path, text] of files) {
      const parts = path.split("/");
      let dir = await navigator.storage.getDirectory();
      for (const p of parts.slice(0, -1)) dir = await dir.getDirectoryHandle(p, { create: true });
      const w = await (await dir.getFileHandle(parts[parts.length - 1], { create: true })).createWritable();
      await w.write(text);
      await w.close();
    }
  }, files);

/** Sign in through the pop-up flow (the mocked status turns to signed in). */
async function signInViaPopup(page: Page, drive: MockDrive) {
  if (!(await page.getByRole("button", { name: "Sign in with Google" }).isVisible())) await openSettings(page);
  const popup = page.waitForEvent("popup");
  await page.getByRole("button", { name: "Sign in with Google" }).click();
  await popup;
  drive.signedIn = true;
  await expect(page.locator(".drive-account")).toHaveText(EMAIL, { timeout: 15_000 });
}

/** Run the comparison of all files now (the app runs it when the browser comes back online). */
const compareAll = (page: Page) => page.evaluate(() => window.dispatchEvent(new Event("online")));

const regionsIn = (text: string) => (text.match(/time_start:/g) ?? []).length;

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
    await expect(page.locator(".drive-copies-status")).toHaveText("Saved");

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
    await expect(page.locator(".drive-copies-status")).toHaveText("Saved");
  });

  test("a Drive copy of a file with no local copy is used, and no set without its regions is uploaded", async ({ page }) => {
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
    // Opening the recording adds its hashes to the set, so it may go back
    // up once, as an update of the same Drive file and with both regions.
    await page.waitForTimeout(7_000);
    expect(drive.uploads.length).toBeLessThanOrEqual(1);
    for (const up of drive.uploads) {
      expect(up.method).toBe("PATCH");
      expect(up.content).toContain("from-drive-1");
      expect(up.content).toContain("from-drive-2");
    }
    expect([...drive.files.values()].filter((f) => f.appProperties.osKind === "batm")).toHaveLength(1);
  });

  test("sign-out with a file open: its annotations stop showing, nothing is deleted, and they come back at sign-in", async ({ page }) => {
    const drive = new MockDrive();
    await mockServer(page, drive);
    await page.goto("/");
    await openSettings(page);
    await expect(page.locator(".drive-account")).toHaveText(EMAIL);
    await open(page, "account-file.wav", noiseWav(96_000, 8, 3333));
    await annotate(page, 0.2, 0.4);
    await expect.poll(() => drive.uploads.length, { timeout: 20_000 }).toBe(1);
    expect(inStore(await savedFiles(page), ACCOUNT)).toHaveLength(1);
    expect(inStore(await savedFiles(page), DEVICE)).toHaveLength(0);

    await openSettings(page);
    await page.getByRole("button", { name: "Sign out" }).click();
    await expect(page.getByRole("button", { name: "Sign in with Google" })).toBeVisible();
    await expect.poll(() => drive.logouts).toBe(1);
    // The account's annotations are not shown any more; nothing was deleted.
    await expect.poll(async () => (await snap(page)).regions.length).toBe(0);
    expect(inStore(await savedFiles(page), ACCOUNT)).toHaveLength(1);
    await page.waitForTimeout(1_000);
    // Nothing was written into this browser's own store.
    expect(inStore(await savedFiles(page), DEVICE)).toHaveLength(0);

    // Signing in again brings them back.
    await signInViaPopup(page, drive);
    await expect.poll(async () => (await snap(page)).regions.length, { timeout: 15_000 }).toBe(1);
    await page.waitForTimeout(6_000);
    expect(drive.uploads).toHaveLength(1);
  });

  test("annotations made while signed out stay in this browser and are not uploaded at the next sign-in", async ({ page }) => {
    const drive = new MockDrive();
    drive.signedIn = false;
    await mockServer(page, drive);
    await page.goto("/");
    await open(page, "made-signed-out.wav", noiseWav(96_000, 8, 4444));
    await annotate(page, 0.2, 0.4);
    await expect.poll(async () => inStore(await savedFiles(page), DEVICE).length).toBe(1);

    // Sign in with that file open: its annotations stay, and edits keep
    // going to this browser's store.
    await signInViaPopup(page, drive);
    await expect.poll(async () => (await snap(page)).regions.length).toBe(1);
    await annotate(page, 0.5, 0.6);
    await expect
      .poll(async () => Object.entries(await savedFiles(page)).some(([k, t]) => k.startsWith(DEVICE) && regionsIn(t) === 2))
      .toBe(true);
    await page.waitForTimeout(7_000);
    expect(drive.uploads).toEqual([]);
    expect(inStore(await savedFiles(page), ACCOUNT)).toHaveLength(0);
    // The line says where they are, with no Drive row.
    const show = page.locator('button[title="Show info panel"]');
    if (await show.isVisible()) await show.click();
    await page.locator(".sidebar-tab-dropdown").click();
    await page.locator(".sidebar-tab-option", { hasText: "Annotations" }).dispatchEvent("mousedown");
    await expect(page.locator(".right-sidebar .annotation-storage")).toHaveText("Saved in this browser");
  });

  test("first sign-in with two files in the account's cache makes one set of folders", async ({ page }) => {
    const drive = new MockDrive();
    drive.signedIn = false;
    await mockServer(page, drive);
    await page.goto("/");
    const set = (file: string) =>
      `version: 3\nid: set-${file}\napp_version: 0.5.200\nmodified_at: 2026-11-01T10:00:00.000Z\n` +
      `file_identity:\n  filename: ${file}.wav\n  file_size: 100\nannotations:\n` +
      `- id: r-${file}\n  type: Region\n  time_start: 1.0\n  time_end: 2.0\n` +
      `  created_at: 2026-11-01T10:00:00.000Z\n  modified_at: 2026-11-01T10:00:00.000Z\n`;
    await writeFiles(page, [
      [`${ACCOUNT}first.wav_100.batm`, set("first")],
      [`${ACCOUNT}second.wav_100.batm`, set("second")],
    ]);
    drive.signedIn = true;
    await page.reload();
    await expect.poll(() => drive.uploads.length, { timeout: 30_000 }).toBe(2);
    expect(drive.uploads.map((u) => u.name).sort()).toEqual(["first.wav.batm", "second.wav.batm"]);
    const folders = (marker: string) =>
      [...drive.files.values()].filter((f) => f.appProperties.osFolder === marker).length;
    expect(folders("root")).toBe(1);
    expect(folders("annotations")).toBe(1);
  });

  test("migration: files synced before move into the account's cache, with no upload and no change", async ({ page }) => {
    const drive = new MockDrive();
    drive.signedIn = false;
    const set = (file: string) =>
      `version: 3\nid: set-${file}\napp_version: 0.5.200\nmodified_at: 2026-11-01T10:00:00.000Z\n` +
      `file_identity:\n  filename: ${file}.wav\n  file_size: 100\nannotations:\n` +
      `- id: r-${file}\n  type: Region\n  time_start: 1.0\n  time_end: 2.0\n` +
      `  created_at: 2026-11-01T10:00:00.000Z\n  modified_at: 2026-11-01T10:00:00.000Z\n`;
    const project = "version: 1\nid: p1\nname: Survey\nmodified_at: 2026-11-01T10:00:00.000Z\n";
    const root = drive.add({ name: "Oversample", mimeType: "application/vnd.google-apps.folder", appProperties: { osFolder: "root" } });
    const a = drive.add({ name: "a.wav.batm", parents: [root.id], appProperties: { osKind: "batm", osKey: "a.wav_100.batm" }, content: set("a") });
    const b = drive.add({ name: "b.wav.batm", parents: [root.id], appProperties: { osKind: "batm", osKey: "b.wav_100.batm" }, content: set("b") });
    const p = drive.add({ name: "Survey.batproj", parents: [root.id], appProperties: { osKind: "batproj", osKey: "p1" }, content: project });
    await mockServer(page, drive);
    await page.goto("/");
    // The state of a browser that used the earlier version, signed in.
    await writeFiles(page, [
      [`${DEVICE}a.wav_100.batm`, set("a")],
      [`${DEVICE}b.wav_100.batm`, set("b")],
      ["oversample-projects/p1.batproj", project],
      [`${DEVICE}c.wav_100.batm`, set("c")],
    ]);
    await page.evaluate(
      ([a, b, p]) =>
        localStorage.setItem(
          "oversample_drive_seen",
          JSON.stringify({
            account: "bat.person@example.com",
            files: {
              "batm:a.wav_100.batm": { id: a.id, rev: a.rev, fp: "old-style" },
              "batm:b.wav_100.batm": { id: b.id, rev: b.rev, fp: "old-style" },
              "batproj:p1": { id: p.id, rev: p.rev, fp: "old-style" },
            },
          }),
        ),
      [a, b, p].map((f) => ({ id: f.id, rev: f.headRevisionId })),
    );
    const before = await savedFiles(page);
    drive.signedIn = true;
    await page.reload();
    await expect
      .poll(async () => inStore(await savedFiles(page), ACCOUNT).sort(), { timeout: 20_000 })
      .toEqual([`${ACCOUNT}a.wav_100.batm`, `${ACCOUNT}b.wav_100.batm`]);
    await expect
      .poll(async () => JSON.parse((await savedFiles(page))[`oversample-accounts/${ACCOUNT_ID}/sync.json`] ?? "{}").migrated)
      .toBe(true);
    const after = await savedFiles(page);
    // Moved unchanged; the file with no record stays in this browser's store.
    expect(after[`${ACCOUNT}a.wav_100.batm`]).toBe(before[`${DEVICE}a.wav_100.batm`]);
    expect(after[`oversample-accounts/${ACCOUNT_ID}/projects/p1.batproj`]).toBe(project);
    expect(inStore(after, DEVICE)).toEqual([`${DEVICE}c.wav_100.batm`]);
    expect(after["oversample-projects/p1.batproj"]).toBeUndefined();
    expect(await page.evaluate(() => localStorage.getItem("oversample_drive_seen"))).toBeNull();
    // Nothing went up, and Drive's files are as they were.
    await page.waitForTimeout(7_000);
    expect(drive.uploads).toEqual([]);
    expect([a, b, p].every((f) => !f.trashed)).toBe(true);
  });

  test("migration: a file whose local copy gained a hash since its upload is updated once on Drive, and not changed here", async ({ page }) => {
    const drive = new MockDrive();
    drive.signedIn = false;
    const set = (hash: string) =>
      `version: 3\nid: set-h\napp_version: 0.5.200\nmodified_at: 2026-11-01T10:00:00.000Z\n` +
      `file_identity:\n  filename: h.wav\n  file_size: 100\n${hash}annotations:\n` +
      `- id: r-h\n  type: Region\n  time_start: 1.0\n  time_end: 2.0\n` +
      `  created_at: 2026-11-01T10:00:00.000Z\n  modified_at: 2026-11-01T10:00:00.000Z\n`;
    const local = set("  full_blake3: " + "ab".repeat(32) + "\n");
    const root = drive.add({ name: "Oversample", mimeType: "application/vnd.google-apps.folder", appProperties: { osFolder: "root" } });
    const h = drive.add({ name: "h.wav.batm", parents: [root.id], appProperties: { osKind: "batm", osKey: "h.wav_100.batm" }, content: set("") });
    await mockServer(page, drive);
    await page.goto("/");
    await writeFiles(page, [[`${DEVICE}h.wav_100.batm`, local]]);
    await page.evaluate(
      ([id, rev]) =>
        localStorage.setItem(
          "oversample_drive_seen",
          JSON.stringify({ account: "bat.person@example.com", files: { "batm:h.wav_100.batm": { id, rev, fp: "old-style" } } }),
        ),
      [h.id, h.headRevisionId],
    );
    const filesBefore = drive.files.size;
    drive.signedIn = true;
    await page.reload();
    await expect.poll(() => drive.uploads.length, { timeout: 20_000 }).toBe(1);
    const up = drive.uploads[0];
    expect(up.method).toBe("PATCH");
    expect(up.content).toContain("full_blake3");
    expect(regionsIn(up.content)).toBe(1);
    expect(drive.files.get(h.id)!.content).toBe(up.content);
    // The local copy moved unchanged, and nothing else went up or was made.
    expect((await savedFiles(page))[`${ACCOUNT}h.wav_100.batm`]).toBe(local);
    await page.waitForTimeout(7_000);
    expect(drive.uploads).toHaveLength(1);
    expect(drive.files.size).toBe(filesBefore);
  });

  test("a file deleted on Drive leaves this browser, also when it is open", async ({ page }) => {
    const drive = new MockDrive();
    await mockServer(page, drive);
    await page.goto("/");
    await open(page, "deleted-on-drive.wav", noiseWav(96_000, 8, 5151));
    await annotate(page, 0.2, 0.4);
    await expect.poll(() => drive.uploads.length, { timeout: 20_000 }).toBe(1);
    const onDrive = [...drive.files.values()].find((f) => f.appProperties.osKind === "batm")!;
    onDrive.trashed = true;
    await compareAll(page);
    await expect.poll(async () => inStore(await savedFiles(page), ACCOUNT).length, { timeout: 15_000 }).toBe(0);
    await expect.poll(async () => (await snap(page)).regions.length).toBe(0);
    await page.waitForTimeout(6_000);
    expect(drive.uploads).toHaveLength(1);
  });

  test("a file deleted on Drive but edited since in another tab is kept and uploaded again", async ({ page, context }) => {
    const drive = new MockDrive();
    await mockServer(page, drive);
    await page.goto("/");
    await open(page, "edited-elsewhere.wav", noiseWav(96_000, 8, 6262));
    await annotate(page, 0.2, 0.4);
    await expect.poll(() => drive.uploads.length, { timeout: 20_000 }).toBe(1);
    const first = [...drive.files.values()].find((f) => f.appProperties.osKind === "batm")!;
    first.trashed = true;
    // An edit in this tab, saved before the other tab compares.
    await annotate(page, 0.5, 0.6);
    await expect
      .poll(async () => Object.entries(await savedFiles(page)).some(([k, t]) => k.startsWith(ACCOUNT) && regionsIn(t) === 2))
      .toBe(true);
    const other = await context.newPage();
    await mockServer(other, drive);
    await other.goto("/");
    await expect
      .poll(() => [...drive.files.values()].some((f) => f.appProperties.osKind === "batm" && !f.trashed), { timeout: 30_000 })
      .toBe(true);
    expect(inStore(await savedFiles(page), ACCOUNT)).toHaveLength(1);
    expect((await snap(page)).regions.length).toBe(2);
  });

  test("Drive no longer showing the Oversample folder removes nothing and says so", async ({ page }) => {
    const drive = new MockDrive();
    await mockServer(page, drive);
    await page.goto("/");
    await open(page, "folder-gone.wav", noiseWav(96_000, 8, 7373));
    await annotate(page, 0.2, 0.4);
    await expect.poll(() => drive.uploads.length, { timeout: 20_000 }).toBe(1);
    for (const f of drive.files.values()) f.trashed = true;
    const filesBefore = drive.files.size;
    await compareAll(page);
    await openSettings(page);
    await expect(page.locator(".drive-settings")).toContainText(
      "Can't find the Oversample folder in Google Drive. Copying to Drive is paused. Everything is still saved in this browser. If you deleted the folder, it may be in Drive's trash.",
      { timeout: 15_000 },
    );
    expect(inStore(await savedFiles(page), ACCOUNT)).toHaveLength(1);
    expect((await snap(page)).regions.length).toBe(1);
    // An edit now waits: no upload, and no new folder.
    await annotate(page, 0.5, 0.6);
    await page.waitForTimeout(7_000);
    expect(drive.uploads).toHaveLength(1);
    expect(drive.files.size).toBe(filesBefore);

    // The folder is back from the trash: the edit goes up.
    for (const f of drive.files.values()) f.trashed = false;
    await compareAll(page);
    await expect.poll(() => drive.uploads.length, { timeout: 20_000 }).toBe(2);
    expect(regionsIn(drive.uploads[1].content)).toBe(2);
    await expect(page.locator(".drive-settings")).not.toContainText("Can't find the Oversample folder");
  });

  test("most synced files missing from Drive at once: nothing is removed, and it says so", async ({ page }) => {
    const drive = new MockDrive();
    drive.signedIn = false;
    await mockServer(page, drive);
    await page.goto("/");
    const set = (file: string) =>
      `version: 3\nid: set-${file}\napp_version: 0.5.200\nmodified_at: 2026-11-01T10:00:00.000Z\n` +
      `file_identity:\n  filename: ${file}.wav\n  file_size: 100\nannotations:\n` +
      `- id: r-${file}\n  type: Region\n  time_start: 1.0\n  time_end: 2.0\n` +
      `  created_at: 2026-11-01T10:00:00.000Z\n  modified_at: 2026-11-01T10:00:00.000Z\n`;
    await writeFiles(page, [
      [`${ACCOUNT}one.wav_100.batm`, set("one")],
      [`${ACCOUNT}two.wav_100.batm`, set("two")],
      [`${ACCOUNT}three.wav_100.batm`, set("three")],
    ]);
    drive.signedIn = true;
    await page.reload();
    await expect.poll(() => drive.uploads.length, { timeout: 30_000 }).toBe(3);
    // All three are deleted on Drive (not trashed); the folders stay.
    for (const [id, f] of [...drive.files.entries()]) if (f.appProperties.osKind === "batm") drive.files.delete(id);
    await compareAll(page);
    await openSettings(page);
    await expect(page.locator(".drive-settings")).toContainText(
      "3 files are missing from the Oversample folder in Google Drive. They are still saved in this browser.",
      { timeout: 15_000 },
    );
    expect(inStore(await savedFiles(page), ACCOUNT)).toHaveLength(3);
    await page.waitForTimeout(3_000);
    expect(drive.uploads).toHaveLength(3);
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
    await expect(page.locator(".drive-copies-status")).toHaveText("Saved");
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

  // ── Projects ──────────────────────────────────────────────────────────────

  async function enableProjects(page: Page) {
    await page.addInitScript(() => localStorage.setItem("oversample_projects_enabled", "true"));
  }

  async function openProjectTab(page: Page) {
    await page.getByRole("button", { name: /^Project/ }).first().click();
  }

  test("a project saved while signed in is uploaded to Oversample/projects with its id", async ({ page }) => {
    const drive = new MockDrive();
    await enableProjects(page);
    await mockServer(page, drive);
    await page.goto("/");
    await open(page, "project-file.wav", noiseWav(96_000, 8, 1212));
    await openProjectTab(page);
    await page.getByRole("button", { name: "Create project" }).click();
    await expect.poll(() => drive.uploads.length, { timeout: 20_000 }).toBe(1);
    const up = drive.uploads[0];
    expect(up.appProperties.osKind).toBe("batproj");
    const id = /^id: (.+)$/m.exec(up.content)![1].trim();
    expect(up.appProperties.osKey).toBe(id);
    expect(up.name).toBe("Untitled project.batproj");
    const folder = [...drive.files.values()].find((f) => f.appProperties.osFolder === "projects")!;
    expect(folder).toBeTruthy();
    const root = [...drive.files.values()].find((f) => f.appProperties.osFolder === "root")!;
    expect(folder.parents).toEqual([root.id]);
    const file = [...drive.files.values()].find((f) => f.appProperties.osKey === id)!;
    expect(file.parents).toEqual([folder.id]);
  });

  test("a project that exists only on Drive appears in the project list, and deleting it trashes the Drive file", async ({ page }) => {
    const drive = new MockDrive();
    const projectId = "0b9c2c1e-0000-4000-8000-00000000d01e";
    const onDrive = drive.add({
      name: "Pond survey.batproj",
      appProperties: { osKind: "batproj", osKey: projectId },
      content:
        `version: 1\nid: ${projectId}\napp_version: 0.5.200\ncreated_at: 2026-11-01T10:00:00.000Z\n` +
        `modified_at: 2026-11-01T10:00:00.000Z\nname: Pond survey\n`,
    });
    await enableProjects(page);
    await mockServer(page, drive);
    await page.goto("/");
    await page.locator(".toolbar").waitFor();
    await openProjectTab(page);
    // The full comparison at sign-in stores it locally; an open list
    // reloads when a project arrives from Drive.
    await page.getByRole("button", { name: "Load project" }).click();
    await expect(page.locator(".project-list-item", { hasText: "Pond survey" })).toHaveCount(1, { timeout: 20_000 });

    await page.locator(".project-list-item", { hasText: "Pond survey" }).locator(".project-list-item-delete").click();
    await expect(page.locator(".project-list-item", { hasText: "Pond survey" })).toHaveCount(0);
    await expect.poll(() => onDrive.trashed, { timeout: 15_000 }).toBe(true);
    expect(drive.uploads).toEqual([]);
  });

  // ── Both copies changed ──────────────────────────────────────────────────

  /** Change a Drive file as another device would: a new revision. */
  function editOnDrive(drive: MockDrive, id: string, content: string) {
    const f = drive.files.get(id)!;
    f.revisions[f.headRevisionId] = f.content;
    f.content = content;
    f.headRevisionId = `${f.headRevisionId}~`;
    f.modifiedTime = new Date().toISOString();
  }

  const annotationIds = (yaml: string) => [...yaml.matchAll(/^- id: (\S+)$/gm)].map((m) => m[1]);

  /** The set without the annotation `id` (a list entry runs to the next "- " or unindented line). */
  function withoutAnnotation(yaml: string, id: string): string {
    const lines = yaml.split("\n");
    const start = lines.indexOf(`- id: ${id}`);
    let end = start + 1;
    while (end < lines.length && lines[end].startsWith("  ")) end++;
    lines.splice(start, end - start);
    return lines.join("\n");
  }

  function withRegion(yaml: string, id: string, start: number): string {
    const at = new Date().toISOString();
    const entry =
      `- id: ${id}\n  type: Region\n  time_start: ${start}\n  time_end: ${start + 0.3}\n  freq_low: 20000.0\n  freq_high: 40000.0\n` +
      `  created_at: ${at}\n  modified_at: ${at}\n`;
    return yaml.replace(/^annotations:\n/m, `annotations:\n${entry}`);
  }

  const batmFile = (drive: MockDrive) => [...drive.files.values()].find((f) => f.appProperties.osKind === "batm")!;

  async function openAnnotationsPanel(page: Page) {
    const show = page.locator('button[title="Show info panel"]');
    if (await show.isVisible()) await show.click();
    await page.locator(".sidebar-tab-dropdown").click();
    await page.locator(".sidebar-tab-option", { hasText: "Annotations" }).dispatchEvent("mousedown");
  }

  test("both copies changed: the changes are merged, the user is told, and it settles", async ({ page }) => {
    const drive = new MockDrive();
    await mockServer(page, drive);
    await page.goto("/");
    await open(page, "merge-clean.wav", noiseWav(96_000, 8, 8181));
    await annotate(page, 0.2, 0.4);
    await expect.poll(() => drive.uploads.length, { timeout: 20_000 }).toBe(1);
    const file = batmFile(drive);
    // Another device adds a region; this browser adds one too.
    editOnDrive(drive, file.id, withRegion(file.content, "from-other", 3.0));
    await annotate(page, 0.5, 0.6);
    await expect.poll(() => drive.uploads.length, { timeout: 30_000 }).toBe(2);
    expect(regionsIn(file.content)).toBe(3);
    expect(file.content).toContain("from-other");
    await expect.poll(async () => (await snap(page)).regions.length).toBe(3);

    await openAnnotationsPanel(page);
    const notice = page.locator(".right-sidebar .merge-notice");
    await expect(notice).toContainText("Merged with the Google Drive copy");
    await expect(notice).toContainText("From Google Drive: 1 annotation added");
    await expect(notice).not.toContainText("earlier version");
    // Nothing more goes up: both sides now hold the merge.
    await page.waitForTimeout(7_000);
    expect(drive.uploads).toHaveLength(2);
    await notice.getByRole("button", { name: "Dismiss" }).click();
    await expect(notice).toHaveCount(0);
  });

  test("a merge that drops part of this browser's copy uploads that copy first, so Drive keeps it as a version", async ({ page }) => {
    const drive = new MockDrive();
    await mockServer(page, drive);
    await page.goto("/");
    await open(page, "merge-drop.wav", noiseWav(96_000, 8, 8282));
    await annotate(page, 0.2, 0.3);
    await annotate(page, 0.5, 0.6);
    await expect.poll(() => regionsIn(batmFile(drive)?.content ?? ""), { timeout: 20_000 }).toBe(2);
    await page.waitForTimeout(1_000);
    const file = batmFile(drive);
    const [keep, gone] = annotationIds(file.content);
    // No stored base: the app reads the revision it last saw from Drive's history.
    await page.evaluate(async (dir) => {
      const root = await navigator.storage.getDirectory();
      const account = await (await root.getDirectoryHandle("oversample-accounts")).getDirectoryHandle(dir);
      await account.removeEntry("bases", { recursive: true });
    }, ACCOUNT_ID);
    const uploadsBefore = drive.uploads.length;
    // Another device deletes one region; this browser adds one.
    editOnDrive(drive, file.id, withoutAnnotation(file.content, gone));
    await annotate(page, 0.8, 0.9);
    await expect.poll(() => drive.uploads.length, { timeout: 30_000 }).toBe(uploadsBefore + 2);
    const [before, merged] = drive.uploads.slice(uploadsBefore);
    // First this browser's copy as it was (3 regions), then the merge.
    expect(regionsIn(before.content)).toBe(3);
    expect(before.content).toContain(gone);
    expect(regionsIn(merged.content)).toBe(2);
    expect(merged.content).toContain(keep);
    expect(merged.content).not.toContain(gone);
    expect(file.content).toBe(merged.content);
    expect(drive.history(file.id)).toContain(before.content);
    await expect.poll(async () => (await snap(page)).regions.length).toBe(2);

    await openAnnotationsPanel(page);
    const notice = page.locator(".right-sidebar .merge-notice");
    await expect(notice).toContainText("From Google Drive: 1 annotation removed");
    await expect(notice).toContainText("from before the merge are saved in Google Drive as an earlier version from");
    await expect(notice.getByRole("link", { name: "Open in Google Drive" })).toHaveAttribute(
      "href",
      `https://drive.google.com/file/d/${file.id}/view`,
    );
    await page.waitForTimeout(7_000);
    expect(drive.uploads).toHaveLength(uploadsBefore + 2);
  });

  test("if the merge's upload fails after this browser's copy went up, the next try puts the merge on Drive", async ({ page }) => {
    const drive = new MockDrive();
    await mockServer(page, drive);
    await page.goto("/");
    await open(page, "merge-fail.wav", noiseWav(96_000, 8, 8383));
    await annotate(page, 0.2, 0.3);
    await annotate(page, 0.5, 0.6);
    await expect.poll(() => regionsIn(batmFile(drive)?.content ?? ""), { timeout: 20_000 }).toBe(2);
    await page.waitForTimeout(1_000);
    const file = batmFile(drive);
    const [, gone] = annotationIds(file.content);
    const uploadsBefore = drive.uploads.length;
    editOnDrive(drive, file.id, withoutAnnotation(file.content, gone));
    // The copy from before the merge goes up; the merge itself fails.
    drive.failUploadsAfter = 1;
    await annotate(page, 0.8, 0.9);
    await expect.poll(() => drive.uploads.length, { timeout: 30_000 }).toBe(uploadsBefore + 1);
    await openSettings(page);
    await expect(page.locator(".drive-settings")).toContainText("Can't reach Google Drive", { timeout: 20_000 });
    expect(file.content).toContain(gone);

    drive.failUploadsAfter = null;
    await page.getByRole("button", { name: "Retry" }).click();
    await expect.poll(() => file.content.includes(gone), { timeout: 30_000 }).toBe(false);
    expect(regionsIn(file.content)).toBe(2);
    await page.waitForTimeout(7_000);
    expect(file.content).not.toContain(gone);
    expect(regionsIn(file.content)).toBe(2);
  });
});
