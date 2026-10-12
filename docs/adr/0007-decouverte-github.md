# ADR 0007 — Découverte GitHub par recherche de dépôts et de code

Statut : accepté le 12 octobre 2026 (jalon B du [plan de collecte](../PLAN_COLLECTE_RECHERCHE_STOCKAGE.md), partie découverte ; l'acquisition des contenus suivra).

## Contexte

Une collecte part d'une seed connue. Pour trouver des dépôts sans description, ou dont seul le code trahit le sujet, il faut interroger la recherche GitHub. Ses contraintes sont fortes :
- 1 000 résultats accessibles au plus par recherche ;
- `incomplete_results` quand la recherche est interrompue ;
- 30 requêtes par minute, 10 pour le code ;
- la recherche de code exige un token, ne voit que la branche par défaut et les fichiers de moins de 384 Ko, et n'indexe un fork que s'il a plus d'étoiles que son dépôt d'origine ;
- la recherche de dépôts exclut les forks par défaut, et ne trouve que des mots.

Les fragments de code renvoyés par la recherche sont du contenu de fichier : leur conservation relève du cadrage RGPD (REM-30).

## Décision

**Une découverte est un scan de source `search`** (migration 0006). Elle reprend la mécanique des scans :
- file, lease, génération et reprise ;
- budget du scan ;
- rythme partagé, réservé sur la ressource de quota de la recherche (`search` ou `code_search`) ;
- cache conditionnel, écriture par lots ;
- droits, plafond de scans en attente et audit (`discovery.create`).

Les dépôts trouvés sont les observations du scan, avec leur propriétaire ; ils apparaissent dans le graphe du scan et dans celui du dossier. La réponse de recherche est la preuve de chaque dépôt : un dépôt trouvé par plusieurs requêtes garde une preuve par requête, donc toutes ses origines.

**Le plan est compilé avant tout appel** (`observatory-domain/src/discovery.rs`), affiché par `POST .../discoveries/preview`, qui n'appelle pas GitHub, puis enregistré avec le scan (`scans.plan`).
- **Requête saisie** : elle est envoyée telle quelle, après vérification des limites GitHub (256 caractères, 5 opérateurs).
- **Liste publiée** : une requête de dépôts par terme des clauses positives, sur les champs cherchables (`in:name,description,topics,readme`). Les conjonctions `all` et les exclusions `none` sont annoncées comme vérifiées localement. Une recherche distante ne doit jamais éliminer silencieusement ce qu'une règle aurait retenu ; chaque écart est un avertissement du plan :
  - les sous-chaînes, que GitHub ne cherche pas ;
  - les champs de compte, non cherchables ;
  - les termes contenant un guillemet.

  La recherche de code part d'une requête saisie seulement : les champs d'une liste visent des métadonnées.
- **Forks** : ils sont inclus par défaut pour les dépôts (`fork:true`, choix visible et réversible), et exclus par défaut pour le code.
- **Limites** : 20 requêtes au plus, 1 à 10 pages de 100 résultats, 0 à 8 subdivisions ; l'aperçu donne le majorant des appels.

**Recherches saturées.** Une requête qui annonce plus de résultats que ses pages ou que les 1 000 accessibles est subdivisée, tant que le plan le permet : par période de création pour les dépôts, par taille de fichier pour le code. La requête d'origine est marquée `subdivided`. Ses deux moitiés deviennent des étapes, enregistrées dans la couverture avant que la requête d'origine soit close, si bien qu'une reprise les retrouve. Au-delà de la limite, la requête est `partial`, avec le nombre de résultats inaccessibles. `incomplete_results` rend aussi l'étape partielle.

**Recherche de code.** Sans token, la découverte échoue avec une raison explicite, sans appel. Avec token, elle demande la représentation standard, sans `text-match`, et retire tout fragment éventuel : la preuve d'un dépôt est le résultat de recherche (dépôt, chemin, empreinte Git du fichier), sans contenu. Les extraits et l'acquisition des fichiers viendront avec le stockage des pièces, désactivés par défaut jusqu'à la validation de REM-30.

## Conséquences

- Un dépôt sans description peut être découvert par son code, puis étendu par « Collecter ses relations ».
- Les découvertes partagent le plafond de dix scans en attente par dossier et le rythme GitHub de tous les workers.
- L'évaluation locale des règles sur les candidats (exclusions, conjonctions) arrive avec le moteur sur le corpus (jalon A). D'ici là, le plan dit ce qui reste à vérifier.
- Un fork piégé reste presque toujours invisible à la recherche de code : il se trouve par la liste des forks des dépôts suivis, déjà collectée.
