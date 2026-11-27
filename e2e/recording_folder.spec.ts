import { test, expect, type Page } from "@playwright/test";
import * as fs from "node:fs";
import * as path from "node:path";

/**
 * The folder for new recordings and file paths, in the desktop app
 * (src/recording_folder.rs, src/components/recording_folder.rs, the Path row
 * in src/components/file_sidebar/metadata_panel.rs).
 *
 * The page runs with a fake `__TAURI_INTERNALS__` that answers the folder
 * commands and serves one WAV at a path on a fake disk, so nothing here
 * checks the native side (src-tauri/src/save_folder.rs has its own tests).
 * In the browser build none of it shows.
 *
 * Set FOLDER_SHOT_DIR to save screenshots of each state there.
 */

const SHOT_DIR = process.env.FOLDER_SHOT_DIR;

/** Also set FOLDER_AUDIT to a ui-audit.js script to run it beside each screenshot. */
const AUDIT = process.env.FOLDER_AUDIT;

async function shot(page: Page, name: string) {
  if (!SHOT_DIR) return;
  await page.screenshot({ path: path.join(SHOT_DIR, name) });
  if (AUDIT) {
    const report = await page.evaluate(fs.readFileSync(AUDIT, "utf8"));
    fs.writeFileSync(path.join(SHOT_DIR, name.replace(/\.png$/, ".audit.json")), JSON.stringify(report, null, 2));
  }
}

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

const HOME = "/home/ann";
const DEFAULT = `${HOME}/Music/Oversample`;
const SHORT_FILE = `${DEFAULT}/batcap_20261127_180328.wav`;
const LONG_FILE =
  "/media/ann/FIELD-SSD-2TB/Surveys/2026/Victoria/Grampians National Park/Site 14 - Mackenzie Falls car park/Night 3 (wet)/batcap_20261127_180328_Chalinolobus_gouldii_candidate_calls_for_review.wav";
const LONG_FOLDER =
  "/media/ann/FIELD-SSD-2TB/Surveys/2026/Victoria/Grampians National Park/Site 14 - Mackenzie Falls car park/Night 3 (wet)";

type Fake = { file: string; custom: string | null; usable: boolean; musicMissing: boolean };

/** Runs the page as the desktop app. `window.__fake` holds what the fake
 * commands answer and `window.__opened` the folders it was asked to open. */
async function desktop(page: Page, audio: Buffer, fake: Fake) {
  await page.addInitScript(
    ({ b64, fake, home, def }) => {
      const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
      const w = window as any;
      w.__fake = fake;
      w.__opened = [] as string[];
      const short = (p: string) => (p.startsWith(home) ? "~" + p.slice(home.length) : p);
      const folder = () => {
        const f = w.__fake as Fake;
        const dflt = f.musicMissing ? "/home/ann/.local/share/com.oversample.app/recordings" : def;
        const p = f.custom && f.usable ? f.custom : dflt;
        return {
          path: p,
          short_path: short(p),
          default_path: dflt,
          is_custom: f.custom !== null,
          unusable_custom: f.custom && !f.usable ? f.custom : null,
          music_missing: f.musicMissing,
        };
      };
      w.__TAURI_INTERNALS__ = {
        transformCallback: () => 0,
        metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
        invoke: (cmd: string, args: Record<string, any>) => {
          switch (cmd) {
            case "google_status":
              return Promise.resolve({ available: false, signedIn: false, persistent: true });
            case "open_file_dialog":
              return Promise.resolve([w.__fake.file]);
            case "read_file_range":
              return Promise.resolve(bytes.slice(args.offset, args.offset + args.length).buffer);
            case "read_file_bytes":
              return Promise.resolve(bytes.slice().buffer);
            case "list_central_annotations":
              return Promise.resolve([]);
            case "read_central_annotations":
            case "write_central_annotations":
            case "read_sidecar":
              return Promise.resolve(null);
            case "read_folder_settings":
              return Promise.resolve({ annotations_beside: false });
            case "mic_recover_recordings":
              return Promise.resolve([]);
            case "recording_folder_get":
              return Promise.resolve(folder());
            case "recording_folder_choose":
              w.__fake.custom = "/media/ann/FIELD-SSD-2TB/Bat recordings";
              w.__fake.usable = true;
              return Promise.resolve(folder());
            case "recording_folder_reset":
              w.__fake.custom = null;
              return Promise.resolve(folder());
            case "open_containing_folder":
              w.__opened.push(args.path);
              return Promise.resolve(null);
            default:
              return Promise.reject(`not faked: ${cmd}`);
          }
        },
      };
    },
    { b64: audio.toString("base64"), fake, home: HOME, def: DEFAULT },
  );
  await page.goto("/");
  await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
}

/** Open the fake disk's file. At phone width the app opens files with the
 * browser's file input, so the file is opened at desktop width first. */
async function openFile(page: Page) {
  const size = page.viewportSize()!;
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.getByText("+ Open files").click();
  await expect(page.locator(".file-item:not(.loading)")).toHaveCount(1, { timeout: 30_000 });
  await page.setViewportSize(size);
  await hideSidebar(page);
  const btn = page.locator('button[title="Show info panel"]');
  if (await btn.count()) await btn.first().click();
  await expect(page.getByTestId("info-path-row")).toBeVisible();
}

/** At phone width the file sidebar covers the page: close it. */
async function hideSidebar(page: Page) {
  if (page.viewportSize()!.width < 768) {
    // The sidebar can close by itself after a resize, so the button may go.
    const hide = page.getByTitle("Hide sidebar").first();
    if (await hide.isVisible()) await hide.click({ timeout: 5_000 }).catch(() => {});
    await expect(page.locator(".sidebar:not(.right-sidebar):not(.collapsed)")).toHaveCount(0);
  }
}

async function openSettings(page: Page) {
  await page.getByTitle("Settings", { exact: true }).click();
  await expect(page.getByTestId("recording-folder-settings")).toBeVisible();
}

/** No element in `sel` is wider than its sidebar, and the page does not scroll sideways. */
async function noOverflow(page: Page, sel: string) {
  const r = await page.evaluate((sel) => {
    const el = document.querySelector(sel) as HTMLElement;
    const side = el.closest(".right-sidebar, .file-sidebar, .sidebar") as HTMLElement | null;
    return {
      el: el.getBoundingClientRect().right,
      side: side ? side.getBoundingClientRect().right : window.innerWidth,
      scroll: document.documentElement.scrollWidth - document.documentElement.clientWidth,
      elScroll: el.scrollWidth - el.clientWidth,
    };
  }, sel);
  expect(r.el).toBeLessThanOrEqual(r.side + 1);
  expect(r.elScroll).toBeLessThanOrEqual(1);
  expect(r.scroll).toBeLessThanOrEqual(0);
}

for (const [label, viewport] of [
  ["desktop", { width: 1440, height: 900 }],
  ["narrow", { width: 375, height: 812 }],
] as const) {
  test.describe(`recording folder and paths (${label})`, () => {
    test.use({ viewport });

    test("Info shows a short path, its folder opens, and it can be copied", async ({ page }) => {
      await desktop(page, wav(48_000, 2), { file: SHORT_FILE, custom: null, usable: true, musicMissing: false });
      await openFile(page);
      const row = page.getByTestId("info-path-row");
      await expect(row.locator(".path-row-value")).toHaveText(SHORT_FILE);
      await expect(row.locator(".path-row-value")).toHaveAttribute("title", SHORT_FILE);
      await row.getByRole("button", { name: "Open folder" }).click();
      expect(await page.evaluate(() => (window as any).__opened)).toEqual([SHORT_FILE]);
      await noOverflow(page, '[data-testid="info-path-row"]');
      await row.scrollIntoViewIfNeeded();
      await shot(page, `info-short-${label}.png`);
    });

    test("a long path wraps and keeps the file name", async ({ page }) => {
      await desktop(page, wav(48_000, 2), { file: LONG_FILE, custom: null, usable: true, musicMissing: false });
      await openFile(page);
      const value = page.getByTestId("info-path-row").locator(".path-row-value");
      await expect(value).toHaveText(LONG_FILE);
      // Every line of it is inside the sidebar: the end (the file name) shows.
      await noOverflow(page, '[data-testid="info-path-row"] .path-row-value');
      await page.getByTestId("info-path-row").scrollIntoViewIfNeeded();
      await shot(page, `info-long-${label}.png`);
    });

    test("Settings shows the default folder, changes it and resets it", async ({ page }) => {
      await desktop(page, wav(48_000, 2), { file: SHORT_FILE, custom: null, usable: true, musicMissing: false });
      await openSettings(page);
      const group = page.getByTestId("recording-folder-settings");
      await expect(group.locator(".folder-path")).toHaveText(DEFAULT);
      await expect(group.getByRole("button", { name: /^Reset to/ })).toHaveCount(0);
      await group.scrollIntoViewIfNeeded();
      await shot(page, `settings-default-${label}.png`);

      await group.getByRole("button", { name: "Change folder…" }).click();
      await expect(group.locator(".folder-path")).toHaveText("/media/ann/FIELD-SSD-2TB/Bat recordings");
      const reset = group.getByRole("button", { name: "Reset to Music/Oversample" });
      await expect(reset).toBeVisible();
      await shot(page, `settings-custom-${label}.png`);
      await reset.click();
      await expect(group.locator(".folder-path")).toHaveText(DEFAULT);
      await expect(reset).toHaveCount(0);
    });

    test("Settings warns when the chosen folder can't be used", async ({ page }) => {
      await desktop(page, wav(48_000, 2), {
        file: SHORT_FILE,
        custom: LONG_FOLDER,
        usable: false,
        musicMissing: false,
      });
      await openSettings(page);
      const group = page.getByTestId("recording-folder-settings");
      await expect(group.locator(".folder-path")).toHaveText(DEFAULT);
      await expect(group.locator(".drive-warning")).toHaveText(
        `Can't save to ${LONG_FOLDER}, so new recordings are saved in the default folder for now.`,
      );
      await noOverflow(page, '[data-testid="recording-folder-settings"]');
      await group.scrollIntoViewIfNeeded();
      await shot(page, `settings-unusable-${label}.png`);
    });

    test("the Record menu shows the folder under To file", async ({ page }) => {
      await desktop(page, wav(48_000, 2), {
        file: SHORT_FILE,
        custom: LONG_FOLDER,
        usable: true,
        musicMissing: false,
      });
      await hideSidebar(page);
      await page.getByTitle("Record settings").first().click();
      await page.getByRole("button", { name: "To file", exact: true }).click();
      const line = page.getByTestId("rec-folder-line");
      await expect(line).toBeVisible();
      await expect(line).toHaveAttribute("title", LONG_FOLDER);
      // One line, with the last folder name shown whole.
      const box = (await line.boundingBox())!;
      expect(box.height).toBeLessThan(32);
      await expect(line.locator(".path-tail")).toHaveText("Night 3 (wet)");
      await noOverflow(page, '[data-testid="rec-folder-line"]');
      await shot(page, `record-menu-${label}.png`);
      await page.getByRole("button", { name: "To memory", exact: true }).click();
      await expect(line).toHaveCount(0);
    });

    test("the Record menu shows the default folder when the chosen one can't be used", async ({ page }) => {
      await desktop(page, wav(48_000, 2), {
        file: SHORT_FILE,
        custom: LONG_FOLDER,
        usable: false,
        musicMissing: false,
      });
      await hideSidebar(page);
      await page.getByTitle("Record settings").first().click();
      await page.getByRole("button", { name: "To file", exact: true }).click();
      const line = page.getByTestId("rec-folder-line");
      await expect(line).toHaveClass(/rec-folder-warn/);
      await expect(line.locator(".rec-folder-path")).toHaveText("~/Music/Oversample");
      await expect(line).toHaveAttribute(
        "title",
        `${DEFAULT}\nCan't save to ${LONG_FOLDER}, so new recordings are saved in the default folder for now.`,
      );
      await shot(page, `record-menu-unusable-${label}.png`);
    });

    test("Settings says when there is no Music folder", async ({ page }) => {
      await desktop(page, wav(48_000, 2), { file: SHORT_FILE, custom: null, usable: true, musicMissing: true });
      await openSettings(page);
      const group = page.getByTestId("recording-folder-settings");
      await expect(group).toContainText("Can't save to the Music folder");
      await noOverflow(page, '[data-testid="recording-folder-settings"]');
      await group.scrollIntoViewIfNeeded();
      await shot(page, `settings-no-music-${label}.png`);
    });
  });
}

test.describe("recording folder in the browser build", () => {
  test("shows no folder setting and no Path row", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
    await page.getByTitle("Settings", { exact: true }).click();
    await expect(page.locator(".setting-group-title", { hasText: "Playback" })).toBeVisible();
    await expect(page.getByTestId("recording-folder-settings")).toHaveCount(0);
    await expect(page.getByTestId("info-path-row")).toHaveCount(0);
  });
});
