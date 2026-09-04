# THE FOUNDRY — Phase 3 Report: Truth Gate

Date: 2026-09-04. Stops here per the authorized scope — no Pixi/visual floor, no `.foundry` heartbeat instrumentation elsewhere, no project/bay/room resolution.

## 1. What's genuinely available (recap + confirmed live)

Everything in `PHASE1_REPORT.md` §1, confirmed a second time against fresh live data: session status buckets, `BREY_REQUIRED` (pending-permission) detection, configured/current/last-served model divergence, git-adjacent metadata, routine schedules with real next/last-fire timestamps, connection health distinguishing a disconnected bridge machine from a genuinely idle one. **Live count at demo time: 20 sessions observed across `anthropic_cloud` and `bridge`, 6 routines.**

## 2. What differed from the design doc's assumptions

Same deltas as Phase 1 (bridge sessions lack `context_usage`; `list_environments` doesn't enumerate bridge machines; session-bound routines have no `last_run.outcome`), plus one new one surfaced by building the real pipeline: **the Remote surface is only reachable via MCP tool-calling inside an authenticated Claude session — there is no documented public REST endpoint.** This is the single biggest open question before V1 can ship as a standalone always-on binary; see `watcher-core/src/observer.rs`'s module doc comment.

## 3. Watcher-runtime winner

**Rust**, decisively — see `PHASE1_REPORT.md` §2 for the measurements (2.4MB vs 43.5MB+ idle RSS, 508KB vs 95MB binary). Not revisited in Phase 3; the whole watcher core is built in Rust.

## 4. The adversarial review — and why this section matters most

Per the routing policy, an Opus-tier agent ran a targeted, read-only adversarial review of the entire Phase 2/3 build against one question: **can this dashboard still lie?** It found **six confirmed, reproducible ways it could** — not theoretical, each with a concrete repro. All six are now fixed and covered by new red-team tests. This is exactly what the truth gate exists to catch, and it caught real bugs my own testing had missed.

| # | Finding | Fix |
|---|---|---|
| 1 | The synthetic canary alone gated "pipeline verified" — a fully DOWN `remote_claude` observer with the in-process canary still ticking rendered `LIVE`. | `pipeline_verified` now also requires no real (non-canary) observer to be Down; the banner names which one broke it. |
| 2 | A missing/malformed `updated_at` defaulted `elapsed_ms` to 0 on **every** poll, so a session with no real timestamp could never go Hung — the staleness clock kept resetting to "just now" forever. | Missing/unparseable/skewed timestamps now propagate as `None` and the reducer only advances `last_activity_at` when it has real data — never resets it on an existing record just because we polled. |
| 3 | Partial capability loss (sessions gone, routines still fine) still read as `Healthy` because status was all-or-nothing. Separately, one malformed record in a session batch failed the *whole* parse, producing a fabricated "0 sessions" from a technically-successful poll. | Degradation is now capability-specific (losing `sessions` degrades only sessions, not routines, and vice versa); per-record tolerant parsing means one bad record no longer loses the batch; the renderer distinguishes a confirmed zero from "capability unavailable, count unknown." |
| 4 | Routines were never degraded at all — a 2-hour-dead trigger snapshot still rendered green `ON SCHEDULE`. Also, an unknown `enabled` field defaulted to `true`, contradicting the schema's own documented contract. | Routines now carry a `stale` flag driven by the same capability check, rendered as `[STALE]` and excluded from the confident overdue count; unknown-enabled now defaults to not-confidently-active. |
| 5 | A session that vanished from the snapshot was silently dropped from the list with no trace — §16 requires a 60s fading grace state, not instant disappearance. | Gone sessions render with a distinct `[FADING/ENDED]` tag for 60s, then fold into a footer count ("N session(s) ended this run") rather than vanishing. |
| 6 | **`redact.rs` was never called anywhere in the pipeline.** A secret, an absolute path, and an email embedded in a raw session title flowed straight to the rendered screen and would have hit the on-disk event log unredacted — plus confirmed false positives (session IDs getting redacted) and false negatives (Windows paths, macOS paths, AWS-style keys) in the regex set itself. | Redaction is now wired into the observer boundary for all free-text fields (session labels, routine names/prompts) and as a defense-in-depth pass over `--audit`'s raw-file preview. Fixed the confirmed false positive (known entity-ID prefixes are exempted) and false negatives (added Windows/macOS path patterns and an AWS-secret-key shape). |

Confirmed live after all six fixes: the redemonstration below shows real redaction firing (a home path and a branch name both got scrubbed), real stall-warnings appearing on the two genuinely-long-running WORKING sessions, and the overdue-routine count correctly reflecting real wall-clock time having passed a scheduled fire time.

**Residual, explicitly not fixed — flagged rather than silently left:**
- The redactor is a **first cut, not a bulletproof secret scanner.** It now over-redacts some non-secret strings (e.g. a git branch name containing digits can get caught by the generic high-entropy pattern) in exchange for fewer false negatives — a deliberate "better to over-redact a path than under-redact a key" tradeoff, but it is not comprehensive.
- `REVIEW_READY` still maps to `Idle`, which the reviewer flagged as potentially under-reporting attention for a session that's been awaiting review for hours. This is a genuine design question (what's the right visual weight for "there's a result ready to look at" vs. a true permission block?), not a bug — left for Brey rather than decided silently.
- `derive_stalls` only re-evaluates sessions the observer reports as `Working`; `Thinking`/`Specialist` can never go Hung today. Currently unreachable in practice (this observer never emits those states — only local/tool-level observers, Phase 2 V2 scope, would), so it's a latent gap, not a live one — tracked for when that observer lands.
- Raw provider enum *values* (e.g. the literal strings `"bridge"`/`"anthropic_cloud"`) still flow through as an opaque `session_kind` field value, even though the *field name* itself no longer leaks the Remote surface's vocabulary (a separate, earlier self-caught fix). Low severity — a location label, not a health signal.
- `--watch`'s poll loop has no jitter/backoff yet (§13) — deferred to the Hardening phase (§17 phase 9), not a truth-gate concern.

## 5. Proof, against the live estate

- **BREY_REQUIRED:** 3 real sessions correctly flagged, including one that has genuinely been waiting on Brey for **191+ hours** and another for **217+ hours** — exactly the kind of thing this dashboard exists to surface.
- **Stale/hung:** 2 real disconnected-bridge sessions correctly render `STALE/UNKNOWN`, never `IDLE`. Hung itself is proven by the `frozen_clock_working_session_renders_hung_not_still_fresh` red-team test (no live session happened to be actually hung at demo time — reported honestly rather than staged) — and the closely-related stall-warning mechanism fired for real on the two live `WORKING` sessions once they crossed the warning threshold.
- **Routines:** 6 real routines rendered — 2 correctly `DISABLED` (not counted as overdue), 1 correctly `OVERDUE` once real wall-clock time passed its scheduled fire, 3 `ON SCHEDULE`, "next routine" correctly sorted by actual `next_run_at` rather than iteration order.
- **Observer degradation:** proven by the new capability-specific red-team tests (partial loss, total loss, and the canary-alone gap), not just asserted.
- **Red-team suite:** 15 integration tests + 29 unit tests, all passing (44 total), covering every adversarial finding plus the original first-cut suite.

## 6. What prevents the graphical design from being truthful today

1. **The data-access bridge is still manual** (a Claude session refreshes `live-feed/` by hand) — not yet a standalone always-on process. This is the open architecture question from §1/§2, not resolved here.
2. **No local `~/.claude` observer yet** (V2 scope) — cloud/bridge sessions only get coarse, snapshot-level fidelity; no real tool-stroke animation data exists yet for the eventual floor to honestly show.
3. **No project/bay/room resolution** (§9, Phase 5) — the truth gate deliberately shows a flat session list, not grouped bays, so there's nothing yet to wire the floor's spatial layout to.
4. **The redactor's residual imperfection** (§4 above) means anything rendered on the eventual always-on wall display still needs the same "first cut, not bulletproof" caveat applied.

None of these block Phase 4 — they're the explicit next steps, not surprises.
