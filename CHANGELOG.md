# Journal des versions

Le format suit [Keep a Changelog](https://keepachangelog.com/fr/1.1.0/) ; le projet suit le [versionnage sémantique](https://semver.org/lang/fr/).

## [Non publié]

## [0.3.0] — 2026-10-12

Collecte plus économe et plus rapide, pivots à l'échelle du dossier. La montée depuis la 0.1 ou la 0.2 applique les migrations manquantes (0002 à 0004) ; répétée sur une copie restaurée de la base pilote, sans changement des données observées.

### Ajouté

- Collecte économe en quota GitHub, migration 0003 ([ADR 0005](docs/adr/0005-collecte-econome.md)) :
  - rythme partagé par ressource de quota, réglé sur les en-têtes `x-ratelimit-*` ;
  - requêtes conditionnelles (ETag) et cache HTTP cloisonné par identité d'accès, avec fenêtre de fraîcheur et purge ;
  - reprise d'un scan interrompu à sa page suivante ;
  - deux nouvelles tentatives au plus après une erreur réseau ou 5xx ;
  - compteurs `stats` dans la couverture de chaque scan.
- Commande `observatory github-status` : quotas de l'accès du worker, rythme partagé et cache, sans consommer de requête.
- Variables facultatives `GITHUB_MIN_INTERVAL_MS`, `GITHUB_CACHE_FRESHNESS_SECONDS` et `GITHUB_CACHE_RETENTION_DAYS`.
- Pivot compte → dépôts par l'API GraphQL lorsqu'un token est configuré : chaque page de 100 dépôts apporte aussi le dépôt d'origine des forks. Les nœuds GraphQL sont conservés tels que reçus comme preuves.
- `WORKER_CONCURRENCY` (2 par défaut, 8 au plus) : scans exécutés en même temps par un worker, sous le même rythme GitHub.
- Graphe cumulé d'un dossier (`GET /api/v1/investigations/{id}/graph`) et option « Tout le dossier » de l'exploration : chaque entité et relation avec sa dernière observation.
- Recherche par fragment de nom dans tous les dossiers de l'analyste (`GET /api/v1/search`), index trigrammes `pg_trgm` (migration 0004), depuis la barre latérale.
- Voisins communs de deux entités dans un dossier (`GET /api/v1/investigations/{id}/common`), depuis l'inspecteur.
- « Collecter ses relations » : lance un scan depuis l'inspecteur d'un compte ou d'un dépôt.
- Index des observations par entité et par relation, et des relations entrantes (migration 0004) : recoupements, vue du dossier et voisins communs sans parcours complet des tables.

### Modifié

- Pages de 100 éléments au lieu de 50 : deux fois plus de relations pour le même nombre de requêtes.
- Écart minimal entre deux requêtes ramené d'une seconde à 250 ms ; le rythme suit désormais l'état annoncé par GitHub.
- Une réponse 304 authentifiée n'est plus comptée dans le budget du scan.
- Écriture des observations par lots (REM-31) : une page de collecte ou un paquet de 1 000 lignes d'import en cinq requêtes au plus. Un instantané de 250 000 dépôts s'importe en 29 s au lieu de 207 s sur la machine de développement, avec les mêmes empreintes.

## [0.2.0] — 2026-10-11

### Modifié

- Backend réécrit en Rust (axum, sqlx, reqwest/rustls, tokio) : un binaire `observatory` avec les sous-commandes `serve`, `worker`, `migrate`, `create-user`, `demo` et `import-legacy`. Les routes et les réponses JSON de l'API sont inchangées ([ADR 0002](docs/adr/0002-backend-rust.md)).
- Les commandes d'administration deviennent `docker compose exec api observatory <commande>`.
- Image d'exécution Alpine 3.24 avec un binaire statique, sans interpréteur ; contrôle de santé par `wget`.
- Frontend découpé en composants et hooks, formaté par Prettier ; Cytoscape n'est chargé qu'à l'ouverture de la vue 2D.
- Le worker remet immédiatement en file le scan en cours lorsqu'il s'arrête, au lieu d'attendre l'expiration du lease. Une attente courte du quota partagé (≤ 5 s) se fait sur place, sans redémarrer le scan.

### Ajouté

- Migrations numérotées et vérifiées par empreinte ; refus de démarrer sur un schéma altéré, en retard ou plus récent ([ADR 0003](docs/adr/0003-migrations-versionnees.md)).
- Migration 0002 : échecs de connexion comptés par poste et par compte ([ADR 0004](docs/adr/0004-limitation-des-echecs-de-connexion.md)), index de recherche des preuves, observations qui ne recopient plus la preuve, index des notes, de l'audit et des sessions.
- Journal d'audit pour la création et l'annulation des scans et la création des notes.
- Seed acceptée sous la forme `github.com/compte/depot`, comme le suggère l'interface.
- CI GitHub : rustfmt, clippy pedantic, tests d'intégration PostgreSQL avec le rôle restreint et plancher de couverture, test de migration depuis le commit de base, ESLint, Prettier, Vitest, cargo-deny, npm audit, revue des dépendances, images, smoke test et parcours navigateur ; CodeQL (Rust, TypeScript, workflows) ; fuzzing cargo-fuzz ; Dependabot.
- `LICENSE` (Apache-2.0), `SECURITY.md`, `CHANGELOG.md`, `.env.example`, modèles d'issues et de pull requests.
- Feuille de route révisée après la 0.2 : file de jobs générique et restauration de la base pilote en R0, cadrage RGPD avant l'ouverture à l'équipe, champs mesurés sur le corpus réel, charges de R5 et R8 relevées, REM-27 à REM-31 ; `scripts/create-backlog-issues.sh` pour créer les issues du backlog.

### Corrigé

- Les flottants des preuves sont lus avec un arrondi exact, comme en Python, et un objet JSON ne peut plus être confondu avec un nombre par serde_json (deux problèmes trouvés par le fuzzing de la forme canonique).
- Une preuve rattachée seulement à une relation est consultable par les membres de l'investigation.
- Les caractères `%` et `_` saisis dans le filtre du graphe sont pris littéralement.
- Les erreurs de validation sont expliquées en français au lieu de « Requête incorrecte ».

### Retiré

- Backend Python (`investigator/`, `requirements/`, `pyproject.toml`) ; il reste disponible dans l'historique Git (commit `Socle 0.1 en Python`).

## [0.1.0] — 2026-10-11

Première tranche fonctionnelle en Python/FastAPI : comptes et droits par investigation, scans GitHub bornés, graphe 3D/2D, comparaison de scans, import historique. Voir [docs/IMPLEMENTATION.md](docs/IMPLEMENTATION.md).
