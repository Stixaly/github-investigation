# Sécurité

## Signaler une vulnérabilité

Ne pas ouvrir d'issue publique. Utiliser le signalement privé de GitHub (onglet **Security** → **Report a vulnerability**) ou contacter directement le responsable du dépôt. Indiquer la version ou le commit, les étapes de reproduction et l'impact attendu. Un accusé de réception est visé sous cinq jours ouvrés.

Seule la dernière version publiée sur `main` reçoit des correctifs.

## Modèle de sécurité

| Sujet | Mesure | Où |
|---|---|---|
| Périmètre des données | Seuls les objets GitHub publics sont collectés ; un objet privé n'est jamais persisté, pas même comme preuve. | `crates/observatory/src/store.rs` |
| Cloisonnement | Droits par investigation (responsable, analyste, lecteur) appliqués avant toute lecture de preuve, relation, note ou recoupement ; un dossier inaccessible répond 404. | `security.rs`, `api/` |
| Authentification | Argon2id, cookie de session HttpOnly/SameSite=Strict/Secure, jeton CSRF lié à la session, contrôle de l'en-tête `Origin` et de l'hôte, sessions de 8 heures révocables. | `security.rs`, `api/extract.rs` |
| Force brute | 5 échecs par poste et 30 par compte sur 15 minutes ; limitation par IP dans Nginx. | [ADR 0004](docs/adr/0004-limitation-des-echecs-de-connexion.md) |
| Navigateur | CSP sans script tiers, `frame-ancestors 'none'`, `nosniff`, `no-referrer`, `no-store`, HSTS en HTTPS. | `api/mod.rs` |
| Collecte | Origine GitHub fixe, redirections refusées, liens de pagination validés, réponses limitées à 2 Mio et 30 s, quota partagé entre workers, aucun proxy implicite. | `collector.rs`, `observatory-domain` |
| Base de données | Rôle applicatif sans DDL ni écriture sur l'historique de migration ; requêtes paramétrées ; délais de requête et de verrou. | [ADR 0003](docs/adr/0003-migrations-versionnees.md) |
| Conteneurs | Images Alpine épinglées par digest, UID non root, système de fichiers en lecture seule, capabilities retirées, `no-new-privileges`, limites mémoire/CPU/PID, réseau PostgreSQL interne, secrets en fichiers. | `compose.yaml`, `Dockerfile` |
| Chaîne de livraison | `Cargo.lock` et `package-lock.json` versionnés et imposés, cargo-deny et npm audit bloquants, revue des dépendances ajoutées, actions GitHub épinglées par SHA avec jeton en lecture seule, CodeQL et fuzzing. | `.github/` |

## Risques acceptés et limites connues

- Pas encore de SSO ni de MFA ; les comptes sont créés en CLI par l'administrateur.
- Pas de sécurité au niveau des lignes dans PostgreSQL (RLS) : les droits sont appliqués par l'application.
- Un utilisateur local du serveur peut joindre le port 8088 sans passer par Nginx et choisir son `X-Real-IP` ; le plafond d'échecs par compte reste appliqué.
- Les secrets Compose sont des fichiers montés, pas un coffre chiffré.
- Aucune exception n'est déclarée dans `deny.toml` ni dans l'audit npm à la date de la 0.2.

## Données historiques et secrets

Le répertoire `ctimap-tool/` (données de l'ancien outil) et `.secrets/` sont exclus de Git et des images. Un jeton GitHub présent dans ces anciens fichiers doit être considéré comme exposé et **révoqué** côté GitHub, même s'il n'a jamais été publié.
