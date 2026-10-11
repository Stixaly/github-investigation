CREATE TABLE users (
  id uuid PRIMARY KEY, login text UNIQUE NOT NULL, password_hash text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE sessions (
  token_hash text PRIMARY KEY, user_id uuid NOT NULL REFERENCES users ON DELETE CASCADE,
  expires_at timestamptz NOT NULL
);
CREATE TABLE login_attempts (login text NOT NULL, attempted_at timestamptz NOT NULL DEFAULT now());
CREATE INDEX login_attempts_recent ON login_attempts(login, attempted_at);
CREATE TABLE investigations (
  id uuid PRIMARY KEY, title text NOT NULL, description text NOT NULL DEFAULT '',
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE memberships (
  investigation_id uuid REFERENCES investigations ON DELETE CASCADE,
  user_id uuid REFERENCES users ON DELETE CASCADE,
  role text NOT NULL CHECK (role IN ('owner','analyst','reader')),
  PRIMARY KEY(investigation_id, user_id)
);
CREATE TABLE scans (
  id uuid PRIMARY KEY, investigation_id uuid NOT NULL REFERENCES investigations,
  requested_by uuid NOT NULL REFERENCES users, source text NOT NULL CHECK(source IN ('github','legacy','demo')),
  seed text NOT NULL, status text NOT NULL CHECK(status IN ('queued','running','waiting_quota','succeeded','partial','failed','cancelled')),
  max_requests integer NOT NULL DEFAULT 12 CHECK(max_requests BETWEEN 1 AND 50),
  requests_used integer NOT NULL DEFAULT 0,
  requested_at timestamptz NOT NULL DEFAULT now(), started_at timestamptz, finished_at timestamptz,
  available_at timestamptz NOT NULL DEFAULT now(), lease_until timestamptz,
  generation integer NOT NULL DEFAULT 0, attempts integer NOT NULL DEFAULT 0,
  coverage jsonb NOT NULL DEFAULT '{}', error text,
  import_key text, idempotency_key uuid NOT NULL,
  UNIQUE(investigation_id,idempotency_key), UNIQUE(investigation_id,import_key)
);
CREATE INDEX scans_queue ON scans(available_at, requested_at) WHERE status IN ('queued','waiting_quota','running');
CREATE INDEX scans_investigation ON scans(investigation_id, requested_at DESC);
CREATE TABLE entities (
  id uuid PRIMARY KEY, kind text NOT NULL CHECK(kind IN ('Account','Repository','Issue','PullRequest')),
  external_id text NOT NULL, label text NOT NULL, provisional boolean NOT NULL DEFAULT false,
  UNIQUE(kind, external_id)
);
CREATE INDEX entities_label ON entities(lower(label) text_pattern_ops);
CREATE TABLE evidence (
  hash text PRIMARY KEY, payload jsonb NOT NULL, source text NOT NULL,
  recorded_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE scan_entities (
  scan_id uuid REFERENCES scans ON DELETE CASCADE, entity_id uuid REFERENCES entities,
  evidence_hash text NOT NULL REFERENCES evidence, data jsonb NOT NULL,
  observed_at timestamptz, time_precision text NOT NULL CHECK(time_precision IN ('instant','day','unknown')),
  recorded_at timestamptz NOT NULL DEFAULT now(), PRIMARY KEY(scan_id, entity_id, evidence_hash)
);
CREATE INDEX scan_entities_identity ON scan_entities(entity_id, scan_id);
CREATE TABLE relations (
  id uuid PRIMARY KEY, source_id uuid NOT NULL REFERENCES entities, target_id uuid NOT NULL REFERENCES entities,
  kind text NOT NULL CHECK(kind IN ('OWNS','FORK_OF','CONTRIBUTED_TO','AUTHORED','IN_REPOSITORY','STARRED','LEGACY_ROLE')),
  context text NOT NULL DEFAULT '', UNIQUE(source_id, kind, target_id, context)
);
CREATE INDEX relations_forward ON relations(source_id, kind, target_id);
CREATE INDEX relations_reverse ON relations(target_id, kind, source_id);
CREATE TABLE scan_relations (
  scan_id uuid REFERENCES scans ON DELETE CASCADE, relation_id uuid REFERENCES relations,
  evidence_hash text NOT NULL REFERENCES evidence, observed_at timestamptz,
  source_event_at timestamptz, time_precision text NOT NULL CHECK(time_precision IN ('instant','day','unknown')),
  recorded_at timestamptz NOT NULL DEFAULT now(), PRIMARY KEY(scan_id, relation_id, evidence_hash)
);
CREATE TABLE annotations (
  id uuid PRIMARY KEY, investigation_id uuid NOT NULL REFERENCES investigations,
  author_id uuid NOT NULL REFERENCES users, body text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE audit_events (
  id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY, actor_id uuid REFERENCES users,
  action text NOT NULL, investigation_id uuid REFERENCES investigations, created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE github_budget (
  id integer PRIMARY KEY CHECK(id=1), next_request_at timestamptz NOT NULL DEFAULT now()
);
INSERT INTO github_budget(id) VALUES(1);
