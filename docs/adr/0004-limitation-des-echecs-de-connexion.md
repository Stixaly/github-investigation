# ADR 0004 — Échecs de connexion comptés par poste et par compte

Statut : appliqué en 0.2, le 11 octobre 2026.

## Contexte

La 0.1 bloquait un identifiant pendant 15 minutes après cinq échecs, quelle que soit leur origine. Connaître le login d'un analyste suffisait donc pour l'empêcher de travailler. Le proxy Nginx limite déjà le débit par adresse IP, mais l'API ne voyait que l'adresse du proxy.

## Décision

- Chaque échec est enregistré avec l'identifiant et l'adresse du client (`login_attempts.client`, migration 0002).
- Sur 15 minutes glissantes, 5 échecs depuis un même client bloquent ce client pour cet identifiant ; 30 échecs tous clients confondus bloquent l'identifiant. Une connexion réussie efface les échecs de ce client seulement.
- L'adresse du client est l'adresse TCP. Derrière le proxy HTTPS (`compose.tls.yaml`), l'API lit `X-Real-IP`, que Nginx remplace systématiquement par l'adresse réelle. Sans `TRUST_PROXY_HEADERS=1`, cet en-tête est ignoré.
- Le compte inexistant suit le même chemin qu'un compte existant (vérification Argon2 sur un hachage factice) pour ne pas révéler son existence par la durée de réponse.

## Conséquences

- Un tiers isolé ne peut plus bloquer un analyste ; il lui faudrait au moins six adresses distinctes et 30 essais pour bloquer le compte, ce qui reste visible dans `login_attempts` et limité par Nginx.
- Une force brute répartie reste plafonnée à 30 essais par compte et par quart d'heure.
- Limite connue : un utilisateur ayant un accès local au serveur peut joindre le port 8088 sans passer par Nginx et choisir son `X-Real-IP`. Le plafond par compte reste appliqué. SSO et MFA restent au programme ([feuille de route](../FEUILLE_DE_ROUTE_INVESTIGATION.md)).
