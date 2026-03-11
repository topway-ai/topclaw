# Hub de Documentation TopClaw (Français)

Cette page est le hub français aligné sur la structure canonique `docs/i18n/<locale>/`.

Dernière mise à jour : **11 mars 2026**.

> Note : les commandes, clés de configuration et chemins API restent en anglais.

## Vue d'ensemble du projet

TopClaw est un agent runtime Rust-first qui combine :

- un CLI pour l'onboarding, le diagnostic et le chat direct
- une boucle agent avec appels d'outils, mémoire et routage provider
- des adaptateurs de channels de chat et une gateway HTTP/WebSocket
- des intégrations matérielles et périphériques optionnelles

Principales surfaces d'architecture publiques :

- providers : `src/providers/traits.rs`
- channels : `src/channels/traits.rs`
- tools : `src/tools/traits.rs`
- memory backends : `src/memory/traits.rs`
- runtime adapters : `src/runtime/traits.rs`
- peripherals : `src/peripherals/traits.rs`

## Accès rapide

- README français (racine) : [docs/i18n/fr/README.md](README.md)
- Hub docs français (compatibilité) : [docs/i18n/fr/README.md](README.md)
- Sommaire français (compatibilité) : [../../SUMMARY.fr.md](../../SUMMARY.fr.md)
- Hub docs anglais : [../../README.md](../../README.md)

## Documents runtime Wave 1

- Référence des commandes : [commands-reference.md](commands-reference.md)
- Référence des providers : [providers-reference.md](providers-reference.md)
- Référence des canaux : [channels-reference.md](channels-reference.md)
- Référence de configuration : [config-reference.md](config-reference.md)
- Runbook d'exploitation : [operations-runbook.md](operations-runbook.md)
- Dépannage : [troubleshooting.md](troubleshooting.md)

État actuel : **parité top-level terminée** (40/40).

## Index complet et gouvernance

- Inventaire documentaire local : [docs-inventory.md](docs-inventory.md)
- Guide d'exécution i18n : [i18n-guide.md](i18n-guide.md)
- Couverture i18n : [i18n-coverage.md](i18n-coverage.md)
- Suivi des écarts i18n : [i18n-gap-backlog.md](i18n-gap-backlog.md)

## Catégories

- Démarrage : [../../getting-started/README.md](../../getting-started/README.md)
- Modèle de runtime : [runtime-model.md](runtime-model.md)
- Référence : [../../reference/README.md](../../reference/README.md)
- Opérations et déploiement : [../../operations/README.md](../../operations/README.md)
- Sécurité : [../../security/README.md](../../security/README.md)
- Matériel : [../../hardware/README.md](../../hardware/README.md)
- Contribution / CI : [../../contributing/README.md](../../contributing/README.md)
- Table des matières locale : [SUMMARY.md](SUMMARY.md)

## Autres langues

- English: [../../README.md](../../README.md)
- 简体中文: [../zh-CN/README.md](../zh-CN/README.md)
- 日本語: [../ja/README.md](../ja/README.md)
- Русский: [../ru/README.md](../ru/README.md)
- Tiếng Việt: [../vi/README.md](../vi/README.md)
- Ελληνικά: [../el/README.md](../el/README.md)
