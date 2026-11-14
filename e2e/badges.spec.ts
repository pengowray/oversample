import { test, expect, type Page } from "@playwright/test";

/**
 * File badges: the heading's badge row (.toolbar-badge-row) and the file list
 * rows (.file-item-info). Facts are plain text, controls are boxes
 * (style.css, "File badges").
 */

/** Mono WAV of a quiet 4 kHz tone: 16-bit, 24-bit or 32-bit float. */
function wav(rate: number, seconds: number, format: "16" | "24" | "float" = "16"): Buffer {
  const n = Math.round(rate * seconds);
  const bps = format === "16" ? 2 : format === "24" ? 3 : 4;
  const data = Buffer.alloc(n * bps);
  for (let i = 0; i < n; i++) {
    const s = Math.sin((2 * Math.PI * 4000 * i) / rate) * 0.25;
    if (format === "float") data.writeFloatLE(s, i * 4);
    else if (format === "24") data.writeIntLE(Math.round(s * 8388607), i * 3, 3);
    else data.writeInt16LE(Math.round(s * 32767), i * 2);
  }
  const h = Buffer.alloc(44);
  h.write("RIFF", 0);
  h.writeUInt32LE(36 + data.length, 4);
  h.write("WAVE", 8);
  h.write("fmt ", 12);
  h.writeUInt32LE(16, 16);
  h.writeUInt16LE(format === "float" ? 3 : 1, 20);
  h.writeUInt16LE(1, 22);
  h.writeUInt32LE(rate, 24);
  h.writeUInt32LE(rate * bps, 28);
  h.writeUInt16LE(bps, 32);
  h.writeUInt16LE(bps * 8, 34);
  h.write("data", 36);
  h.writeUInt32LE(data.length, 40);
  return Buffer.concat([h, data]);
}

/**
 * Opens demo XC815138 as a heterodyne recording (the remark names a
 * heterodyne detector) with the given licence, as a 1.9 s 44.1 kHz 32-bit
 * float file, so its badge row is "44.1kHz · 32 bit float · 1.9s",
 * "Heterodyne recording" and the licence.
 */
async function openHeterodyneDemo(page: Page, lic: string) {
  const sidecar = {
    id: "815138",
    en: "Fringed Long-footed Myotis",
    gen: "Myotis",
    sp: "fimbriatus",
    rec: "Test",
    cnt: "Brazil",
    method: "field recording",
    rmk: "handheld hetrodyne detector set at 31,9 kHz",
    // A sound type that names no echolocation keeps the species call-range
    // note out of the heading.
    type: "social call",
    url: "https://xeno-canto.org/815138",
    lic,
  };
  const audio = wav(44_100, 1.9, "float");
  const handler = async (route: any) => {
    const url: string = route.request().url();
    if (url.endsWith("/index.json")) {
      return route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          version: 1,
          sounds: [
            {
              en: "Fringed Long-footed Myotis",
              filename: "XC815138 demo.wav",
              metadata: "XC815138 demo.xc.json",
              source: "xeno-canto",
              species: "Myotis fimbriatus",
              xc_id: 815138,
            },
          ],
        }),
      });
    }
    if (url.endsWith(".xc.json")) {
      return route.fulfill({ contentType: "application/json", body: JSON.stringify(sidecar) });
    }
    if (url.endsWith(".wav")) return route.fulfill({ contentType: "audio/wav", body: audio });
    return route.fulfill({ status: 404, body: "" });
  };
  await page.route("https://archive.oversample.com/**", handler);
  await page.route("https://cdn.jsdelivr.net/gh/pengowray/bat-demo-sounds@main/**", handler);
  await page.goto("/#XC815138");
  await expect(page.locator(".toolbar-badge-row .file-badge-heterodyne")).toBeVisible({
    timeout: 30_000,
  });
  await expect(page.locator(".toolbar-cc-badge")).toBeVisible();
}

/** Two recordings of a sequence, each in two track files, and one 384 kHz
 *  32-bit float file. */
const LOCAL_FILES = [
  { name: "260305_0058_1-2.wav", buffer: wav(192_000, 1.2, "24") },
  { name: "260305_0058_3.wav", buffer: wav(192_000, 1.2, "24") },
  { name: "260305_0059_1-2.wav", buffer: wav(192_000, 0.9, "24") },
  { name: "260305_0059_3.wav", buffer: wav(192_000, 0.9, "24") },
  { name: "Myotis daubentonii, long name test.wav", buffer: wav(384_000, 1.5, "float") },
];

async function openLocalFiles(page: Page) {
  await page.goto("/");
  await expect(page.locator(".app").first()).toBeVisible({ timeout: 30_000 });
  await showFilesSidebar(page);
  await page
    .locator('input[type="file"]')
    .first()
    .setInputFiles(LOCAL_FILES.map((f) => ({ ...f, mimeType: "audio/wav" })));
  await expect(page.locator(".file-item")).toHaveCount(LOCAL_FILES.length, { timeout: 30_000 });
}

async function showFilesSidebar(page: Page) {
  const tab = page.locator("button.toolbar-sidebar-tab-left");
  if (!(await tab.evaluate((e) => e.classList.contains("active")))) await tab.click();
  await expect(page.locator(".sidebar:not(.right-sidebar)")).not.toHaveClass(/collapsed/);
}

function row(page: Page, name: string) {
  return page.locator(".file-item", {
    has: page.locator(".file-item-name", { hasText: name }),
  });
}

/** Drags the Files sidebar's resize handle so the handle is at x = `width`. */
async function setSidebarWidth(page: Page, width: number) {
  const handle = page.locator(".sidebar:not(.right-sidebar) .sidebar-resize-handle");
  const box = (await handle.boundingBox())!;
  // The handle straddles the sidebar's edge; grab its inner half.
  const x = box.x + 1.5;
  const y = box.y + box.height / 2;
  const centre = box.x + box.width / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + (width - centre), y, { steps: 8 });
  await page.mouse.up();
  await expect
    .poll(async () => {
      const b = (await handle.boundingBox())!;
      return Math.round(b.x + b.width / 2);
    })
    .toBe(width);
}

/** The number of lines a badge row's items take up. */
function badgeLines(page: Page, selector: string) {
  return page.locator(selector).evaluate((el) => {
    const mids = [...el.children]
      .filter((k) => getComputedStyle(k).display !== "none" && k.getBoundingClientRect().width)
      .map((k) => {
        const b = k.getBoundingClientRect();
        return (b.top + b.bottom) / 2;
      })
      .sort((a, b) => a - b);
    return mids.filter((m, i) => i === 0 || m - mids[i - 1] > 6).length;
  });
}

test.describe("sequence and track menus in the heading", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test.beforeEach(async ({ page }) => {
    await openLocalFiles(page);
    await row(page, "260305_0059_3").locator(".file-item-name").click();
    await expect(page.locator(".toolbar-title-filename")).toHaveText("260305_0059_3.wav");
  });

  const seqButton = (page: Page) => page.locator(".toolbar-badge-row > button.file-badge-seq");
  const menu = (page: Page) => page.locator(".badge-dropdown-panel");

  test("Escape closes the menu and goes no further", async ({ page }) => {
    // Escape also stops listening or recording (app.rs), so while a menu is
    // open, Escape must not reach the window.
    await page.evaluate(() => {
      (window as any).escapesAtWindow = 0;
      window.addEventListener("keydown", (e) => {
        if (e.key === "Escape") (window as any).escapesAtWindow++;
      });
    });
    await seqButton(page).focus();
    await page.keyboard.press("Enter");
    await expect(menu(page)).toHaveCount(1);
    await page.keyboard.press("Escape");
    await expect(menu(page)).toHaveCount(0);
    expect(await page.evaluate(() => (window as any).escapesAtWindow)).toBe(0);
    // With no menu open, Escape reaches the window as before.
    await page.keyboard.press("Escape");
    expect(await page.evaluate(() => (window as any).escapesAtWindow)).toBe(1);
  });

  test("a press outside the menu closes it", async ({ page }) => {
    await seqButton(page).click();
    await expect(menu(page)).toHaveCount(1);
    await page.locator(".main-view").click({ position: { x: 200, y: 100 } });
    await expect(menu(page)).toHaveCount(0);

    // The track button swaps the sequence menu for the track menu.
    await seqButton(page).click();
    await expect(menu(page)).toHaveCount(1);
    await page.locator(".toolbar-badge-row > button.file-badge-track").click();
    await expect(page.locator(".badge-dropdown-panel .badge-dropdown-item")).toHaveText([
      /^\[1-2\]/,
      /^\[3\]/,
    ]);
  });

  test("the menu's own button still opens and closes it, and its items still work", async ({
    page,
  }) => {
    await seqButton(page).click();
    await expect(menu(page)).toHaveCount(1);
    await seqButton(page).click();
    await expect(menu(page)).toHaveCount(0);

    await seqButton(page).click();
    await page.locator(".badge-dropdown-item", { hasText: "#58" }).click();
    await expect(menu(page)).toHaveCount(0);
    await expect(page.locator(".toolbar-title-filename")).toHaveText("260305_0058_3.wav");
  });

  test("opening a sidebar closes the menu", async ({ page }) => {
    const hideInfo = page.locator('button[title="Hide info panel"]');
    if (await hideInfo.count()) await hideInfo.click();
    await expect(page.locator(".right-sidebar")).toHaveClass(/collapsed/);
    await seqButton(page).click();
    await expect(menu(page)).toHaveCount(1);
    await page.locator('button[title="Show info panel"]').click();
    await expect(page.locator(".right-sidebar")).not.toHaveClass(/collapsed/);
    await expect(menu(page)).toHaveCount(0);
  });
});

test.describe("file list rows in a narrow Files sidebar", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("a 384 kHz 32-bit float row stays on one line at 170px", async ({ page }) => {
    await openLocalFiles(page);
    await setSidebarWidth(page, 170);
    // Before the fix, its duration wrapped onto a second line.
    expect(await badgeLines(page, ".file-item:has-text('Myotis daubentonii') .file-item-info")).toBe(1);
  });

  test("a wrapped line never starts with a middle dot", async ({ page }) => {
    await openLocalFiles(page);
    for (const width of [140, 150, 170]) {
      await setSidebarWidth(page, width);
      const lineStarts = await page.locator(".file-item-info").evaluateAll((infos) =>
        infos.flatMap((info) => {
          const left = info.getBoundingClientRect().left;
          return [...info.children]
            .filter((k) => getComputedStyle(k).display !== "none")
            .filter((k) => Math.abs(k.getBoundingClientRect().left - left) < 1)
            .map((k) => getComputedStyle(k, "::before").content);
        }),
      );
      expect(lineStarts.length).toBeGreaterThan(0);
      for (const content of lineStarts) expect(content).not.toContain("·");
    }
    // The dot ends the sample rate and bit depth instead.
    const after = await page
      .locator(".file-item:has-text('Myotis daubentonii') .badge-sample-rate")
      .evaluate((e) => getComputedStyle(e, "::after").content);
    expect(after).toContain("·");
  });

  test("a heterodyne row with a licence keeps two lines at 172px", async ({ page }) => {
    await openHeterodyneDemo(page, "//creativecommons.org/licenses/by-nc-sa/4.0/");
    await showFilesSidebar(page);
    // Before the fix, the licence icon wrapped onto a third line at this width.
    await setSidebarWidth(page, 172);
    expect(await badgeLines(page, ".file-item:has-text('XC815138') .file-item-info")).toBe(2);
  });
});

test.describe("licence button in the heading", () => {
  test("on a phone, a long licence shares the heterodyne badge's line", async ({ browser }) => {
    const context = await browser.newContext({
      viewport: { width: 375, height: 812 },
      isMobile: true,
      hasTouch: true,
      userAgent:
        "Mozilla/5.0 (Linux; Android 14; Pixel 7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36",
    });
    const page = await context.newPage();
    await openHeterodyneDemo(page, "//creativecommons.org/publicdomain/mark/1.0/");
    const licence = page.locator(".toolbar-cc-badge");
    await expect(licence.locator(".toolbar-cc-label")).toHaveText("Public Domain Mark 1.0");
    // Before the fix the licence moved to a third badge line.
    expect(await badgeLines(page, ".toolbar-badge-row")).toBe(2);
    const het = (await page.locator(".toolbar-badge-row .file-badge-heterodyne").boundingBox())!;
    const lic = (await licence.boundingBox())!;
    expect(Math.abs(lic.y - het.y)).toBeLessThan(2);
    // The label is cut off with an ellipsis, and the button stays inside the row.
    const label = licence.locator(".toolbar-cc-label");
    expect(await label.evaluate((e) => e.scrollWidth > e.clientWidth)).toBe(true);
    const rowBox = (await page.locator(".toolbar-badge-row").boundingBox())!;
    expect(lic.x + lic.width).toBeLessThanOrEqual(rowBox.x + rowBox.width + 0.5);
    await context.close();
  });

  test("on a desktop, the whole licence shows", async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await openHeterodyneDemo(page, "//creativecommons.org/licenses/by-nc-sa/2.5/scotland/");
    const label = page.locator(".toolbar-cc-badge .toolbar-cc-label");
    await expect(label).toHaveText("BY-NC-SA 2.5 SCOTLAND");
    expect(await label.evaluate((e) => e.scrollWidth > e.clientWidth)).toBe(false);
    expect(await badgeLines(page, ".toolbar-badge-row")).toBe(1);
  });
});

test.describe("[.batm] toggle in a file row", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  // The toggle only renders in the desktop app, so this test adds its markup
  // to a row of the browser build.
  test("uses the row's 11px control size", async ({ page }) => {
    await openLocalFiles(page);
    const style = await page.locator(".file-item-name").first().evaluate((name) => {
      const b = document.createElement("button");
      b.className = "file-badge badge-btn file-badge-batm";
      b.textContent = "[.batm view]";
      b.style.display = "inline-block";
      name.prepend(b);
      const cs = getComputedStyle(b);
      return { fontSize: cs.fontSize, padding: cs.paddingLeft, width: b.getBoundingClientRect().width };
    });
    expect(style.fontSize).toBe("11px");
    expect(style.padding).toBe("3px");
    // At 13px it was 85px wide.
    expect(style.width).toBeLessThan(70);
  });
});
