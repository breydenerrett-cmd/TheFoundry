import type { FloorState, StationState } from "./state";
import { STATE_LABEL } from "./theme";

function fmtAge(secs: number | null): string {
  if (secs === null) return "n/a";
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) return `${Math.round(secs / 60)}m`;
  return `${Math.round(secs / 3600)}h`;
}

export function Marquee({ state }: { state: FloorState }) {
  const counts: Partial<Record<StationState, number>> = {};
  for (const s of state.sessions) {
    counts[s.state] = (counts[s.state] ?? 0) + 1;
  }
  const breyCount = counts.brey_required ?? 0;
  const pipeline = state.pipeline;
  const blind = !pipeline.verified || pipeline.remote_estate !== "live";

  const orderedStates: StationState[] = [
    "brey_required",
    "failed",
    "hung",
    "stale_unknown",
    "working",
    "thinking",
    "specialist",
    "waiting_on_agent",
    "waiting_on_system",
    "blocked",
    "idle",
    "completed",
    "fading_ended",
  ];

  return (
    <div className={`marquee ${blind ? "marquee-blind" : ""}`}>
      <div className="marquee-row marquee-counts">
        <span className="marquee-brey">
          {breyCount} BREY REQUIRED
        </span>
        {orderedStates
          .filter((s) => s !== "brey_required" && counts[s])
          .map((s) => (
            <span key={s} className="marquee-count">
              {counts[s]} {STATE_LABEL[s]}
            </span>
          ))}
      </div>
      <div className="marquee-row marquee-status">
        <span>LAST OUTPUT {fmtAge(pipeline.last_output_age_secs)}</span>
        <span>NEXT ROUTINE {pipeline.next_routine ?? "n/a"}</span>
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
