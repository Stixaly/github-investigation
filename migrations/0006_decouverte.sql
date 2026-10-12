-- 0.4 : découverte GitHub par recherche de dépôts ou de code.
-- Ne jamais modifier ce fichier une fois appliqué : ajouter une migration 0007.

-- Une découverte est un scan de source « search » : même file, mêmes droits,
-- mêmes points de reprise. Les dépôts trouvés sont ses observations ; chaque
-- requête qui les a trouvés reste leur preuve.
ALTER TABLE scans DROP CONSTRAINT scans_source_check;
ALTER TABLE scans ADD CONSTRAINT scans_source_check CHECK (source IN ('github', 'legacy', 'demo', 'search'));

-- Plan de requêtes compilé et affiché avant le lancement, propre aux découvertes.
ALTER TABLE scans ADD COLUMN plan jsonb;
ALTER TABLE scans ADD CONSTRAINT scans_search_plan CHECK ((source = 'search') = (plan IS NOT NULL));

-- Quota de la recherche de code (10 requêtes par minute avec un token).
INSERT INTO github_rate_limits(resource) VALUES ('code_search') ON CONFLICT DO NOTHING;
