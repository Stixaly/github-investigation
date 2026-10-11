# Journal des versions

Le format suit [Keep a Changelog](https://keepachangelog.com/fr/1.1.0/) ; le projet suit le [versionnage sémantique](https://semver.org/lang/fr/).

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

- Les nombres JSON des preuves sont conservés à l'identique, y compris les grands entiers et les flottants (problème trouvé par le fuzzing de la forme canonique).
- Une preuve rattachée seulement à une relation est consultable par les membres de l'investigation.
- Les caractères `%` et `_` saisis dans le filtre du graphe sont pris littéralement.
- Les erreurs de validation sont expliquées en français au lieu de « Requête incorrecte ».

### Retiré

- Backend Python (`investigator/`, `requirements/`, `pyproject.toml`) ; il reste disponible dans l'historique Git (commit `Socle 0.1 en Python`).

## [0.1.0] — 2026-10-11

Première tranche fonctionnelle en Python/FastAPI : comptes et droits par investigation, scans GitHub bornés, graphe 3D/2D, comparaison de scans, import historique. Voir [docs/IMPLEMENTATION.md](docs/IMPLEMENTATION.md).
