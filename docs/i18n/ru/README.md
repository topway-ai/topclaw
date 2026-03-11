# Документация TopClaw (Русский)

Этот файл — русскоязычный хаб в канонической структуре `docs/i18n/<locale>/`.

Последняя синхронизация: **2026-03-11**.

> Примечание: команды, ключи конфигурации и API-пути сохраняются на английском.

## Обзор проекта

TopClaw — это Rust-first agent runtime, который объединяет:

- CLI для onboarding, диагностики и прямого чата
- agent loop с вызовом инструментов, памятью и маршрутизацией provider
- адаптеры chat channel и HTTP/WebSocket gateway
- опциональные интеграции с оборудованием и периферией

Ключевые публичные архитектурные поверхности:

- providers: `src/providers/traits.rs`
- channels: `src/channels/traits.rs`
- tools: `src/tools/traits.rs`
- memory backends: `src/memory/traits.rs`
- runtime adapters: `src/runtime/traits.rs`
- peripherals: `src/peripherals/traits.rs`

## Быстрые ссылки

- Русский root README: [docs/i18n/ru/README.md](README.md)
- Русский docs hub (совместимость): [docs/i18n/ru/README.md](README.md)
- Русский SUMMARY (совместимость): [../../SUMMARY.ru.md](../../SUMMARY.ru.md)
- English docs hub: [../../README.md](../../README.md)

## Документы Wave 1 (рантайм)

- Справочник команд: [commands-reference.md](commands-reference.md)
- Справочник провайдеров: [providers-reference.md](providers-reference.md)
- Справочник каналов: [channels-reference.md](channels-reference.md)
- Справочник конфигурации: [config-reference.md](config-reference.md)
- Операционный runbook: [operations-runbook.md](operations-runbook.md)
- Troubleshooting: [troubleshooting.md](troubleshooting.md)

Текущее состояние: **top-level parity закрыт** (40/40).

## Полный индекс и governance

- Локальный каталог документов: [docs-inventory.md](docs-inventory.md)
- Руководство i18n: [i18n-guide.md](i18n-guide.md)
- Покрытие i18n: [i18n-coverage.md](i18n-coverage.md)
- Трекинг gap: [i18n-gap-backlog.md](i18n-gap-backlog.md)

## Категории

- Начало работы: [../../getting-started/README.md](../../getting-started/README.md)
- Модель рантайма: [runtime-model.md](runtime-model.md)
- Справочники: [../../reference/README.md](../../reference/README.md)
- Операции и деплой: [../../operations/README.md](../../operations/README.md)
- Безопасность: [../../security/README.md](../../security/README.md)
- Аппаратная часть: [../../hardware/README.md](../../hardware/README.md)
- Вклад и CI: [../../contributing/README.md](../../contributing/README.md)
- Единый TOC: [SUMMARY.md](SUMMARY.md)

## Другие языки

- English: [../../README.md](../../README.md)
- 简体中文: [../zh-CN/README.md](../zh-CN/README.md)
- 日本語: [../ja/README.md](../ja/README.md)
- Français: [../fr/README.md](../fr/README.md)
- Tiếng Việt: [../vi/README.md](../vi/README.md)
- Ελληνικά: [../el/README.md](../el/README.md)
