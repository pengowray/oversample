import { test, expect, type Page } from "@playwright/test";
import { EMAIL, MockDrive, desktopApp, mockServer, type Stub } from "./drive_mock";

/**
 * Google Drive copies in the desktop app (src/cloud/, provider "Desktop").
 *
 * The page runs as if inside the desktop app: `__TAURI_INTERNALS__` is a
 * stub that answers the Google commands (status, sign-in, cancel, token,
 * sign-out) and keeps the app's central annotations folder in memory. The
 * Drive REST API is mocked as in google_drive.spec.ts. The Rust side of the
 * sign-in is covered by the tests in src-tauri/src/google_auth.rs.
 */

const stubState = (page: Page) =>
  page.evaluate(() => {
    const st = (window as unknown as { __googleStub: Stub }).__googleStub;
    return { signInCalls: st.signInCalls, central: st.central };
  });

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

type Snap = { currentFile: string | null; scrollOffset: number; zoom: number; sampleRate: number; duration: number; regions: unknown[] };
const snap = (page: Page): Promise<Snap> =>
  page.evaluate(() => (window as unknown as { __oversample_test: () => Snap }).__oversample_test());

async function openSettings(page: Page) {
  await page.locator(".toolbar").waitFor();
  const show = page.locator('button[title="Show sidebar"]');
  if (await show.isVisible()) await show.click();
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await expect(page.locator(".setting-group-title", { hasText: "Beta" })).toBeVisible();
}

async function openFileAndAnnotate(page: Page, name: string) {
  await page.getByRole("button", { name: "Files", exact: true }).click();
  await page
    .locator('input[type="file"][accept*=".wav"]')
    .first()
    .setInputFiles({ name, mimeType: "audio/wav", buffer: noiseWav(96_000, 8, 2468) });
  await expect.poll(async () => (await snap(page)).currentFile).toBe(name);
  await expect(page.locator(".chart-stage canvas").first()).toBeVisible({ timeout: 30_000 });
  await page.locator(".tool-combo-btn").click();
  await page.locator(".layer-panel button", { hasText: "Selection" }).click();
  const box = (await page.locator(".main-view .chart-stage > canvas").boundingBox())!;
  await page.mouse.move(box.x + box.width * 0.2, box.y + box.height * 0.3);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.4, box.y + box.height * 0.6, { steps: 5 });
  await page.mouse.up();
  await page.locator('.canvas-overflow-btn[title="Selection options"]').click();
  await page.locator(".canvas-overflow-item", { hasText: "Annotate Region" }).click();
  await expect(page.locator(".annotation-inline-label-editor")).toBeVisible();
  await page.keyboard.press("Enter");
  await expect.poll(async () => (await snap(page)).regions.length).toBe(1);
}

test.describe("Google Drive copies (desktop app)", () => {
  test("hidden when the build has no Google client (or on Android)", async ({ page }) => {
    await desktopApp(page, { available: false });
    await mockServer(page, new MockDrive());
    await page.goto("/");
    await openSettings(page);
    await page.waitForTimeout(500);
    await expect(page.locator(".drive-settings")).toHaveCount(0);
  });

  test("sign-in waits for the browser, can be cancelled, and says when it timed out", async ({ page }) => {
    await desktopApp(page, { nextSignIn: "wait" });
    await mockServer(page, new MockDrive());
    await page.goto("/");
    await openSettings(page);
    const group = page.locator(".drive-settings");
    await page.getByRole("button", { name: "Sign in with Google" }).click();
    await expect(group).toContainText("Google's sign-in page is open in your web browser.");
    await expect(page.getByRole("button", { name: "Signing in…" })).toBeDisabled();
    await page.getByRole("button", { name: "Cancel" }).click();
    await expect(page.getByRole("button", { name: "Sign in with Google" })).toBeEnabled();
    await expect(group).not.toContainText("Sign-in");

    await page.evaluate(() => {
      (window as unknown as { __googleStub: Stub }).__googleStub.nextSignIn = "timed_out";
    });
    await page.getByRole("button", { name: "Sign in with Google" }).click();
    await expect(group).toContainText("Sign-in timed out after 5 minutes.");
    expect((await stubState(page)).signInCalls).toBe(2);
  });

  test("signed in: a read-only check of Drive, and nothing is uploaded or created", async ({ page }) => {
    const drive = new MockDrive();
    const root = drive.add({ name: "Oversample", mimeType: "application/vnd.google-apps.folder", appProperties: { osFolder: "root" } });
    const folder = drive.add({ name: "annotations", mimeType: "application/vnd.google-apps.folder", parents: [root.id], appProperties: { osFolder: "annotations" } });
    for (const n of ["a", "b"]) {
      drive.add({ name: `${n}.wav.batm`, parents: [folder.id], appProperties: { osKind: "batm", osKey: `${n}.wav_1.batm` }, content: "x" });
    }
    drive.add({ name: "Survey.batproj", appProperties: { osKind: "batproj", osKey: "p1" }, content: "x" });
    const before = drive.files.size;
    await desktopApp(page, { signedIn: true, persistent: false });
    await mockServer(page, drive);
    await page.goto("/");
    await openSettings(page);
    const group = page.locator(".drive-settings");
    await expect(page.locator(".drive-account")).toHaveText(EMAIL);
    await expect(group).toContainText("Oversample folder found in Google Drive: 2 annotation files, 1 project.");
    await expect(group).toContainText("The desktop app does not save to Google Drive yet.");
    await expect(group).toContainText("Signed in until Oversample closes, because no keyring is available.");

    // An edit is saved on this device only.
    await openFileAndAnnotate(page, "desktop-local.wav");
    await expect.poll(async () => Object.keys((await stubState(page)).central).length).toBe(1);
    await page.waitForTimeout(7_000);
    expect(drive.uploads).toEqual([]);
    expect(drive.files.size).toBe(before);

    const show = page.locator('button[title="Show info panel"]');
    if (await show.isVisible()) await show.click();
    await page.locator(".sidebar-tab-dropdown").click();
    await page.locator(".sidebar-tab-option", { hasText: "Annotations" }).dispatchEvent("mousedown");
    const line = page.locator(".right-sidebar .annotation-storage");
    await expect(line).toHaveText("Saved on this device");
    await expect(line.getByRole("button")).toHaveCount(0);
  });

  test("signed in, no Oversample folder visible: says so and creates nothing", async ({ page }) => {
    const drive = new MockDrive();
    await desktopApp(page, { signedIn: true });
    await mockServer(page, drive);
    await page.goto("/");
    await openSettings(page);
    await expect(page.locator(".drive-settings")).toContainText("No Oversample folder is visible to the desktop app.");
    await page.waitForTimeout(1_000);
    expect(drive.files.size).toBe(0);
    expect(drive.uploads).toEqual([]);
  });
});
