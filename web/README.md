# TopClaw Web Dashboard

This directory contains the single-page dashboard embedded into the TopClaw
gateway binary.

## Purpose

The dashboard is an operator-facing surface for:

- pairing a browser session with the runtime
- viewing status, health, cost, logs, and diagnostics
- browsing tools, memory, and integrations
- editing `config.toml`
- chatting with the agent over the gateway WebSocket API

The production build is bundled into the Rust gateway through `rust-embed`, so
the web UI ships as part of the normal TopClaw runtime rather than as a
separate deployment unit.

## Source Layout

- `src/main.tsx`: React/Vite bootstrap and router setup
- `src/App.tsx`: auth gate, locale context, and route table
- `src/components/layout/`: shared navigation shell
- `src/pages/`: route-level operational screens
- `src/lib/`: low-level API, auth, i18n, SSE, and WebSocket helpers
- `src/hooks/`: React hooks that wrap the low-level clients
- `src/types/api.ts`: shared TypeScript shapes for gateway responses

## API Contract

The dashboard talks only to the TopClaw gateway:

- `/health` and `/pair` are used before authentication
- `/api/*` endpoints are used for authenticated reads and writes
- `/api/events` provides SSE event streaming
- `/ws/chat` provides the live agent chat session

If a Rust gateway response shape changes, update `src/types/api.ts`,
`src/lib/api.ts`, and any consuming pages/hooks in the same patch.

## Development

```bash
cd web
npm install
npm run dev
```

Build the production bundle with:

```bash
npm run build
```
