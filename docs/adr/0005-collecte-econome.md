# ADR 0005 — Collecte économe en quota GitHub

Statut : accepté le 11 octobre 2026, pour la version qui suit la 0.2.

## Contexte

Le quota GitHub est la ressource rare de la collecte : 60 requêtes REST par heure sans token, 5 000 avec. En 0.2 :

- un intervalle fixe d'une seconde espaçait les requêtes de tous les workers, sans tenir compte de l'état annoncé par GitHub ;
- chaque scan relisait toutes ses pages, même inchangées depuis le scan précédent ;
- un scan repris après une attente de quota ou un arrêt du worker recommençait depuis la seed, avec un budget déjà entamé ;
- une erreur réseau passagère suffisait à marquer un endpoint en erreur ;
- les pages comptaient 50 éléments, la moitié du maximum de l'API.

La feuille de route demande des checkpoints par page, des retries bornés, le traitement ETag/304 et un cache cloisonné par identité d'accès et représentation (§6, REM-07 et REM-08).

## Décision

- **Rythme par ressource de quota.** La table `github_rate_limits` (migration 0003) remplace `github_budget` : une ligne par ressource (`core`, `search`, `graphql`), avec la prochaine requête autorisée et le dernier état annoncé par les en-têtes `x-ratelimit-*`. Chaque réservation décrémente l'estimation ; chaque réponse la corrige, sans jamais la remonter dans une même fenêtre, sauf pour rendre une requête qu'une réponse 304 n'a pas consommée. Tant qu'il reste plus d'un cinquième du quota, seul l'écart minimal (250 ms par défaut) s'applique. En deçà, le reste est réparti jusqu'à la remise à zéro. À la réserve (1 % du quota, au moins une requête), tous les workers attendent la remise à zéro.
- **Cache HTTP.** Chaque réponse 200 est gardée dans `http_cache`, sous la clé URL, identité d'accès et représentation. L'identité vaut `anonymous` ou un préfixe d'empreinte SHA-256 du token, jamais le token. La représentation regroupe le type de média et la version d'API. Le corps est stocké tel que sérialisé par serde_json (`float_roundtrip`), si bien que sa relecture redonne la même valeur, donc la même empreinte de preuve.
  - La requête suivante envoie `If-None-Match`. Une réponse 304 produit une nouvelle observation, datée de la réponse et liée à la preuve existante. Pour un accès authentifié, GitHub ne la décompte pas, et elle est rendue au budget du scan.
  - Pendant la fenêtre de fraîcheur (10 minutes par défaut), la réponse est réutilisée sans requête. L'observation est alors datée de la dernière confirmation par GitHub, pas de l'instant du scan.
  - Le worker purge chaque heure les réponses non reconfirmées depuis la durée de rétention (7 jours par défaut).
- **Points de reprise.** `scan_checkpoints` garde, pour chaque étape d'un scan (la seed puis chaque endpoint), l'URL de la page suivante et la couverture acquise. Le point de reprise est écrit dans la transaction qui enregistre la page. L'étape est close dans la transaction qui enregistre sa couverture finale. Un scan repris saute les étapes closes et reprend les autres à leur page suivante.
- **Nouvelles tentatives.** Après une erreur réseau ou une réponse 500, 502, 503 ou 504, deux nouvelles tentatives au plus, après 1 s puis 3 s, à ±50 % (aléa pour désynchroniser les workers). Chaque tentative est réservée dans le budget du scan comme une requête ordinaire.
- **Pages de 100 éléments.** Le plafond de deux pages par endpoint ne change pas : un scan couvre deux fois plus de relations pour le même nombre de requêtes.
- **Pivot compte → dépôts par GraphQL**, si un token est configuré : une page de 100 dépôts coûte un point du quota GraphQL, réservé sur sa propre ressource, et apporte le dépôt d'origine de chaque fork. Chaque nœud reçu est la preuve, tel quel ; sa forme REST, recopiée dans l'observation, crée les mêmes entités et relations que la liste REST. Sans token, GitHub refuse GraphQL et la liste REST reste utilisée. Un point de reprise écrit par l'autre API (token ajouté ou retiré entre deux passages) n'est pas réutilisé.
- **Diagnostic.** `observatory github-status` interroge `GET /rate_limit`, que GitHub ne décompte pas, reporte le relevé dans la base et affiche le rythme partagé et l'occupation du cache. La couverture de chaque scan porte des compteurs `stats` : requêtes, réponses 304, réponses servies par le cache, nouvelles tentatives.
- `github_budget` n'est plus lue. Elle sera supprimée par une migration ultérieure, les suppressions se faisant en deux temps (ADR 0003).

## Conséquences

- Avec un token, un nouveau scan d'un voisinage inchangé ne consomme presque plus de quota. Sans token, la fenêtre de fraîcheur évite les requêtes répétées à court terme ; au-delà, une réponse 304 anonyme reste décomptée par GitHub.
- Un scan isolé reste rapide, et un quota presque épuisé se répartit au lieu de produire des refus en série.
- Le cache duplique des réponses publiques qui contiennent des données personnelles (logins, profils), le temps de sa rétention. Cette durée fait partie du cadrage RGPD (REM-30), comme celle des preuves.
- Une requête envoyée juste avant un arrêt brutal peut être refaite à la reprise : on ne promet pas « exactement une requête » par page, seulement aucune perte ni duplication logique.
- Le budget d'un scan compte désormais les requêtes décomptées par GitHub, et non les requêtes envoyées.
- Plusieurs workers concurrents s'appuieront sur les mêmes tables : la réservation verrouille la ligne de la ressource.
