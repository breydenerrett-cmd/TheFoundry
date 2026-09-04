// Mirrors watcher-core/src/schema.rs, reducer.rs and health.rs.
// This is the ONLY vocabulary the renderer is allowed to know about.
// V-01 fixture-driven: no fields here that the real StateStore couldn't supply.

export type Fidelity = "observed" | "inferred" | "unknown";

// The twelve §6 states + the fading/ended tail state used by the floor to
// give a 60s grace window before a gone session folds away (see reducer.rs
// GONE_FADE_SECS). "stale_unknown" also covers a restored-but-not-yet-fresh
// record (SessionRecord.restored).
export type StationState =
  | "working"
  | "thinking"
  | "specialist"
  | "waiting_on_agent"
  | "waiting_on_system"
  | "blocked"
  | "brey_required"
  | "failed"
  | "hung"
  | "idle"
  | "completed"
  | "stale_unknown"
  | "fading_ended";

export const ATTENTION_STATES: ReadonlySet<StationState> = new Set([
  "brey_required",
  "failed",
  "hung",
  "stale_unknown",
]);

export type BayName =
  | "SPORTS LAB"
  | "AI BUSINESS COMPLEX"
  | "SERVERFORGE"
  | "MUSIC LAB"
  | "EXPERIMENTS"
  | "PERSONAL/MISC"
  | "UNRESOLVED";

export const BAYS: readonly BayName[] = [
  "SPORTS LAB",
  "AI BUSINESS COMPLEX",
  "SERVERFORGE",
  "MUSIC LAB",
  "EXPERIMENTS",
  "PERSONAL/MISC",
  "UNRESOLVED",
];

/** §5a output-velocity artifact tokens shown on a bay's OUTPUT SHELF. */
export type ArtifactKind = "commit" | "test" | "deploy" | "capture" | "completed";

export interface OutputShelfSlot {
  kind: ArtifactKind;
  /** undefined/null slot = no instrumentation for this artifact type at all
   *  (must render hatched/empty, never as a confident zero). */
  count: number | null;
  fidelity: Fidelity;
}

/** Mirrors reducer.rs SessionRecord, flattened for the renderer. */
export interface SessionRecord {
  id: string;
  bay: BayName;
  state: StationState;
  fidelity: Fidelity;
  model: string | null;
  model_current: string | null;
  /** Elapsed time in the current state, seconds. */
  elapsed_secs: number;
  label: string;
  /** True when this record's values came from a loaded snapshot, not a
   *  live observation in this process — must render as STALE regardless
   *  of `state` (reducer.rs SessionRecord.restored). */
  restored?: boolean;
  session_kind?: string | null;
}

/** Mirrors reducer.rs RoutineRecord. */
export interface RoutineRecord {
  id: string;
  name: string;
  bay: BayName;
  enabled: boolean;
  overdue: boolean;
  next_run_at: string | null;
  /** True when the routines capability was missing on the most recent poll
   *  — fields are frozen/untrustworthy (reducer.rs RoutineRecord.stale). */
  stale: boolean;
  restored?: boolean;
}

/** Mirrors reducer.rs CheckRecord (§1a EQUIPMENT — deterministic, no
 *  reasoning loop, so no StationState — just an observed label). */
export interface CheckRecord {
  id: string;
  label: string;
  bay: BayName;
  ok: boolean | null; // null = unknown/unobserved
  last_event_ts: string | null;
  restored?: boolean;
}

export type ObserverStatus = "healthy" | "degraded" | "down" | "unverified";

/** Mirrors health.rs ObserverHealth. */
export interface ObserverHealth {
  name: string;
  status: ObserverStatus;
  last_success_at: string | null;
  last_error: string | null;
  consecutive_failures: number;
}

export type RemoteEstate = "live" | "degraded" | "not_running";

/** Mirrors reducer.rs PipelineHealth + render.rs's pipeline_verified /
 *  remote-estate summary line — the marquee's core truth-gate fields. */
export interface PipelineSummary {
  verified: boolean;
  remote_estate: RemoteEstate;
  last_sync_age_secs: number | null;
  last_output_age_secs: number | null;
  next_routine: string | null;
}

/** Top-level fixture / live-feed document shape. */
export interface FloorState {
  generated_at: string;
  sessions: SessionRecord[];
  routines: RoutineRecord[];
  checks: CheckRecord[];
  observers: ObserverHealth[];
  pipeline: PipelineSummary;
  output_shelf: Record<BayName, OutputShelfSlot[]>;
}
