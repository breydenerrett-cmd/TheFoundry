import { test, expect } from "@playwright/test";
import { spawn, spawnSync, type ChildProcessWithoutNullStreams } from "node:child_process";
import * as net from "node:net";
import * as path from "node:path";
import * as fs from "node:fs";
import * as os from "node:os";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// L-01: the real watcher binary, driving the app end-to-end over its
// `--serve` HTTP feed — no fixture, no fakery. Builds watcher-core first if
// the debug binary isn't already there (it normally is, from CONTEXT setup).

const REPO_ROOT = path.resolve(__dirname, "..", "..");
const WATCHER_DIR = path.join(REPO_ROOT, "watcher-core");
const WATCHER_BIN = path.join(WATCHER_DIR, "target", "debug", "foundry");
const BAY_MAP = path.join(REPO_ROOT, "foundry.bays.toml");

async function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.listen(0, "127.0.0.1", () => {
      const addr = srv.address();
      if (addr && typeof addr === "object") {
        const port = addr.port;
        srv.close(() => resolve(port));
      } else {
        srv.close(() => reject(new Error("could not allocate a free port")));
      }
    });
  });
}

/** Waits not just for the httpd listener to accept connections (`/health`
 *  200s immediately on bind) but for at least one real poll cycle to have
 *  run — `/state` starts as the literal placeholder `"{}"` until then, which
 *  has no `sessions`/`routines` fields at all. */
function waitForRealState(port: number, timeoutMs = 15_000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  return new Promise((resolve, reject) => {
    const tick = async () => {
      try {
        const res = await fetch(`http://127.0.0.1:${port}/state`, {
          signal: AbortSignal.timeout(1000),
        });
        if (res.ok) {
          const json = await res.json();
          if (Array.isArray(json?.sessions)) {
            resolve();
            return;
          }
        }
      } catch {
        // not up yet
      }
      if (Date.now() > deadline) {
        reject(new Error(`watcher never produced real /state on port ${port}`));
        return;
      }
      setTimeout(tick, 200);
    };
    void tick();
  });
}

let watcherLogDir: string;

function spawnWatcher(port: number): ChildProcessWithoutNullStreams {
  watcherLogDir = fs.mkdtempSync(path.join(os.tmpdir(), "foundry-live-"));
  const child = spawn(
    WATCHER_BIN,
    [
      "--no-remote",
      "--git-dir",
      REPO_ROOT,
      "--serve",
      `127.0.0.1:${port}`,
      "--watch",
      "1",
      "--bay-map",
      BAY_MAP,
      "--log-dir",
      watcherLogDir,
    ],
    { stdio: ["ignore", "pipe", "pipe"] }
  );
  child.stdout.on("data", () => {});
  child.stderr.on("data", () => {});
  return child;
}

test.beforeAll(() => {
  if (!fs.existsSync(WATCHER_BIN)) {
    const res = spawnSync("cargo", ["build", "-q"], { cwd: WATCHER_DIR, stdio: "inherit" });
    if (res.status !== 0) {
      throw new Error("cargo build of watcher-core failed");
    }
  }
});

test("live feed: real watcher drives data-feed=live, matches /state, then goes down on kill", async ({
  page,
}) => {
  const port = await freePort();
  const watcher = spawnWatcher(port);
  try {
    await waitForRealState(port);

    await page.goto(`/?feed=http:http://127.0.0.1:${port}`);

    const shell = page.locator(".app-shell");
    await expect(shell).toHaveAttribute("data-feed", "live", { timeout: 15_000 });

    // At least one session sourced from the live watcher shows up — the
    // current local Claude session, inferred WORKING.
    const mirrorRows = page.locator("#scene-mirror div[data-station-id]");
    await expect(mirrorRows).not.toHaveCount(0, { timeout: 10_000 });

    const stateRes = await fetch(`http://127.0.0.1:${port}/state`);
    const liveState = await stateRes.json();
    expect(liveState.sessions.length).toBeGreaterThan(0);

    const firstId = liveState.sessions[0].id;
    const row = page.locator(`#scene-mirror div[data-station-id="${firstId}"]`);
    await expect(row).toHaveCount(1);
    await expect(row).toHaveAttribute("data-state", liveState.sessions[0].state);

    // Marquee PIPELINE / REMOTE ESTATE lines match /state's pipeline fields.
    const marqueeText = await page.locator(".marquee").innerText();
    console.log("MARQUEE WHILE LIVE:\n" + marqueeText);
    const expectedRemote =
      liveState.pipeline.remote_estate === "live"
        ? "LIVE"
        : liveState.pipeline.remote_estate === "degraded"
          ? "DEGRADED"
          : "NOT RUNNING";
    expect(marqueeText).toContain(`REMOTE ESTATE: ${expectedRemote}`);
    expect(marqueeText).toContain(liveState.pipeline.verified ? "PIPELINE: VERIFIED" : "PIPELINE: UNVERIFIED");

    await page.waitForTimeout(300);
    await page.screenshot({ path: "screenshots/live-floor.png", fullPage: false });

    // Kill the watcher; the app must go DOWN with the overlay within ~10s,
    // and no station may render as confidently WORKING (solid) any more.
    watcher.kill("SIGKILL");
    await expect(shell).toHaveAttribute("data-feed", "down", { timeout: 10_000 });
    await expect(page.locator('[data-testid="feed-overlay"]')).toBeVisible();
    await expect(page.locator('[data-testid="feed-overlay"]')).toContainText("WATCHER DOWN");

    const workingSolid = page.locator(
      '#scene-mirror div[data-state="working"][data-motion="solid"]'
    );
    await expect(workingSolid).toHaveCount(0);

    await page.waitForTimeout(300);
    await page.screenshot({ path: "screenshots/live-down.png", fullPage: false });
  } finally {
    watcher.kill("SIGKILL");
  }
});

test("http feed against a closed port goes down immediately, never falls back to a fixture", async ({
  page,
}) => {
  const closedPort = await freePort(); // allocated then released — nothing listens on it
  await page.goto(`/?feed=http:http://127.0.0.1:${closedPort}`);
  const shell = page.locator(".app-shell");
  await expect(shell).toHaveAttribute("data-feed", "down", { timeout: 5_000 });
  await expect(page.locator('[data-testid="feed-overlay"]')).toBeVisible();
  // No fixture chip, and no sessions rendered from a fixture.
  await expect(page.locator('[data-testid="feed-fixture-chip"]')).toHaveCount(0);
  await expect(page.locator("#scene-mirror div[data-station-id]")).toHaveCount(0);
});
