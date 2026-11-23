import { test, expect } from "@playwright/test";

/**
 * Start-up: the page asks for few files before the app shows, and the app
 * records how long each start-up stage took.
 */

test("start-up loads no wasm-bindgen snippet files", async ({ page }) => {
  // Old builds left folders in target/wasm-bindgen/release/snippets, and
  // Trunk had the page preload every file in them (382 requests). The app's
  // JS helpers are in js/app-glue.js now, so there should be none.
  const snippets: string[] = [];
  page.on("request", (r) => {
    if (r.url().includes("/snippets/")) snippets.push(r.url());
  });
  await page.goto("/");
  await expect(page.locator("#boot-splash")).toHaveCount(0);
  expect(snippets).toEqual([]);
  expect(await page.locator('link[rel="modulepreload"]').count()).toBeLessThanOrEqual(2);
});

test("start-up stage times are recorded in order", async ({ page }) => {
  await page.goto("/");
  await page.waitForFunction(() => (window as any).__oversampleStartup);
  const [wasmReady, appBuilt, firstFrame] = await page.evaluate(
    () => (window as any).__oversampleStartup as number[],
  );
  expect(wasmReady).toBeGreaterThan(0);
  expect(appBuilt).toBeGreaterThanOrEqual(wasmReady);
  expect(firstFrame).toBeGreaterThanOrEqual(appBuilt);
});

test("the helpers in app-glue.js are there when the app starts", async ({ page }) => {
  await page.goto("/");
  await page.waitForFunction(() => (window as any).__oversampleStartup);
  const kinds = await page.evaluate(() =>
    ["language_name", "language_autonym", "opfs_audio_supported", "opfs_audio_free_bytes"].map(
      (n) => typeof (window as any)[n],
    ),
  );
  expect(kinds).toEqual(["function", "function", "function", "function"]);
});
