import type { FloorState, StationState } from "./state";
import { isOpusModel } from "./state";
import { MODE_LABEL, type Mode } from "./modes";

function fmtAge(secs: number | null): string {
  if (secs === null) return "n/a";
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) return `${Math.round(secs / 60)}m`;
  return `${Math.round(secs / 3600)}h`;
}

function cls(nonZero: boolean, attention: boolean): string {
  if (!nonZero) return "marquee-count marquee-dim";
  return attention ? "marquee-count marquee-attention" : "marquee-count marquee-bright";
}

export function Marquee({ state, mode }: { state: FloorState; mode?: Mode }) {
  const counts: Partial<Record<StationState, number>> = {};
  for (const s of state.sessions) {
    counts[s.state] = (counts[s.state] ?? 0) + 1;
  }
  const breyCount = counts.brey_required ?? 0;
  const failedCount = counts.failed ?? 0;
  const hungCount = counts.hung ?? 0;
  const workingCount = counts.working ?? 0;
  const waitingCount =
    (counts.waiting_on_agent ?? 0) + (counts.waiting_on_system ?? 0) + (counts.blocked ?? 0);
  const staleCount = counts.stale_unknown ?? 0;
  const opusCount = state.sessions.filter((s) => isOpusModel(s.model_current ?? s.model)).length;

  const breyLabels = state.sessions
    .filter((s) => s.state === "brey_required")
    .map((s) => s.label)
    .join(", ");

  const pipeline = state.pipeline;
  const blind = !pipeline.verified || pipeline.remote_estate !== "live";

  return (
    <div className={`marquee ${blind ? "marquee-blind" : ""}`}>
      <div className="marquee-row marquee-counts">
        <span className={`marquee-brey ${breyCount === 0 ? "marquee-dim" : ""}`}>
          {breyCount} BREY REQUIRED{breyCount > 0 ? ` — ${breyLabels}` : ""}
        </span>
        <span className={cls(failedCount > 0, true)}>{failedCount} FAILED</span>
        <span className={cls(hungCount > 0, true)}>{hungCount} HUNG</span>
        <span className={cls(workingCount > 0, false)}>{workingCount} WORKING</span>
        <span className={cls(waitingCount > 0, false)}>{waitingCount} WAITING</span>
        <span className={cls(staleCount > 0, true)}>{staleCount} STALE</span>
        <span className={cls(opusCount > 0, false)}>{opusCount} OPUS</span>
        {mode && (
          <span className="marquee-mode" data-testid="mode-indicator">
            MODE: {MODE_LABEL[mode]}
          </span>
        )}
      </div>
      <div className="marquee-row marquee-status">
        <span>LAST OUTPUT {fmtAge(pipeline.last_output_age_secs)}</span>
        <span>
          NEXT ROUTINE {pipeline.next_routine ?? "n/a"}
        </span>
        <span>
          REMOTE ESTATE:{" "}
          {pipeline.remote_estate === "live"
            ? "LIVE"
            : pipeline.remote_estate === "degraded"
              ? `DEGRADED (${fmtAge(pipeline.last_sync_age_secs)})`
              : "NOT RUNNING"}
        </span>
        <span>LAST SYNC {fmtAge(pipeline.last_sync_age_secs)}</span>
        <span className={pipeline.verified ? "pipeline-ok" : "pipeline-bad"}>
          PIPELINE: {pipeline.verified ? "VERIFIED" : "UNVERIFIED"}
        </span>
      </div>
    </div>
  );
}
