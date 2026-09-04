import { test, expect } from "@playwright/test";
import fixture from "../public/fixtures/floor.json" with { type: "json" };

test("every fixture session appears with its state token", async ({ page }) => {
  await page.goto("/");
  const mirror = page.locator("#scene-mirror");
  await expect(mirror.locator("div")).toHaveCount(fixture.sessions.length);

  for (const session of fixture.sessions) {
    const row = page.locator(`#scene-mirror div[data-station-id="${session.id}"]`);
    await expect(row).toHaveAttribute("data-state", session.state);
    await expect(row).toHaveAttribute("data-fidelity", session.fidelity);
    await expect(row).toHaveAttribute("data-bay", session.bay);
  }
});

test("unverified pipeline shows the blind overlay", async ({ page }) => {
  await page.goto("/");
  expect(fixture.pipeline.verified).toBe(false);
  const marquee = page.locator(".marquee");
  await expect(marquee).toHaveClass(/marquee-blind/);
  await expect(marquee).toContainText("PIPELINE: UNVERIFIED");
});

test("BREY_REQUIRED count in marquee matches fixture", async ({ page }) => {
  await page.goto("/");
  const breyCount = fixture.sessions.filter((s) => s.state === "brey_required").length;
  expect(breyCount).toBeGreaterThan(0);
  await expect(page.locator(".marquee-brey")).toContainText(`${breyCount} BREY REQUIRED`);
});

test("screenshot of the fixture floor", async ({ page }) => {
  await page.goto("/");
  await page.waitForTimeout(300);
  await page.screenshot({ path: "screenshots/floor-fixture.png", fullPage: false });
});
