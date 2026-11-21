import { test, expect, type Page } from "@playwright/test";

/**
 * Australian Acoustic Observatory links. Both A2O servers are mocked: no
 * request leaves the machine. An event link opens the event's clip and makes
 * a project of it; "Add after" and "Add before" fetch the audio beside it and
 * put it on one timeline, in recording order.
 */

const SEARCH = "https://api.search.acousticobservatory.org/api/v1/a2o";
const MAIN = "https://api.acousticobservatory.org";
const EVENT_LINK = "https://data.acousticobservatory.org/library/55743/audio_events/30169";
const STEM = "20200331T190000+1100_Tarcutta-Hills-Wet-A_55743";
const SR = 22_050;

type Snap = {
  isPlaying: boolean;
  currentFile: string | null;
  timelineFiles: string[] | null;
  timelineStarts: number[] | null;
  timelineDuration: number | null;
  timelineStreamFiles: string[] | null;
};

/** The app's test snapshot. Empty until the app has started. */
const snapshot = (page: Page): Promise<Snap> =>
  page.evaluate(() => {
    const hook = (window as unknown as { __oversample_test?: () => Snap }).__oversample_test;
    return hook ? hook() : ({} as Snap);
  });

/** Mono 16-bit WAV with a steady tone. The app reads the format from the bytes. */
function toneWav(secs: number, toneHz: number): Buffer {
  const n = Math.round(SR * secs);
  const data = Buffer.alloc(n * 2);
  for (let i = 0; i < n; i++) {
    const s = 0.3 * Math.sin((2 * Math.PI * toneHz * i) / SR) + (((i * 7919) % 101) / 101 - 0.5) * 0.01;
    data.writeInt16LE(Math.round(s * 32767), i * 2);
  }
  const header = Buffer.alloc(44);
  header.write("RIFF", 0);
  header.writeUInt32LE(36 + data.length, 4);
  header.write("WAVEfmt ", 8);
  header.writeUInt32LE(16, 16);
  header.writeUInt16LE(1, 20);
  header.writeUInt16LE(1, 22);
  header.writeUInt32LE(SR, 24);
  header.writeUInt32LE(SR * 2, 28);
  header.writeUInt16LE(2, 32);
  header.writeUInt16LE(16, 34);
  header.write("data", 36);
  header.writeUInt32LE(data.length, 40);
  return Buffer.concat([header, data]);
}

const OPEN = { "access-control-allow-origin": "*" };
const json = (data: unknown) => ({
  status: 200,
  headers: { ...OPEN, "content-type": "application/json" },
  body: JSON.stringify({ meta: { status: 200, message: "OK" }, data }),
});

/**
 * Mock A2O. Returns the list of clip requests made, as "start-end". With
 * `eventsOpen` false, requests to the main API fail, which is what a page
 * sees when the browser blocks the reply: A2O's main API does not let
 * app.oversample.com read it. (A mocked reply cannot leave the header out:
 * Playwright adds it.)
 */
async function mockA2o(
  page: Page,
  eventsOpen = true,
  relayAsked: string[] | null = null,
): Promise<string[]> {
  const clips: string[] = [];
  // The Oversample server's /a2o route, which passes event and tag lookups
  // on to A2O. Down unless `relayAsked` is given; then it answers, and the
  // paths asked are added to that list.
  await page.route("https://api.oversample.com/a2o/**", async (route) => {
    if (!relayAsked) return route.abort("failed");
    const path = new URL(route.request().url()).pathname.replace("/a2o", "");
    relayAsked.push(path);
    if (path === "/audio_recordings/55743/audio_events/30169") {
      return route.fulfill(
        json({
          id: 30169,
          audio_recording_id: 55743,
          start_time_seconds: 1506.0,
          end_time_seconds: 1516.0,
          taggings: [{ id: 1, tag_id: 105 }],
        }),
      );
    }
    if (path === "/tags/105") return route.fulfill(json({ id: 105, text: "frogs" }));
    return route.fulfill({ status: 404, headers: OPEN, body: '{"error":"A2O has no such thing."}' });
  });
  await page.route(`${SEARCH}/**`, async (route) => {
    const url = new URL(route.request().url());
    const path = url.pathname.replace("/api/v1/a2o", "");
    if (path === "/audio_recordings/55743") {
      return route.fulfill(
        json({
          id: 55743,
          recorded_date: "2020-03-31T08:00:00.000Z",
          site_id: 58,
          duration_seconds: 14389.685,
          sample_rate_hertz: SR,
          canonical_file_name: `${STEM}.flac`,
          recorded_date_timezone: "Australia/Sydney",
        }),
      );
    }
    if (path === "/sites/58") return route.fulfill(json({ id: 58, name: "Tarcutta Hills Wet A" }));
    if (path === "/audio_recordings/download/flac/55743") {
      const start = Number(url.searchParams.get("start_offset"));
      const end = Number(url.searchParams.get("end_offset"));
      clips.push(`${start}-${end}`);
      // A different tone for each clip, by where it starts. Every reply is
      // different bytes, also for a clip asked for a second time: the app
      // must know a clip by where it came from, not by its bytes.
      return route.fulfill({
        status: 200,
        headers: { ...OPEN, "content-type": "audio/x-flac" },
        body: toneWav(end - start, 1000 + (start % 100) * 50 + clips.length),
      });
    }
    return route.fulfill({ status: 404, headers: OPEN, body: "Not Found" });
  });
  await page.route(`${MAIN}/**`, async (route) => {
    if (!eventsOpen) return route.abort("failed");
    const path = new URL(route.request().url()).pathname;
    const reply = (data: unknown) => route.fulfill(json(data));
    if (path === "/audio_recordings/55743/audio_events/30169") {
      return reply({
        id: 30169,
        audio_recording_id: 55743,
        start_time_seconds: 1506.0,
        end_time_seconds: 1516.0,
        taggings: [{ id: 1, tag_id: 105 }, { id: 2, tag_id: 114 }],
      });
    }
    if (path === "/tags/105") return reply({ id: 105, text: "frogs" });
    if (path === "/tags/114") return reply({ id: 114, text: "birds" });
    return route.fulfill({ status: 404, headers: OPEN, body: "Not Found" });
  });
  return clips;
}

const clipName = (start: number, end: number) => `${start}s-${end}s_${STEM}.flac`;

test.describe("A2O links", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("an event link opens its clip, and audio before and after joins it on a timeline", async ({
    page,
  }) => {
    test.setTimeout(240_000);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    const clips = await mockA2o(page);
    await page.goto(`/#a2o:${EVENT_LINK}`);

    // The event runs from 1506 s to 1516 s of its recording.
    await expect
      .poll(async () => (await snapshot(page)).currentFile, { timeout: 60_000 })
      .toBe(clipName(1506, 1516));
    expect(clips).toEqual(["1506-1516"]);

    const after = page.getByRole("button", { name: "Add after" });
    const before = page.getByRole("button", { name: "Add before" });
    await expect(after).toBeVisible();
    await expect(before).toBeVisible();

    // The 30 s after the clip goes after it on a timeline.
    await after.click();
    await expect
      .poll(async () => (await snapshot(page)).timelineFiles, { timeout: 60_000 })
      .toEqual([clipName(1506, 1516), clipName(1516, 1546)]);
    expect(clips).toEqual(["1506-1516", "1516-1546"]);
    let snap = await snapshot(page);
    expect(snap.timelineStarts).toEqual([0, 10]);
    expect(snap.timelineDuration).toBeCloseTo(40, 2);

    // The 30 s before the clip goes in front.
    await before.click();
    await expect
      .poll(async () => (await snapshot(page)).timelineFiles, { timeout: 60_000 })
      .toEqual([clipName(1476, 1506), clipName(1506, 1516), clipName(1516, 1546)]);
    expect(clips).toEqual(["1506-1516", "1516-1546", "1476-1506"]);
    snap = await snapshot(page);
    expect(snap.timelineStarts).toEqual([0, 30, 40]);
    expect(snap.timelineDuration).toBeCloseTo(70, 2);

    // A second "after" carries on from the end of what is open.
    await after.click();
    await expect.poll(() => clips.length, { timeout: 60_000 }).toBe(4);
    expect(clips[3]).toBe("1546-1576");
    await expect
      .poll(async () => (await snapshot(page)).timelineStarts, { timeout: 60_000 })
      .toEqual([0, 30, 40, 70]);

    // The files can still be placed one after another, with a pause, and play.
    await page
      .locator(".layer-btn", { has: page.locator(".layer-btn-category", { hasText: /^Order$/ }) })
      .click();
    await page
      .locator(".layer-panel")
      .getByRole("button", { name: "One after another, 1 s pause", exact: true })
      .click();
    await expect
      .poll(async () => (await snapshot(page)).timelineStarts)
      .toEqual([0, 31, 42, 73]);
    await page.keyboard.press("Space");
    await expect.poll(async () => (await snapshot(page)).isPlaying).toBe(true);
    expect((await snapshot(page)).timelineStreamFiles).toEqual([
      clipName(1476, 1506),
      clipName(1506, 1516),
      clipName(1516, 1546),
      clipName(1546, 1576),
    ]);
    await page.keyboard.press("Space");

    // The length menu between the buttons sets how much one press adds, and
    // the choice is kept for the next visit.
    const length = page.getByLabel("How much audio to add");
    await expect(length).toHaveValue("30");
    await length.selectOption({ label: "1\u00a0min" });
    await after.click();
    await expect.poll(() => clips.length, { timeout: 60_000 }).toBe(5);
    expect(clips[4]).toBe("1576-1636");
    await expect
      .poll(async () => (await snapshot(page)).timelineStarts, { timeout: 60_000 })
      .toEqual([0, 31, 42, 73, 104]);
    expect(await page.evaluate(() => localStorage.getItem("oversample_a2o_add_secs"))).toBe("60");
    if (process.env.A2O_OUT) await page.screenshot({ path: `${process.env.A2O_OUT}/a2o-timeline.png` });
    expect(errors).toEqual([]);
  });

  test("a saved project fetches its A2O clips again after a reload", async ({ page }) => {
    test.setTimeout(240_000);
    const clips = await mockA2o(page);
    await page.goto(`/#a2o:${EVENT_LINK}`);
    await expect
      .poll(async () => (await snapshot(page)).currentFile, { timeout: 60_000 })
      .toBe(clipName(1506, 1516));

    // The link made a project of the clip, named after the site. Add the
    // 30 s after it: the new clip joins the project, and the project gets
    // the timeline.
    await page.getByRole("button", { name: /^Project/ }).first().click();
    await expect(page.locator(".project-file-item")).toHaveCount(1, { timeout: 30_000 });
    await expect(page.locator(".project-name-input")).toHaveValue("A2O Tarcutta Hills Wet A");
    await page.getByRole("button", { name: "Add after" }).click();
    await expect
      .poll(async () => (await snapshot(page)).timelineStarts, { timeout: 60_000 })
      .toEqual([0, 10]);
    await expect(page.locator(".project-file-item")).toHaveCount(2, { timeout: 30_000 });
    await expect(page.locator(".project-timeline-item")).toHaveCount(1);
    const save = page.getByRole("button", { name: "Save", exact: true });
    if (await save.isEnabled()) await save.click();
    await expect(save).toBeDisabled({ timeout: 20_000 });
    await page.waitForTimeout(1500);

    // After a reload nothing is open. The project says which clips are
    // missing and gets them from A2O with one press.
    await page.goto("/");
    await page.reload();
    await page.getByRole("button", { name: /^Project/ }).first().click();
    await page.getByRole("button", { name: "Load project" }).click();
    await page.locator(".project-list-item").first().click();
    await expect(page.locator(".project-file-item.missing")).toHaveCount(2);
    await expect(page.locator(".project-new-files-banner")).toContainText(
      "2 A2O clips are not open",
    );
    clips.length = 0;
    await page.getByRole("button", { name: "Get from A2O" }).click();
    await expect(page.locator(".project-file-item.missing")).toHaveCount(0, { timeout: 60_000 });
    expect([...clips].sort()).toEqual(["1506-1516", "1516-1546"]);
    // The fetched clips are the project's two files: none is listed as new,
    // and none is added a second time.
    await expect(page.locator(".project-new-files-banner")).toHaveCount(0);
    await expect(page.locator(".project-file-item")).toHaveCount(2);

    // The saved timeline opens with both clips in place.
    await page.locator(".project-timeline-item").click();
    await expect
      .poll(async () => (await snapshot(page)).timelineFiles)
      .toEqual([clipName(1506, 1516), clipName(1516, 1546)]);
    expect((await snapshot(page)).timelineStarts).toEqual([0, 10]);

    // Adding more audio keeps the project at one entry per clip, and the
    // saved timeline lists all three.
    await page.getByRole("button", { name: "Add before" }).click();
    await expect
      .poll(async () => (await snapshot(page)).timelineStarts, { timeout: 60_000 })
      .toEqual([0, 30, 40]);
    await expect(page.locator(".project-file-item")).toHaveCount(3, { timeout: 30_000 });
    await expect(page.locator(".project-timeline-item")).toHaveCount(1);
    await expect(page.locator(".project-timeline-item .project-file-info")).toContainText("3 files");
    await expect(page.locator(".project-new-files-banner")).toHaveCount(0);
  });

  test("a download link with a start and end opens without the main API", async ({ page }) => {
    test.setTimeout(120_000);
    const clips = await mockA2o(page, false);
    await page.goto("/");
    await page.getByRole("button", { name: "Find recordings" }).first().click();
    await page
      .getByPlaceholder(/A2O link/)
      .fill(`${SEARCH}/audio_recordings/download/flac/55743?start_offset=4035&end_offset=4040`);
    await page.getByRole("button", { name: "Load A2O clip" }).click();
    await expect
      .poll(async () => (await snapshot(page)).currentFile, { timeout: 60_000 })
      .toBe(clipName(4035, 4040));
    expect(clips).toEqual(["4035-4040"]);

    // The same link again opens nothing new: the clip is already open.
    await page.getByRole("button", { name: "Find recordings" }).first().click();
    await page
      .getByPlaceholder(/A2O link/)
      .fill(`${SEARCH}/audio_recordings/download/flac/55743?start_offset=4035&end_offset=4040`);
    await page.getByRole("button", { name: "Load A2O clip" }).click();
    await expect(page.getByPlaceholder(/A2O link/)).toHaveCount(0, { timeout: 30_000 });
    expect(clips).toEqual(["4035-4040"]);
    await expect(page.locator(".file-item")).toHaveCount(1);

    // Phone width: both Add buttons are whole and inside the screen.
    await page.setViewportSize({ width: 375, height: 812 });
    await page.waitForTimeout(1500);
    for (const name of ["Add before", "Add after"]) {
      const box = (await page.getByRole("button", { name }).boundingBox())!;
      expect(box.x).toBeGreaterThanOrEqual(0);
      expect(box.x + box.width).toBeLessThanOrEqual(375);
      expect(box.height).toBeGreaterThanOrEqual(40);
    }
    if (process.env.A2O_OUT) await page.screenshot({ path: `${process.env.A2O_OUT}/a2o-phone.png` });
  });

  test("an event link opens through the Oversample server when A2O's main API does not answer", async ({
    page,
  }) => {
    test.setTimeout(120_000);
    const relayAsked: string[] = [];
    const clips = await mockA2o(page, false, relayAsked);
    await page.goto(`/#${EVENT_LINK}`);
    await expect
      .poll(async () => (await snapshot(page)).currentFile, { timeout: 60_000 })
      .toBe(clipName(1506, 1516));
    expect(clips).toEqual(["1506-1516"]);
    expect(relayAsked).toEqual(["/audio_recordings/55743/audio_events/30169", "/tags/105"]);
  });

  test("an event link that reaches neither A2O's main API nor the Oversample server opens nothing and says so", async ({
    page,
  }) => {
    test.setTimeout(120_000);
    const clips = await mockA2o(page, false);
    await page.goto(`/#${EVENT_LINK}`);
    await expect(page.getByText("Could not reach A2O for event 30169")).toBeVisible({
      timeout: 60_000,
    });
    expect(clips).toEqual([]);
    expect((await snapshot(page)).currentFile ?? null).toBeNull();
  });
});
