import { test, expect, Page, Route } from "@playwright/test";

/**
 * Find recordings dialog against a stubbed `/search/*` server: lists that load their next
 * page as they scroll, sorting by column heading, the sample-rate filter, and an older
 * server that takes neither `sort` nor `min_smp`.
 */

type Seen = { species: URL[]; recordings: URL[]; facets: URL[] };

const SPECIES_TOTAL = 250;
const RECORDINGS_TOTAL = 120;

function json(route: Route, body: unknown, status = 200) {
  return route.fulfill({
    status,
    contentType: "application/json",
    headers: { "Access-Control-Allow-Origin": "*" },
    body: JSON.stringify(body),
  });
}

function species(i: number) {
  return {
    key: `b:Genus${i} sp${i}`,
    gen: `Genus${i}`,
    sp: `sp${i}`,
    rank: 0,
    grp: "bats",
    en: `Test Bat ${i}`,
    n: 1000 - i,
    sources: { xc: 1000 - i },
  };
}

function recording(i: number) {
  return {
    id: `xc:${100000 + i}`,
    source: "xc",
    gen: "Myotis",
    sp: "daubentonii",
    en: "Daubenton's Bat",
    cc: "GB",
    country: "United Kingdom",
    place: `Pond ${i}`,
    smp: 384000,
    len_s: 10 + i,
    date: "2024-06-01",
    recordist: "A Recordist",
    licence: "//creativecommons.org/licenses/by-nc-sa/4.0/",
    quality: "A",
    page_url: `https://xeno-canto.org/${100000 + i}`,
    audio_url: "",
    audio_via: "xc",
  };
}

/** A silent mono 16-bit WAV, for the preview's audio element. */
function silentWav(seconds: number, rate = 8000): Buffer {
  const bytes = seconds * rate * 2;
  const b = Buffer.alloc(44 + bytes);
  b.write("RIFF", 0);
  b.writeUInt32LE(36 + bytes, 4);
  b.write("WAVE", 8);
  b.write("fmt ", 12);
  b.writeUInt32LE(16, 16);
  b.writeUInt16LE(1, 20);
  b.writeUInt16LE(1, 22);
  b.writeUInt32LE(rate, 24);
  b.writeUInt32LE(rate * 2, 28);
  b.writeUInt16LE(2, 32);
  b.writeUInt16LE(16, 34);
  b.write("data", 36);
  b.writeUInt32LE(bytes, 40);
  return b;
}

/**
 * Serves species in pages of 100 and recordings in pages of 50 (cursors are offsets).
 * `modern` servers list `rates` in their facets; `failSecondRecordingsPage` fails the
 * first request for the second page of recordings.
 */
async function stubSearch(
  page: Page,
  opts: { modern: boolean; failSecondRecordingsPage?: boolean },
): Promise<Seen> {
  const seen: Seen = { species: [], recordings: [], facets: [] };
  let failed = false;
  await page.route("**/search/**", (route) => {
    const url = new URL(route.request().url());
    const path = url.pathname;
    if (!opts.modern && (url.searchParams.has("sort") || url.searchParams.has("min_smp"))) {
      return json(route, { error: "Unknown parameter" }, 400);
    }
    if (path.endsWith("/search/facets")) {
      seen.facets.push(url);
      return json(route, {
        groups: [{ grp: "bats", n: 5000, species: SPECIES_TOTAL }],
        countries: [{ cc: "GB", name: "United Kingdom", n: 5000, species: SPECIES_TOTAL }],
        sources: [{ source: "xc", n: 5000 }],
        ...(opts.modern
          ? {
              rates: [
                { min_smp: 96000, n: 4000, species: 200 },
                { min_smp: 192000, n: 3500, species: 180 },
              ],
            }
          : {}),
        status: { xc: "ready" },
      });
    }
    if (path.endsWith("/search/species")) {
      seen.species.push(url);
      const offset = Number(url.searchParams.get("cursor") ?? 0);
      const end = Math.min(offset + 100, SPECIES_TOTAL);
      const list = [];
      for (let i = offset; i < end; i++) list.push(species(i));
      return json(route, {
        total: SPECIES_TOTAL,
        cursor: end < SPECIES_TOTAL ? String(end) : null,
        species: list,
        status: { xc: "ready" },
        approx: false,
      });
    }
    if (path.endsWith("/search/recordings")) {
      seen.recordings.push(url);
      const offset = Number(url.searchParams.get("cursor") ?? 0);
      if (opts.failSecondRecordingsPage && offset === 50 && !failed) {
        failed = true;
        return json(route, { error: "The server is busy." }, 500);
      }
      const end = Math.min(offset + 50, RECORDINGS_TOTAL);
      const items = [];
      for (let i = offset; i < end; i++) items.push(recording(i));
      return json(route, {
        total: RECORDINGS_TOTAL,
        total_exact: true,
        by_source: { xc: RECORDINGS_TOTAL },
        cursor: end < RECORDINGS_TOTAL ? String(end) : null,
        truncated: false,
        items,
        status: { xc: "ready" },
        ...(url.searchParams.has("min_smp") ? { no_rate: { inat: 4 } } : {}),
      });
    }
    return json(route, { error: "not stubbed" }, 404);
  });
  return seen;
}

async function openDialog(page: Page) {
  await page.goto("/");
  await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
  const open = page.locator("button.add-files-btn", { hasText: "Find recordings" });
  await expect(open).toBeVisible({ timeout: 15_000 });
  await open.click();
  await expect(page.locator(".find-modal")).toBeVisible();
}

/** Scrolls a pane to its end. The scroll event is sent too, since a page in a hidden
 *  window gets none for a scroll made by script. */
async function scrollToEnd(page: Page, pane: string) {
  await page.locator(pane).evaluate((el) => {
    el.scrollTop = el.scrollHeight;
    el.dispatchEvent(new Event("scroll"));
  });
}

test.describe("Find recordings", () => {
  test("species list loads the next page as it scrolls, with a count at the end", async ({
    page,
  }) => {
    const seen = await stubSearch(page, { modern: true });
    await openDialog(page);
    const rows = page.locator(".find-species .find-species-row:not(.find-fixed-row)");
    const end = page.locator(".find-species .find-list-end");
    await expect(rows).toHaveCount(100);
    await expect(end).toContainText("100 of 250 species");

    await scrollToEnd(page, ".find-species");
    await expect(rows).toHaveCount(200);
    await expect(end).toContainText("200 of 250 species");
    await scrollToEnd(page, ".find-species");
    await expect(rows).toHaveCount(250);
    await expect(end).toHaveText("250 species");
    expect(seen.species.map((u) => u.searchParams.get("cursor"))).toEqual([null, "100", "200"]);
    await expect(page.locator(".find-more")).toHaveCount(0);
  });

  test("species sort by name, recordings sort by column heading", async ({ page }) => {
    const seen = await stubSearch(page, { modern: true });
    await openDialog(page);
    await expect(page.locator(".find-species-row:not(.find-fixed-row)")).toHaveCount(100);

    await page.locator(".find-list-head .find-sort", { hasText: "Species" }).click();
    await expect.poll(() => seen.species.at(-1)?.searchParams.get("sort")).toBe("name");
    // Sorted by scientific name, rows lead with it.
    await expect(page.locator(".find-species-row:not(.find-fixed-row) .find-sp-name i").first()).toHaveText(
      "Genus0 sp0",
    );

    await page.locator(".find-fixed-row", { hasText: "All recordings" }).click();
    await expect(page.locator(".find-rec-row")).toHaveCount(50);
    expect(seen.recordings.at(-1)?.searchParams.has("sort")).toBe(false);
    const quality = page.locator(".find-rec-head .find-sort", { hasText: "Quality" });
    await expect(quality).toHaveClass(/active/);

    const date = page.locator(".find-rec-head .find-sort", { hasText: "Date" });
    await date.click();
    await expect.poll(() => seen.recordings.at(-1)?.searchParams.get("sort")).toBe("newest");
    await expect(date).toHaveClass(/active/);
    await date.click();
    await expect.poll(() => seen.recordings.at(-1)?.searchParams.get("sort")).toBe("oldest");

    // The next page keeps the order.
    await expect(page.locator(".find-rec-row")).toHaveCount(50);
    await scrollToEnd(page, ".find-detail");
    await expect(page.locator(".find-rec-row")).toHaveCount(100);
    const last = seen.recordings.at(-1)!;
    expect(last.searchParams.get("cursor")).toBe("50");
    expect(last.searchParams.get("sort")).toBe("oldest");
  });

  test("sample-rate filter is sent with every request and notes what it left out", async ({
    page,
  }) => {
    const seen = await stubSearch(page, { modern: true });
    await openDialog(page);
    const rate = page.locator("label", { hasText: "Sample rate" }).locator("select");
    await expect(rate).toBeVisible();
    await expect(rate.locator("option")).toHaveText([
      "Any",
      "96 kHz or more (4,000)",
      "192 kHz or more (3,500)",
    ]);
    await page.locator(".find-fixed-row", { hasText: "All recordings" }).click();
    await expect(page.locator(".find-rec-row")).toHaveCount(50);

    await rate.selectOption("96000");
    await expect.poll(() => seen.recordings.at(-1)?.searchParams.get("min_smp")).toBe("96000");
    await expect.poll(() => seen.species.at(-1)?.searchParams.get("min_smp")).toBe("96000");
    await expect.poll(() => seen.facets.at(-1)?.searchParams.get("min_smp")).toBe("96000");
    await expect(page.locator(".find-detail-head")).toContainText("at 96 kHz or more");
    await expect(page.locator(".find-detail-head")).toContainText(
      "Left out: 4 recordings with an unknown sample rate (iNaturalist).",
    );
  });

  test("a failed page offers Try again", async ({ page }) => {
    await stubSearch(page, { modern: true, failSecondRecordingsPage: true });
    await openDialog(page);
    await page.locator(".find-fixed-row", { hasText: "All recordings" }).click();
    await expect(page.locator(".find-rec-row")).toHaveCount(50);
    await scrollToEnd(page, ".find-detail");
    const end = page.locator(".find-detail .find-list-end");
    await expect(end).toContainText("Couldn't load more");
    await expect(end).toContainText("50 of 120 recordings");
    await end.locator("button", { hasText: "Try again" }).click();
    await expect(page.locator(".find-rec-row")).toHaveCount(100);
    await expect(end).not.toContainText("Couldn't load more");
  });

  test("near me: refused location lists recordings again", async ({ page }) => {
    await stubSearch(page, { modern: true });
    await openDialog(page);
    await page.locator(".find-fixed-row", { hasText: "All recordings" }).click();
    await expect(page.locator(".find-rec-row")).toHaveCount(50);
    await page.locator("label", { hasText: "Near" }).locator("select").selectOption("me");
    await expect(page.locator(".xc-error")).toContainText("Location access denied");
    await expect(page.locator(".find-rec-row")).toHaveCount(50);
    await expect(page.locator(".find-detail .xc-loading")).toHaveCount(0);
  });

  test("near me: distance sorts first, other sorts keep the point", async ({ page, context }) => {
    await context.grantPermissions(["geolocation"]);
    await context.setGeolocation({ latitude: 51.5, longitude: -0.12 });
    const seen = await stubSearch(page, { modern: true });
    await openDialog(page);
    await page.locator(".find-fixed-row", { hasText: "All recordings" }).click();
    await expect(page.locator(".find-rec-row")).toHaveCount(50);
    await page.locator("label", { hasText: "Near" }).locator("select").selectOption("me");
    await expect.poll(() => seen.recordings.at(-1)?.searchParams.get("near")).toBe("51.5,-0.1");
    expect(seen.recordings.at(-1)?.searchParams.has("sort")).toBe(false);
    const distance = page.locator(".find-rec-head .find-sort", { hasText: "Distance" });
    await expect(distance).toHaveClass(/active/);
    await page.locator(".find-rec-head .find-sort", { hasText: "Quality" }).click();
    await expect.poll(() => seen.recordings.at(-1)?.searchParams.get("sort")).toBe("best");
    expect(seen.recordings.at(-1)?.searchParams.get("near")).toBe("51.5,-0.1");
    await distance.click();
    await expect.poll(() => seen.recordings.at(-1)?.searchParams.has("sort")).toBe(false);
  });

  test("an older server gets no sort or sample-rate parameters", async ({ page }) => {
    const seen = await stubSearch(page, { modern: false });
    await openDialog(page);
    await expect(page.locator(".find-species-row:not(.find-fixed-row)")).toHaveCount(100);
    await page.locator(".find-fixed-row", { hasText: "All recordings" }).click();
    await expect(page.locator(".find-rec-row")).toHaveCount(50);
    await expect(page.locator("label", { hasText: "Sample rate" })).toHaveCount(0);
    await expect(page.locator(".find-sort")).toHaveCount(0);
    await scrollToEnd(page, ".find-detail");
    await expect(page.locator(".find-rec-row")).toHaveCount(100);
    for (const u of [...seen.species, ...seen.recordings, ...seen.facets]) {
      expect(u.searchParams.has("sort")).toBe(false);
      expect(u.searchParams.has("min_smp")).toBe(false);
    }
    await expect(page.locator(".xc-error")).toHaveCount(0);
  });

  test("no recordings under a filter: Clear filters lists them again", async ({ page }) => {
    await stubSearch(page, { modern: true });
    // Registered after stubSearch, so it answers first: nothing at a set sample rate.
    await page.route("**/search/recordings**", (route) => {
      if (!new URL(route.request().url()).searchParams.has("min_smp")) return route.fallback();
      return json(route, {
        total: 0,
        total_exact: true,
        by_source: {},
        cursor: null,
        truncated: false,
        items: [],
        status: { xc: "ready" },
      });
    });
    await openDialog(page);
    const rate = page.locator("label", { hasText: "Sample rate" }).locator("select");
    await page.locator(".find-fixed-row", { hasText: "All recordings" }).click();
    await expect(page.locator(".find-rec-row")).toHaveCount(50);
    await rate.selectOption("96000");
    await expect(page.locator(".find-empty")).toContainText("No recordings match the filters above.");
    await page.locator(".find-empty button", { hasText: "Clear filters" }).click();
    await expect(rate).toHaveValue("");
    await expect(page.locator(".find-rec-row")).toHaveCount(50);
    await expect(page.locator(".find-empty")).toHaveCount(0);
  });

  test("preview plays audible recordings and leaves out ultrasonic ones", async ({ page }) => {
    await stubSearch(page, { modern: true });
    const items = [
      { ...recording(0), id: "xc:200001", smp: 48000 },
      { ...recording(1), id: "xc:200002", smp: 384000 },
      {
        ...recording(2),
        id: "inat:5/0",
        source: "inat",
        smp: null,
        audio_via: "proxy",
        audio_url: "https://static.inaturalist.org/sounds/5.m4a",
      },
    ];
    // Registered after stubSearch, so it answers first.
    await page.route("**/search/recordings**", (route) =>
      json(route, {
        total: items.length,
        total_exact: true,
        by_source: { xc: 2, inat: 1 },
        cursor: null,
        truncated: false,
        items,
        status: { xc: "ready" },
      }),
    );
    const audio: string[] = [];
    await page.route("**/xc/audio/**", (route) => {
      audio.push(route.request().url());
      return route.fulfill({
        status: 200,
        contentType: "audio/wav",
        headers: { "Access-Control-Allow-Origin": "*" },
        body: silentWav(5),
      });
    });
    // The relay's "/search/audio/inat%3A5%2F0" falls through to stubSearch's 404.

    await openDialog(page);
    await page.locator(".find-fixed-row", { hasText: "All recordings" }).click();
    const rows = page.locator(".find-rec-row");
    await expect(rows).toHaveCount(3);
    // 384 kHz: no preview button, Load only.
    await expect(rows.nth(1).locator(".find-rec-preview")).toHaveCount(0);
    await expect(rows.nth(1).locator(".find-rec-load")).toBeVisible();

    const play = rows.nth(0).locator(".find-rec-preview");
    await expect(play).toHaveAttribute("aria-label", "Play preview");
    await play.click();
    await expect(play).toHaveClass(/playing/);
    await expect(play).toHaveAttribute("aria-label", "Stop preview");
    expect(audio[0]).toMatch(/\/xc\/audio\/200001$/);
    await play.click();
    await expect(play).toHaveAttribute("aria-label", "Play preview");
    await expect(play).not.toHaveClass(/playing/);

    // Starting another preview stops the first; this one's file is missing.
    await play.click();
    await expect(play).toHaveClass(/playing/);
    const missing = rows.nth(2).locator(".find-rec-preview");
    await missing.click();
    await expect(play).toHaveAttribute("aria-label", "Play preview");
    await expect(missing).toHaveClass(/failed/);
    await expect(page.locator(".xc-error")).toContainText("Could not play preview: Daubenton's Bat");

    // Closing the dialog removes the player.
    await play.click();
    await expect(play).toHaveClass(/playing/);
    await page.locator(".find-modal .xc-modal-close").click();
    await expect(page.locator("audio")).toHaveCount(0);
  });
});
