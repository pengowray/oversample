import { test, expect, type Page } from "@playwright/test";

/**
 * "View in 3D" from the selection menu: the dialog opens, Plotly draws the
 * surface, and dragging on it rotates the camera. Headless Chromium has no GPU,
 * so WebGL runs on SwiftShader.
 */
test.use({
  launchOptions: { args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"] },
  viewport: { width: 1280, height: 860 },
});

type Eye = { x: number; y: number; z: number };
const webglWarnings: string[] = [];

function cameraEye(page: Page): Promise<Eye> {
  return page.evaluate(() => {
    const el = document.querySelector(".surface-3d-plot") as unknown as {
      _fullLayout: { scene: { _scene: { getCamera(): { eye: Eye } } } };
    };
    return { ...el._fullLayout.scene._scene.getCamera().eye };
  });
}

async function openSurface(page: Page) {
  page.on("console", (m) => {
    if (/WebGL/i.test(m.text())) webglWarnings.push(m.text());
  });
  await page.goto("/");
  await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
  await page
    .locator('input[type="file"][accept*=".wav"]')
    .first()
    .setInputFiles("e2e/fixtures/fake-audio.wav");

  const canvas = page.locator(".main-view .chart-stage > canvas");
  await expect(canvas).toBeVisible({ timeout: 30_000 });
  await page.locator(".tool-combo-btn").click();
  await page.locator(".layer-panel button", { hasText: "Selection" }).click();

  const box = (await canvas.boundingBox())!;
  await page.mouse.move(box.x + box.width * 0.05, box.y + box.height * 0.1);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.3, box.y + box.height * 0.9, { steps: 5 });
  await page.mouse.up();

  await page.locator(".canvas-overflow-btn").click();
  await page.locator(".canvas-overflow-item", { hasText: "View in 3D" }).click();
  await expect(page.locator(".surface-3d-dialog")).toBeVisible();
  // Plotly adds its own SVG layers once the surface is drawn.
  await expect(page.locator(".surface-3d-plot .main-svg").first()).toBeAttached({
    timeout: 30_000,
  });
  await expect(page.locator(".surface-3d-status")).toHaveCount(0);
}

test("View in 3D draws the selection and dragging rotates it", async ({ page }, testInfo) => {
  // SwiftShader renders slowly: opening plus the redraws below take about a minute.
  test.setTimeout(150_000);
  await openSurface(page);
  await page.screenshot({ path: testInfo.outputPath("surface3d-open.png") });

  const before = await cameraEye(page);
  const plot = (await page.locator(".surface-3d-plot").boundingBox())!;
  const cx = plot.x + plot.width / 2;
  const cy = plot.y + plot.height / 2;
  await page.mouse.move(cx, cy);
  await page.mouse.down();
  // Release outside the dialog: the dialog must stay open.
  await page.mouse.move(cx + plot.width * 0.6, cy + 40, { steps: 10 });
  await page.mouse.up();
  await expect(page.locator(".surface-3d-dialog")).toBeVisible();

  const after = await cameraEye(page);
  const moved = Math.hypot(after.x - before.x, after.y - before.y, after.z - before.z);
  expect(moved).toBeGreaterThan(0.1);
  await page.screenshot({ path: testInfo.outputPath("surface3d-rotated.png") });

  // The floor slider redraws without resetting the rotation.
  await page.locator(".surface-3d-floor input").fill("-30");
  await expect(page.locator(".surface-3d-floor-value")).toHaveText("-30 dB");
  await page.waitForTimeout(300);
  const afterFloor = await cameraEye(page);
  expect(Math.hypot(afterFloor.x - after.x, afterFloor.y - after.y)).toBeLessThan(0.01);

  // Many redraws must not use up WebGL contexts (browsers drop the oldest).
  const slider = page.locator(".surface-3d-floor input");
  await slider.focus();
  for (let i = 0; i < 25; i++) await page.keyboard.press(i % 2 ? "ArrowRight" : "ArrowLeft");
  await page.waitForTimeout(500);
  expect(webglWarnings).toEqual([]);
  expect(await cameraEye(page)).toBeTruthy();

  // Reset appears once the floor is changed, and puts the starting value back.
  const reset = page.locator(".surface-3d-controls button", { hasText: "Reset" });
  await expect(reset).toBeVisible();
  await reset.click();
  await expect(reset).toHaveCount(0);

  // Hover reports the cell under the pointer: its time in the recording is the
  // selection start plus x, and its level matches the drawn height.
  const start = parseFloat((await page.locator(".surface-3d-range").textContent())!);
  const floor = parseFloat((await page.locator(".surface-3d-floor-value").textContent())!);
  const points = await page.evaluate(async () => {
    const plot = document.querySelector(".surface-3d-plot") as HTMLElement & {
      on(ev: string, f: (d: { points: { x: number; z: number; customdata: number[] }[] }) => void): void;
    };
    const got: { x: number; z: number; customdata: number[] }[] = [];
    plot.on("plotly_hover", (d) => got.push(d.points[0]));
    const r = plot.getBoundingClientRect();
    for (let k = 0; k < 30; k++) {
      const x = r.left + r.width * (0.3 + k * 0.015);
      const y = r.top + r.height * 0.6;
      const t = document.elementFromPoint(x, y)!;
      t.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, clientX: x, clientY: y, view: window }));
      await new Promise((res) => setTimeout(res, 120));
    }
    return got.map((p) => ({ x: p.x, z: p.z, customdata: p.customdata }));
  });
  expect(points.length).toBeGreaterThan(1);
  for (const p of points) {
    // The header rounds the start to 1 ms; a wrong cell is off by far more.
    expect(p.customdata[1]).toBeCloseTo(start + p.x / 1000, 2);
    expect(Math.max(p.customdata[0], floor)).toBeCloseTo(p.z, 3);
  }

  await page.keyboard.press("Escape");
  await expect(page.locator(".surface-3d-dialog")).toHaveCount(0);
});
