# ADR 0006 — Profils de collecte et listes de termes versionnés

Statut : accepté le 12 octobre 2026 (jalon A0 du [plan de collecte](../PLAN_COLLECTE_RECHERCHE_STOCKAGE.md)).

## Contexte

Avant la 0.4, un scan ne choisissait que son budget (6, 12 ou 24 requêtes dans l'interface) et parcourait toujours les mêmes endpoints. La feuille de route décrit des listes de termes configurables (§4.3 à 4.5), mais leur format n'était qu'un exemple non importable. Le plan de collecte demande d'abord un socle : des réglages de lancement et des listes reproductibles, sur lesquels s'appuieront la découverte GitHub (jalon B), puis l'évaluation sur le corpus et le triage (jalon A).

Reproductible signifie qu'un résultat garde le profil, la liste et le moteur qui l'ont produit, même après leur évolution.

## Décision

**Profils de collecte.** `collection_profiles(name, version)` contient des réglages immuables : relations lues, pages par endpoint, budget par défaut et plafond. La migration 0005 publie deux profils, « Aperçu » et « Réseau » ; « Réseau » v1 reprend exactement la collecte de la 0.3.
- Chaque scan enregistre le nom, la version et une copie des réglages de son profil. Un scan antérieur, sans profil, se lit comme « Réseau » v1.
- Le budget demandé est borné par le profil.
- Une relation exclue par le profil apparaît dans la couverture comme `not_requested`, avec la raison « Hors du profil ».
- Un nouveau profil, ou une nouvelle version, se publie par migration : c'est une décision d'administration.

**Lancement groupé.** Un lancement accepte jusqu'à 10 seeds, dédoublonnées sans tenir compte de la casse, avec le même profil et le même budget.
- Chaque seed reçoit une clé d'idempotence dérivée de celle de la demande (UUID v5) : rejouer la demande rend les mêmes scans.
- Le lancement est tout ou rien vis-à-vis du plafond de scans en attente, porté de 3 à 10 par dossier.

**Listes de termes.** Le format est `observatory.keyword-list`, schéma 1 : celui de la feuille de route (§4.5), désormais importable. Le serveur ne conserve que des versions publiées ; le brouillon reste dans l'éditeur.
- Publier exige une liste valide et la réussite de tous ses tests intégrés.
- Une version garde la définition normalisée et la version du moteur qui l'a validée.
- Le nom d'une liste ne change pas.
- L'export ajoute `version` et `status`, que l'import ignore : un export se réimporte tel quel.
- Un essai sur un exemple saisi n'enregistre rien.

**Immuabilité garantie par la base.** Un déclencheur refuse toute modification ou suppression d'une version publiée de profil ou de liste, y compris par le rôle applicatif.

**Moteur, version 1** (`observatory-domain/src/rules.rs`, pur et fuzzé) :
- **Normalisation** : décomposition de compatibilité Unicode caractère par caractère, minuscules et espaces regroupés ; accents retirés seulement sur demande. Deux textes ont la même forme NFKC si et seulement s'ils ont la même forme NFKD : la décomposition suffit pour comparer. Chaque octet normalisé garde la position du caractère d'origine.
- **Frontières de mots** : tout caractère qui n'est ni alphanumérique ni une marque combinante.
- **Modes de correspondance** :
  - « mot entier » : un seul mot ;
  - « expression exacte » : plusieurs mots, dans un même champ ;
  - « sous-chaîne » : choix explicite.
- **Logique à trois états de Kleene** : un champ absent est inconnu, jamais vide.
  - Une clause est vraie dès qu'un terme apparaît dans un champ connu. Elle est inconnue si aucun champ connu ne la satisfait et qu'un champ manque.
  - Une règle combine `any`, `all` et `none` ; elle rend `match`, `no_match` ou `not_evaluated`.
- **Explication** : au plus cinq occurrences par clause, avec le champ, le terme, la position dans le texte d'origine et un extrait.

**Mesures.** `observatory metrics` produit un relevé JSON :
- volumes des tables et de la base ;
- croissance par jour d'enregistrement ;
- durées des scans ;
- appels envoyés et quota décompté ;
- réponses 304, réutilisations du cache, nouvelles tentatives et octets reçus, désormais comptés dans `coverage.stats`.

La commande n'appelle pas GitHub.

## Conséquences

- Un scan, une liste et un futur résultat d'évaluation restent interprétables avec les réglages qui les ont produits.
- La correction d'une version publiée passe par une nouvelle version ; seule une intervention d'administration sur la base pourrait en effacer une.
- L'évaluation des listes sur le corpus d'un dossier (exécutions reprenables, résultats, triage) n'est pas encore livrée : elle vient après la découverte GitHub, selon l'ordre du plan.
- Les champs README et code sont déjà acceptés par le format, et restent `not_evaluated` tant qu'ils ne sont pas collectés.
- Un changement de normalisation ou de sémantique imposera une version 2 du moteur ; les versions publiées gardent la trace de celle qui les a validées.
