# GitHub Investigation · Observatory

Plateforme d'investigation des relations entre comptes et dépôts **publics** GitHub. Un compte ou un dépôt sert de point de départ ; chaque scan conserve ses observations, ses preuves et sa couverture. Le corpus est mutualisé, avec des droits par investigation.

**Version 0.1 : première tranche fonctionnelle, destinée à un pilote.** L'API, le worker, PostgreSQL et le proxy HTTPS utilisent des images Alpine. Le frontend React/TypeScript est compilé sous Alpine et servi par l'API. Le nom Observatory est provisoire.

## Démarrer sous Linux

Prérequis : Docker Engine avec Docker Compose v2 ou ultérieur. Aucun Python, Node ou PostgreSQL à installer sur le serveur.

```sh
sh scripts/setup.sh
docker compose up --build -d --wait
docker compose exec api python -m investigator.cli create-user analyste
docker compose exec api python -m investigator.cli demo --login analyste
```

Le mot de passe est demandé de manière interactive : au moins 14 caractères. Ouvrir **http://localhost:8088**. La démonstration est facultative et contient uniquement des données fictives.

Sur un serveur distant, utiliser d'abord un tunnel SSH :

```sh
ssh -L 8088:127.0.0.1:8088 utilisateur@serveur
```

Pour l'accès de l'équipe, configurer le proxy HTTPS selon le [guide de déploiement Linux](docs/DEPLOIEMENT_LINUX.md). Le port HTTP reste lié à la boucle locale. Les secrets générés se trouvent dans `.secrets/`, exclu de Git et des images.

## Fonctionnalités disponibles

- Comptes nominatifs, sessions, rôles responsable/analyste/lecteur et révocation des accès par investigation.
- Création de dossiers, notes, scans publics bornés, suivi des états et annulation.
- Graphe interactif 3D à forces, avec bascule 2D, filtres, voisinage et export PNG ; comptes, dépôts, issues et pull requests reliés par les propriétaires, forks, contributions, auteurs et étoiles observées dans les événements publics.
- Index PostgreSQL partagé, identités GitHub stables, preuves JSON dédupliquées et recoupements entre dossiers auxquels l'analyste a accès.
- Consultation d'un scan antérieur et comparaison des liens observés. Une absence n'est jamais présentée comme une suppression certaine.
- Import en flux des anciens `users_db.json` et `repos_db.json`, sans lecture des pickles ni du fichier de token historique.

Le collecteur explore un voisinage borné : il ne parcourt pas encore récursivement tout un réseau. Les planifications récurrentes, la détection de communautés, les vues temporelles globales et la qualification en charge restent à construire. Voir l'[état d'implémentation](docs/IMPLEMENTATION_V0_1.md).

## Validation et développement

```sh
make test
docker compose logs --tail=100 api worker
```

`make test` construit aussi le frontend et lance pytest/Ruff contre un PostgreSQL de test isolé. Sans `make`, les commandes équivalentes figurent dans [CONTRIBUTING.md](CONTRIBUTING.md).

Validation locale : **28 tests Python**, Ruff et compilation TypeScript ; tests navigateur Chromium/Alpine, collecte publique réelle, import historique et proxy HTTPS. Les résultats détaillés et les limites sont consignés dans le document d'implémentation. Ubuntu sous WSL a été testé ; les hôtes AlmaLinux et Debian restent à qualifier.

## Documentation

- [Travail restant, mots-clés configurables et feuille de route](docs/FEUILLE_DE_ROUTE_INVESTIGATION.md)
- [Utiliser l'atelier graphe](docs/ATELIER_GRAPHE.md)
- [Déploiement, HTTPS, administration et sauvegarde](docs/DEPLOIEMENT_LINUX.md)
- [État livré, validations et prochain lot](docs/IMPLEMENTATION_V0_1.md)
- [Cadrage complet, architecture cible et feuille de route](docs/PROJET_INVESTIGATION_GITHUB.md)
- [Inventaire historique](docs/INVENTAIRE_HISTORIQUE.md)
- [Alignement avec CTIParsor](docs/ALIGNEMENT_CTIPARSOR.md)
- [Décision d'architecture du socle](docs/adr/0001-socle-alpine-postgresql.md)

Les données historiques de `ctimap-tool/`, les captures locales et les secrets sont exclus de la publication. La destination GitHub, la licence et le nom définitif du dépôt ne sont pas encore définis.
