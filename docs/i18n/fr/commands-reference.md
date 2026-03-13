# Référence des commandes (Français)

Cette page est une localisation initiale Wave 1 pour retrouver rapidement les commandes CLI TopClaw.

Source anglaise:

- [../../commands-reference.md](../../commands-reference.md)

## Quand l'utiliser

- Rechercher les commandes par workflow
- Vérifier les options et limites de comportement
- Comparer le résultat attendu pendant le debug

## Règle

- Les noms de commandes, flags et clés de config restent en anglais.
- La définition finale du comportement est la source anglaise.

## Mise à jour récente

- `topclaw gateway` prend en charge `--new-pairing` pour effacer les tokens appairés et générer un nouveau code d'appairage.
- `topclaw update` permet maintenant de vérifier puis d'installer plus clairement la dernière release. Le flux recommandé est `topclaw update --check` -> `topclaw update` -> `topclaw service restart` si le service tourne en arrière-plan.
- Utilisez uniquement les noms de commande canoniques de la source anglaise pour cette version.
- `topclaw status --diagnose` devient le chemin recommandé pour voir d'abord le résumé puis le diagnostic détaillé.
- Pour les canaux always-on, commencez par `topclaw service status`. `topclaw channel start` reste surtout un outil de debug au premier plan. Voir aussi [runtime-model.md](runtime-model.md).
- Si vous cherchez seulement les commandes les plus fréquentes, commencez par le bloc “Most Common Commands” ajouté en tête de la source anglaise.
