# Contribuer

Le développement et les tests de référence se font dans des conteneurs Alpine. Conserver le code historique en lecture seule ; ne jamais charger ses fichiers pickle ni copier de secret dans un test, une image ou un commit.

## Tests

```sh
sh scripts/setup.sh
docker compose --profile test build test
docker compose --profile test run --rm test
```

Ces commandes compilent TypeScript/Vite, démarrent `test-postgres`, vérifient le schéma puis lancent pytest et Ruff. La fixture pytest refuse d'effacer des tables si `DB_HOST` n'est pas exactement `test-postgres`. Le conteneur de test ne reçoit aucun secret d'exploitation. Ne pas changer cette protection pour utiliser une base contenant des données utiles.

Les tests de collecte simulent GitHub ; ils ne nécessitent ni token ni réseau GitHub. Les contraintes métier prioritaires sont les ACL, la provenance, le temps, les quotas et la reprise concurrente.

Pour libérer seulement le service PostgreSQL de test :

```sh
docker compose --profile test stop test-postgres
docker compose --profile test rm -f test-postgres
```

## Vérification du navigateur

Le parcours E2E attend un compte et le dossier fictif créé par `investigator.cli demo`. Le mot de passe de ce compte est lu dans un fichier local ignoré par Git. Il n'est jamais transmis en argument de commande.

```sh
docker build -f frontend/Dockerfile.browser -t github-investigation:browser .
mkdir -p .local/screenshots
docker run --rm --network host --user "$(id -u):$(id -g)" \
  --cap-drop ALL --security-opt no-new-privileges \
  -e HOME=/tmp -e LOGIN=analyste \
  -v "$PWD/.secrets/analyste_password:/bootstrap_password:ro" \
  -v "$PWD/.local/screenshots:/output" \
  github-investigation:browser
```

Adapter le chemin du fichier de mot de passe à son environnement. Le test se connecte à `http://localhost:8088` par défaut ; `BASE_URL` permet une autre origine. Chromium est installé depuis Alpine uniquement dans cette image de test. Son lancement sans sandbox est limité au conteneur de test de l'application locale ; ce navigateur ne fait pas partie de l'architecture d'exploitation.

Le test 3D utilise Mesa EGL en rendu logiciel (`LIBGL_ALWAYS_SOFTWARE=1`, `EGL_PLATFORM=surfaceless`, ANGLE `gl-egl`). Il ne nécessite pas de GPU sur l'hôte. Ces réglages du navigateur de test ne sont pas appliqués aux navigateurs des analystes. Le parcours exige un rendu 3D effectif, puis vérifie séparément le repli 2D en simulant l'indisponibilité de WebGL.

## Dépendances et migrations

- Fixer les versions npm et conserver `package-lock.json`.
- Régénérer les locks Python avec hashes **dans Python Alpine**, à partir de `requirements/*.in`, puis reconstruire les tests.
- Épingler les images par digest et vérifier qu'elles contiennent `/etc/alpine-release`.
- Ne pas modifier un schéma déjà appliqué. Ajouter une migration numérotée et un test d'upgrade à partir d'une copie de la version précédente.
- Ne pas interpréter une absence dans une collecte partielle comme une suppression.
- Appliquer les ACL avant de charger preuves, relations, annotations ou recoupements.

La CI actuelle est dans `.github/workflows/ci.yml`. Les protections de branche, la licence et le dépôt distant devront être définis lors de la publication.
