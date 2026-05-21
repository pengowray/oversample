import { defineConfig, devices } from "@playwright/test";

// Port chosen to avoid clashing with common dev servers on Windows (8080 in use).
const PORT = Number(process.env.OVERSAMPLE_PORT ?? 21259);
const BASE_URL = `http://127.0.0.1:${PORT}`;

export default defineConfig({
  testDir: "./e2e",
  timeout: 60_000,
  expect: { timeout: 10_000 },
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  workers: 1,
  reporter: process.env.CI ? [["github"], ["html", { open: "never" }]] : "list",
  use: {
    baseURL: BASE_URL,
    trace: "retain-on-failure",
    video: "retain-on-failure",
    screenshot: "only-on-failure",
    ignoreHTTPSErrors: true,
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  // Trunk's first --release build is slow (several minutes on a cold cache);
  // launch with a generous timeout and reuse any server you already have running.
  webServer: {
    command: `trunk serve --release --port ${PORT} --no-autoreload`,
    url: BASE_URL,
    reuseExistingServer: true,
    timeout: 600_000,
    stdout: "pipe",
    stderr: "pipe",
  },
});
