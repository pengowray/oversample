import { test, expect, type Page, type ConsoleMessage } from "@playwright/test";

/**
 * Smoke tests for the Oversample web build.
 *
 * These are deliberately shallow — they verify the WASM bundle loads, Leptos
 * mounts the root `<App>`, the toolbar/sidebar render, and the obvious empty
 * state appears when no file has been opened. They do NOT exercise audio
 * decoding, playback, or microphone capture, which need real files and user
 * gestures that don't survive headless Chromium runs cleanly.
 *
 * If WASM compile output ever changes the bundle name, no test selectors here
 * depend on it — we drive the UI via stable CSS classes and visible text.
 */

// Errors we treat as fatal. Anything else (favicon 404s, harmless deprecation
// warnings, etc.) is recorded but doesn't fail the test.
const FATAL_PATTERNS = [
  /uncaught/i,
  /panicked/i,
  /panic at/i,
  /RuntimeError/i,
  /wasm.*aborted/i,
  /expect_context/i,
];

// Patterns we expect and silently ignore.
const IGNORED_PATTERNS = [
  /Failed to load resource.*favicon/i,
  /AudioContext was not allowed to start/i, // expected without user gesture
];

interface PageErrors {
  console: string[];
  pageerrors: string[];
}

function attachErrorRecorder(page: Page): PageErrors {
  const errors: PageErrors = { console: [], pageerrors: [] };
  page.on("console", (msg: ConsoleMessage) => {
    if (msg.type() !== "error") return;
    const text = msg.text();
    if (IGNORED_PATTERNS.some((p) => p.test(text))) return;
    errors.console.push(text);
  });
  page.on("pageerror", (err) => {
    const text = err.message;
    if (IGNORED_PATTERNS.some((p) => p.test(text))) return;
    errors.pageerrors.push(text);
  });
  return errors;
}

function failOnFatal(errors: PageErrors) {
  const all = [...errors.console, ...errors.pageerrors];
  const fatal = all.filter((e) => FATAL_PATTERNS.some((p) => p.test(e)));
  expect(fatal, `Unexpected fatal errors:\n${fatal.join("\n")}`).toEqual([]);
}

test.describe("Oversample web smoke", () => {
  test("loads index and serves the page", async ({ page }) => {
    const response = await page.goto("/");
    expect(response?.status(), "GET / returns 2xx").toBeLessThan(400);
    await expect(page).toHaveTitle(/Oversample/);
  });

  test("WASM boots and mounts the root <App>", async ({ page }) => {
    const errors = attachErrorRecorder(page);
    await page.goto("/");
    // .app is the root element Leptos mounts inside <body>. If we can see it,
    // the WASM module initialised and components rendered.
    await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
    failOnFatal(errors);
  });

  test("toolbar renders with brand", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator(".toolbar")).toBeVisible();
    // The brand contains "Oversample" plus a "beta" italic span; match loosely.
    await expect(page.locator(".toolbar-brand")).toContainText("Oversample");
  });

  test("sidebar renders with Files tab", async ({ page }) => {
    await page.goto("/");
    // The left sidebar is `.sidebar` (gets `collapsed` / `mobile-overlay`
    // modifiers depending on state — we don't care about those here).
    await expect(page.locator(".sidebar").first()).toBeAttached();
    await expect(
      page.locator(".sidebar-header-label", { hasText: "Files" }).first(),
    ).toBeVisible();
  });

  test("empty state appears when no file is loaded", async ({ page }) => {
    await page.goto("/");
    // Desktop viewport (Playwright default) — empty state text is the desktop
    // hint about dropping files. On mobile widths it changes to a hamburger hint.
    const empty = page.locator(".empty-state");
    await expect(empty).toBeVisible({ timeout: 30_000 });
    await expect(empty).toContainText(/Drop|Tap/);
  });

  test("clicking the brand opens the About dialog", async ({ page }) => {
    await page.goto("/");
    await page.locator(".toolbar-brand").click();
    // The About dialog mounts `.about-overlay > .about-dialog` (toolbar.rs).
    await expect(page.locator(".about-dialog")).toBeVisible({ timeout: 5_000 });
    // Close button restores normal state — sanity-check the close path too.
    await page.locator(".about-close").click();
    await expect(page.locator(".about-dialog")).toHaveCount(0);
  });

  test("unknown URL hash does not crash the app", async ({ page }) => {
    const errors = attachErrorRecorder(page);
    // A hash that won't resolve to a demo entry; the app should show a toast
    // (or silently no-op) but not throw.
    await page.goto("/#XC0000001");
    await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
    // Allow the spawn_local hash handler time to run its fetch + toast logic.
    await page.waitForTimeout(500);
    failOnFatal(errors);
  });

  test("loading a demo bat + clicking HFR does not hang", async ({ page }) => {
    // Regression guard for the HFR-click freeze where Effect B subscribed to
    // het_cutoff while also writing it, looping until the page locked up.
    test.setTimeout(60_000);
    const errors = attachErrorRecorder(page);
    await page.goto("/");
    await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });

    // Open the demo picker, then load a random bat.
    const loadDemo = page.locator("button.add-files-btn", { hasText: "Load demo" });
    await expect(loadDemo).toBeVisible({ timeout: 15_000 });
    await loadDemo.click();
    const randomBat = page.locator(".demo-random-bat").first();
    await expect(randomBat).toBeVisible({ timeout: 10_000 });
    await randomBat.click();

    // Spectrogram should mount after decode (canvas inside the container).
    await expect(page.locator(".spectrogram-container").first()).toBeVisible({
      timeout: 30_000,
    });

    // The HFR combo's left half shows literal "HFR" in its .layer-btn-value.
    const hfr = page
      .locator("button .layer-btn-value", { hasText: /^HFR$/ })
      .first();
    await expect(hfr).toBeVisible({ timeout: 5_000 });
    await hfr.click();

    // Probe page responsiveness: a hung WASM event loop would block
    // waitForFunction's polling. The explicit timeout fails the test rather
    // than hanging the runner forever.
    await page.waitForFunction(() => Date.now() > 0, undefined, {
      timeout: 5_000,
    });

    failOnFatal(errors);
  });

  test("debug-build banner is absent in release build", async ({ page }) => {
    // The dev-helper banner only renders when cfg!(debug_assertions); the
    // webServer in playwright.config.ts launches trunk with --release, so this
    // should never appear. If it does, someone changed how we serve tests.
    await page.goto("/");
    await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
    await expect(page.locator(".debug-build-banner")).toHaveCount(0);
  });
});
