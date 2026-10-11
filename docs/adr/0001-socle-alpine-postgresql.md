# ADR 0001 — Socle Alpine et graphe explicite PostgreSQL

Statut : appliqué au socle 0.1, le 11 octobre 2026.

## Contexte

L'équipe souhaite explorer des relations GitHub publiques, conserver les scans dans le temps et recouper des investigations confidentielles. Tous les services doivent fonctionner sous Alpine. La familiarité avec CTIParsor favorise Python/FastAPI, React/TypeScript et PostgreSQL.

## Décision

PostgreSQL est la source canonique : entités et relations partagées, preuves JSONB identifiées par hash, observations liées à un scan et appartenances aux investigations. Les index relationnels permettent le voisinage et le recoupement initial sans moteur graphe séparé. La file de jobs et le budget GitHub utilisent aussi PostgreSQL.

L'API et le worker utilisent la même image Python Alpine, avec des processus et des secrets séparés. Le frontend est compilé par Node Alpine puis servi statiquement par l'API. Un gateway Nginx Alpine termine HTTPS pour les accès d'équipe. Les services persistants tournent sans root, sans capabilities et avec un système de fichiers racine en lecture seule.

La première version expose des observations par scan. Elle conserve assez de provenance pour construire ensuite des vues temporelles plus riches, mais ne prétend pas déjà fournir un graphe bitemporel consolidé. Les preuves ne servent jamais de raccourci pour contourner les droits d'une investigation.

## Conséquences

Moins de services à exploiter et des transactions cohérentes entre preuve, observation et job. Les requêtes de voisinage sont explicitement bornées. PostgreSQL reste un composant central à superviser, sauvegarder et dimensionner.

Elasticsearch, un moteur graphe spécialisé et des workers d'analytics pourront être évalués sur des mesures réelles. Toute nouvelle image devra respecter Alpine ; tout index dérivé devra pouvoir être reconstruit à partir du corpus canonique. Le fait de découvrir une communauté dans un graphe ne constitue pas une attribution malveillante.
