-- 0.3 : collecte plus économe en quota GitHub.
-- Ne jamais modifier ce fichier une fois appliqué : ajouter une migration 0004.

-- Quota GitHub par ressource (`core` pour REST, `search`, `graphql`), d'après
-- les en-têtes x-ratelimit-*. `remaining` est décrémenté à chaque réservation
-- puis corrigé par la réponse ; NULL tant que GitHub ne l'a pas annoncé.
-- `next_request_at` espace les requêtes de tous les workers.
CREATE TABLE github_rate_limits (
  resource text PRIMARY KEY,
  next_request_at timestamptz NOT NULL DEFAULT now(),
  remaining integer CHECK(remaining >= 0),
  quota integer CHECK(quota > 0),
  reset_at timestamptz,
  updated_at timestamptz NOT NULL DEFAULT now()
);
INSERT INTO github_rate_limits(resource, next_request_at)
SELECT 'core', next_request_at FROM github_budget WHERE id = 1;
INSERT INTO github_rate_limits(resource) VALUES ('core'), ('search'), ('graphql') ON CONFLICT DO NOTHING;
-- github_budget n'est plus lue. Elle sera supprimée par une migration
-- ultérieure : les suppressions se font en deux temps (ADR 0003).

-- Réponses GitHub réutilisables : requête conditionnelle (ETag ; une réponse
-- 304 authentifiée n'est pas décomptée du quota) ou réutilisation directe
-- pendant la fenêtre de fraîcheur. Cloisonnées par identité d'accès (anonyme
-- ou empreinte du token, jamais le token) et par représentation, purgées par
-- le worker après la durée de rétention. Le corps est gardé tel que sérialisé
-- par la collecte, pour que les empreintes de preuve restent identiques.
CREATE TABLE http_cache (
  url text NOT NULL,
  identity text NOT NULL,
  representation text NOT NULL,
  etag text,
  body text NOT NULL,
  link text NOT NULL DEFAULT '',
  fetched_at timestamptz NOT NULL DEFAULT now(),
  validated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY(url, identity, representation)
);
CREATE INDEX http_cache_validated ON http_cache(validated_at);

-- Point de reprise par étape d'un scan (la seed, puis chaque endpoint), écrit
-- dans la transaction qui enregistre la page : un scan repris après une
-- attente de quota ou un arrêt du worker repart de la page suivante.
-- `state` porte la couverture de l'étape ; `done` est posé avec la couverture
-- finale, dans la même transaction.
CREATE TABLE scan_checkpoints (
  scan_id uuid NOT NULL REFERENCES scans ON DELETE CASCADE,
  step text NOT NULL,
  next_url text,
  state jsonb NOT NULL DEFAULT '{}',
  done boolean NOT NULL DEFAULT false,
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY(scan_id, step)
);
