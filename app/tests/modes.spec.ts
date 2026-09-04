import { test, expect } from "@playwright/test";
import fixture from "../public/fixtures/floor.json" with { type: "json" };
import statesFixture from "../public/fixtures/floor-states.json" with { type: "json" };

test("hotkeys switch modes and set data-mode", async ({ page }) => {
  await page.goto("/");
  const shell = page.locator(".app-shell");
  await expect(shell).toHaveAttribute("data-mode", "incident"); // floor.json auto-enters INCIDENT
  await page.waitForSelector('[data-testid="incident-panel"]');

  await page.keyboard.press("Escape");
  await expect(shell).toHaveAttribute("data-mode", "command");

  await page.keyboard.press("2");
  await expect(shell).toHaveAttribute("data-mode", "focus");

  await page.keyboard.press("3");
  await expect(shell).toHaveAttribute("data-mode", "ambient");

  await page.keyboard.press("5");
  await expect(shell).toHaveAttribute("data-mode", "debug");

  await page.keyboard.press("1");
  await expect(shell).toHaveAttribute("data-mode", "command");
});

test("INCIDENT auto-enters on floor.json (FAILED + BREY_REQUIRED) and not on floor-idle.json", async ({
  page,
}) => {
  const hasObservedFault = fixture.sessions.some(
    (s) => s.fidelity === "observed" && (s.state === "failed" || s.state === "brey_required")
  );
  expect(hasObservedFault).toBe(true);

  await page.goto("/");
  await expect(page.locator(".app-shell")).toHaveAttribute("data-mode", "incident");
  await expect(page.locator('[data-testid="incident-panel"]')).toBeVisible();

  await page.goto("/?fixture=floor-idle.json");
  await expect(page.locator(".app-shell")).toHaveAttribute("data-mode", "command");
  await expect(page.locator('[data-testid="incident-panel"]')).toHaveCount(0);
});

test("AMBIENT mode still shows the PIPELINE / REMOTE ESTATE truth line", async ({ page }) => {
  await page.goto("/?fixture=floor-idle.json");
  await page.waitForSelector(".marquee");
  await page.keyboard.press("3");
  await expect(page.locator(".app-shell")).toHaveAttribute("data-mode", "ambient");
  const status = page.locator(".marquee-status");
  await expect(status).toContainText("REMOTE ESTATE");
  await expect(status).toContainText("PIPELINE");
  await page.waitForTimeout(400);
  await page.screenshot({ path: "screenshots/mode-ambient.png", fullPage: false });
});

test("PROJECT FOCUS shows the selected bay name", async ({ page }) => {
  await page.goto("/?fixture=floor-idle.json");
  await page.waitForSelector(".marquee");
  await page.keyboard.press("2");
  await expect(page.locator(".app-shell")).toHaveAttribute("data-mode", "focus");
  const bayName = page.locator(".focus-bay-name");
  await expect(bayName).toBeVisible();
  const text = await bayName.textContent();
  expect(fixture.output_shelf).toHaveProperty(text!.trim());
  await page.waitForTimeout(300);
  await page.screenshot({ path: "screenshots/mode-focus.png", fullPage: false });
});

test("INCIDENT screenshot", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator(".app-shell")).toHaveAttribute("data-mode", "incident");
  await page.waitForTimeout(300);
  await page.screenshot({ path: "screenshots/mode-incident.png", fullPage: false });
});

test("DEEP DEBUG renders the tape rows count", async ({ page }) => {
  await page.goto("/");
  await page.waitForSelector('[data-testid="incident-panel"]');
  await page.keyboard.press("Escape");
  await page.keyboard.press("5");
  await expect(page.locator(".app-shell")).toHaveAttribute("data-mode", "debug");
  const rows = page.locator('[data-testid="tape-row"]');
  await expect(rows).toHaveCount((fixture as { tape: unknown[] }).tape.length);
  expect((fixture as { tape: unknown[] }).tape.length).toBeGreaterThanOrEqual(20);
  await expect(page.locator('[data-testid="machines-list"]')).toBeVisible();
  await expect(page.locator('[data-testid="observer-health"]')).toBeVisible();
  await page.waitForTimeout(300);
  await page.screenshot({ path: "screenshots/mode-debug.png", fullPage: false });
});

test("label collisions: no two station label rects intersect", async ({ page }) => {
  await page.goto("/?fixture=floor-states.json");
  const rows = page.locator("#scene-mirror div[data-label-rect]");
  await expect(rows).toHaveCount(statesFixture.sessions.length);
  const count = await rows.count();

  const rects: { x: number; y: number; w: number; h: number }[] = [];
  for (let i = 0; i < count; i++) {
    const attr = await rows.nth(i).getAttribute("data-label-rect");
    const [x, y, w, h] = attr!.split(",").map(Number);
    rects.push({ x, y, w, h });
  }

  function intersects(a: (typeof rects)[0], b: (typeof rects)[0]): boolean {
    return a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y;
  }

  for (let i = 0; i < rects.length; i++) {
    for (let j = i + 1; j < rects.length; j++) {
      expect(intersects(rects[i], rects[j])).toBe(false);
    }
  }
});
