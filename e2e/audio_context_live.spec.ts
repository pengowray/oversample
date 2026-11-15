import { test, expect } from "@playwright/test";
import path from "path";

/**
 * Live listening when the audio context made at the mic's sample rate never
 * starts (see audio_context.spec.ts). The app used to wait for that context
 * for ever, so Listen did nothing. It now uses a context at the browser's
 * default rate when that one starts.
 *
 * Chromium's fake mic (the chirp from global-setup) is the mic. `AudioContext`
 * is replaced so a context made at a named sample rate reports "suspended"
 * for ever when `stall` is set.
 */
const wavPath = path.resolve(process.cwd(), "e2e", "fixtures", "fake-audio.wav");

test.use({
  launchOptions: {
    args: [
      "--use-fake-device-for-media-stream",
      "--use-fake-ui-for-media-stream",
      `--use-file-for-fake-audio-capture=${wavPath}`,
    ],
  },
});

type Made = { asked: number | null; rate: number };

function watchContexts(stall: boolean) {
  const made: Made[] = [];
  (window as unknown as { __contextsMade: Made[] }).__contextsMade = made;
  const Real = window.AudioContext;
  window.AudioContext = new Proxy(Real, {
    construct(target, args: [AudioContextOptions?]) {
      const ctx = new target(...args);
      const asked = args[0]?.sampleRate ?? null;
      made.push({ asked, rate: ctx.sampleRate });
      if (asked && stall) {
        Object.defineProperty(ctx, "state", { get: () => "suspended" });
        ctx.resume = () => new Promise<void>(() => {});
      }
      return ctx;
    },
  });
}

type Snap = { listening: boolean; liveActive: boolean; liveTotalTime: number };

for (const stall of [false, true]) {
  test(`Listen starts when the mic-rate context ${stall ? "never starts" : "starts"}`, async ({
    page,
  }) => {
    const warnings: string[] = [];
    page.on("console", (m) => {
      if (m.type() === "warning") warnings.push(m.text());
    });
    await page.addInitScript(watchContexts, stall);
    await page.goto("/");
    await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
    await page.waitForFunction(
      () => typeof (window as { __oversample_test?: unknown }).__oversample_test === "function",
      undefined,
      { timeout: 15_000 },
    );
    await page.locator('button[title="Toggle live listening (L)"]').first().click();
    // The live display fills only once the mic's context runs.
    await page.waitForFunction(
      () => {
        const s = (window as unknown as { __oversample_test: () => Snap }).__oversample_test();
        return s.listening && s.liveActive && s.liveTotalTime > 0.05;
      },
      undefined,
      { timeout: 25_000 },
    );

    const made = await page.evaluate(
      () => (window as unknown as { __contextsMade: Made[] }).__contextsMade,
    );
    const rated = made.filter((m) => m.asked !== null);
    expect(rated.length, "a context was asked for at the mic's rate").toBeGreaterThan(0);
    const stalled = warnings.filter((w) => w.includes("did not start"));
    if (stall) {
      expect(stalled.length, "the console says the context did not start").toBe(1);
    } else {
      expect(stalled).toEqual([]);
    }
  });
}
