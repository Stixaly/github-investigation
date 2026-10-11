-- 0.2 : durcissement après la revue du socle 0.1.
-- Ne jamais modifier ce fichier une fois appliqué : ajouter une migration 0003.

-- Les échecs de connexion sont comptés par compte ET par client, pour qu'un
-- tiers ne puisse plus bloquer un analyste avec cinq mauvais mots de passe.
ALTER TABLE login_attempts ADD COLUMN client text NOT NULL DEFAULT '';
DROP INDEX login_attempts_recent;
CREATE INDEX login_attempts_recent ON login_attempts(login, client, attempted_at);

-- Le contrôle d'accès d'une preuve la cherche par empreinte dans les
-- observations d'entités et de relations : sans index, chaque consultation
-- parcourait les deux tables.
CREATE INDEX scan_entities_evidence ON scan_entities(evidence_hash);
CREATE INDEX scan_relations_evidence ON scan_relations(evidence_hash);

-- Une observation ne recopie plus la réponse GitHub quand elle est identique à
-- la preuve : NULL signifie « voir evidence.payload ». Les lignes existantes
-- sont compactées ; les vues lisent coalesce(scan_entities.data, evidence.payload).
ALTER TABLE scan_entities ALTER COLUMN data DROP NOT NULL;
UPDATE scan_entities se SET data = NULL
FROM evidence e
WHERE e.hash = se.evidence_hash AND se.data = e.payload;

CREATE INDEX annotations_investigation ON annotations(investigation_id, created_at DESC);
CREATE INDEX audit_events_investigation ON audit_events(investigation_id, created_at DESC);
CREATE INDEX sessions_expiry ON sessions(expires_at);
