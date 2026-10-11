# ADR 0002 — Backend en Rust, compatible avec les données de la 0.1

Statut : appliqué en 0.2, le 11 octobre 2026. Remplace la partie « API et worker Python » de l'[ADR 0001](0001-socle-alpine-postgresql.md) ; le reste de l'ADR 0001 (PostgreSQL canonique, Alpine partout, graphe explicite) est inchangé.

## Contexte

La 0.1 était un prototype FastAPI/psycopg, sans typage vérifié ni mesure de couverture. Le responsable du projet a demandé sa conversion en Rust à l'occasion d'une revue des pratiques de développement (alignement sur CTIParsor). Une base pilote 0.1 existe : comptes, sessions, preuves et observations doivent survivre à la conversion.

La plateforme est surtout limitée par les entrées-sorties (quota GitHub, PostgreSQL). Le gain recherché n'est donc pas la vitesse, mais :

- des contrats vérifiés à la compilation (états de scan, rôles, types d'entités) ;
- un binaire statique musl, naturel sous Alpine, qui tient dans une image de quelques mégaoctets ;
- une mémoire bornée pour l'import en flux des instantanés de plusieurs centaines de mégaoctets ;
- la sûreté mémoire des analyseurs d'entrées tierces, soumis au fuzzing.

## Décision

Un seul binaire `observatory` (crate `crates/observatory`) avec des sous-commandes `serve`, `worker`, `migrate`, `create-user`, `demo` et `import-legacy`. L'API et le worker restent deux conteneurs distincts, avec des secrets distincts. Les règles pures (seed, pagination, JSON canonique, lecture des instantanés, identifiants) sont isolées dans `crates/observatory-domain`, sans réseau ni base, pour les tests unitaires et le fuzzing.

Pile retenue : axum (HTTP), sqlx (PostgreSQL, requêtes vérifiées à l'exécution, pas de base nécessaire à la compilation), reqwest + rustls (GitHub, sans OpenSSL), argon2 (RustCrypto), tokio.

Le frontend React/TypeScript n'est pas converti : l'API conserve les mêmes routes et les mêmes corps JSON, y compris le format d'erreur `{"detail": "..."}`.

### Compatibilité imposée avec la 0.1

| Élément | Règle |
|---|---|
| Mots de passe | Hachages Argon2id PHC de pwdlib relus tels quels ; nouveaux hachages avec les mêmes paramètres (m = 64 Mio, t = 3, p = 4). Test sur un hachage produit par pwdlib. |
| Sessions et CSRF | Même cookie, même empreinte SHA-256 du jeton, même HMAC CSRF : les sessions ouvertes survivent à la mise à jour. |
| Schéma | La migration 0001 est le `schema.sql` de la 0.1 à l'octet près ; son empreinte enregistrée par la 0.1 est reconnue ([ADR 0003](0003-migrations-versionnees.md)). |
| Preuves | `evidence_hash = SHA-256(source + "\n" + JSON canonique)`, où le JSON canonique reproduit `json.dumps(sort_keys=True, separators=(",", ":"), ensure_ascii=False)` : clés triées par point de code, échappements de Python, flottants au format `repr` de Python. Vecteurs de test générés avec CPython. |
| Nombres JSON | serde_json est compilé avec `arbitrary_precision` : un nombre garde le texte envoyé par GitHub. La preuve stockée est fidèle à l'octet près pour les entiers au-delà de 64 bits, et un flottant est relu avec un arrondi correct comme en Python. Le fuzzing a montré que sans ce réglage, un flottant pouvait changer d'un ULP et un grand entier devenir un flottant. |
| Import historique | Même clé d'idempotence (`json.dumps(manifest, sort_keys=True)` avec les séparateurs par défaut de Python) : un instantané déjà importé par la 0.1 est reconnu. |

## Conséquences

- Une seule chaîne de compilation côté serveur (Rust stable épinglé dans `rust-toolchain.toml`), plus nightly pour le seul fuzzing.
- Contrôles bloquants : rustfmt, clippy pedantic sans avertissement, tests unitaires et d'intégration PostgreSQL avec plancher de couverture, cargo-deny, fuzzing, CodeQL Rust.
- Les tests d'intégration s'exécutent avec le rôle applicatif restreint, dans une base créée par test.
- Coût : temps de compilation plus long qu'en Python et compétence Rust requise pour la maintenance. Les contributions sur le frontend et la documentation n'en dépendent pas.
- Les calculs d'analyse de graphe à venir (communautés, centralité) pourront être écrits dans le même langage, sur les mêmes types.

## Alternatives écartées

- **Garder Python et ne porter en Rust que l'import ou l'analyse** : moins de changements, mais deux langages côté serveur et deux implémentations des règles d'ingestion à maintenir en parallèle.
- **sqlx avec requêtes vérifiées à la compilation** (`query!`) : exige une base ou des métadonnées hors ligne à chaque compilation ; reporté.
- **Recalculer les empreintes avec un format canonique standard (RFC 8785)** : plus propre, mais aurait cassé la déduplication avec les preuves de la 0.1. Un changement de format devra passer par une colonne de version d'empreinte.
