# Package repository backend

The public package repository for the mod: where `pkg` and the App Store read
packages from, and where players publish them. The contract is
`docs/REPO_API.md`; this directory implements it.

## The pieces

| Crate | What it is |
|---|---|
| `auth-api` | Accounts (email + password, Argon2id), email verification and reset, web sessions, API keys, the device flow, the web pages, and `/internal/introspect`. |
| `repo-api` | Channels served as `ecm-repo-1` repositories, the catalogue, publishing, yank, likes, favorites, reports, admin operations, the `admin` command and the catalogue page. |
| `common` | What both share: the error shape, environment reading, the rate limiter, token hashing, SQLite setup, the page layout and stylesheet. |

How they fit:

- Each service has one SQLite file (WAL mode). Migrations run at startup.
- `repo-api` never sees passwords or sessions. For a bearer token it asks
  `auth-api` (`POST /internal/introspect` with `X-Internal-Secret`) and caches
  a positive answer for 30 seconds.
- Package files go to a blob store behind a trait: a directory (`STORAGE=fs`)
  or an S3 bucket (`STORAGE=s3`: DigitalOcean Spaces, MinIO). Keys are
  `packages/<id>/<version>/<file>.ecmpkg`; icons and screenshots are stored
  next to it under `members/`.
- Package, manifest, index and signature formats come from the mod's own code:
  `ecm_pkg` (`../rust/wasm-programs/pkg`), a path dependency. That is why this
  is its own cargo workspace and why Docker builds from the repository root.
- A proxy (Caddy) sends `/auth/*`, `/account*`, `/signin`, `/signup`,
  `/signout`, `/verify`, `/forgot`, `/reset`, `/device` and `/static/auth/*`
  to `auth-api` and everything else to `repo-api`. It must not route
  `/internal/*`.

## Run it with cargo

Two terminals, from `backend/`. Mails (verification and reset links) are
printed by `auth-api`.

```sh
export INTERNAL_SECRET=local-development-internal-secret
export PUBLIC_ORIGIN=http://localhost:8081   # no proxy: links point at auth-api

# terminal 1
LISTEN=127.0.0.1:8081 DATABASE_PATH=./data/auth.db EMAIL_PROVIDER=log \
ADMIN_EMAILS=you@example.com cargo run -p auth-api

# terminal 2
LISTEN=127.0.0.1:8082 DATABASE_PATH=./data/repo.db STORAGE=fs STORAGE_DIR=./data/blobs \
AUTH_INTERNAL_URL=http://127.0.0.1:8081 \
REPO_SIGNING_KEY=0101010101010101010101010101010101010101010101010101010101010101 \
cargo run -p repo-api
```

Then open http://localhost:8081/signup and http://localhost:8082/. Without a
proxy the two services are on different ports, so links between their pages
(`/account` from the catalogue) do not work; use compose for the whole site.

`.env.example` lists every variable.

## Run it with compose

```sh
docker compose -f backend/docker-compose.yml up --build
```

This starts both services, MinIO (the bucket is created for you) and Caddy on
http://localhost:8080. Sign up there; the verification link is in
`docker compose -f backend/docker-compose.yml logs auth-api`. The account
`admin@example.com` is an admin once its email is confirmed. In the game:

```
pkg repo add local http://<your address>:8080/r/community/ --key <key>
```

where the key is printed by
`docker compose -f backend/docker-compose.yml exec repo-api repo-api admin pubkey`.

The image (`backend/Dockerfile`) holds both binaries in `/usr/local/bin`, has
no entrypoint (the command is `auth-api` or `repo-api`), runs as uid 10001 and
keeps its data in `/data`. Build it from the repository root:

```sh
docker build -f backend/Dockerfile -t ecm-backend .
```

Both services answer `GET /healthz`. The image has no curl; a container
health check can run `auth-api healthcheck` or `repo-api healthcheck`.

## Tests

```sh
cd backend && cargo test
```

The integration tests start the real services on free ports with temporary
SQLite files and the `fs` store: `auth-api/tests/auth.rs` and
`repo-api/tests/repo.rs` (which starts both, so tokens go through
introspection). `cargo test` does not reach an S3 store; that is what the
ignored test `repo-api/tests/stack.rs` is for. With the compose stack up, it
signs up, publishes, reads the index, downloads and verifies through Caddy and
MinIO, and repeats the reads on a plain socket as the game does:

```sh
cd backend
ECM_STACK_URL=http://localhost:8080 \
ECM_MAIL_LOG_CMD="docker compose -f $PWD/docker-compose.yml logs --no-color auth-api" \
cargo test -p repo-api --test stack -- --ignored --nocapture
```

## The admin command

Run on the server, with the service's environment. It uses the same database
and blob store as the running service, which picks the changes up at once.

```sh
repo-api admin promote <id> <version> <channel>
repo-api admin demote <id> <version> <channel>
repo-api admin takedown <id> [<version>]
repo-api admin channel <name> [<title>]
repo-api admin reports
repo-api admin pubkey     # the repository's public key, from REPO_SIGNING_KEY
repo-api admin keygen     # a new REPO_SIGNING_KEY line and its public key
```

With compose: `docker compose exec repo-api repo-api admin reports`.

`auth-api` has one too, for first-time setup and for automated tests that
cannot click through a browser. It works on the same `DATABASE_PATH` as the
running service:

```sh
auth-api admin user-add <email> <handle> <password>   # the email counts as confirmed
auth-api admin key-add <handle> <name> [scope,scope]  # default social,publish
auth-api admin users                                  # handle, email, verified, admin
```

`user-add` enforces the same email, handle and password rules as the sign-up
page and fails if the email or the handle exists. `key-add` prints only the
new `ecm_k_...` key on stdout, so a script can capture it:

```sh
KEY=$(docker compose exec -T auth-api auth-api admin key-add evan ci)
```

The password is a command argument, so it shows in the shell history and the
process list; change it on the account page afterwards for a real account.

The same repository operations exist over HTTP under `/v1/admin/` for a token of an admin
account. An account is an admin when its confirmed email is in `ADMIN_EMAILS`,
or when `users.is_admin` is 1 in `auth.db`.

`ADMIN_EMAILS` is checked each time a token is looked up, not when the account
is made. So adding an address later makes the existing account an admin after
`auth-api` restarts with the new value (and at most 30 seconds later at
`repo-api`, the introspection cache); removing it takes the right away the
same way. Nothing is written to the database, and the account does not need to
be recreated. An unconfirmed address never counts.

## Things to know

- A 204 response has no `Content-Length`: HTTP forbids it and the server
  library removes it. Every other response on `/v1/*`, `/r/*` and
  `/auth/v1/*` has one and none is chunked.
- A revoked token keeps working at `repo-api` for up to 30 seconds (the
  introspection cache).
- Rate limits are in memory: they reset on restart and are per process.
- Set `TRUST_PROXY=true` only behind the proxy. Then the client address is the
  last entry of `X-Forwarded-For`.
- A taken-down package keeps its id: nobody can publish under it again. A
  taken-down or yanked version number cannot be reused.
- The index lists the account's handle as `publisher`, whatever the manifest
  says.
