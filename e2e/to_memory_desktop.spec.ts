import { test, expect, type Page } from "@playwright/test";

/**
 * A "To memory" recording in the desktop app: it is kept in memory, shown as
 * unsaved, and written only when Save is pressed (src/audio/live_recording.rs
 * `save_unsaved_recording`, src/components/toolbar.rs).
 *
 * The page runs with a fake `__TAURI_INTERNALS__` that plays the cpal mic: a
 * 4 kHz tone at 48 kHz, 16-bit mono. Its `mic_stop_recording` answers as the
 * native side does for "To memory" (no saved path, the samples stashed, the
 * GUANO mic fields). Nothing here runs the native side (src-tauri/src/recording.rs
 * has its own tests).
 */

const RATE = 48_000;
const SAVED = "/home/ann/Music/Oversample/batcap_test.wav";

async function desktopWithMic(page: Page) {
  await page.addInitScript(
    ({ rate, saved }) => {
      const w = window as any;
      w.__calls = [] as { cmd: string; args: any }[];
      w.__written = [] as number[];
      let phase = 0;
      const tone = (frames: number) => {
        const out = new Float32Array(frames);
        for (let i = 0; i < frames; i++) {
          out[i] = Math.round(Math.sin(phase) * 8000) / 32768;
          phase += (2 * Math.PI * 4000) / rate;
        }
        return out;
      };
      let recorded: number[] = [];
      let recording = false;
      w.__TAURI_INTERNALS__ = {
        transformCallback: () => 0,
        metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
        invoke: (cmd: string, args: any, options: any) => {
          w.__calls.push({ cmd, args: args instanceof Uint8Array ? `${args.length} bytes` : args, headers: options?.headers });
          switch (cmd) {
            case "google_status":
              return Promise.resolve({ available: false, signedIn: false, persistent: true });
            case "mic_recover_recordings":
            case "list_central_annotations":
              return Promise.resolve([]);
            case "read_folder_settings":
              return Promise.resolve({ annotations_beside: false });
            case "recording_folder_get":
              return Promise.resolve({
                path: "/home/ann/Music/Oversample",
                short_path: "~/Music/Oversample",
                default_path: "/home/ann/Music/Oversample",
                is_custom: false,
                unusable_custom: null,
                music_missing: false,
              });
            case "mic_list_devices":
              return Promise.resolve({
                devices: [
                  {
                    name: "DUO-CAPTURE EX",
                    is_default: true,
                    sample_rate_ranges: [{ min: rate, max: rate, channels: 1, format: "I16" }],
                    is_usb: true,
                    manufacturer: "Roland",
                    source_name: null,
                  },
                ],
                host_name: "ALSA",
              });
            case "mic_open":
              return Promise.resolve({
                device_name: "DUO-CAPTURE EX",
                sample_rate: rate,
                bits_per_sample: 16,
                is_float: false,
                format: "I16",
                supported_sample_rates: [rate],
                host_name: "ALSA",
                channels: 1,
                device_channels: 1,
                is_usb: true,
                manufacturer: "Roland",
              });
            case "mic_pull_audio": {
              const chunk = tone(rate / 10);
              if (recording) recorded.push(...chunk);
              return Promise.resolve(chunk.buffer);
            }
            case "mic_get_status":
              return Promise.resolve({
                is_open: true,
                is_recording: recording,
                is_streaming: false,
                samples_recorded: recorded.length,
                sample_rate: rate,
                effective_bits: null,
              });
            case "mic_start_recording":
              recording = true;
              recorded = [];
              return Promise.resolve(null);
            case "mic_stop_recording":
              recording = false;
              return Promise.resolve({
                filename: "",
                saved_path: "",
                sample_rate: rate,
                bits_per_sample: 16,
                is_float: false,
                duration_secs: recorded.length / rate,
                num_samples: recorded.length,
                channels: 1,
                has_memory_samples: recorded.length > 0,
                file_size_bytes: 0,
                mic: { interface: "USB", name: "DUO-CAPTURE EX", make: "Roland" },
                preroll_frames: 0,
              });
            case "mic_take_recorded_samples":
              return Promise.resolve(Float32Array.from(recorded).buffer);
            case "save_open_recording":
              if (w.__failSave) return Promise.reject(`Couldn't save ${args.filename}: ${saved}: Permission denied`);
              return Promise.resolve({ id: 7, path: saved });
            case "save_write":
              for (const b of args as Uint8Array) w.__written.push(b);
              return Promise.resolve(null);
            case "save_close":
              return Promise.resolve(null);
            default:
              return Promise.resolve(null);
          }
        },
      };
    },
    { rate: RATE, saved: SAVED },
  );
  await page.goto("/");
  await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
}

/** Record about a second "To memory" and stop. */
async function recordToMemory(page: Page) {
  await page.getByTitle("Record settings").first().click();
  await page.getByRole("button", { name: "To memory", exact: true }).click();
  await page.keyboard.press("Escape");
  const record = page.getByTitle("Record", { exact: true }).first();
  await record.click();
  // The first press asks which mic to use, opens it, and asks to record.
  await page.getByText("System default", { exact: true }).click();
  await expect(page.getByText("Ready to record")).toBeVisible();
  await page.locator('button:text-is("Record")').click();
  await expect
    .poll(async () => (await calls(page)).find((c) => c.cmd === "mic_start_recording")?.args, { timeout: 10_000 })
    .toMatchObject({ toMemory: true, enableRecovery: false });
  await page.waitForTimeout(1000);
  // While recording, Save is not offered.
  await expect(page.locator(".badge-btn.unsaved")).toHaveCount(0);
  await record.click();
  await expect(page.locator(".badge-btn.unsaved")).toBeVisible({ timeout: 15_000 });
}

const calls = (page: Page) => page.evaluate(() => (window as any).__calls as { cmd: string; args: any }[]);

test.use({ viewport: { width: 1440, height: 900 } });

test("a To memory recording writes no file until Save, then saves with the mic's GUANO fields", async ({
  page,
}) => {
  await desktopWithMic(page);
  await recordToMemory(page);

  // Stopped: shown as unsaved, and nothing was written.
  await expect(page.locator(".toolbar .badge-fact.unsaved")).toHaveText("* File unsaved");
  await expect(page.locator(".file-item .file-unsaved-asterisk")).toHaveCount(1);
  expect((await calls(page)).filter((c) => c.cmd.startsWith("save_"))).toEqual([]);
  await page.getByRole("button", { name: "Info" }).click();
  await expect(page.getByTestId("info-path-row")).toHaveCount(0);

  await page.locator(".badge-btn.unsaved").click();
  await expect(page.locator(".badge-btn.unsaved")).toHaveCount(0, { timeout: 15_000 });
  const cmds = (await calls(page)).map((c) => c.cmd);
  expect(cmds).toContain("save_open_recording");
  expect(cmds).toContain("save_close");
  const text = await page.evaluate(() => new TextDecoder().decode(Uint8Array.from((window as any).__written)));
  expect(text.startsWith("RIFF")).toBe(true);
  expect(text).toContain("Oversample|Mic|Interface: USB\n");
  expect(text).toContain("Oversample|Mic|Name: DUO-CAPTURE EX\n");
  expect(text).toContain("Make: Roland\n");
  await expect(page.locator(".file-item .file-unsaved-asterisk")).toHaveCount(0);
  await expect(page.getByTestId("info-path-row").locator(".path-row-value")).toHaveText(SAVED);
});

test("a failed save keeps the recording unsaved and says why", async ({ page }) => {
  await desktopWithMic(page);
  await page.evaluate(() => ((window as any).__failSave = true));
  await recordToMemory(page);
  // Let the "Saving recording…" message from the stop time out first: its
  // timer clears whatever message is showing then.
  await page.waitForTimeout(3500);
  await page.locator(".badge-btn.unsaved").click();
  await expect(page.locator(".status-toast")).toHaveText(
    /^Recording not saved: Couldn't save batcap_\d{8}_\d{6}\.wav: \/home\/ann\/Music\/Oversample\/batcap_test\.wav: Permission denied$/,
    { timeout: 2_000 },
  );
  await expect(page.locator(".badge-btn.unsaved")).toBeVisible();
  expect((await calls(page)).map((c) => c.cmd)).not.toContain("save_write");
});
