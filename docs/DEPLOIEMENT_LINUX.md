# Déployer le socle sur Linux

## 1. Périmètre validé et prérequis

La pile utilise Docker Engine et le plugin Compose, sur un serveur Linux. Le test local a été effectué sous **Ubuntu/WSL, Docker 29.8.1 et Compose 5.5.1**, avec les images Alpine épinglées dans les fichiers du projet. Les hôtes Debian et AlmaLinux sont des cibles de déploiement, pas des environnements déjà certifiés par ces essais.

Installer Docker à partir des instructions officielles de l'environnement : [Debian](https://docs.docker.com/engine/install/debian/), [Ubuntu](https://docs.docker.com/engine/install/ubuntu/), [RHEL](https://docs.docker.com/engine/install/rhel/). AlmaLinux est une distribution dérivée : qualifier sa version, son noyau, les paquets Docker choisis et SELinux sur un serveur de test. La [matrice officielle Docker](https://docs.docker.com/engine/install/) distingue les distributions prises en charge des dérivées.

Point de départ pour le pilote : 4 vCPU, 8 Go de RAM et un disque SSD avec sauvegardes. C'est une allocation initiale à mesurer, **pas un dimensionnement validé pour 20 analystes**. Prévoir aussi de l'espace pour les images de compilation et le corpus de preuves JSON.

Le compte de déploiement doit pouvoir utiliser Docker ; l'accès au démon Docker est un accès privilégié au serveur. Aucun service ne monte le socket Docker. Ne pas lancer simultanément deux copies du projet avec le même nom Compose `github-investigation`.

## 2. Installation initiale

Placer le code dans un répertoire appartenant au compte de déploiement, puis :

```sh
sh scripts/setup.sh
docker compose up --build -d --wait
docker compose exec api observatory create-user analyste
```

Le script génère deux secrets indépendants pour PostgreSQL et ne remplace aucun secret existant. La migration démarre avant l'API et le worker. Le rôle applicatif `investigator` n'a pas les droits de création du schéma ; le service ponctuel `migrate` dispose des identifiants administrateur.

La configuration initiale de `.env` est :

```dotenv
BIND_ADDRESS=127.0.0.1
PORT=8088
PUBLIC_ORIGIN=http://localhost:8088
```

Ouvrir exactement cette origine, ou utiliser un tunnel SSH depuis le poste analyste. Les vérifications d'origine empêchent de substituer silencieusement `127.0.0.1` à `localhost` pendant la connexion.

`docker compose down` arrête les services et conserve le volume PostgreSQL. Ne pas ajouter `-v` sur une installation contenant des investigations à conserver.

## 3. Activer HTTPS pour l'équipe

1. Préparer un nom DNS et un certificat valide pour ce nom. L'émission et le renouvellement du certificat restent à la charge de l'exploitation.
2. Placer le certificat PEM dans `.secrets/tls.crt` et sa clé dans `.secrets/tls.key`.
3. Vérifier les permissions : répertoire `.secrets` accessible uniquement au compte de déploiement (`700`), fichiers montés en secrets lisibles par les UID des conteneurs (`444`). Les secrets Compose locaux sont des fichiers montés ; ils ne constituent pas un coffre chiffré.
4. Adapter `.env` :

```dotenv
BIND_ADDRESS=127.0.0.1
PORT=8088
PUBLIC_ORIGIN=https://investigation.example.org:8443
TLS_BIND_ADDRESS=0.0.0.0
TLS_PORT=8443
```

Puis :

```sh
chmod 700 .secrets
chmod 444 .secrets/tls.crt .secrets/tls.key
docker compose -f compose.yaml -f compose.tls.yaml up --build -d --wait
docker compose -f compose.yaml -f compose.tls.yaml exec -T gateway nginx -t
```

Pour le port public 443, utiliser `TLS_PORT=443` et une `PUBLIC_ORIGIN` sans `:8443`. Nginx écoute toujours sur le port non privilégié 8443 dans son conteneur. N'ouvrir dans le pare-feu que le port HTTPS retenu, pour les réseaux analystes autorisés. PostgreSQL n'expose aucun port sur l'hôte.

Conserver **les deux fichiers Compose** dans les commandes d'exploitation HTTPS. Après renouvellement du certificat, recréer le gateway pour remonter les fichiers renouvelés :

```sh
docker compose -f compose.yaml -f compose.tls.yaml up -d --force-recreate gateway
```

L'API utilise `PUBLIC_ORIGIN` pour le cookie Secure, la vérification d'origine et HSTS. Le proxy limite les corps HTTP à 64 Kio et les tentatives de connexion par IP. Il remplace l'en-tête `X-Real-IP` envoyé par le client par l'adresse réelle ; `compose.tls.yaml` active `TRUST_PROXY_HEADERS` pour que l'API compte les échecs de connexion par poste : 5 échecs en 15 minutes bloquent ce poste pour ce compte, 30 échecs tous postes confondus bloquent le compte. Sans `compose.tls.yaml`, l'API ignore cet en-tête. Un utilisateur du serveur lui-même peut atteindre le port local 8088 et usurper l'en-tête ; le plafond global par compte s'applique quand même (voir [ADR 0004](adr/0004-limitation-des-echecs-de-connexion.md)).

Le certificat autosigné éventuellement présent dans l'environnement local de validation est temporaire et doit être remplacé pour un serveur d'équipe.

### AlmaLinux / SELinux

Conserver SELinux actif. Le montage de la configuration Nginx utilise `:ro,Z`. Vérifier les refus AVC si les fichiers de secrets ne sont pas accessibles ; adapter le contexte du seul répertoire dédié au projet selon la politique locale. Ne pas relabelliser un répertoire partagé contenant d'autres projets. Le volume nommé PostgreSQL évite un bind mount direct du répertoire de données.

## 4. Configurer la collecte GitHub

Sans token, le collecteur utilise les accès anonymes de l'API publique. Pour configurer un token dédié, écrire sa valeur dans `.secrets/github_token` depuis un éditeur ou un gestionnaire de secrets, sans le placer dans une commande, Git ou `.env`. Le fichier est initialement vide ; rendre temporairement le fichier modifiable par son propriétaire si nécessaire, puis rétablir `444`.

```sh
docker compose up -d --force-recreate worker
```

Seul le worker reçoit ce token. Les requêtes utilisent une origine GitHub fixe, refusent les redirections et valident les liens de pagination. Une même base PostgreSQL coordonne le budget global. Chaque scan possède son propre plafond, compris entre 1 et 50 requêtes ; l'interface propose 6, 12 ou 24. Les droits d'un token ne changent pas le périmètre public du produit.

### Token recommandé

Un token *fine-grained* limité aux dépôts publics en lecture (« Public repositories (read-only) »), sans autre permission, suffit : le produit ne lit que des données publiques. Lui donner une date d'expiration et prévoir son renouvellement. Il porte le quota REST de 60 requêtes par heure (accès anonyme) à 5 000. Une GitHub App obtient davantage sur une grande organisation, mais ses jetons d'installation expirent après une heure et le worker ne sait pas encore les renouveler. Ne pas multiplier les tokens ou les comptes pour contourner les quotas.

Vérifier l'accès et les quotas sans consommer de requête :

```sh
docker compose exec worker observatory github-status
```

### Économie de quota

Le collecteur ([ADR 0005](adr/0005-collecte-econome.md)) :

- demande des pages de 100 éléments, le maximum de l'API ;
- règle son rythme sur les en-têtes `x-ratelimit-*`, par ressource de quota et pour tous les workers : pleine vitesse tant qu'il reste plus d'un cinquième du quota, puis répartition du reste jusqu'à la remise à zéro, et attente de la remise à zéro quand il ne reste que la réserve (1 % du quota) ;
- garde chaque réponse dans un cache cloisonné par accès et la revalide par requête conditionnelle : une réponse 304 n'est pas décomptée par GitHub pour un accès authentifié, ni par le budget du scan ;
- réutilise sans requête une réponse confirmée depuis moins de 10 minutes ; l'observation est alors datée de cette confirmation ;
- reprend un scan interrompu (attente de quota, arrêt du worker) à sa page suivante ;
- refait au plus deux fois une requête après une erreur réseau ou 5xx, chaque tentative comptant dans le budget du scan.

La couverture de chaque scan détaille ces effets dans `stats` : requêtes envoyées, réponses 304, réponses servies par le cache, nouvelles tentatives. Réglages facultatifs, à placer dans `.env` :

| Variable | Défaut | Effet |
|---|---|---|
| `GITHUB_MIN_INTERVAL_MS` | 250 | Écart minimal entre deux requêtes, tous workers confondus |
| `GITHUB_CACHE_FRESHNESS_SECONDS` | 600 | Réutilisation sans requête ; `0` pour toujours interroger GitHub |
| `GITHUB_CACHE_RETENTION_DAYS` | 7 | Purge des réponses que GitHub n'a pas reconfirmées depuis |

Le cache contient des données personnelles publiques (logins, profils) : sa durée de conservation relève du cadrage RGPD (REM-30), comme celle des preuves.

Un scan peut être partiel même avec un statut terminal normal : consulter sa couverture. Les étoiles sont issues des `WatchEvent` disponibles, pas d'un inventaire exhaustif. Les limitations proviennent notamment du [flux d'événements GitHub](https://docs.github.com/en/rest/activity/events) et des [restrictions sur les listings annoncées en 2026](https://github.blog/changelog/2026-06-30-upcoming-access-restrictions-to-public-api-endpoints-and-ui-views/).

## 5. Comptes et import historique

Créer les comptes localement ; le responsable d'un dossier ajoute ensuite les membres depuis l'onglet Accès :

```sh
docker compose exec api observatory create-user autre-analyste
docker compose exec api observatory demo --login analyste
```

Pour importer un snapshot, créer d'abord une investigation dans l'interface et récupérer son UUID via la réponse API ou la liste authentifiée `/api/v1/investigations`. Monter uniquement le répertoire choisi, en lecture seule :

```sh
docker compose run --rm \
  -v /chemin/absolu/vers/snapshot:/legacy:ro \
  api import-legacy /legacy \
  --investigation UUID_DU_DOSSIER --login analyste --observed-date 2026-07-19
```

Le répertoire doit être lisible par l'UID applicatif 10001. Sous SELinux, préparer une copie dédiée avec le contexte adapté. Omettre `--observed-date` lorsque la date n'est pas connue : la date d'import ne sera pas utilisée à sa place. Une date fournie est enregistrée avec une précision au jour.

Seuls `users_db.json` et `repos_db.json` sont reconnus. Les identités sans ID GitHub restent provisoires ; les relations qui ne sont présentes que dans les anciens pickles ne sont pas reconstruites. Le même snapshot, avec la même date et dans le même dossier, réutilise le scan d'import existant.

## 6. Sauvegarde et exploitation

Exemple de sauvegarde logique, exécutée depuis un shell Linux :

```sh
umask 077
mkdir -p backups
docker compose exec -T postgres pg_dump -U postgres -d investigation -Fc > backups/investigation.dump
```

Le chemin de destination est un exemple : choisir un nom daté pour éviter d'écraser une sauvegarde précédente. Conserver une copie chiffrée hors du serveur, avec la configuration et les secrets dans un stockage séparé. Le corpus contient également des notes et les droits d'accès des investigations.

La restauration doit être répétée sur une **instance PostgreSQL séparée**, à partir de `pg_restore`, avant tout engagement de reprise en production. Un test de restauration complet n'a pas encore été réalisé pour cette version ; ne pas présenter la simple création d'un dump comme une validation de PRA.

```sh
docker compose ps
docker compose logs --tail=100 api worker migrate
curl --fail http://localhost:8088/ready
```

`/health` vérifie que l'API répond ; `/ready` vérifie sa connexion à PostgreSQL. Les états et erreurs de collecte sont consultables dans l'interface. La supervision, la rotation/rétention des journaux, les alertes disque et les sauvegardes planifiées restent à configurer par l'exploitation.

## 7. Mettre à jour

Sauvegarder (section 6) et lire le [CHANGELOG](../CHANGELOG.md) avant de reconstruire :

```sh
git pull
docker compose up --build -d --wait
docker compose run --rm migrate migrate --check
```

Le service ponctuel `migrate` applique les migrations numérotées de `migrations/` avant le démarrage de l'API et du worker. Chaque migration appliquée est vérifiée par SHA-256 : un fichier modifié après application, ou une base plus récente que le binaire, bloque le démarrage au lieu de laisser l'application lire un schéma inattendu (voir [ADR 0003](adr/0003-migrations-versionnees.md)).

### Depuis la 0.1 (backend Python)

La base 0.1 est reprise telle quelle : sa migration 1 est reconnue par son empreinte, puis la migration 2 est appliquée (index, compteur d'échecs par poste, dédoublonnage des observations). Les comptes, mots de passe et sessions ouvertes restent valides. Les preuves déjà enregistrées gardent leur empreinte : une même réponse GitHub observée avant et après la mise à jour n'est stockée qu'une fois. Les commandes d'administration deviennent `observatory <commande>` au lieu de `python -m investigator.cli <commande>`.

Les images de base sont fixées par digest, Cargo compile avec `--locked` à partir de `Cargo.lock` et npm installe un lock exact. La CI contrôle les dépendances (cargo-deny, npm audit, Dependabot). Une mise à jour de sécurité nécessite donc une modification explicite et les validations de contribution. Le fait d'utiliser Alpine, un UID non root et un système de fichiers en lecture seule ne remplace pas l'analyse des vulnérabilités des images.
