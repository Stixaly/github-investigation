# Contribuer

Le développement et les tests de référence se font dans des conteneurs Alpine ; les mêmes contrôles tournent en local avec `rustup` (la version est fixée par `rust-toolchain.toml`) et Node 24. Ne jamais charger les pickles historiques ni copier un secret dans un test, une image ou un commit.

## Flux de travail

1. Créer une branche depuis `main` (`feat/…`, `fix/…`, `docs/…`).
2. Lancer `make check` (ou `make test` sans chaîne locale) avant de pousser.
3. Ouvrir une pull request : la CI doit être verte, et une décision d'architecture notable s'accompagne d'un ADR dans `docs/adr/`.
4. Toute modification visible par l'exploitation ou les analystes est notée dans [CHANGELOG.md](CHANGELOG.md).

## Commandes

| Commande | Effet |
|---|---|
| `make test` | Construit l'image de test Rust, démarre un PostgreSQL jetable, lance tous les tests ; puis Prettier, ESLint, TypeScript et Vitest dans Node Alpine. |
| `make check` | `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `npm run check`, avec les outils locaux. |
| `make audit` | `cargo deny check` et `npm audit` des dépendances de production. |
| `make e2e` | Parcours navigateur Chromium sous Alpine, contre la pile démarrée (voir plus bas). |

Sans base configurée, `cargo test` exécute les tests unitaires et **ignore** les tests d'intégration PostgreSQL en le signalant. Pour les lancer en local :

```sh
docker run -d --rm --name observatory-test-db -e POSTGRES_PASSWORD=ephemeral-tests-only -p 127.0.0.1:55432:5432 postgres:18-alpine
TEST_DATABASE_URL=postgres://postgres:ephemeral-tests-only@127.0.0.1:55432/postgres cargo test
```

Chaque test crée sa propre base `observatory_test_<uuid>` et travaille avec le rôle applicatif restreint : aucune base existante n'est vidée. En CI, `REQUIRE_DB_TESTS=1` transforme un test ignoré en échec.

## Règles du code

- **Rust** : `cargo fmt` ; clippy `all` en erreur et `pedantic` sans avertissement ; `unsafe` interdit ; pas de `unwrap()` hors des tests. Les règles pures, en particulier tout ce qui lit une entrée tierce, vont dans `crates/observatory-domain`, avec tests unitaires et, si besoin, une cible de fuzzing.
- **SQL** : paramètres liés uniquement (`$1`), jamais de valeur interpolée ; un motif `LIKE` saisi par un analyste passe par `escape_like`.
- **Frontend** : Prettier, ESLint (`typescript-eslint` + règles des hooks React), TypeScript strict, tests Vitest/Testing Library à côté des composants. Les libellés et rôles ARIA servent au test navigateur : les modifier avec `frontend/e2e.mjs`.
- **Domaine** : ne jamais interpréter une absence dans une collecte partielle comme une suppression ; appliquer les droits de l'investigation avant de charger preuves, relations, notes ou recoupements ; ne jamais persister un objet privé, pas même comme preuve.

## Migrations

- Ne jamais modifier une migration déjà appliquée : ajouter `migrations/NNNN_nom.sql` et l'entrée correspondante dans `crates/observatory/src/migrate.rs`.
- Ajouter un test d'upgrade dans `crates/observatory/tests/migrations.rs` quand la migration transforme des données.
- La CI migre une base créée par le commit de base (`scripts/check-migration-from-base.sh`) et vérifie qu'aucune ligne n'est perdue.

Voir l'[ADR 0003](docs/adr/0003-migrations-versionnees.md).

## Dépendances

- Rust : `Cargo.lock` est versionné, toutes les commandes utilisent `--locked`. Une nouvelle dépendance doit passer `cargo deny check` (licence permissive, aucune vulnérabilité connue, registre crates.io uniquement).
- npm : versions exactes dans `package.json`, `package-lock.json` versionné. Vérifier que le lock contient encore les binaires `linux-x64-musl` nécessaires au build Alpine.
- Images : épinglées par digest ; vérifier qu'une nouvelle image contient `/etc/alpine-release`. Dependabot propose les mises à jour chaque semaine.

## Fuzzing

```sh
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz --locked
cd fuzz && cargo +nightly fuzz run canonical corpus/canonical seeds/canonical -- -max_total_time=60
```

Cibles : `seed`, `pagination`, `canonical`, `snapshot`. Une entrée qui a fait échouer une cible est ajoutée à `fuzz/seeds/<cible>/` une fois corrigée.

## Vérification du navigateur

Le parcours E2E attend un compte `analyste` et le dossier fictif créé par `observatory demo`. Son mot de passe est lu dans `.secrets/analyste_password`, jamais passé en argument de commande :

```sh
docker compose up --build -d --wait
docker compose exec -T api observatory create-user analyste --password-stdin < .secrets/analyste_password
docker compose exec -T api observatory demo --login analyste
make e2e
```

Chromium est installé depuis Alpine uniquement dans cette image de test ; son lancement sans sandbox est limité à ce conteneur. Le rendu 3D utilise Mesa EGL en rendu logiciel, sans GPU. Le parcours exige un rendu 3D effectif, puis vérifie le repli 2D en simulant l'indisponibilité de WebGL. Les captures sont écrites dans `.local/screenshots/`, ignoré par Git.
