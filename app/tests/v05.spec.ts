import { test, expect } from "@playwright/test";

// V-05 cuts: marquee layout, bay click -> PROJECT FOCUS, AMBIENT hue drift,
// and the query-param-driven autopilot test harness.

test("marquee: MODE indicator sits in its own column and never overlaps status row", async ({
  page,
}) => {
  await page.goto("/?fixture=floor-idle.json");
  await page.waitForSelector(".marquee");

  const modeBox = await page.locator('[data-testid="mode-indicator"]').boundingBox();
  const countsBox = await page.locator(".marquee-counts").boundingBox();
  const statusBox = await page.locator(".marquee-status").boundingBox();
  expect(modeBox).toBeTruthy();
  expect(countsBox).toBeTruthy();
  expect(statusBox).toBeTruthy();

  function overlaps(a: NonNullable<typeof modeBox>, b: NonNullable<typeof modeBox>): boolean {
    return a.x < b.x + b.width && a.x + a.width > b.x && a.y < b.y + b.height && a.y + a.height > b.y;
  }

  // The mode chip must not clip/overlap either counts row or status row —
  // this was the mode-incident.png defect (MODE: overlapping line 2).
  expect(overlaps(modeBox!, countsBox!)).toBe(false);
  expect(overlaps(modeBox!, statusBox!)).toBe(false);

  // And nothing should render outside the 76px marquee band.
  const marqueeBox = await page.locator(".marquee").boundingBox();
  expect(modeBox!.y).toBeGreaterThanOrEqual(marqueeBox!.y - 1);
  expect(modeBox!.y + modeBox!.height).toBeLessThanOrEqual(marqueeBox!.y + marqueeBox!.height + 1);
});

test("clicking a bay's platform on the canvas enters PROJECT FOCUS for that bay", async ({ page }) => {
  await page.goto("/?fixture=floor-idle.json");
  await page.waitForSelector(".marquee");
  await expect(page.locator(".app-shell")).toHaveAttribute("data-mode", "command");

  const rows = page.locator("#bay-mirror div[data-bay-rect]");
  await expect(rows).not.toHaveCount(0);
  const count = await rows.count();

  // Pick a real (non-UNRESOLVED) bay's rect.
  let bay: string | null = null;
  let rect: { x: number; y: number; w: number; h: number } | null = null;
  for (let i = 0; i < count; i++) {
    const b = await rows.nth(i).getAttribute("data-bay");
    if (b && b !== "UNRESOLVED") {
      bay = b;
      const attr = await rows.nth(i).getAttribute("data-bay-rect");
      const [x, y, w, h] = attr!.split(",").map(Number);
      rect = { x, y, w, h };
      break;
    }
  }
  expect(bay).toBeTruthy();
  expect(rect).toBeTruthy();

  const canvasBox = await page.locator(".floor-host canvas").boundingBox();
  const clickX = canvasBox!.x + rect!.x + rect!.w / 2;
  // The bay hit area is the platform diamond, tallest/widest near its
  // vertical center-lower area (skirt); click a touch below the rect's
  // vertical midpoint where the diamond is at its widest.
  const clickY = canvasBox!.y + rect!.y + rect!.h * 0.5;

  await page.mouse.click(clickX, clickY);
  await expect(page.locator(".app-shell")).toHaveAttribute("data-mode", "focus");
  await expect(page.locator('[data-testid="focus-panel"]')).toContainText(bay!);
});

test("AMBIENT drifts the scene position over time (burn-in hygiene) when motion isn't reduced", async ({
  page,
}) => {
  await page.goto("/?fixture=floor-idle.json");
  await page.waitForSelector(".marquee");
  await page.keyboard.press("3");
  await expect(page.locator(".app-shell")).toHaveAttribute("data-mode", "ambient");
  // `data-ambient-drift` (world.x offset, hue-drift degrees) is written
  // every ambient-mode frame — see floor.ts's ticker "mode === ambient &&
  // !reducedMotion" branch. Wait for the first ambient frame (slow CI
  // runners can take well over a second under software rendering), then
  // it should move over time, unlike the reduced-motion case below.
  const host = page.locator(".floor-host");
  await expect
    .poll(async () => host.getAttribute("data-ambient-drift"), { timeout: 15_000 })
    .toBeTruthy();
  const d0 = await host.getAttribute("data-ambient-drift");
  let moved = false;
  for (let i = 0; i < 20; i++) {
    await page.waitForTimeout(300);
    const d = await page.locator(".floor-host").getAttribute("data-ambient-drift");
    if (d !== d0) {
      moved = true;
      break;
    }
  }
  expect(moved).toBe(true);
});

test("reduced-motion disables AMBIENT hue drift and the 1px offset", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/?fixture=floor-idle.json");
  await page.waitForSelector(".marquee");
  await page.keyboard.press("3");
  await expect(page.locator(".app-shell")).toHaveAttribute("data-mode", "ambient");
  await page.waitForTimeout(1200);
  // Under reduced motion, the drift branch never runs, so this attribute
  // is never written at all — no 1px offset, no hue drift.
  const drift = await page.locator(".floor-host").getAttribute("data-ambient-drift");
  expect(drift).toBeNull();
  // Scene bounds also stay put (no 1px offset accumulated into layout).
  const b1 = await page.locator(".floor-host").getAttribute("data-scene-bounds");
  await page.waitForTimeout(800);
  const b2 = await page.locator(".floor-host").getAttribute("data-scene-bounds");
  expect(b1).toBe(b2);
});

test("autopilot (test-only query params): COMMAND CENTER drifts to PROJECT FOCUS on the most active bay, then back", async ({
  page,
}) => {
  await page.goto("/?fixture=floor-idle.json&autopilotIdleMs=500&autopilotDwellMs=600");
  await page.waitForSelector(".marquee");
  await expect(page.locator(".app-shell")).toHaveAttribute("data-mode", "command");

  await expect(page.locator(".app-shell")).toHaveAttribute("data-mode", "focus", { timeout: 3000 });
  await expect(page.locator('[data-testid="focus-panel"]')).toBeVisible();

  await expect(page.locator(".app-shell")).toHaveAttribute("data-mode", "command", { timeout: 3000 });
});

test("autopilot: a FAILED fixture snaps to INCIDENT instead of drifting to PROJECT FOCUS", async ({
  page,
}) => {
  // floor.json carries an observed FAILED/BREY_REQUIRED station, so it
  // auto-enters INCIDENT on load — before autopilot's idle timer would
  // ever fire — and must stay there rather than drifting to focus.
  await page.goto("/?fixture=floor.json&autopilotIdleMs=500&autopilotDwellMs=600");
  await page.waitForSelector(".marquee");
  await expect(page.locator(".app-shell")).toHaveAttribute("data-mode", "incident");
  await page.waitForTimeout(1500); // long enough autopilot would have fired
  await expect(page.locator(".app-shell")).toHaveAttribute("data-mode", "incident");
});
