-- 0.4 : évaluation des listes de termes sur les observations d'un dossier, et triage.
-- Ne jamais modifier ce fichier une fois appliqué : ajouter une migration 0008.

-- Une évaluation lit les observations enregistrées avant son démarrage
-- (manifeste figé à `manifest_at`), par lots reprenables (`cursor`), avec une
-- version de liste et une version du moteur fixées.
CREATE TABLE rule_runs (
  id uuid PRIMARY KEY,
  investigation_id uuid NOT NULL REFERENCES investigations,
  list_id uuid NOT NULL,
  list_version integer NOT NULL,
  engine_version integer NOT NULL,
  requested_by uuid NOT NULL REFERENCES users,
  status text NOT NULL CHECK (status IN ('queued', 'running', 'succeeded', 'failed', 'cancelled')),
  manifest_at timestamptz,
  cursor uuid,
  counts jsonb NOT NULL DEFAULT '{}',
  error text,
  generation integer NOT NULL DEFAULT 0,
  lease_until timestamptz,
  requested_at timestamptz NOT NULL DEFAULT now(),
  started_at timestamptz,
  finished_at timestamptz,
  FOREIGN KEY (list_id, list_version) REFERENCES keyword_list_versions
);
CREATE INDEX rule_runs_pending ON rule_runs(requested_at) WHERE status IN ('queued', 'running');
CREATE INDEX rule_runs_investigation ON rule_runs(investigation_id, requested_at DESC);

-- Résultat expliqué d'une règle sur une entité : occurrences, exclusions,
-- champs non collectés, preuve et date de l'observation évaluée. Les absences
-- de correspondance sont seulement comptées dans l'évaluation.
CREATE TABLE rule_matches (
  run_id uuid NOT NULL REFERENCES rule_runs ON DELETE CASCADE,
  entity_id uuid NOT NULL REFERENCES entities,
  rule_id text NOT NULL,
  outcome text NOT NULL CHECK (outcome IN ('match', 'not_evaluated')),
  review_priority text NOT NULL,
  hits jsonb NOT NULL,
  exclusions jsonb NOT NULL,
  not_evaluated jsonb NOT NULL,
  evidence_hash text NOT NULL REFERENCES evidence,
  observed_at timestamptz,
  PRIMARY KEY (run_id, entity_id, rule_id)
);

-- File de triage : une tâche par dossier, entité, liste et règle. `signature`
-- résume les occurrences (champ, terme, extrait) : une occurrence inchangée ne
-- crée pas de nouvelle tâche, une occurrence différente rouvre une tâche close.
CREATE TABLE triage_items (
  id uuid PRIMARY KEY,
  investigation_id uuid NOT NULL REFERENCES investigations,
  entity_id uuid NOT NULL REFERENCES entities,
  list_id uuid NOT NULL REFERENCES keyword_lists,
  rule_id text NOT NULL,
  state text NOT NULL DEFAULT 'new' CHECK (state IN ('new', 'in_review', 'retained', 'dismissed')),
  signature text NOT NULL,
  evidence_hash text NOT NULL REFERENCES evidence,
  last_run_id uuid NOT NULL REFERENCES rule_runs,
  note text NOT NULL DEFAULT '',
  updated_by uuid REFERENCES users,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (investigation_id, entity_id, list_id, rule_id)
);
CREATE INDEX triage_items_queue ON triage_items(investigation_id, state, updated_at DESC);
