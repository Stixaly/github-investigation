# Alignement avec les méthodes de CTIParsor

CTIParsor est l'outil d'extraction CTI de la même équipe. Ses pratiques de développement servent de référence à ce projet, adaptées à deux contraintes propres : **Alpine sur tous les services** et un backend **Rust** depuis la 0.2 ([ADR 0002](adr/0002-backend-rust.md)).

Première lecture : checkout local de CTIParsor du 11 octobre 2026 (révision `dcd685e`). Mise à jour le même jour, lors de la revue qui a conduit à la 0.2 (révision `59bf56f`). Les réglages GitHub distants de CTIParsor n'ont pas été vérifiés.

## 1. Pratiques reprises

| Dans CTIParsor | Dans ce projet (0.2) | État |
|---|---|---|
| Développement documenté sous Docker | Tests et images sous Alpine (`make test`), mêmes contrôles en local (`make check`) | Fait |
| Travail par branche et pull request, CI bloquante (ADR-0071) | Dépôt Git, CI bloquante sur chaque PR, aucune vérification déterministe en simple avertissement | Fait ; protection de `main` à activer sur GitHub |
| API, worker et file PostgreSQL séparés | Binaire unique, conteneurs `api`, `worker` et `migrate` distincts, file `SKIP LOCKED` | Fait |
| Lease, heartbeat et reprise de jobs | Lease prolongé à chaque étape, génération de fencing, remise en file à l'arrêt du worker | Fait |
| Ruff (E, F, W, I, S, B), mypy | rustfmt, clippy `all` + `pedantic` sans avertissement, `unsafe` interdit, typage du compilateur | Fait (équivalent Rust) |
| ESLint, TypeScript, Vitest | ESLint (`typescript-eslint` + hooks React), TypeScript strict, Vitest/Testing Library, mêmes versions d'outils | Fait |
| Couverture de branches, plancher global de 87 % | Couverture de lignes avec `cargo llvm-cov`, plancher en CI fixé juste sous la mesure | Fait ; couverture de branches Rust encore instable dans l'outillage |
| Tests PostgreSQL isolés, services externes simulés | Une base par test, rôle applicatif restreint, API GitHub simulée par wiremock | Fait |
| Locks avec hashes (ADR-0080), images épinglées | `Cargo.lock` et `--locked`, lock npm exact, images par digest | Fait |
| pip-audit, npm audit, dependency review | cargo-deny (vulnérabilités, licences, sources), npm audit, dependency review, Dependabot | Fait |
| CodeQL et fuzzing des analyseurs exposés (ADR-0081) | CodeQL Rust/TypeScript/workflows ; cargo-fuzz sur seeds, en-têtes `Link`, JSON canonique et instantanés | Fait ; le fuzzing a déjà trouvé une perte de précision numérique |
| Migrations versionnées et test depuis le commit de base (ADR-0077) | Migrations numérotées par empreinte, `scripts/check-migration-from-base.sh` en CI | Fait ([ADR 0003](adr/0003-migrations-versionnees.md)) |
| ADR, SECURITY.md, CHANGELOG, licence | ADR 0001 à 0004, `SECURITY.md`, `CHANGELOG.md`, Apache-2.0 | Fait |
| SBOM, attestations, publication d'image | — | À faire lors de la première publication d'image |
| OpenSSF Scorecard | — | À activer si le dépôt devient public |

## 2. Écarts volontaires

### Images Alpine

Les images applicatives de CTIParsor sont basées sur Debian (`bookworm-slim`). Ce projet n'utilise que des images Alpine ; le binaire Rust est statique (musl), sans interpréteur.

### Prettier pour le frontend

CTIParsor vérifie le style avec `@stylistic/eslint-plugin` pour ne pas réécrire un code déjà aligné à la main. Ici, le frontend 0.1 était écrit sur des lignes de plusieurs milliers de caractères : il a été reformaté une fois par Prettier, qui le vérifie désormais.

### Règles « React Compiler » non activées

Comme CTIParsor, seules les deux règles classiques des hooks sont actives ; l'application n'utilise pas le React Compiler.

### Authentification et gestion d'équipe

La documentation d'architecture de CTIParsor décrit un proxy avec mot de passe partagé. Ce produit exige des comptes nominatifs, des droits par investigation, la révocation et un audit ; le proxy ne suffit pas.

### STIX et LLM

La plateforme n'a pas besoin d'un LLM pour indexer ou explorer ses relations. STIX pourra devenir un format d'échange, sans remplacer le modèle natif des scans, stars, forks et preuves GitHub. Ces intégrations nécessiteraient un contrat séparé.

## 3. Références dans CTIParsor

Fichiers consultés dans le dépôt CTIParsor : `CONTRIBUTING.md`, `pyproject.toml`, `frontend/package.json`, `frontend/eslint.config.js`, `docs/architecture.md`, `Dockerfile`, `compose.yaml`, `api/queue_loop.py`, `docs/adr/0071`, `0077`, `0078`, `0080`, `0081`, `.github/workflows/ci.yml`, `codeql.yml`, `fuzz.yml` et `.github/dependabot.yml`.
