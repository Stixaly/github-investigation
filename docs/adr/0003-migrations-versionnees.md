# ADR 0003 — Migrations numérotées et rôle applicatif sans DDL

Statut : appliqué en 0.2, le 11 octobre 2026.

## Contexte

La 0.1 vérifiait une unique version 1 du schéma par empreinte et refusait toute autre version : la documentation demandait d'« ajouter une migration numérotée », mais le mécanisme ne le permettait pas. CTIParsor a fait le même constat (ADR-0077 de CTIParsor) et versionne désormais ses migrations, avec un test de migration depuis le commit de base.

## Décision

- Les migrations sont des fichiers SQL `migrations/NNNN_nom.sql`, inclus dans le binaire à la compilation, numérotés sans trou à partir de 1.
- `observatory migrate`, exécuté par le service ponctuel `migrate` avec le compte administrateur PostgreSQL, prend un verrou consultatif puis applique chaque migration manquante dans sa propre transaction et enregistre `version`, SHA-256 (fins de ligne normalisées en LF) et date.
- La table `schema_migrations` est celle de la 0.1, complétée d'une colonne `applied_at`. La migration 0001 est `schema.sql` à l'octet près, si bien qu'une base 0.1 est reconnue et passe directement à la 0002.
- Au démarrage, l'API et le worker exigent un historique identique à celui du binaire. Une migration modifiée après application, une version inconnue (base plus récente que le binaire) ou une migration manquante arrêtent le démarrage avec un message explicite.
- Le rôle `investigator` de l'API et du worker lit et écrit les données mais ne possède aucune table et ne peut ni créer, ni modifier, ni supprimer de table, ni écrire dans `schema_migrations`. Les tests d'intégration utilisent ce rôle, pas le superutilisateur.
- Une migration appliquée ne se modifie jamais : on en ajoute une nouvelle.

## Vérifications

- Tests d'intégration : base neuve ; base créée exactement comme par la 0.1 puis migrée, avec relecture par l'API ; historique altéré ; version inconnue ; schéma en retard ; droits du rôle applicatif.
- CI, sur chaque pull request : `scripts/check-migration-from-base.sh` construit le binaire du commit de base, crée une base et des données avec lui, puis la migre avec le binaire de la PR et compare les comptes de lignes.

## Conséquences

Une évolution du modèle coûte une migration et un test d'upgrade, mais aucune base pilote n'est plus bloquée par une modification du schéma. Les migrations destructrices (suppression de colonne) devront être faites en deux temps : ne plus lire la colonne, puis la supprimer dans une version ultérieure.
