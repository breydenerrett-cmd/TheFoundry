# THE FOUNDRY — V-05 perf results

Measured in this environment: headless `chromium headless_shell`,
**software rendering (no GPU)**, single sample run of 6000ms per
combo after a 500ms ladder-settle warmup. These numbers characterize the
software-rendered floor in this sandbox only — GPU-backed numbers on real
hardware will be materially better (lower busy%, likely full fps on all
steps including DEEP DEBUG 60fps, which isn't gated here).

| fixture | mode | fps | main-thread busy | JS heap | particles/frame | frames sampled | gate |
|---|---|---|---|---|---|---|---|
| floor-idle.json | command | 8.1 | 0.1% | 9.5 MB | 0 | 49 | PASS — busy<=25% (ok) |
| floor-idle.json | ambient | 8.2 | 0.1% | 9.5 MB | 0 | 50 | PASS — fps<=~12 (ok), busy<=8% (ok) |
| floor.json | command | 4.5 | 0.1% | 9.5 MB | 19 | 27 | PASS — busy<=25% (ok) |
| floor.json | ambient | 4.5 | 0.1% | 9.5 MB | 0 | 28 | PASS — fps<=~12 (ok), busy<=8% (ok) |

Gates (per mission spec):
- AMBIENT: achieved fps at/near the 12fps ladder target, main-thread busy <= 8% of wall time.
- COMMAND CENTER: main-thread busy <= 25% of wall time.

`app/perf/results.json` carries the raw numbers this table was generated from.
