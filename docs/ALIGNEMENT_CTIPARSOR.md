# Alignement avec les méthodes de CTIParsor

Lecture du checkout local le **11 octobre 2026**, révision courte observée : `dcd685e`. Aucun fichier de CTIParsor modifié ; tests et réglages GitHub distants non vérifiés pendant cette lecture.

L'objectif est de reprendre les méthodes adaptées au nouveau produit, en respectant sa contrainte **Alpine sur tous les services** et son périmètre de 20 analystes.

## 1. Pratiques observées et adaptation

| Dans CTIParsor | Pour la plateforme GitHub |
|---|---|
| Développement documenté sous Docker uniquement | Même principe ; images dev et runtime Alpine |
| API FastAPI et frontend React/TypeScript | Stack familière proposée pour limiter le coût de maintenance |
| API, worker et file PostgreSQL séparés | Séparer collecte, analytics et HTTP ; conserver une file PostgreSQL initiale |
| Lease, heartbeat et reprise de jobs | Ajouter contrat d'idempotence, génération de lease et mutualisation entre investigations |
| PostgreSQL 18 Alpine dans Compose | PostgreSQL Alpine ; résoudre version/digest pendant la fondation |
| ADR et documentation de référence | Une décision explicite pour temps, graphe, ACL, Alpine et quotas |
| Ruff, mypy, pytest ; ESLint, TypeScript, Vitest | Même famille de contrôles ; tests métier propres à ce projet |
| Couverture de branches avec plancher global de 87 % | Référence méthodologique ; seuil du nouveau projet établi sur ses risques et son premier lot |
| Tests PostgreSQL isolés, chemins temporaires, services externes simulés | API GitHub simulée, aucun token requis pour développer ou tester |
| Locks Python avec hashes et images épinglées | Même discipline pour les dépendances Alpine et frontend |
| Workflows CI, CodeQL et fuzzing | Contrôles bloquants, fuzzing ciblé sur formats/imports et parsing d'URLs |
| Contrôles de chaîne de livraison, SBOM et attestations documentés | Vérifier les livrables publiés et leur origine ; tester leur base Alpine |
| Migrations numérotées avec historique et vérification | Même objectif, rôle de migration dédié et tests d'upgrade avant rollout |
| Distinction des preuves, inférences et décisions de revue | Observations GitHub, liens dérivés, hypothèses et validations séparés |

## 2. Écarts à ne pas recopier

### Les images applicatives CTIParsor ne sont pas Alpine

Le Dockerfile examiné utilise `node:26-bookworm-slim` et `python:3.14-slim-bookworm`. Compose inclut aussi un proxy de capture basé sur Ubuntu. Le nouveau projet ne peut donc pas reprendre ces images telles quelles. Sa pile plus légère peut éviter les dépendances NLP, OCR et navigateurs du runtime CTIParsor.

### Authentification et gestion d'équipe

La documentation d'architecture CTIParsor décrit notamment un proxy avec mot de passe partagé. Pour le nouveau produit, il faut des comptes nominatifs, des droits par investigation, la révocation et un audit. Le mécanisme de proxy ne suffit pas à lui seul.

### Migrations : suivre la décision récente

Le guide de contribution contient encore des passages décrivant les anciens tuples DDL idempotents. L'ADR-0077 et `api/migrations/` documentent une histoire versionnée plus récente. La recommandation reprend ce modèle récent, sans recopier les passages devenus incohérents entre documents.

### Charge et types de jobs

Une ingestion de rapport CTI et un crawl de milliers de ressources GitHub n'ont ni le même coût ni les mêmes contraintes. Les plafonds mémoire, la taille des lots et la concurrence doivent être mesurés pour ce projet. La concurrence GitHub est surtout limitée par les droits et quotas, pas uniquement par le nombre de workers.

### STIX et LLM

La nouvelle plateforme n'a pas besoin d'un LLM pour indexer ou explorer ses relations. STIX peut devenir un format d'échange, mais ne remplace pas le modèle natif des scans, stars, forks et preuves GitHub. Ces intégrations nécessiteraient un contrat séparé.

## 3. Sources locales vérifiées

Les liens suivants pointent vers le checkout local de référence ; ils ne constituent pas des dépendances du nouveau dépôt.

- CONTRIBUTING.md (dépôt CTIParsor : `CONTRIBUTING.md`)
- pyproject.toml (dépôt CTIParsor : `pyproject.toml`)
- Frontend package.json (dépôt CTIParsor : `frontend/package.json`)
- Architecture (dépôt CTIParsor : `docs/architecture.md`)
- Dockerfile (dépôt CTIParsor : `Dockerfile`)
- Compose (dépôt CTIParsor : `compose.yaml`)
- Boucle des workers (dépôt CTIParsor : `api/queue_loop.py`)
- ADR-0077 — migrations versionnées (dépôt CTIParsor : `docs/adr/0077-versioned-database-migrations.md`)
- ADR-0078 — paramètres du dépôt et workflows (dépôt CTIParsor : `docs/adr/0078-repository-security-settings-and-workflow-hardening.md`)
- CI (dépôt CTIParsor : `.github/workflows/ci.yml`)
- CodeQL (dépôt CTIParsor : `.github/workflows/codeql.yml`)
- Fuzzing (dépôt CTIParsor : `.github/workflows/fuzz.yml`)

Les réglages de protection GitHub décrits par les documents ne sont pas présumés actifs sur le futur dépôt. Leur configuration et leur vérification figurent dans la feuille de route.
