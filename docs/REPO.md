# The package repository

Where `pkg` and the App Store get apps, programs, libraries and source from, and where
players publish them. This page is for server owners and for people working on the mod.
Players have `man repo`, `man pkg` and `man store` in the game; the service itself is in
[`backend/`](../backend/README.md), its HTTP contract in [REPO_API.md](REPO_API.md), and how
to run it in [DEPLOY.md](DEPLOY.md).

## Pieces

| Piece | Where | What it does |
|---|---|---|
| `pkg`, the App Store | `rust/wasm-programs/pkg`, `rust/crates/ecm-apps-ui/src/store` | The client: search, install with dependencies, update, remove, mirror, serve, sign in, publish, like, favorite |
| Repository link | `api/repo/RepoLink`, `computer/repo/RepoPort`, sidecar `HttpsRepoLink` | The Minecraft server fetching from the repository for a computer (`host://repo/...`), with a package cache |
| Secret store | `computer/repo/KeyStore` | Sign-ins and the author key, outside the computer's files |
| Provisioning | `computer/repo/RepoProvision` | Keeps `official` and `community` in each computer's `/etc/pkg/repos` in step with the server's settings |
| Backend | `backend/auth-api`, `backend/repo-api` | Accounts, channels, publishing, likes and favorites |

## Packages

A package is a folder with an `app.toml`, packed by `pkg build` into a `.ecmpkg`.

| `kind` | Installed to | Sandboxed |
|---|---|---|
| `app` (default) | `/apps/<id>` | yes: permissions asked as for any app |
| `program` | each file of `bin = [...]` into `/bin` | no: it runs like any command |
| `library` | `python/` into `/lib/python`, `lua/` into `/lib/lua` (both interpreters import from there) | no |
| `source` | `/home/src/<id>` | nothing runs |

`depends = ["id", "id >= 1.2.0", "id = 1.0.0"]` names what a package needs; `pkg install`
installs those first and `pkg autoremove` removes the ones nothing needs any more.
`provides = [...]` lists a library's module names. `level` is the format level a package
needs; a client that is too old says so instead of installing something it cannot run.

Every package in a repository is signed by the repository. A published package also carries
its author's signature, made with a key in the author's secret store; the first one seen for
an id is remembered, and a later version signed by someone else is refused.

## How computers reach the official repository

`config/evanscomputermod-common.toml`, section `[repo]`:

| Setting | Default | Meaning |
|---|---|---|
| `access` | `internet` | `open`, `internet`, `datacenter` or `off` (below) |
| `url` | `https://repo.evanwitulski.com` | The repository's origin |
| `key` | (empty) | Its public key, 64 hex digits (`repo-api admin pubkey`). **Nothing is provisioned while this is empty.** |
| `allowCommunity` | `true` | Also offer the `community` channel (everything players published, unreviewed) |
| `allowPublish` | `true` | Let computers publish, like and favorite through the server's link |
| `cacheMiB` | `256` | The server's package cache (`ecm-repo-cache/`) |

and `[internet] maxKbps` caps what the Internet Gateway carries per computer (0 = no cap).

| `access` | A computer's `/etc/pkg/repos` | Path of a download |
|---|---|---|
| `open` | `official host://repo/r/stable/ <key>` (and `community`) | The Minecraft server fetches it (needs the `evanscomputermod-internet` add-on on the server). No network in the game is needed. |
| `internet` | `official <url>/r/stable/ <key>` (and `community`) | Over the computer's own network to an Internet Gateway, TLS made by the computer itself. `pkg online` says what is missing (cable, address, route, name, answer). |
| `datacenter` | `host://` lines on repository servers only | A repository server mirrors (`pkg mirror`, all or a chosen list) and serves (`pkg serve`); everyone else installs from it over the game's network. The copies keep the repository's signatures, so a pinned key verifies them like the originals. |
| `off` | none | Repositories inside the game still work |

Repository servers are the Tech Village ring's package server (docs/TECH_VILLAGE_NETWORK.md,
section 8a) and the computers an operator names:

```
/ecm repo status            what this server does
/ecm repo grant <x y z>     make the computer there a repository server
/ecm repo revoke <x y z>
/ecm repo list
```

A computer reads the settings when it starts; restart it (break and place, or reload its
chunk) after changing them or after a grant.

## Accounts

Anyone can download. With an account on the repository's website a player can publish, like
and favorite.

* `pkg login` shows a code and puts a link in the player's chat; they approve the code in
  their real browser. The password is never typed in the game.
* `pkg login --key <key>` takes an API key made on the account page.
* The sign-in is kept in the computer's secret store (`<world>/computer-secrets/`), not on its
  disk: only `pkg` and the App Store are given it, and only while the player who signed in
  is the one at the computer.
* `pkg publish <dir>` asks the player to confirm in chat (`[Allow]` / `[Deny]`). Uploads go to
  the `community` channel at once; `repo-api admin promote` puts a version in `stable`.
* A sign-in is only ever sent over `https://` or the server's link, never plain `http://`.

The server's owner can see everything on their server, including what its computers send
through the server's link. `man repo` tells players not to sign in on a server they do not
trust.

## Trying it

`/ecm scenario spawn <name>` (`/ecm scenario commands <name>` lists the steps):

| Scenario | Shows | Needs |
|---|---|---|
| `repo_kinds`, `repo_programs`, `repo_mirror` | Kinds, dependencies, mirroring, serving over TLS | nothing |
| `repo_open`, `repo_datacenter`, `repo_curated`, `repo_account`, `repo_login`, `repo_village` | The access modes and accounts against the real backend | the backend running locally, the internet add-on |
| `repo_internet` | A computer reaching it through an Internet Gateway | the same, and LAN destinations allowed |

See TESTING.md, "Repository backend", for how to start the backend for these.

## Not there yet

* **The real repository over the gateway is untested in the game.** `repo_internet` runs
  against a local backend over plain HTTP. HTTPS to public sites from a computer is tested
  natively (`ecm-http`, the ignored test `public_site_over_https`) and against in-game
  authorities, not in the game against the deployed repository.
* The data center's copy is refreshed when its `pkg serve --mirror` starts, not on a timer.
* `pkg discover` announces plain-HTTP repositories only.
* Python and Lua have no module for the secret store.
* Each different program that carries TLS (`pkg`, `curl`, `browser`, `python`, `lua`,
  `httpd`, `store`, `cert`) costs the server about 30 to 40 MB of heap once loaded.
