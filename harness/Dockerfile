# Track 1 harness image. One image, three roles (see docker-compose.yml):
#   - fake-langfuse   (Rust bin, same implementation the Track 2 tests use)
#   - stub-model      (python3 harness/stub-model/server.py)
#   - runner          (real `claude` CLI + code-trace hooks + scenario script)
#
# Build context is the repo root: docker build -f harness/Dockerfile .

FROM rust:1.85-slim AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY tests ./tests
# The binary embeds the agent plugin sources via include_str!, so they must be
# present in the build context.
COPY plugin ./plugin
RUN cargo build --release --bin code-trace \
 && cargo build --release --bin fake-langfuse --features harness

FROM node:22-slim
# Pinned CLI: bumping this is a deliberate contract re-verification (NOTES.md).
ARG CLAUDE_CODE_VERSION=2.1.198
RUN apt-get update \
 && apt-get install -y --no-install-recommends python3 curl ca-certificates procps \
 && rm -rf /var/lib/apt/lists/* \
 && npm install -g @anthropic-ai/claude-code@${CLAUDE_CODE_VERSION}
COPY --from=build /src/target/release/code-trace /usr/local/bin/code-trace
COPY --from=build /src/target/release/fake-langfuse /usr/local/bin/fake-langfuse
COPY harness/stub-model/server.py /harness/stub-model/server.py
COPY harness/run-scenarios.sh /harness/run-scenarios.sh
RUN chmod +x /harness/run-scenarios.sh
WORKDIR /work
