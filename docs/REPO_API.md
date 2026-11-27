# Package repository backend: API contract

The public package repository for the mod: two Rust services behind Caddy on
one DigitalOcean droplet. This file is the contract between the backend
(`backend/`), the deployment (`infra/`) and the in-game client (`pkg`, the App
Store). Package, manifest and index formats are in `docs/gui/APPS.md`; the
backend uses the same code for them (`ecm_pkg`, `rust/wasm-programs/pkg`).

Public origin: `https://repo.evanwitulski.com` (configurable, `PUBLIC_ORIGIN`).

| Service | Listens on | Routes (as Caddy forwards them) |
|---|---|---|
| `auth-api` | `:8081` | `/auth/*`, `/account*`, `/signin`, `/signup`, `/signout`, `/verify`, `/forgot`, `/reset`, `/device`, `/static/auth/*` |
| `repo-api` | `:8082` | everything else: `/v1/*`, `/r/*`, `/` |

Everyone may read and download. A signed-in user may also publish, yank their
own packages, like, favorite and report.

## Conventions

- JSON in and out, UTF-8. Errors are `{"error":"<code>","message":"<text>"}`
  with the HTTP status below. Codes: `bad_request` 400, `unauthorized` 401,
  `forbidden` 403, `not_found` 404, `conflict` 409, `gone` 410,
  `too_large` 413, `pending` 428, `rate_limited` 429, `internal` 500.
- Every response carries `Content-Length` (the in-game client also accepts
  chunked, but old clients do not).
- The in-game client speaks HTTP/1.1 with `Connection: close`; the services
  must not require keep-alive, HTTP/2, cookies, compression or redirects on
  `/v1/*`, `/r/*` and `/auth/v1/*`.
- Authenticated calls send `Authorization: Bearer <token>`.
- Times are RFC 3339 UTC strings.

## Tokens

Opaque strings, stored as SHA-256 hashes. Shown once.

| Prefix | What | Lifetime |
|---|---|---|
| `ecm_k_` | API key, made on the account page | until revoked |
| `ecm_d_` | device token, from the device flow | until revoked |
| `ecm_s_` | web session (cookie `ecm_session`, HttpOnly, Secure, SameSite=Lax) | 30 days |

Scopes: `social` (like, favorite, report), `publish` (publish, yank),
`admin` (only settable in the database or by `ADMIN_EMAILS`). A token has a
name (for example the computer it was made for), a creation time and a
last-used time, and can be revoked on the account page.

## auth-api

### Web pages (HTML, server rendered, no script needed)

`GET /signup`, `GET /signin`, `POST /signout`, `GET /verify?token=`,
`GET /forgot`, `GET /reset?token=`, `GET /account` (API keys and devices:
list, create, revoke; resend verification; change password),
`GET /device?code=ABCD-EFGH` (approve or deny a device; signs in first).
Forms post to the same paths and carry a CSRF token.

Passwords: Argon2id, at least 10 characters. Handles: 3..32 of
`[a-z0-9_-]`, unique, shown as the publisher. Email must be verified before
publishing.

### JSON API

| Call | Auth | Body | Result |
|---|---|---|---|
| `POST /auth/v1/device/start` | none | `{"client_name":"..","scopes":["social","publish"]}` | `{"device_code","user_code":"ABCD-EFGH","verification_uri","verification_uri_complete","interval":3,"expires_in":600}` |
| `POST /auth/v1/device/token` | none | `{"device_code":".."}` | 200 `{"token","handle","scopes"}`; 428 `pending`; 403 denied; 410 expired |
| `GET /auth/v1/whoami` | bearer | | `{"handle","email_verified","scopes","token_name"}` |
| `POST /auth/v1/token/revoke` | bearer | | 204; revokes the presented token |

`POST /internal/introspect` (not routed by Caddy; header
`X-Internal-Secret: $INTERNAL_SECRET`), body `{"token":".."}`:
`{"active":true,"user_id":1,"handle":"..","email_verified":true,"scopes":[..],"admin":false}`
or `{"active":false}`. `repo-api` caches a positive answer for 30 s.

Rate limits (per IP and per account, in memory): sign-in 10/min,
sign-up 5/hour, forgot 5/hour, device start 20/min.

### Email

`EMAIL_PROVIDER=log` prints mails to stdout (local use). `resend` and
`postmark` send through that provider's HTTPS API with `EMAIL_API_KEY`;
`EMAIL_FROM` is the sender.

## repo-api

### Repositories (what `pkg` and the App Store read)

A channel is a repository URL: `pkg repo add official
https://repo.evanwitulski.com/r/stable/ --key <repo key>`.

| Call | Result |
|---|---|
| `GET /v1/health` | 200 `ok\n` (text/plain); the client's connectivity check |
| `GET /r/<channel>/index.json` | the channel's index (`ecm-repo-1`), newest version of each package first |
| `GET /r/<channel>/<file>.ecmpkg` | the package (`application/octet-stream`); counts a download |
| `GET /r/<channel>/<file>.ecmpkg/<member>` | an icon or screenshot of that package (only members the index lists) |

Channels: `stable` and `beta` (curated), `community` (every upload), and any
others an admin creates (name: 1..32 of `[a-z0-9-]`). The index's
`publisher_key` is the repository key (`REPO_SIGNING_KEY`'s public half) and
every entry's `signature` is made with it over
`ecmpkg-sig-v1\n<id>\n<version>\n<sha256>\n` (`ecm_pkg::package::signed_message`).
When the uploader signed too, the entry also has `author_key` and
`author_signature`. An entry has `kind`, `depends`, `commands`, `provides`
and `level` as `ecm_pkg::package::index_entry` writes them.

### Catalogue

| Call | Auth | Result |
|---|---|---|
| `GET /v1/packages?channel=&q=&kind=&sort=likes\|new\|downloads\|name&page=1` | optional | `{"total","page","per_page":50,"packages":[Summary]}` |
| `GET /v1/packages/<id>` | optional | `Detail` |
| `GET /v1/me/packages` | bearer | `[Summary]` the caller owns |
| `GET /v1/me/favorites` | bearer | `[Summary]` |
| `GET /v1/me/likes` | bearer | `[Summary]` |

`Summary`: `{"id","name","summary","kind","version","owner","channels":[..],"likes","downloads","updated","liked":bool,"favorited":bool}`
(`liked` and `favorited` are false without a token). `Detail` is a
`Summary` plus `"versions":[{"version","size","sha256","published","yanked"}]`,
`"permissions"`, `"depends"`, `"icon"`, `"screenshots"`.

### Social (scope `social`)

`PUT` and `DELETE /v1/packages/<id>/like`, `PUT` and `DELETE
/v1/packages/<id>/favorite`: 204, idempotent.
`POST /v1/packages/<id>/report` `{"reason":".."}` (at most 1000 characters): 204.

### Publishing (scope `publish`, verified email)

`PUT /v1/publish` with the raw `.ecmpkg` as the body
(`Content-Type: application/octet-stream`, at most 8 MiB). Optional headers
`X-Ecm-Author-Key` and `X-Ecm-Author-Signature` (hex; the signature must
verify). Result 201
`{"id","version","kind","channel":"community","file","sha256"}`.

Checks, in order: token and scope (401/403), size (413), the archive opens
with `ecm_pkg::package::open` (400), the id is not reserved (`dev.ecm.*` and
`com.evanwitulski.*` are admin only, 403), the id is new or owned by the
caller (403), that version does not exist yet (409, versions are immutable),
the author signature if given (400), the account quota (200 MiB, 413), the
rate limit (30 uploads/hour, 429).

`POST /v1/packages/<id>/yank` `{"version":".."}`: 204. A yanked version
leaves every index but its file stays downloadable by exact name for 30
days.

### Admin (scope `admin`)

`POST /v1/admin/promote` `{"id","version","channel"}`,
`POST /v1/admin/demote` (same body), `POST /v1/admin/takedown`
`{"id","version":null}` (removes files and index entries),
`POST /v1/admin/channels` `{"name","title"}`, `GET /v1/admin/reports`.
The same operations exist as `repo-api admin <verb> ...` on the server.

### Web page

`GET /` is a small server-rendered catalogue (search, kinds, likes) linking
to `/account`.

## Storage and configuration

Environment variables (both services read a `.env`-style environment):

| Variable | Service | Meaning |
|---|---|---|
| `PUBLIC_ORIGIN` | both | `https://repo.evanwitulski.com` |
| `LISTEN` | both | default `0.0.0.0:8081` / `0.0.0.0:8082` |
| `DATABASE_PATH` | both | SQLite file: `/data/auth.db`, `/data/repo.db` |
| `INTERNAL_SECRET` | both | shared secret for `/internal/*` |
| `AUTH_INTERNAL_URL` | repo | `http://auth-api:8081` |
| `EMAIL_PROVIDER`, `EMAIL_API_KEY`, `EMAIL_FROM` | auth | see Email |
| `ADMIN_EMAILS` | auth | comma-separated; these accounts are admins |
| `STORAGE` | repo | `fs` or `s3` |
| `STORAGE_DIR` | repo | for `fs`: `/data/blobs` |
| `S3_ENDPOINT`, `S3_REGION`, `S3_BUCKET`, `S3_ACCESS_KEY`, `S3_SECRET_KEY` | repo | for `s3` (DigitalOcean Spaces; MinIO locally) |
| `REPO_SIGNING_KEY` | repo | 64 hex digits: the ed25519 seed the repository signs with |
| `MAX_PACKAGE_BYTES`, `ACCOUNT_QUOTA_BYTES` | repo | defaults 8 MiB, 200 MiB |

SQLite runs in WAL mode; Litestream replicates both files to the bucket.
Blob keys: `packages/<id>/<version>/<file>.ecmpkg`. Indexes are built from
the database and cached in memory.
