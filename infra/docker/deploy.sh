#!/usr/bin/env bash
# Deploy or update the package repository stack on the droplet.
#
#   ./deploy.sh            deploy the IMAGE_TAG already in .env
#   ./deploy.sh <tag>      write IMAGE_TAG=<tag> to .env, then deploy
#                          (a commit SHA or a branch name; also how to roll back)
#
# Run it in the compose directory (/opt/ecm-repo) as the deploy user. It:
#   1. checks .env and the compose file
#   2. logs in to ghcr.io when GHCR_USER and GHCR_READ_TOKEN are set
#      (environment first, then .env)
#   3. pulls the images
#   4. restores the databases from the bucket if they are missing
#   5. docker compose up -d
#   6. waits for both services to pass `<binary> healthcheck` (GET /healthz)
#   7. prints the status
#
# Safe to run again at any time.

set -euo pipefail

cd "$(dirname "$0")"

HEALTH_TIMEOUT="${HEALTH_TIMEOUT:-120}"

log() { printf '\n==> %s\n' "$*"; }
die() {
	printf 'deploy.sh: %s\n' "$*" >&2
	exit 1
}

# Read one KEY=value from .env without running the file as shell.
env_get() {
	local line
	line="$(grep -E "^$1=" .env | tail -n 1 || true)"
	printf '%s' "${line#*=}"
}

# --- 1. checks ----------------------------------------------------------------
command -v docker >/dev/null || die "docker is not installed"
docker compose version >/dev/null 2>&1 || die "the docker compose plugin is not installed"
[ -f docker-compose.yml ] || die "docker-compose.yml not found in $PWD"
[ -f .env ] || die ".env not found in $PWD (copy .env.example to .env and fill it in)"
chmod 600 .env

if [ "$#" -ge 1 ] && [ -n "$1" ]; then
	tag="$1"
	[[ "$tag" =~ ^[A-Za-z0-9_][A-Za-z0-9_.-]{0,127}$ ]] || die "not a valid image tag: $tag"
	previous="$(env_get IMAGE_TAG)"
	if grep -qE '^IMAGE_TAG=' .env; then
		sed -i.bak -E "s|^IMAGE_TAG=.*|IMAGE_TAG=${tag}|" .env
		rm -f .env.bak
	else
		printf 'IMAGE_TAG=%s\n' "$tag" >>.env
	fi
	if [ -n "$previous" ] && [ "$previous" != "$tag" ]; then
		printf '%s\n' "$previous" >.previous-image-tag
		log "Image tag: $previous -> $tag (previous tag saved in .previous-image-tag)"
	fi
fi

image="$(env_get IMAGE):$(env_get IMAGE_TAG)"

# A changed config file must recreate its container (see docker-compose.yml).
CADDYFILE_SHA256="$(sha256sum Caddyfile | cut -d' ' -f1)"
LITESTREAM_CONFIG_SHA256="$(sha256sum litestream.yml | cut -d' ' -f1)"
export CADDYFILE_SHA256 LITESTREAM_CONFIG_SHA256

log "Checking the compose file and .env"
docker compose config --quiet

# --- 2. registry login --------------------------------------------------------
ghcr_user="${GHCR_USER:-$(env_get GHCR_USER)}"
ghcr_token="${GHCR_READ_TOKEN:-$(env_get GHCR_READ_TOKEN)}"
if [ -n "$ghcr_user" ] && [ -n "$ghcr_token" ]; then
	log "Logging in to ghcr.io as $ghcr_user"
	printf '%s' "$ghcr_token" | docker login ghcr.io --username "$ghcr_user" --password-stdin >/dev/null
else
	log "No GHCR_USER/GHCR_READ_TOKEN: using the existing docker login, if any"
fi
unset ghcr_token

# --- 3. pull ------------------------------------------------------------------
log "Pulling $image and the other images"
docker compose pull --quiet

log "Validating the Caddyfile"
if ! out="$(docker compose run --rm --no-deps -T caddy caddy validate --config /etc/caddy/Caddyfile 2>&1)"; then
	printf '%s\n' "$out" >&2
	die "the Caddyfile is not valid (nothing was changed)"
fi

# --- 4. restore if needed -----------------------------------------------------
# On a new droplet ./data is empty: fetch the databases from the bucket before
# anything can create empty ones. When the files exist, or the bucket has no
# replica yet (the very first deploy), these do nothing. A failure here stops
# the deploy on purpose.
mkdir -p data
log "Preparing ./data and restoring databases from the bucket if they are missing"
docker compose run --rm -T data-init
for svc in restore-auth restore-repo; do
	docker compose run --rm --no-deps -T "$svc" ||
		die "$svc failed: the bucket could not be read. Check the LITESTREAM_* values in .env. Nothing was started."
done

# --- 5. start -----------------------------------------------------------------
# "up" waits for the services' health checks (docker-compose.yml) before it
# starts Litestream and Caddy, and fails when one never becomes healthy.
log "Starting the stack"
if ! docker compose up -d --remove-orphans; then
	docker compose ps --all || true
	docker compose logs --tail 60 auth-api repo-api || true
	die "the stack did not start (status and service logs above)"
fi

# --- 6. wait for health -------------------------------------------------------
# The image has no curl or wget; each binary checks its own /healthz.
healthy() {
	docker compose exec -T "$1" "$1" healthcheck >/dev/null 2>&1
}

log "Waiting up to ${HEALTH_TIMEOUT}s for auth-api and repo-api"
deadline=$((SECONDS + HEALTH_TIMEOUT))
until healthy auth-api && healthy repo-api; do
	if [ "$SECONDS" -ge "$deadline" ]; then
		docker compose ps --all || true
		docker compose logs --tail 60 auth-api repo-api || true
		die "the services did not become healthy within ${HEALTH_TIMEOUT}s (logs above)"
	fi
	sleep 2
done
echo "auth-api and repo-api answer /healthz"

# Through Caddy and TLS, with the certificate verified: curl connects to this
# machine but asks for the real host name. Only a warning, because on the
# first deploy the certificate cannot be issued until DNS points here.
domain="$(env_get DOMAIN)"
if command -v curl >/dev/null 2>&1; then
	tls_ok=""
	for _ in 1 2 3 4 5 6 7 8 9 10; do
		if curl -fsS -m 10 -o /dev/null --resolve "${domain}:443:127.0.0.1" "https://${domain}/v1/health" 2>/dev/null; then
			tls_ok=1
			break
		fi
		sleep 3
	done
	if [ -n "$tls_ok" ]; then
		echo "https://${domain}/v1/health answers with a valid certificate"
	else
		echo "WARNING: https://${domain}/v1/health does not answer with a valid certificate yet." >&2
		echo "         Check that the A record of ${domain} points at this machine, then:" >&2
		echo "         docker compose logs --tail 50 caddy" >&2
	fi
fi

# --- 7. status ----------------------------------------------------------------
log "Status"
docker compose ps
docker image prune --force >/dev/null 2>&1 || true
log "Deployed $image"
