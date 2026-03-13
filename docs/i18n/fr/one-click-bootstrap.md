# Passerelle de localisation: One Click Bootstrap

Cette page est une passerelle enrichie. Elle fournit le positionnement du sujet, un guidage par sections source et des conseils d'exécution.

Source anglaise:

- [../../one-click-bootstrap.md](../../one-click-bootstrap.md)

## Positionnement du sujet

- Catégorie : Runtime et canaux
- Profondeur : passerelle enrichie (guidage de sections + conseils d'exécution)
- Usage : comprendre la structure puis appliquer les étapes selon la source normative anglaise.

## Navigation de la source

- Utilisez directement les titres réels du document anglais pour naviguer dans la source.
- Si la structure de cette passerelle diverge de la version anglaise actuelle, la version anglaise prévaut.

## Conseils d'exécution

- Pour une installation existante, utiliser d'abord `topclaw update --check`, puis `topclaw update`, puis `topclaw service restart` si TopClaw tourne comme service.
- Le one-line installer hébergé privilégie désormais d'abord le binaire compatible de la dernière release et ne clone le dépôt qu'en cas de repli vers une compilation source. Pour valider des changements locaux, utilisez un checkout puis `./bootstrap.sh --force-source-build`.
- Commencer par la structure des sections source, puis cibler les parties directement liées au changement en cours.
- Les noms de commandes, clés de configuration, chemins API et identifiants de code restent en anglais.
- En cas d'ambiguïté d'interprétation, la source anglaise fait foi.

## Entrées liées

- [README.md](README.md)
- [SUMMARY.md](SUMMARY.md)
- [docs-inventory.md](docs-inventory.md)
