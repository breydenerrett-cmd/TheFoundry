import { test, expect } from "@playwright/test";
import fixture from "../public/fixtures/floor.json" with { type: "json" };
import idleFixture from "../public/fixtures/floor-idle.json" with { type: "json" };
import blindFixture from "../public/fixtures/floor-blind.json" with { type: "json" };
import statesFixture from "../public/fixtures/floor-states.json" with { type: "json" };
import { STATE_TABLE, motionFor, beaconFor } from "../src/states";

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

test("marquee renders in required order with correct counts", async ({ page }) => {
  await page.goto("/");
  const counts: Record<string, number> = {};
  for (const s of fixture.sessions) counts[s.state] = (counts[s.state] ?? 0) + 1;
  const failed = counts.failed ?? 0;
  const hung = counts.hung ?? 0;
  const working = counts.working ?? 0;
  const waiting =
    (counts.waiting_on_agent ?? 0) + (counts.waiting_on_system ?? 0) + (counts.blocked ?? 0);
  const stale = counts.stale_unknown ?? 0;
  const opus = fixture.sessions.filter(
    (s) => /opus/i.test(s.model_current ?? s.model ?? "")
  ).length;

  const countsRow = page.locator(".marquee-counts");
  const text = (await countsRow.innerText()).replace(/\s+/g, " ");
  const breyIdx = text.indexOf("BREY REQUIRED");
  const failedIdx = text.indexOf("FAILED");
  const hungIdx = text.indexOf("HUNG");
  const workingIdx = text.indexOf("WORKING");
  const waitingIdx = text.indexOf("WAITING");
  const staleIdx = text.indexOf("STALE");
  const opusIdx = text.indexOf("OPUS");
  expect(breyIdx).toBeGreaterThanOrEqual(0);
  expect(failedIdx).toBeGreaterThan(breyIdx);
  expect(hungIdx).toBeGreaterThan(failedIdx);
  expect(workingIdx).toBeGreaterThan(hungIdx);
  expect(waitingIdx).toBeGreaterThan(workingIdx);
  expect(staleIdx).toBeGreaterThan(waitingIdx);
  expect(opusIdx).toBeGreaterThan(staleIdx);

  await expect(countsRow).toContainText(`${failed} FAILED`);
  await expect(countsRow).toContainText(`${hung} HUNG`);
  await expect(countsRow).toContainText(`${working} WORKING`);
  await expect(countsRow).toContainText(`${waiting} WAITING`);
  await expect(countsRow).toContainText(`${stale} STALE`);
  await expect(countsRow).toContainText(`${opus} OPUS`);
  expect(opus).toBeGreaterThan(0);
});

test("inferred WORKING station renders ghost motion, unknown renders none", async ({ page }) => {
  await page.goto("/");
  const inferredWorking = fixture.sessions.find(
    (s) => s.state === "working" && s.fidelity === "inferred"
  )!;
  expect(inferredWorking).toBeTruthy();
  const row = page.locator(`#scene-mirror div[data-station-id="${inferredWorking.id}"]`);
  await expect(row).toHaveAttribute("data-fidelity", "inferred");
  await expect(row).toHaveAttribute("data-motion", "ghost");

  const unknownStation = fixture.sessions.find((s) => s.fidelity === "unknown")!;
  expect(unknownStation).toBeTruthy();
  const unknownRow = page.locator(`#scene-mirror div[data-station-id="${unknownStation.id}"]`);
  await expect(unknownRow).toHaveAttribute("data-motion", "none");
});

test("idle fixture: no blind overlay, lower ambient", async ({ page }) => {
  expect(idleFixture.pipeline.verified).toBe(true);
  expect(idleFixture.pipeline.remote_estate).toBe("live");

  await page.goto("/");
  const busyAmbient = parseFloat((await page.locator(".floor-host").getAttribute("data-ambient"))!);

  await page.goto("/?fixture=floor-idle.json");
  const marquee = page.locator(".marquee");
  await expect(marquee).not.toHaveClass(/marquee-blind/);
  await expect(marquee).toContainText("PIPELINE: VERIFIED");
  const host = page.locator(".floor-host");
  await expect(host).toHaveAttribute("data-ambient", /.+/);
  const idleAmbient = parseFloat((await host.getAttribute("data-ambient"))!);
  expect(idleAmbient).toBeLessThan(busyAmbient);

  await page.waitForTimeout(300);
  await page.screenshot({ path: "screenshots/floor-idle.png", fullPage: false });
});

test("state-mapping table: scene-mirror matches src/states.ts for every state x fidelity", async ({
  page,
}) => {
  await page.goto("/?fixture=floor-states.json");
  const mirror = page.locator("#scene-mirror");
  await expect(mirror.locator("div")).toHaveCount(statesFixture.sessions.length);

  for (const session of statesFixture.sessions) {
    const row = page.locator(`#scene-mirror div[data-station-id="${session.id}"]`);
    const expectedMotion = motionFor(
      session.state as keyof typeof STATE_TABLE,
      session.fidelity as "observed" | "inferred" | "unknown",
      (session as { restored?: boolean }).restored
    );
    const expectedBeacon = beaconFor(
      session.state as keyof typeof STATE_TABLE,
      (session as { restored?: boolean }).restored
    );
    await expect(row).toHaveAttribute("data-state", session.state);
    await expect(row).toHaveAttribute("data-fidelity", session.fidelity);
    await expect(row).toHaveAttribute("data-motion", expectedMotion);
    await expect(row).toHaveAttribute("data-beacon", expectedBeacon);
    if ((session as { restored?: boolean }).restored) {
      await expect(row).toHaveAttribute("data-restored", "true");
    }
  }

  // The 13 core states are all covered at observed fidelity.
  const observedStates = new Set(
    statesFixture.sessions.filter((s) => s.fidelity === "observed" && !("restored" in s)).map((s) => s.state)
  );
  for (const state of Object.keys(STATE_TABLE)) {
    expect(observedStates.has(state)).toBe(true);
  }

  // The restored WORKING record must never be trusted — it renders with no
  // motion (STALE treatment), regardless of its nominal `working` state.
  const restored = statesFixture.sessions.find((s) => (s as { restored?: boolean }).restored)!;
  const restoredRow = page.locator(`#scene-mirror div[data-station-id="${restored.id}"]`);
  await expect(restoredRow).toHaveAttribute("data-motion", "none");

  await page.waitForTimeout(300);
  await page.screenshot({ path: "screenshots/floor-states.png", fullPage: false });
});

test("layout: scene bounds fill most of the viewport at 1280x720 and 1920x1080", async ({
  page,
}) => {
  for (const [width, height] of [
    [1280, 720],
    [1920, 1080],
  ] as const) {
    await page.setViewportSize({ width, height });
    await page.goto("/");
    await page.waitForTimeout(300);

    const host = page.locator(".floor-host");
    const boundsAttr = await host.getAttribute("data-scene-bounds");
    expect(boundsAttr).toBeTruthy();
    const [, , w, h] = boundsAttr!.split(",").map(Number);

    const hostBox = await host.boundingBox();
    expect(hostBox).toBeTruthy();

    expect(w).toBeGreaterThanOrEqual(hostBox!.width * 0.8);
    expect(h).toBeGreaterThanOrEqual(hostBox!.height * 0.7);
  }
});

test("blind fixture: overlay + PIPELINE: UNVERIFIED", async ({ page }) => {
  expect(blindFixture.observers.every((o) => o.status === "down")).toBe(true);
  expect(blindFixture.pipeline.verified).toBe(false);
  await page.goto("/?fixture=floor-blind.json");
  const marquee = page.locator(".marquee");
  await expect(marquee).toHaveClass(/marquee-blind/);
  await expect(marquee).toContainText("PIPELINE: UNVERIFIED");
  await page.waitForTimeout(300);
  await page.screenshot({ path: "screenshots/floor-blind.png", fullPage: false });
});
