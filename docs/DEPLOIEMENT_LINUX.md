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
docker compose exec api python -m investigator.cli create-user analyste
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

L'API utilise `PUBLIC_ORIGIN` pour le cookie Secure, la vérification d'origine et HSTS. Elle n'accorde pas sa confiance aux en-têtes de proxy envoyés par un client. Le proxy limite les corps HTTP à 64 Kio et les tentatives de connexion par IP ; l'API limite aussi les échecs par identifiant.

Le certificat autosigné éventuellement présent dans l'environnement local de validation est temporaire et doit être remplacé pour un serveur d'équipe.

### AlmaLinux / SELinux

Conserver SELinux actif. Le montage de la configuration Nginx utilise `:ro,Z`. Vérifier les refus AVC si les fichiers de secrets ne sont pas accessibles ; adapter le contexte du seul répertoire dédié au projet selon la politique locale. Ne pas relabelliser un répertoire partagé contenant d'autres projets. Le volume nommé PostgreSQL évite un bind mount direct du répertoire de données.

## 4. Configurer la collecte GitHub

Sans token, le collecteur utilise les accès anonymes de l'API publique. Pour configurer un token dédié, écrire sa valeur dans `.secrets/github_token` depuis un éditeur ou un gestionnaire de secrets, sans le placer dans une commande, Git ou `.env`. Le fichier est initialement vide ; rendre temporairement le fichier modifiable par son propriétaire si nécessaire, puis rétablir `444`.

```sh
docker compose up -d --force-recreate worker
```

Seul le worker reçoit ce token. Les requêtes utilisent une origine GitHub fixe, refusent les redirections et valident les liens de pagination. Une même base PostgreSQL coordonne le budget global. Chaque scan possède son propre plafond, compris entre 1 et 50 requêtes ; l'interface propose 6, 12 ou 24. Les droits d'un token ne changent pas le périmètre public du produit.

Un scan peut être partiel même avec un statut terminal normal : consulter sa couverture. Les étoiles sont issues des `WatchEvent` disponibles, pas d'un inventaire exhaustif. Les limitations proviennent notamment du [flux d'événements GitHub](https://docs.github.com/en/rest/activity/events) et des [restrictions sur les listings annoncées en 2026](https://github.blog/changelog/2026-06-30-upcoming-access-restrictions-to-public-api-endpoints-and-ui-views/).

## 5. Comptes et import historique

Créer les comptes localement ; le responsable d'un dossier ajoute ensuite les membres depuis l'onglet Accès :

```sh
docker compose exec api python -m investigator.cli create-user autre-analyste
docker compose exec api python -m investigator.cli demo --login analyste
```

Pour importer un snapshot, créer d'abord une investigation dans l'interface et récupérer son UUID via la réponse API ou la liste authentifiée `/api/v1/investigations`. Monter uniquement le répertoire choisi, en lecture seule :

```sh
docker compose run --rm \
  -v /chemin/absolu/vers/snapshot:/legacy:ro \
  api python -m investigator.cli import-legacy /legacy \
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

Sauvegarder et vérifier les notes de migration avant de reconstruire. La version initiale du schéma est vérifiée par SHA-256 : modifier `schema.sql` après son application est refusé. Toute évolution du modèle devra passer par une migration supplémentaire et un test d'upgrade.

Les images de base sont fixées par digest, Python utilise des locks avec hashes et npm un lock exact. Une mise à jour de sécurité nécessite donc une modification explicite et les validations de contribution. Le fait d'utiliser Alpine, un UID non root et un système de fichiers en lecture seule ne remplace pas l'analyse des vulnérabilités des images.
