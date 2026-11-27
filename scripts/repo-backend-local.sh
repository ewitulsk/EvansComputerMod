#!/bin/bash
# The package repository's backend on this machine, for the scenarios that
# need the real thing (docs/REPO.md, TESTING.md "Repository backend"):
#
#   scripts/repo-backend-local.sh up     build and start it, make accounts, publish the fixtures
#   scripts/repo-backend-local.sh env    print the settings the GameTests read (source this)
#   scripts/repo-backend-local.sh down   stop it and delete its data
#
# It is backend/docker-compose.yml (auth-api, repo-api, MinIO, Caddy), with
# Caddy on port 18080 of every interface so a computer in the game can reach
# it through the Internet Gateway at this machine's LAN address.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
PORT=18080
STATE="$ROOT/runs/repo-backend"
mkdir -p "$STATE"
LAN_IP="$(ipconfig getifaddr en0 2>/dev/null || hostname -I 2>/dev/null | awk '{print $1}' || true)"
[ -n "$LAN_IP" ] || { echo "cannot find this machine's LAN address"; exit 1; }
ORIGIN="http://$LAN_IP:$PORT"
cat > "$STATE/override.yml" <<YML
services:
  caddy:
    ports: !override
      - "$PORT:8080"
  auth-api:
    environment:
      PUBLIC_ORIGIN: $ORIGIN
      ADMIN_EMAILS: admin@ecmtest.example
  repo-api:
    environment:
      PUBLIC_ORIGIN: $ORIGIN
YML
dc() { docker compose -p ecm-repo-test -f backend/docker-compose.yml -f "$STATE/override.yml" "$@"; }

case "${1:-}" in
up)
    dc up -d --build
    for i in $(seq 1 60); do
        curl -fsS -m 2 "$ORIGIN/v1/health" >/dev/null 2>&1 && break
        sleep 1
    done
    curl -fsS -m 2 "$ORIGIN/v1/health" >/dev/null || { echo "the backend did not come up at $ORIGIN"; dc logs --tail 30; exit 1; }
    PASSWORD="ecm-test-password-1"
    dc exec -T auth-api auth-api admin user-add admin@ecmtest.example ecmadmin "$PASSWORD" >/dev/null 2>&1 || true
    dc exec -T auth-api auth-api admin user-add tester@ecmtest.example tester "$PASSWORD" >/dev/null 2>&1 || true
    ADMIN_KEY="$(dc exec -T auth-api auth-api admin key-add ecmadmin seed social,publish | tr -d '\r\n')"
    SEED_KEY="$(dc exec -T auth-api auth-api admin key-add ecmadmin fixtures social,publish | tr -d '\r\n')"
    USER_KEY="$(dc exec -T auth-api auth-api admin key-add tester gametest social,publish | tr -d '\r\n')"
    REPO_KEY="$(dc exec -T repo-api repo-api admin pubkey | tr -d '\r\n' | grep -oE '[0-9a-f]{64}' | head -1)"
    [ -f rust/target/wasm32-wasip1/release/echo.wasm ] || (cd rust && cargo build --release --target wasm32-wasip1 -p echo)
    (cd rust && cargo run -q -p pkg --example seed_repo -- "$ORIGIN" "$SEED_KEY" "$ADMIN_KEY" target/wasm32-wasip1/release/echo.wasm)
    cat > "$STATE/env" <<ENV
export ECM_TEST_REPO_URL=$ORIGIN
export ECM_TEST_REPO_KEY=$REPO_KEY
export ECM_TEST_REPO_APIKEY=$USER_KEY
export ECM_TEST_REPO_EMAIL=tester@ecmtest.example
export ECM_TEST_REPO_PASSWORD=$PASSWORD
ENV
    echo "The backend is up at $ORIGIN (fixtures published). Settings: $STATE/env"
    ;;
env)
    [ -f "$STATE/env" ] || { echo "not up: run $0 up" >&2; exit 1; }
    cat "$STATE/env"
    ;;
down)
    dc down -v --remove-orphans
    rm -f "$STATE/env"
    ;;
*)
    echo "usage: $0 up | env | down" >&2
    exit 2
    ;;
esac
