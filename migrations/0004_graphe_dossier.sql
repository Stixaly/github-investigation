-- 0.3 : graphe à l'échelle du dossier, recherche transversale et voisins communs.
-- Ne jamais modifier ce fichier une fois appliqué : ajouter une migration 0005.

-- Recherche par fragment de nom (`ILIKE '%fragment%'`) et tri par
-- ressemblance : pg_trgm est une extension de confiance de PostgreSQL.
CREATE EXTENSION IF NOT EXISTS pg_trgm;
CREATE INDEX entities_label_trgm ON entities USING gin (label gin_trgm_ops);

-- Parcours depuis une entité ou une relation vers les scans qui l'ont
-- observée : recoupements, graphe du dossier, voisins communs. Les clés
-- primaires commencent par scan_id et ne servent pas ces lectures.
CREATE INDEX scan_entities_entity ON scan_entities(entity_id, scan_id);
CREATE INDEX scan_relations_relation ON scan_relations(relation_id, scan_id);
-- Relations entrantes d'une entité ; les sortantes sont servies par l'unicité (source_id, ...).
CREATE INDEX relations_target ON relations(target_id);
