# THE FOUNDRY — Dedicated Repository Migration Plan

Status: **designed and locally verified, not executed.** No new GitHub repository has been created. PR #1 on `aisportsanalysis` remains unmerged and untouched.

## Recommended method: `git subtree split`

Evaluated three approaches:

| Method | History preserved? | Risk | Notes |
|---|---|---|---|
| **`git subtree split`** | Yes — real commit history for every commit that touched `ops-room/`, correctly re-rooted | None — purely additive, creates a new local branch, never modifies existing branches/commits | **Recommended.** |
| Cherry-pick into a fresh repo | Partial — recreates commits with new hashes/dates, easy to drop context, tedious for 3+ commits and any future ones | Low, but manual and error-prone; provenance (original commit messages/order) can drift | Only worth it for a one-off single-commit move. |
| `git filter-repo` / `filter-branch` on a full clone | Yes, and more powerful (can also rewrite the *original* repo to remove the extracted paths) | **Higher** — typically used to rewrite a whole repo's history, easy to misuse destructively if pointed at the wrong branch/remote | Overkill here; `subtree split` does the one thing needed (extract, don't rewrite the source) with far less blast radius. |

`git subtree split` is the standard, purpose-built tool for exactly this: "take this subdirectory's history and hand me a branch as if it had always been its own repo," without touching anything else.

## What was actually done (safe, local, reversible)

```bash
git subtree split --prefix=ops-room -b foundry-extracted
```

Verified:
1. **Original branch is completely untouched** — same HEAD commit hash before and after (`93b3553`).
2. **`foundry-extracted` carries real history** — all 3 commits that ever touched `ops-room/`, with their original messages and content, re-rooted so `ops-room/watcher-core/` becomes `watcher-core/` etc.
3. **The extracted tree actually builds and passes all 49 tests**, checked out standalone in a separate worktree (`git worktree add`, now removed — the branch itself remains in this local clone).

This exists only in this session's local clone right now. Nothing has been pushed anywhere; no new GitHub repository exists yet.

## Remaining steps (require Brey's go-ahead before executing)

1. **Confirm the repository name.** You floated `the-foundry` or `claude-foundry` as tentative — pick one (or a different name) before anything is created, since repo creation/naming is the one step here that isn't cleanly reversible without leaving a stale placeholder behind.
2. Create the new GitHub repository (empty, private by default — confirm visibility too).
3. `git remote add foundry-origin <new-repo-url>` (in a scratch clone, not this session's working copy) and `git push foundry-origin foundry-extracted:main`.
4. Verify on GitHub: commit history intact, files build, README reads correctly as a standalone project (it already documents itself as standalone — see `ops-room/README.md`).
5. **Decide what happens to PR #1 on `aisportsanalysis`.** Per your instruction it stays unmerged until the new repo exists and is verified — once it is, the natural options are: close PR #1 without merging (recommended — the work now lives in its real home), or leave it open as a historical record. Your call, not mine to make unilaterally.
6. Optionally, once the new repo is confirmed working, remove `ops-room/` from `aisportsanalysis`'s branch (a separate, explicit commit) so the sports repo doesn't carry a stale, superseded copy — only after step 5 is settled.
7. Local organization on your machine: clone the new repo to `C:\Users\KC\Projects\infrastructure\the-foundry` (or whatever path/name you settle on) — outside this session's control, your own machine.

Nothing in steps 2-6 has been executed. Say the word (and the name) and step 2 onward is a five-minute job from here.
