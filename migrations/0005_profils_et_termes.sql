-- 0.4 : profils de collecte et listes de termes, versionnés et immuables.
-- Ne jamais modifier ce fichier une fois appliqué : ajouter une migration 0006.

-- Une version publiée ne change plus : la modifier, c'est en publier une autre.
CREATE FUNCTION forbid_published_change() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION '% : une version publiée est immuable', TG_TABLE_NAME;
END $$;

-- Profils de collecte : réglages d'un lancement (relations, pages, budgets).
-- Un scan garde le nom, la version et une copie des réglages de son profil.
CREATE TABLE collection_profiles (
  name text NOT NULL CHECK (name ~ '^[a-z][a-z0-9-]{0,39}$'),
  version integer NOT NULL CHECK (version > 0),
  label text NOT NULL,
  description text NOT NULL DEFAULT '',
  settings jsonb NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (name, version)
);
CREATE TRIGGER collection_profiles_immutable BEFORE UPDATE OR DELETE ON collection_profiles
  FOR EACH ROW EXECUTE FUNCTION forbid_published_change();

INSERT INTO collection_profiles(name, version, label, description, settings) VALUES
  ('apercu', 1, 'Aperçu',
   'La seed et une page de contributeurs, de forks ou de dépôts : un premier regard à faible coût.',
   '{"relations": ["contributors", "forks", "repositories"], "max_pages": 1, "default_requests": 6, "max_requests": 12}'),
  ('reseau', 1, 'Réseau',
   'Contributeurs, forks, issues, événements ou dépôts, deux pages chacun : la collecte de la 0.3.',
   '{"relations": ["contributors", "forks", "issues", "events", "repositories"], "max_pages": 2, "default_requests": 12, "max_requests": 50}');

-- Les scans antérieurs restent sans profil : ils ont été collectés comme « reseau » v1.
ALTER TABLE scans
  ADD COLUMN profile_name text,
  ADD COLUMN profile_version integer,
  ADD COLUMN profile jsonb,
  ADD CONSTRAINT scans_profile FOREIGN KEY (profile_name, profile_version) REFERENCES collection_profiles;

-- Listes de termes d'un dossier. Le serveur ne garde que les versions publiées ;
-- le brouillon vit dans l'éditeur jusqu'à sa publication.
CREATE TABLE keyword_lists (
  id uuid PRIMARY KEY,
  investigation_id uuid NOT NULL REFERENCES investigations,
  name text NOT NULL,
  created_by uuid NOT NULL REFERENCES users,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (investigation_id, name)
);
CREATE TABLE keyword_list_versions (
  list_id uuid NOT NULL REFERENCES keyword_lists,
  version integer NOT NULL CHECK (version > 0),
  definition jsonb NOT NULL,
  -- Version du moteur qui a validé la liste et exécuté ses tests.
  engine_version integer NOT NULL,
  published_by uuid NOT NULL REFERENCES users,
  published_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (list_id, version)
);
CREATE TRIGGER keyword_list_versions_immutable BEFORE UPDATE OR DELETE ON keyword_list_versions
  FOR EACH ROW EXECUTE FUNCTION forbid_published_change();
