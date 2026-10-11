// Shapes returned by the Observatory API (crates/observatory/src/api).

export type User = { id: string; login: string; csrf: string };

export type Role = 'owner' | 'analyst' | 'reader';

export type Investigation = {
  id: string;
  title: string;
  description: string;
  role: Role;
  scan_count: number;
};

export type ScanStatus =
  'queued' | 'running' | 'waiting_quota' | 'succeeded' | 'partial' | 'failed' | 'cancelled';

export type Coverage = Record<string, { status: string; reason?: string; items?: number; pages?: number }>;

export type Scan = {
  id: string;
  seed: string;
  source: string;
  status: ScanStatus;
  requested_at: string;
  coverage: Coverage;
  error?: string | null;
  requests_used: number;
  max_requests: number;
};

export type Note = { id: string; body: string; login: string; created_at: string };

export type Member = { login: string; role: Role };

export type Overlap = { id: string; title: string };

export type Comparison = {
  newly_observed: number;
  not_reobserved: number;
  shared: number;
  notice: string;
};

export type Node = {
  id: string;
  kind: string;
  label: string;
  evidence_hash: string;
  observed_at: string | null;
  time_precision: string;
  provisional: boolean;
  data: Record<string, unknown>;
};

export type Edge = {
  id: string;
  source_id: string;
  target_id: string;
  kind: string;
  evidence_hash: string;
  observed_at: string | null;
  source_event_at?: string | null;
  time_precision: string;
};

export type GraphData = { nodes: Node[]; edges: Edge[]; truncated: boolean; notice: string };

/** Entity found by name, with the analyst's investigations that observed it. */
export type Found = { id: string; kind: string; label: string; investigations: Overlap[] };

export type CommonNeighbour = { id: string; kind: string; label: string; with_a: string[]; with_b: string[] };

export type CommonNeighbours = { a: string; b: string; common: CommonNeighbour[]; notice: string };

/** View of the exploration tab that gathers every scan of the investigation. */
export const ALL_SCANS = 'all';

/** Pending scans may still consume the GitHub budget and can be cancelled. */
export const PENDING: ScanStatus[] = ['queued', 'running', 'waiting_quota'];
