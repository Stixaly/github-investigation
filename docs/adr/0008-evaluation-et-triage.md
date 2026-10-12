# ADR 0008 — Évaluation des listes sur un dossier et file de triage

Statut : accepté le 12 octobre 2026 (jalon A du [plan de collecte](../PLAN_COLLECTE_RECHERCHE_STOCKAGE.md)).

## Contexte

Les listes de termes sont publiées, versionnées et testées (ADR 0006), et la découverte ajoute des candidats aux dossiers (ADR 0007). Il reste à les évaluer sur les observations d'un dossier, puis à faire examiner les correspondances. La feuille de route recommande d'abord la chaîne « mots-clés, résultats expliqués, triage ». Elle demande :
- qu'une évaluation travaille sur un manifeste figé, avec reprise et annulation ;
- qu'un résultat garde sa preuve, son extrait et les versions qui l'ont produit ;
- qu'une occurrence inchangée ne crée pas de nouvelle tâche.

## Décision

**Évaluation** (`rule_runs`, migration 0007). Une évaluation porte une version publiée d'une liste et la version du moteur.
- **Manifeste figé** : à son premier démarrage, elle fixe son manifeste, c'est-à-dire les observations enregistrées jusqu'à cet instant.
- **Lots reprenables** : le worker la traite par lots de 500 entités, dans l'ordre de leur identifiant, en lisant la dernière observation de chaque entité. Chaque lot écrit ses résultats, alimente le triage et avance le curseur dans une seule transaction.
- **Mécanique des scans** : réservation `SKIP LOCKED`, lease, génération ; une voie du worker la prend quand aucun scan n'attend, et la rend à la file à l'arrêt.
- **Concurrence** : une seule évaluation en attente par liste ; elle s'annule comme un scan.

**Champs évalués.** Pour un dépôt : `repository.name` (sans propriétaire), `full_name`, `description` et `topics`. Pour un compte : `login` et `name`. Un champ n'est renseigné que si l'objet observé le contient, éventuellement à `null`. Le README et le code restent non évalués tant qu'ils ne sont pas collectés. Une règle ne s'applique qu'aux types d'entité dont elle cite des champs : une règle sur des dépôts ne produit rien sur les comptes.

**Résultats** (`rule_matches`).
- Les correspondances et les non-évaluations sont conservées avec :
  - les occurrences (champ, terme, position, extrait) et les exclusions ;
  - les champs non collectés et la priorité de revue ;
  - la preuve et la date de l'observation évaluée.
- Les absences de correspondance sont seulement comptées.

**Triage** (`triage_items`). Une tâche par dossier, entité, liste et règle, à l'état nouveau, en cours d'examen, retenu ou écarté, avec une note et l'auteur de la dernière décision.
- La signature des occurrences (champ, terme, extrait) décide de leur nouveauté : une réévaluation sur des données identiques, ou dont seuls d'autres champs ont changé (nombre d'étoiles, date de mise à jour), ne crée ni ne rouvre aucune tâche.
- Une occurrence différente rouvre une tâche retenue ou écartée.
- La file est paginée, avec les priorités hautes d'abord, et rappelle qu'une correspondance n'est pas une qualification.

## Conséquences

- L'analyste voit pourquoi un dépôt ou un compte est remonté, selon quelle version de liste et de moteur, et à partir de quelle observation.
- Une évaluation de 250 000 entités se fait en lots courts, reprenables après un arrêt du worker.
- Les candidats de la découverte deviennent vérifiables localement, avec leurs exclusions et conjonctions.
- Restent à venir : l'affectation à un analyste, le README et le code comme champs évaluables (avec l'acquisition des contenus, après REM-30), et le lancement automatique d'une évaluation après un scan.
