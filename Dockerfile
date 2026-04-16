# syntax=docker/dockerfile:1.7

# ── Stage 1: Build ────────────────────────────────────────────
FROM rust:1.94-slim@sha256:cf09adf8c3ebaba10779e5c23ff7fe4df4cccdab8a91f199b0c142c53fef3e1a AS builder

WORKDIR /app
ARG TOPCLAW_CARGO_FEATURES=""

# Install build dependencies
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update && apt-get install -y \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

# 1. Copy manifests to cache dependencies
COPY Cargo.toml Cargo.lock ./
COPY crates/robot-kit/Cargo.toml crates/robot-kit/Cargo.toml
COPY crates/skill-vetter/Cargo.toml crates/skill-vetter/Cargo.toml
# Create dummy targets declared in Cargo.toml so manifest parsing succeeds.
RUN mkdir -p src benches crates/robot-kit/src crates/skill-vetter/src \
    && echo "fn main() {}" > src/main.rs \
    && echo "fn main() {}" > benches/agent_benchmarks.rs \
    && echo "pub fn placeholder() {}" > crates/robot-kit/src/lib.rs \
    && echo "pub fn placeholder() {}" > crates/skill-vetter/src/lib.rs
RUN --mount=type=cache,id=topclaw-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=topclaw-cargo-git,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,id=topclaw-target,target=/app/target,sharing=locked \
    if [ -n "$TOPCLAW_CARGO_FEATURES" ]; then \
      cargo build --release --locked --features "$TOPCLAW_CARGO_FEATURES"; \
    else \
      cargo build --release --locked; \
    fi
RUN rm -rf src benches crates/robot-kit/src crates/skill-vetter/src

# 2. Copy only build-relevant source paths (avoid cache-busting on docs/tests/scripts)
COPY src/ src/
COPY benches/ benches/
COPY crates/ crates/
COPY firmware/ firmware/
COPY data/ data/
COPY skills/ skills/
COPY web/ web/
# Keep release builds resilient when frontend dist assets are not prebuilt in Git.
RUN mkdir -p web/dist && \
    if [ ! -f web/dist/index.html ]; then \
      printf '%s\n' \
        '<!doctype html>' \
        '<html lang="en">' \
        '  <head>' \
        '    <meta charset="utf-8" />' \
        '    <meta name="viewport" content="width=device-width,initial-scale=1" />' \
        '    <title>TopClaw Dashboard</title>' \
        '  </head>' \
        '  <body>' \
        '    <h1>TopClaw Dashboard Unavailable</h1>' \
        '    <p>Frontend assets are not bundled in this build. Build the web UI to populate <code>web/dist</code>.</p>' \
        '  </body>' \
        '</html>' > web/dist/index.html; \
    fi
RUN --mount=type=cache,id=topclaw-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=topclaw-cargo-git,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,id=topclaw-target,target=/app/target,sharing=locked \
    if [ -n "$TOPCLAW_CARGO_FEATURES" ]; then \
      cargo build --release --locked --features "$TOPCLAW_CARGO_FEATURES"; \
    else \
      cargo build --release --locked; \
    fi && \
    cp target/release/topclaw /app/topclaw && \
    strip /app/topclaw

# Prepare runtime directory structure and default config inline (no extra stage)
RUN mkdir -p /topclaw-data/.topclaw /topclaw-data/workspace && \
    cat > /topclaw-data/.topclaw/config.toml <<EOF && \
    chown -R 65534:65534 /topclaw-data
workspace_dir = "/topclaw-data/workspace"
config_path = "/topclaw-data/.topclaw/config.toml"
api_key = ""
default_provider = "openrouter"
default_model = "anthropic/claude-sonnet-4-20250514"
default_temperature = 0.7

[gateway]
port = 42617
host = "127.0.0.1"
allow_public_bind = false
EOF

# ── Stage 2: Development Runtime (Debian) ────────────────────
FROM debian:trixie-slim@sha256:4ffb3a1511099754cddc70eb1b12e50ffdb67619aa0ab6c13fcd800a78ef7c7a AS dev

# Install essential runtime dependencies and desktop automation helpers.
# Xvfb provides a virtual X11 display so computer_use / screenshot / app_launch
# work inside the container. xdotool/wmctrl/scrot are the Linux desktop
# helpers the computer-use sidecar shells out to.
RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    xvfb \
    xdotool \
    wmctrl \
    scrot \
    xdg-utils \
    chromium \
    fonts-noto-color-emoji \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /topclaw-data /topclaw-data
COPY --from=builder /app/topclaw /usr/local/bin/topclaw

# Overwrite minimal config with DEV template (Ollama defaults)
COPY dev/config.template.toml /topclaw-data/.topclaw/config.toml
RUN chown 65534:65534 /topclaw-data/.topclaw/config.toml

# Environment setup
# Use consistent workspace path
ENV TOPCLAW_WORKSPACE=/topclaw-data/workspace
ENV HOME=/topclaw-data
# Defaults for local dev (Ollama) - matches config.template.toml
ENV PROVIDER="ollama"
ENV TOPCLAW_MODEL="llama3.2"
ENV TOPCLAW_GATEWAY_PORT=42617
# Virtual display for computer_use / desktop automation in headless containers.
# Xvfb listens on :99; DISPLAY is exported by the dev-entrypoint.sh script.
ENV XVFB_DISPLAY=:99

# Note: API_KEY is intentionally NOT set here to avoid confusion.
# It is set in config.toml as the Ollama URL.

COPY dev/dev-entrypoint.sh /usr/local/bin/dev-entrypoint.sh
RUN chmod +x /usr/local/bin/dev-entrypoint.sh

WORKDIR /topclaw-data
USER 65534:65534
EXPOSE 42617
ENTRYPOINT ["/usr/local/bin/dev-entrypoint.sh"]
CMD ["gateway"]

# ── Stage 3: Production Runtime (Distroless) ─────────────────
FROM gcr.io/distroless/cc-debian13:nonroot@sha256:8f960b7fc6a5d6e28bb07f982655925d6206678bd9a6cde2ad00ddb5e2077d78 AS release

COPY --from=builder /app/topclaw /usr/local/bin/topclaw
COPY --from=builder /topclaw-data /topclaw-data

# Environment setup
ENV TOPCLAW_WORKSPACE=/topclaw-data/workspace
ENV HOME=/topclaw-data
# Default provider and model are set in config.toml, not here,
# so config file edits are not silently overridden
#ENV PROVIDER=
ENV TOPCLAW_GATEWAY_PORT=42617

# API_KEY must be provided at runtime!

WORKDIR /topclaw-data
USER 65534:65534
EXPOSE 42617
ENTRYPOINT ["topclaw"]
CMD ["gateway"]
