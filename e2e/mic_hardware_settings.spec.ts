import { test, expect, type Page } from "@playwright/test";
import * as fs from "node:fs";
import * as path from "node:path";

/**
 * What the desktop app shows of a microphone's real hardware settings: the
 * depth line in Choose Microphone (src/components/file_sidebar/mic_chooser.rs)
 * and the rates in the mic menu's Max sample rate list
 * (MaxSampleRateSelect in src/components/bottom_toolbar.rs).
 *
 * The page runs with a fake `__TAURI_INTERNALS__` whose `mic_list_devices`
 * answers with the devices below, shaped like this Linux machine's (a Roland
 * DUO-CAPTURE EX at 48 kHz, 24-bit). Nothing here checks the native side
 * (src-tauri/src/recording.rs has its own tests).
 *
 * Set MIC_SHOT_DIR to save screenshots of each state there, and MIC_AUDIT to
 * a ui-audit.js script to run it beside each screenshot.
 */

const SHOT_DIR = process.env.MIC_SHOT_DIR;
const AUDIT = process.env.MIC_AUDIT;

async function shot(page: Page, name: string) {
  if (!SHOT_DIR) return;
  await page.screenshot({ path: path.join(SHOT_DIR, name) });
  if (AUDIT) {
    const report = await page.evaluate(fs.readFileSync(AUDIT, "utf8"));
    fs.writeFileSync(path.join(SHOT_DIR, name.replace(/\.png$/, ".audit.json")), JSON.stringify(report, null, 2));
  }
}

const range = (min: number, max: number, format: string) => ({ min, max, channels: 2, format });
const containers = (rate: number) => ["F32", "I16", "I24", "I32"].map((f) => range(rate, rate, f));

/** One row each: depth known, float depth known, depth unknown with several
 * real rates, and an older backend's row with neither field. */
const DEVICES = [
  {
    name: "Default ALSA Output (currently PipeWire Media Server)",
    is_default: true,
    sample_rate_ranges: containers(48000),
    hardware_rates: [48000],
    hardware_depth: { bits: 24, is_float: false },
    is_usb: true,
    manufacturer: "Roland",
    source_name: "DUO-CAPTURE EX",
  },
  {
    name: "DUO-CAPTURE EX",
    is_default: false,
    sample_rate_ranges: containers(48000),
    hardware_rates: [48000],
    hardware_depth: { bits: 24, is_float: false },
    is_usb: true,
    manufacturer: "Roland",
  },
  {
    name: "Float Interface",
    is_default: false,
    sample_rate_ranges: [...containers(96000), ...containers(192000)],
    hardware_rates: [96000, 192000],
    hardware_depth: { bits: 32, is_float: true },
  },
  {
    name: "HD-Audio Generic, ALCS1200A Analog",
    is_default: false,
    sample_rate_ranges: [44100, 48000, 96000, 192000].flatMap((r) => [range(r, r, "I16"), range(r, r, "I32")]),
    hardware_rates: [44100, 48000, 96000, 192000],
  },
  {
    name: "Old Backend Mic",
    is_default: false,
    sample_rate_ranges: [range(44100, 44100, "I16"), range(48000, 48000, "I16")],
  },
];

/** DEVICES with `name` as the system default instead. */
function withDefault(name: string) {
  return DEVICES.map((d) => ({ ...d, is_default: d.name === name }));
}

/** At phone width the file sidebar covers the page: close it. */
async function hideSidebar(page: Page) {
  if (page.viewportSize()!.width < 768) {
    const hide = page.getByTitle("Hide sidebar").first();
    if (await hide.isVisible()) await hide.click({ timeout: 5_000 }).catch(() => {});
    await expect(page.locator(".sidebar:not(.right-sidebar):not(.collapsed)")).toHaveCount(0);
  }
}

async function desktop(page: Page, devices: unknown[], hostName = "ALSA") {
  await page.addInitScript(
    ({ devices, hostName }) => {
      const w = window as any;
      w.__TAURI_INTERNALS__ = {
        transformCallback: () => 0,
        metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
        invoke: (cmd: string) => {
          switch (cmd) {
            case "google_status":
              return Promise.resolve({ available: false, signedIn: false, persistent: true });
            case "mic_list_devices":
              return Promise.resolve({ devices, host_name: hostName });
            case "mic_recover_recordings":
            case "list_central_annotations":
              return Promise.resolve([]);
            case "recording_folder_get":
              return Promise.resolve({
                path: "/home/ann/Music/Oversample",
                short_path: "~/Music/Oversample",
                default_path: "/home/ann/Music/Oversample",
                is_custom: false,
                unusable_custom: null,
                music_missing: false,
              });
            default:
              return Promise.reject(`not faked: ${cmd}`);
          }
        },
      };
    },
    { devices, hostName },
  );
  await page.goto("/");
  await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
  await hideSidebar(page);
}

function row(page: Page, name: string) {
  return page.locator(".mic-chooser-device", {
    has: page.locator(".mic-chooser-device-name", { hasText: new RegExp(`^${name}`) }),
  });
}

async function openChooser(page: Page) {
  await page.locator(".mic-combo-row button").first().click();
  await expect(page.locator(".xc-modal-title", { hasText: "Choose Microphone" })).toBeVisible();
  await expect(page.getByText("Loading devices")).toHaveCount(0);
}

async function choose(page: Page, name: string) {
  await openChooser(page);
  await row(page, name).click();
  await expect(page.locator(".xc-modal-title", { hasText: "Choose Microphone" })).toHaveCount(0);
}

function rateSelect(page: Page) {
  return page
    .locator(".layer-panel-slider-row", { has: page.locator("label", { hasText: "Max sample rate" }) })
    .locator("select");
}

async function openMicMenu(page: Page) {
  await page.getByTitle("Microphone options (strategy, capture format)").click();
  await expect(rateSelect(page)).toBeVisible();
}

async function optionLabels(page: Page) {
  return rateSelect(page).locator("option").allTextContents();
}

for (const width of [1440, 375]) {
  test.describe(`at ${width} px`, () => {
    test.beforeEach(async ({ page }) => {
      await page.setViewportSize({ width, height: width === 375 ? 812 : 900 });
    });

    test("Choose Microphone rows show the hardware depth when known", async ({ page }) => {
      await desktop(page, DEVICES);
      await openChooser(page);
      const caps = (name: string) => row(page, name).locator(".mic-chooser-device-caps").first();
      await expect(caps("DUO-CAPTURE EX")).toHaveText("48k • 24-bit");
      await expect(caps("Float Interface")).toHaveText("96k, 192k • 32-bit float");
      await expect(caps("HD-Audio Generic, ALCS1200A Analog")).toHaveText("44.1k, 48k, 96k, 192k • I16, I32");
      await expect(caps("Old Backend Mic")).toHaveText("44.1k, 48k • I16");
      await shot(page, `chooser-${width}.png`);
    });

    test("Max sample rate offers the chosen mic's rates", async ({ page }) => {
      await desktop(page, DEVICES);
      await choose(page, "HD-Audio Generic, ALCS1200A Analog");
      await openMicMenu(page);
      await expect.poll(() => optionLabels(page)).toEqual(["Auto", "44.1k", "48k", "96k", "192k"]);
      await expect(rateSelect(page)).toHaveValue("0");
      await rateSelect(page).selectOption("96000");
      await expect(rateSelect(page)).toHaveValue("96000");
      await shot(page, `rates-several-${width}.png`);
    });

    test("the system default's rates show before a mic is chosen", async ({ page }) => {
      await desktop(page, DEVICES);
      await openMicMenu(page);
      await expect.poll(() => optionLabels(page)).toEqual(["48k"]);
      await expect(rateSelect(page)).toBeDisabled();
    });

    test("a saved maximum the mic lacks shows the rate that will be used", async ({ page }) => {
      // The system default's rates are not known, so the list is the full one.
      await desktop(page, withDefault("Old Backend Mic"));
      await openMicMenu(page);
      await expect.poll(() => optionLabels(page)).toEqual(["Auto", "44.1k", "48k", "96k", "192k", "256k", "384k", "500k"]);
      await rateSelect(page).selectOption("384000");
      await shot(page, `rates-unknown-${width}.png`);
      await choose(page, "Float Interface");
      await openMicMenu(page);
      await expect.poll(() => optionLabels(page)).toEqual(["Auto", "96k", "192k"]);
      await expect(rateSelect(page)).toHaveValue("192000");
      await shot(page, `rates-saved-above-${width}.png`);
    });

    test("a mic with one rate shows it in a disabled list", async ({ page }) => {
      await desktop(page, DEVICES);
      await choose(page, "DUO-CAPTURE EX");
      await openMicMenu(page);
      await expect.poll(() => optionLabels(page)).toEqual(["48k"]);
      await expect(rateSelect(page)).toBeDisabled();
      await expect(rateSelect(page)).toHaveAttribute("title", "This microphone has only one sample rate: 48 kHz");
      await shot(page, `rates-one-${width}.png`);
    });

    test("a mic from an older backend keeps the full list", async ({ page }) => {
      await desktop(page, DEVICES);
      await choose(page, "Old Backend Mic");
      await openMicMenu(page);
      await expect.poll(() => optionLabels(page)).toEqual(["Auto", "44.1k", "48k", "96k", "192k", "256k", "384k", "500k"]);
    });
  });
}

test.describe("in the browser", () => {
  test("Max sample rate keeps the full list", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
    await openMicMenu(page);
    await expect.poll(() => optionLabels(page)).toEqual(["Auto", "44.1k", "48k", "96k", "192k", "256k", "384k", "500k"]);
  });
});
