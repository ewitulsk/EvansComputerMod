# syntax=docker/dockerfile:1
#
# One image with both services. Build it from the REPOSITORY ROOT, because the
# backend depends on rust/wasm-programs/pkg by path:
#
#   docker build -f backend/Dockerfile -t ecm-backend .
#
# The root .dockerignore keeps the context small (backend/, rust/Cargo.toml,
# rust/crates, rust/wasm-programs/pkg). There is no ENTRYPOINT: run
# `auth-api` or `repo-api` as the command (see docker-compose.yml).

FROM rust:1-slim-bookworm AS build
WORKDIR /src
# What cargo needs of the mod's workspace: its root manifest (pkg inherits
# `workspace = true` dependencies from it), pkg, and the crates pkg uses by path.
COPY rust/Cargo.toml rust/Cargo.toml
COPY rust/crates rust/crates
COPY rust/wasm-programs/pkg rust/wasm-programs/pkg
COPY backend backend
WORKDIR /src/backend
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/backend/target \
    cargo build --release --locked --bin auth-api --bin repo-api \
    && mkdir -p /out \
    && cp target/release/auth-api target/release/repo-api /out/

FROM debian:bookworm-slim
RUN groupadd --system --gid 10001 ecm \
    && useradd --system --uid 10001 --gid 10001 --home-dir /data --shell /usr/sbin/nologin ecm \
    && mkdir -p /data \
    && chown ecm:ecm /data
COPY --from=build /out/auth-api /out/repo-api /usr/local/bin/
USER ecm
WORKDIR /data
VOLUME /data
# auth-api listens on 8081, repo-api on 8082.
EXPOSE 8081 8082
