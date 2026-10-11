# Atelier graphe

L'exploration reprend les principes de la capture EclecticIQ fournie : entités typées, liens orientés et outils regroupés au-dessus du graphe. La vue du scan utilise les fonds clairs, bordures discrètes et accents verts du reste d'Observatory. La [page EclecticIQ de référence](https://www.eclecticiq.com/take-action-with-cti/three-steps-to-aligning-cti-soc-and-ir-for-action) illustre l'intérêt d'explorer les relations autour d'une observation. Aucune donnée de la capture n'est importée dans le corpus.

## Utilisation

- **Moteurs 3D / 2D** : la vue démarre en 3D avec `3d-force-graph`. Le sélecteur permet de revenir au graphe 2D Cytoscape. En l'absence de WebGL, ou en cas de perte du contexte graphique, la vue bascule automatiquement en 2D avec un message explicatif.
- **Formes et couleurs** : comptes verts, organisations olive, dépôts violet, issues ambre et pull requests rose. En 3D, les comptes sont des sphères, les dépôts des cubes et les issues/pull requests des octaèdres. La 2D conserve les icônes GitHub spécifiques. Une organisation reste une entité Account dans la base ; sa couleur utilise le type GitHub observé.
- **Relations** : les flèches indiquent le sens du lien, avec un libellé français. Le bouton de libellés masque ou réaffiche les textes des relations. Pour limiter l'encombrement en 3D, au-delà de 60 entités ou 45 liens chargés, seuls les libellés des éléments sélectionnés restent affichés ; le survol permet de lire les autres. En 2D, les textes trop petits sont masqués au zoom éloigné.
- **Sélection** : cliquer sur une entité ou un lien ouvre son inspecteur et atténue le reste du réseau. Les preuves et les recoupements continuent d'appliquer les droits du dossier.
- **Dispositions** : réseau à forces, circulaire, plans par type ou grille en 3D ; réseau, circulaire, hiérarchique ou grille en 2D. Les plans par type constituent un rangement visuel, pas une détection de communautés. Le bouton de réorganisation réapplique la disposition choisie.
- **Filtres** : cliquer sur un type masque ses entités et les liens associés. Les compteurs du bas décrivent les éléments actuellement affichés, ceux des filtres décrivent la vue chargée.
- **Recherche** : trouver une entité dans la vue chargée, puis la sélectionner pour centrer le graphe. Elle ne lance pas de collecte supplémentaire.
- **Voisinage** : sélectionner une entité, puis activer la cible pour isoler ses voisins directs déjà chargés. « Ouvrir le voisinage » dans l'inspecteur demande une vue bornée à l'API du scan.
- **Navigation** : en 3D, glisser sur le fond fait tourner la caméra, le clic droit permet de déplacer la vue et la molette règle le zoom. Les nœuds restent déplaçables. En 2D, glisser déplace le canevas. Les deux moteurs disposent du cadrage, du masquage des preuves et du mode agrandi. Échap quitte le mode agrandi.
- **Export** : télécharger une image PNG de la fenêtre du graphe, avec ses filtres et sa sélection. L'export n'est pas une archive complète des preuves ou de l'investigation.

## Portée

Les changements portent sur le frontend React : `3d-force-graph` 1.80.1, Three.js 0.186.1, `three-spritetext` 1.10.0 et Cytoscape 3.34.3. Le module 3D est chargé à la demande dans un fichier JavaScript séparé. Les bibliothèques sont intégrées au build local, sans CDN. La simulation travaille sur des copies des entités pour ne pas modifier les observations de l'API. Les textes issus de GitHub sont affichés comme du texte, y compris dans les infobulles.

Le rendu 3D se fait dans le navigateur de l'analyste : le serveur ne nécessite pas de GPU ni de service supplémentaire. Les conteneurs restent Alpine. Le stockage, les quotas et les droits restent ceux du socle. La vue reste limitée au scan choisi et au sous-graphe renvoyé par l'API ; l'apparence de l'atelier ne garantit pas l'exhaustivité des relations.

Les positions manuelles, filtres et dispositions ne sont pas encore sauvegardés entre les ouvertures. La détection de communautés, les chemins entre plusieurs comptes, les annotations directement sur le canevas et la sauvegarde d'une composition sont des prolongements possibles, pas des fonctionnalités déjà livrées.

## Vérification

Compilation TypeScript/Vite et parcours `frontend/e2e.mjs` réussis sous Alpine. Le parcours couvre le rendu 3D, la rotation, la sélection d'un nœud avec consultation de sa preuve, le voisinage, les quatre dispositions 3D, l'export PNG et le passage en 2D. Il vérifie aussi les filtres, les dispositions 2D, le zoom, le cadrage, l'agrandissement/Échap, l'inspecteur, le clic sur une relation, la comparaison temporelle, le mobile, la déconnexion et un navigateur dont WebGL est volontairement indisponible. Chromium utilise Mesa EGL en rendu logiciel dans le conteneur de test Alpine.

Un contrôle complémentaire réussi sur le scan public du corpus A déjà présent a chargé 300 entités en 3D, puis recherché la seed et isolé son voisinage. Aucune nouvelle collecte GitHub n'a été lancée. Les captures locales se trouvent dans `.local/screenshots/`.
