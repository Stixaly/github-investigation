# Inventaire du projet historique

Date de lecture : **11 octobre 2026**. Périmètre : dossier local `ctimap-tool/`.

Cet inventaire repose sur la lecture des sources, des documents, des structures JSON et du texte de la présentation. Aucun appel de collecte GitHub n'a été effectué, aucun token n'a été lu, aucun fichier pickle n'a été désérialisé. Les tests de l'ancien outil n'ont pas été exécutés dans cette phase documentaire.

## 1. Documents et composants examinés

| Élément | Apport au nouveau projet |
|---|---|
| README historique (`ctimap-tool/README.md`, hors dépôt) | Objectif CTI, organisation des snapshots et rapports, limites de couverture |
| Changelog (`ctimap-tool/CHANGELOG.md`, hors dépôt) | Évolution des rôles, collecte des événements et mentions, résultats déclarés |
| Guide (`GUIDE.md`, hors dépôt) | Exploitation, reprise, scoring, limites connues |
| Présentation historique (`ctimap-tool/CTI-methodology-findings.pptx`, hors dépôt) | Principes méthodologiques, résultats et biais ; texte des 14 diapositives consulté |
| Client GitHub (`ctimap/client.py`, hors dépôt) | Pagination, retries, limites d'API, lecture du token par environnement |
| Collecteur (`ctimap/collect.py`, hors dépôt) | Rôles, profils, dépôts par compte et checkpoints |
| Fusion (`ctimap/merge.py`, hors dépôt) | Union des rôles et remplacement des profils/listes par source prioritaire |
| Scoring (`ctimap/score.py`, hors dépôt) | Score lexical explicable, poids, pénalités, tiers |
| Rapprochement (`ctimap/crossref.py`, hors dépôt) | Heuristique de spécificité des pseudonymes |
| Lexique (`ctimap/keywords.yaml`, hors dépôt) et `harvest.py` | Ressources et méthodes multilingues à reprendre après validation |
| Provenance de la fusion (`ctimap-tool/data/corpus-a-merged/provenance.json`, hors dépôt) | Sources, tailles des rôles, dates déclarées ou issues du mtime |
| Rapport agrégé du corpus A (`ctimap-tool/reports/corpus-a-full/aggregated_report.md`, hors dépôt) | Présentation du double signal et réserve sur l'identification |

Aucun dépôt Git n'a été détecté à la racine de l'espace de travail ni dans `ctimap-tool/`. L'historique consultable ici est documentaire et constitué de snapshots ; aucune chronologie de commits n'a été établie.

## 2. Volumes effectivement comptés

Méthode : décodage JSON ; nombre de clés de `users_db.json` ; nombre de profils contenant une clé `error` ; somme des longueurs des valeurs de type liste de `repos_db.json` ; comptage des objets dont `fork` est faux ou absent. Ce sont des **entrées de fichiers**, pas un décompte global de personnes ou de dépôts uniques.

| Snapshot | Entrées utilisateurs | Profils en erreur | Entrées de dépôts, forks inclus | Entrées non-forks | Taille exacte de `repos_db.json` |
|---|---:|---:|---:|---:|---:|
| `corpus-a-2026-06` | 7 461 | 42 | 250 002 | 41 336 | 83 825 990 octets |
| `corpus-a-2026-07` | 2 111 | 42 | 788 476 | 36 331 | 313 973 519 octets |
| `corpus-a-contributors` | 1 | 0 | 2 | 2 | 1 278 octets |
| `corpus-a-merged` | 7 464 | 42 | 930 844 | 68 126 | 356 995 162 octets |
| `corpus-b-2026-07` | 1 390 | 12 | 318 199 | 16 129 | 125 433 334 octets |

Le couple `corpus-a-merged` + `corpus-b-2026-07` représente **1 249 043 entrées de dépôts**, dont **84 255 non-forks**, avant déduplication entre fichiers. Les 8 854 entrées utilisateurs de ces deux fichiers ne sont pas nécessairement 8 854 comptes distincts.

Les snapshots fusionnés recouvrent les snapshots sources : additionner toutes les lignes du tableau surestimerait le corpus. Le chiffre de 80 451 non-forks présent dans le changelog diffère du total de 84 255 des deux fichiers actuels ; les extractions et versions devront être identifiées dans les futurs rapports.

## 3. Données conservées et données perdues

### Utilisateurs

Les profils réussis disposent généralement d'un `id` GitHub et d'un `login`. Les anciens profils ne contiennent que `id`, `login`, `bio`, `html_url` ; les plus récents ajoutent notamment des métadonnées de profil et des compteurs. Un profil en erreur reste une observation d'échec, pas un compte inexistant.

### Dépôts

**Aucune entrée de dépôt des cinq fichiers examinés ne contient de champ `id`.** Les champs incluent `full_name`, `html_url`, `fork`, `description`, `language`, `topics`, des compteurs, et parfois `created_at` et `pushed_at`.

Conséquences :

- Un `full_name` permet une clé provisoire, mais ne prouve pas l'identité après un renommage, un transfert ou une réutilisation de nom.
- `fork: true` n'indique pas à lui seul le dépôt parent.
- Une date de création de dépôt ne date pas la relation entre ce dépôt et un compte observé.
- Les compteurs de stars ne fournissent pas les identités des stargazers.

### Rôles

Le collecteur conserve des ensembles de logins. Dans plusieurs parcours, il perd l'objet d'origine, son identifiant, son horodatage et le contexte précis : issue, commentaire, PR, fork ou événement. La migration devra produire des relations historiques de rôle, sans fabriquer les objets manquants.

La provenance de fusion atteste **6 133 logins dans le rôle stargazers** du snapshot de juin, ainsi que 501 subscribers. Ces nombres proviennent du manifeste existant, pas d'une nouvelle désérialisation de `sets.pkl`.

Le rôle `followers` concerne les abonnés du **propriétaire** du dépôt. Il ne décrit pas des abonnés au dépôt. L'ancien nom `subscribers` devra être rapproché du rôle `watchers`, sans le confondre avec les stars.

### Dates et fusion

La fusion unit les ensembles de rôles et privilégie certaines sources pour les profils et listes de dépôts. Elle ne constitue pas un journal temporel complet. Certaines dates sont déclarées ; d'autres sont explicitement issues du mtime. Les chemins de provenance peuvent désigner d'anciens emplacements aujourd'hui absents.

Importer d'abord les sources disponibles, puis utiliser le snapshot fusionné pour la réconciliation. Si une information n'existe que dans la fusion, la conserver avec sa granularité de provenance réelle.

## 4. Points d'architecture à conserver

- Analyse hors ligne reproductible, sans nouvelle collecte pour chaque recalcul de score.
- Reprise après interruption et écritures de checkpoints atomiques.
- Séparation entre score lexical d'un dépôt et rapprochement de pseudonymes.
- Justification des scores, gestion du multilingue et des faux positifs.
- Comparaison entre populations de plusieurs dépôts.
- Conservation des snapshots devenus difficiles à recollecter.

## 5. Points à corriger lors de la réécriture

| Observation dans le code ou les documents | Conséquence pour la cible |
|---|---|
| `collect_sets` réutilise le cache dès que `sets.pkl` existe | Versionner les options de collecte et permettre un rafraîchissement explicite |
| `fetch_all` conserve les entrées réussies sans expiration | Définir une fraîcheur par ressource et un planning de revalidation |
| Les erreurs 401/404 peuvent être présentées comme une restriction de rôle | Séparer token invalide, ressource inaccessible et capacité effectivement restreinte |
| Les erreurs 403/429 sont regroupées dans le client | Distinguer refus d'autorisation et quota avant de réessayer |
| Le collecteur supprime les bots | Conserver le type de compte et filtrer l'affichage ; les bots peuvent expliquer un motif relationnel |
| Les dépôts n'ont pas d'ID enregistré | Capturer les IDs désormais ; résoudre les anciens noms avec prudence |
| Les rôles ne conservent pas l'objet justificatif | Sauvegarder les preuves au moment de la collecte |
| Documentation historique : événements sur 90 jours | La documentation GitHub consultée indique 30 jours et 300 événements ; paramétrer cette limite |
| `pickle.load` est utilisé pour les caches | Concevoir un import contrôlé vers un format inerte ; pas de chargement pickle dans l'application web |
| Un fichier `token_key.txt` est présent et décrit comme contenant un token dans les README | L'exclure des contextes Docker et des publications ; remplacer le stockage par une injection de secret |

La validité du token n'a pas été vérifiée. Sa présence dans cette copie ne démontre pas une fuite ; si la copie a déjà été partagée, vérifier son exposition et le révoquer en cas d'exposition confirmée.

## 6. Limites de cette lecture

Les métriques historiques d'efficacité du scoring et de tests réussis sont des résultats **déclarés par les documents existants**, pas des résultats reproduits ici. Les scores ne sont pas des probabilités calibrées. La couverture des endpoints actuels reste à tester avec les droits choisis pour le nouveau projet.

Source externe vérifiée pour la fenêtre d'événements : [GitHub — Events](https://docs.github.com/en/rest/activity/events). La proposition de migration et d'architecture figure dans le [document de projet](PROJET_INVESTIGATION_GITHUB.md).
