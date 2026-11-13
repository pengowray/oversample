import { test, expect, Page, Route } from "@playwright/test";
import * as fs from "fs";
import * as path from "path";

/**
 * A recording's licence in the Find recordings list, the toolbar badge and the licence
 * dialog, for licence values the search server sends: GBIF's short name with a
 * description, and a Wikimedia Commons licence that is not Creative Commons.
 */

const WAV = fs.readFileSync(path.resolve(process.cwd(), "e2e", "fixtures", "fake-audio.wav"));

function json(route: Route, body: unknown) {
  return route.fulfill({
    status: 200,
    contentType: "application/json",
    headers: { "Access-Control-Allow-Origin": "*" },
    body: JSON.stringify(body),
  });
}

function recording(n: number, source: string, licence: string) {
  return {
    id: `${source}:${n}/0`,
    source,
    gen: "Myotis",
    sp: "daubentonii",
    en: "Daubenton's Bat",
    cc: "GB",
    country: "United Kingdom",
    place: `Pond ${n}`,
    smp: 48000,
    len_s: 2,
    date: "2024-06-01",
    recordist: `Recordist ${n}`,
    licence,
    quality: "",
    page_url: `https://example.org/${source}/${n}`,
    audio_url: `https://example.org/audio/${source}-${n}.wav`,
    audio_via: "direct",
  };
}

const ITEMS = [
  recording(1, "gbif", "CC BY-NC (Attribution-Non-Commercial)"),
  recording(2, "commons", "GFDL"),
  recording(3, "gbif", "http://creativecommons.org/licenses/by/3.0/au/"),
];

async function stubSearch(page: Page) {
  await page.route("https://example.org/audio/**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "audio/wav",
      headers: { "Access-Control-Allow-Origin": "*" },
      body: WAV,
    }),
  );
  await page.route("**/search/**", (route) => {
    const p = new URL(route.request().url()).pathname;
    const status = { gbif: "ready", commons: "ready" };
    if (p.endsWith("/search/facets")) {
      return json(route, {
        groups: [{ grp: "bats", n: 3, species: 1 }],
        countries: [{ cc: "GB", name: "United Kingdom", n: 3, species: 1 }],
        sources: [
          { source: "gbif", n: 2 },
          { source: "commons", n: 1 },
        ],
        status,
      });
    }
    if (p.endsWith("/search/species")) {
      return json(route, { total: 0, cursor: null, species: [], status, approx: false });
    }
    if (p.endsWith("/search/recordings")) {
      return json(route, {
        total: ITEMS.length,
        total_exact: true,
        by_source: { gbif: 2, commons: 1 },
        cursor: null,
        truncated: false,
        items: ITEMS,
        status,
      });
    }
    return json(route, { error: "not stubbed" });
  });
}

/** Opens Find recordings, lists all recordings and loads the row whose second line is
 *  `detail`. */
async function loadRow(page: Page, detail: string) {
  await stubSearch(page);
  await page.goto("/");
  await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
  await page.locator("button.add-files-btn", { hasText: "Find recordings" }).click();
  await page.locator(".find-fixed-row", { hasText: "All recordings" }).click();
  await expect(page.locator(".find-rec-detail")).toHaveText([
    "Recordist 1 · CC BY-NC",
    "Recordist 2 · GFDL",
    "Recordist 3 · CC BY 3.0 AU",
  ]);
  const row = page.locator(".find-rec-row", {
    has: page.locator(".find-rec-detail", { hasText: detail }),
  });
  await row.locator("button", { hasText: "Load" }).click();
  await expect(page.locator(".find-modal")).toHaveCount(0, { timeout: 30_000 });
}

test.describe("Licence badge and dialog", () => {
  test("GBIF's short name: same label in the list and the badge, no repeated row", async ({
    page,
  }) => {
    await loadRow(page, "CC BY-NC");
    const badge = page.locator(".toolbar-cc-badge");
    await expect(badge.locator(".toolbar-cc-label")).toHaveText("BY-NC");
    await expect(badge.locator(".toolbar-cc-icon")).toHaveCount(1);
    await badge.click();
    const dialog = page.locator(".cc-modal");
    await expect(dialog.locator(".modal-title")).toHaveText("CC BY-NC license");
    // No version, so no full name or link: the row would repeat the title.
    await expect(dialog.locator(".cc-license-link")).toHaveCount(0);
    await expect(dialog.locator(".cc-fields")).toContainText("Recordist");
    await expect(dialog.locator(".cc-fields")).toContainText("Recordist 1");
  });

  test("a ported CC licence keeps its port and links to its page", async ({ page }) => {
    await loadRow(page, "CC BY 3.0 AU");
    const badge = page.locator(".toolbar-cc-badge");
    await expect(badge.locator(".toolbar-cc-label")).toHaveText("BY 3.0 AU");
    await badge.click();
    const link = page.locator(".cc-modal .cc-license-link a");
    await expect(link).toHaveText("Creative Commons Attribution 3.0 AU");
    await expect(link).toHaveAttribute("href", "http://creativecommons.org/licenses/by/3.0/au/");
  });

  test("a licence that isn't Creative Commons gets the document icon", async ({ page }) => {
    await loadRow(page, "GFDL");
    const badge = page.locator(".toolbar-cc-badge");
    await expect(badge.locator(".toolbar-cc-label")).toHaveText("GFDL");
    await expect(badge.locator(".toolbar-cc-icon")).toHaveCount(0);
    await expect(badge.locator("svg.toolbar-license-icon")).toHaveCount(1);
    await badge.click();
    const dialog = page.locator(".cc-modal");
    await expect(dialog.locator(".modal-title")).toHaveText("License");
    await expect(dialog.locator(".modal-title svg.cc-modal-license-icon")).toHaveCount(1);
    await expect(dialog.locator(".cc-license-link")).toHaveText("GFDL");
  });
});
