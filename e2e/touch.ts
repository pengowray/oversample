import { type Page } from "@playwright/test";

/**
 * Synthetic touch-gesture helpers for driving the canvas event handlers in
 * headless Chromium (which has no real touch input). Requires a touch-capable
 * context — set `test.use({ hasTouch: true })` in the spec.
 */

/**
 * Dispatch a two-finger pinch on the main spectrogram canvas.
 * `axis` "x" spreads the fingers horizontally (time), "y" vertically (freq).
 * The gap grows from startGap to endGap across several moves so the axis-lock
 * (≥10px spread) engages like a real gesture.
 */
export async function pinch(page: Page, axis: "x" | "y", startGap: number, endGap: number) {
  await page.evaluate(
    ({ axis, startGap, endGap }) => {
      const canvas = document.querySelector(".chart-stage canvas") as HTMLElement;
      const r = canvas.getBoundingClientRect();
      const cx = r.left + r.width / 2;
      const cy = r.top + r.height / 2;
      const mk = (id: number, x: number, y: number) =>
        new Touch({ identifier: id, target: canvas, clientX: x, clientY: y, pageX: x, pageY: y, screenX: x, screenY: y, radiusX: 1, radiusY: 1, rotationAngle: 0, force: 1 });
      const fire = (type: string, touches: Touch[]) =>
        canvas.dispatchEvent(new TouchEvent(type, { cancelable: true, bubbles: true, composed: true, touches, targetTouches: touches, changedTouches: touches }));
      const pts = (gap: number): Touch[] =>
        axis === "x"
          ? [mk(0, cx - gap / 2, cy), mk(1, cx + gap / 2, cy)]
          : [mk(0, cx, cy - gap / 2), mk(1, cx, cy + gap / 2)];
      fire("touchstart", pts(startGap));
      const steps = 6;
      for (let i = 1; i <= steps; i++) fire("touchmove", pts(startGap + ((endGap - startGap) * i) / steps));
      fire("touchend", []);
    },
    { axis, startGap, endGap },
  );
}

/**
 * Dispatch a one-finger horizontal drag on the main spectrogram canvas.
 * Positive `dxPx` moves the finger right (pans back into history); negative
 * moves it left (pans toward the live edge / future).
 */
export async function drag(page: Page, dxPx: number) {
  await page.evaluate((dxPx) => {
    const canvas = document.querySelector(".chart-stage canvas") as HTMLElement;
    const r = canvas.getBoundingClientRect();
    const x0 = r.left + r.width / 2;
    const cy = r.top + r.height / 2;
    const mk = (id: number, x: number, y: number) =>
      new Touch({ identifier: id, target: canvas, clientX: x, clientY: y, pageX: x, pageY: y, screenX: x, screenY: y, radiusX: 1, radiusY: 1, rotationAngle: 0, force: 1 });
    const fire = (type: string, touches: Touch[]) =>
      canvas.dispatchEvent(new TouchEvent(type, { cancelable: true, bubbles: true, composed: true, touches, targetTouches: touches, changedTouches: touches }));
    fire("touchstart", [mk(0, x0, cy)]);
    const steps = 8;
    for (let i = 1; i <= steps; i++) fire("touchmove", [mk(0, x0 + (dxPx * i) / steps, cy)]);
    fire("touchend", []);
  }, dxPx);
}

/** Zoom the spectrogram in via ctrl+wheel `steps` times (each ~1.1x). */
export async function zoomIn(page: Page, steps: number) {
  await page.evaluate((steps) => {
    const canvas = document.querySelector(".chart-stage canvas") as HTMLElement;
    const r = canvas.getBoundingClientRect();
    const x = r.left + r.width / 2;
    const y = r.top + r.height / 2;
    for (let i = 0; i < steps; i++) {
      canvas.dispatchEvent(
        new WheelEvent("wheel", { deltaY: -120, ctrlKey: true, clientX: x, clientY: y, bubbles: true, cancelable: true }),
      );
    }
  }, steps);
}

/** True if a numeric value meaningfully changed (treats null = "auto"). */
export function changed(a: number | null, b: number | null): boolean {
  if (a === null && b === null) return false;
  if (a === null || b === null) return true;
  return Math.abs(a - b) > Math.max(1e-6, Math.abs(a) * 1e-4);
}
