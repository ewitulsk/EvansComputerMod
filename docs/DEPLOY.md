# Deploying the package repository backend

From nothing to a working `https://repo.evanwitulski.com`, then how to run it.
The services and their API are described in `docs/REPO_API.md`; this guide is
about the machine they run on.

## What gets built

```
                        DNS: repo.evanwitulski.com  A  <reserved IP>
                                      |
   DigitalOcean cloud firewall: 80, 443 from anywhere; 22 from the addresses you list
                                      |
   one droplet (Ubuntu 24.04, Docker Compose, /opt/ecm-repo)
     caddy       :80 redirects to HTTPS; :443 TLS (Let's Encrypt), routes by path
     auth-api    :8081  SQLite /data/auth.db
     repo-api    :8082  SQLite /data/repo.db, package files in Spaces
     litestream         copies both databases to Spaces, continuously
                                      |
   Spaces: <prefix>-packages (package files)   <prefix>-db (Litestream replicas)
```

Nothing that matters lives only on the droplet. The databases are replicated
to the `-db` bucket within about a second of every write, and the package
files are in the `-packages` bucket. A destroyed droplet is replaced by a new
one that restores both databases from the bucket before the services start.
The one thing you must keep yourself is the `.env` file (step 5).

Files:

| Path | What |
|---|---|
| `infra/terraform/` | droplet, reserved IP, firewall, two buckets, optional project and DNS record |
| `infra/docker/` | `docker-compose.yml`, `Caddyfile`, `litestream.yml`, `.env.example`, `deploy.sh`, `backup-check.sh` |
| `.github/workflows/backend.yml` | test, build the image, push to GHCR, deploy |
| `.dockerignore` | build context of the image |

## Decisions to make before you start

1. **Region.** Default `nyc3`. The buckets go in the same region; if you pick
   a region without Spaces, set `spaces_region` to one that has it.
2. **Where the DNS zone of `evanwitulski.com` is hosted.** If at DigitalOcean,
   Terraform creates the record (`manage_dns = true`). Anywhere else, you
   create one A record by hand (step 7).
3. **Who may reach SSH.** The default is nobody. Either your own address only
   (then deploys are run by hand from your machine), or the whole internet so
   that GitHub Actions can deploy (GitHub's runners have no fixed address).
   Logins are key-only either way. See step 6 and step 12.
4. **E-mail provider:** Resend or Postmark (step 4), and the sender address.
5. **Is the image private?** A GHCR package is private when first pushed. Keep
   it private and give the droplet a read-only token, or make it public and
   skip the token (step 8).
6. **Where the `.env` lives.** Only on the droplet plus your password manager
   (the default here), or also as the GitHub secret `DEPLOY_ENV_FILE` so every
   CI deploy rewrites it (step 12).
7. **Bucket name prefix.** Bucket names are global across DigitalOcean.
8. **Automatic reboots.** The droplet installs security updates by itself and
   reboots at 09:30 UTC when an update needs it (about a minute of downtime).
   To change that, edit `52ecm-unattended-upgrades` in
   `infra/terraform/cloud-init.yaml.tftpl` before step 6.

## Costs

Approximate, in US dollars per month, as remembered when this was written.
**Check every line on <https://www.digitalocean.com/pricing> before you
apply.**

| Item | Roughly |
|---|---|
| Droplet `s-1vcpu-1gb` (1 vCPU, 1 GB, 25 GB disk) | about $6 |
| Spaces subscription (covers both buckets and the optional state bucket; includes a storage and transfer allowance far above what this needs) | about $5 |
| Reserved IP | free while assigned to a droplet; charged when left unassigned |
| Cloud firewall, monitoring, DNS hosting | free |
| Droplet backups (off by default, `droplet_backups`) | a percentage of the droplet price |
| Outbound transfer | included allowance, then per GB |
| E-mail provider | free tier is enough to start; check its limits |
| GHCR | free for public images; private images count against the GitHub plan's package storage |
| Let's Encrypt certificate | free |

So about $11 a month, plus tax.

---

## 1. Prerequisites

On your machine:

```sh
terraform version        # 1.6 or newer
ssh -V
openssl version
git --version
```

An SSH key pair. If you have none:

```sh
ssh-keygen -t ed25519 -C "you@laptop"
cat ~/.ssh/id_ed25519.pub
```

Accounts:

- DigitalOcean, with a payment method.
- GitHub, with admin rights on `ewitulsk/EvansComputerMod`.
- Resend or Postmark (step 4).
- Access to the DNS records of `evanwitulski.com`.

What the deployment relies on from the backend (`backend/README.md`):

- One image with `auth-api` and `repo-api` in `/usr/local/bin`, no
  entrypoint, running as uid 10001 with its data in `/data`.
- Both services answer `GET /healthz`. The image has no curl or wget, so the
  compose health checks and `deploy.sh` run `auth-api healthcheck` and
  `repo-api healthcheck`.
- The admin commands `repo-api admin ...` and `auth-api admin ...` (steps 5,
  8 and 9).
- `TRUST_PROXY=true` behind Caddy: the client address is then the last entry
  of `X-Forwarded-For`.

## 2. Get the image built

The image is built by GitHub Actions, not on the droplet. Merge the backend
to `main` (or push to `staging`); the workflow `Backend` runs `cargo test` in
`backend/`, builds `backend/Dockerfile` with the repository root as context
and pushes:

```
ghcr.io/ewitulsk/ecm-repo-backend:<commit sha>
ghcr.io/ewitulsk/ecm-repo-backend:<branch name>
```

The first push to `main` also starts the `deploy` job. It waits for approval
or fails because the `production` environment has no secrets yet. That is
expected; ignore it until step 12.

Check that the image exists: GitHub > your profile > Packages >
`ecm-repo-backend`.

## 3. DigitalOcean credentials

Two different credentials are needed, and Spaces keys are not the same thing
as the API token.

**API token** (droplet, firewall, reserved IP, DNS):

1. Control panel > API > Tokens > Generate New Token.
2. Name `ecm-repo-terraform`, an expiry you are comfortable with, scope
   **Full Access**. (Custom scopes also work if you grant create/read/update/
   delete on droplets, reserved IPs, firewalls, SSH keys, tags, projects and
   domains; the exact minimal set was not tested.)
3. Copy the token now. It is shown once.

**Spaces key for Terraform** (creates the buckets):

1. Control panel > Spaces Object Storage > Access Keys > Create Access Key.
2. Access scope **Full Access** (a limited key cannot create buckets). Name
   `ecm-repo-terraform`.
3. Copy the access key ID and the secret. The secret is shown once.

If the account has never used Spaces, the panel asks you to start the Spaces
subscription first.

Put all three in your shell for this session. They are read from the
environment and are never written to a file in the repository:

```sh
export DIGITALOCEAN_TOKEN='dop_v1_...'
export SPACES_ACCESS_KEY_ID='DO00...'
export SPACES_SECRET_ACCESS_KEY='...'
```

A second, weaker Spaces key for the droplet is made in step 6, after the
buckets exist.

## 4. E-mail provider

The services send verification and password-reset mail through the provider's
**HTTPS API**, not SMTP. DigitalOcean commonly blocks outbound SMTP (ports 25,
465 and 587) on new accounts, so SMTP from a droplet often does not work at
all; HTTPS on 443 always does.

Choose one:

- **Resend** (`EMAIL_PROVIDER=resend`)
- **Postmark** (`EMAIL_PROVIDER=postmark`). New Postmark accounts start in a
  test mode that only delivers to your own domain until you request approval.

Then:

1. Create the account and add a sending domain. Decide the sender address
   first. If `evanwitulski.com` already sends mail (Google Workspace,
   Fastmail, ...), add a subdomain such as `mail.evanwitulski.com` at the
   provider and send from `noreply@mail.evanwitulski.com`, so the existing SPF
   record stays untouched. Otherwise use `evanwitulski.com` and
   `noreply@evanwitulski.com`.
2. The provider shows DNS records to create. Create exactly what it shows.
   What to expect:

   | Record | Purpose | Typical shape |
   |---|---|---|
   | DKIM | signs the mail | a TXT (or CNAME) at `<selector>._domainkey.<domain>` with a long key |
   | SPF | says who may send | a TXT `v=spf1 include:<provider host> ~all`, usually on a bounce subdomain the provider names, together with an MX (Resend) or as a CNAME for the return path (Postmark) |
   | DMARC | tells receivers what to do when both fail | a TXT at `_dmarc.<domain>` |

   A name may have only one SPF record. If the name the provider asks for
   already has one, merge the `include:` into it instead of adding a second.

   If `_dmarc.evanwitulski.com` does not exist yet, start with:

   ```
   _dmarc.evanwitulski.com.  TXT  "v=DMARC1; p=none; rua=mailto:you@evanwitulski.com"
   ```

   and tighten `p=none` to `p=quarantine` once mail is arriving.
3. Wait until the provider shows the domain as verified.
4. Create an API key that may only send mail. This is `EMAIL_API_KEY`.

## 5. Generate the secrets and write the `.env`

On your machine, outside the repository:

```sh
mkdir -p ~/ecm-repo-secrets && cd ~/ecm-repo-secrets && chmod 700 .
cp /path/to/EvansComputerMod/infra/docker/.env.example .env
chmod 600 .env

openssl rand -hex 32     # INTERNAL_SECRET (must be at least 16 characters)
```

`REPO_SIGNING_KEY` is 64 hex digits, the ed25519 seed the repository signs
with. If Docker runs on your machine and you can pull the image (log in first
when it is private, step 8), let the backend make it; this needs no
configuration and prints the public key too:

```sh
docker run --rm ghcr.io/ewitulsk/ecm-repo-backend:main repo-api admin keygen
```

```
REPO_SIGNING_KEY=<64 hex digits>
# public key (pkg repo add ... --key): <64 hex digits>
```

Copy the first line into `.env` and keep the public key for step 11. Without
Docker, `openssl rand -hex 32` makes an equally good `REPO_SIGNING_KEY`, and
the public key is printed in step 8.

Edit `.env`:

| Variable | Value |
|---|---|
| `IMAGE` | `ghcr.io/ewitulsk/ecm-repo-backend` (owner in lower case) |
| `IMAGE_TAG` | `main` |
| `GHCR_USER`, `GHCR_READ_TOKEN` | step 8; empty when the image is public |
| `DOMAIN` | `repo.evanwitulski.com` |
| `PUBLIC_ORIGIN` | `https://repo.evanwitulski.com` |
| `ACME_EMAIL` | your address; Let's Encrypt sends expiry warnings there |
| `INTERNAL_SECRET` | the `openssl` output |
| `EMAIL_PROVIDER`, `EMAIL_API_KEY`, `EMAIL_FROM` | step 4. `EMAIL_FROM` may carry a display name: `ECM packages <noreply@evanwitulski.com>` |
| `ADMIN_EMAILS` | the address you will sign up with (step 9) |
| `REPO_SIGNING_KEY` | from `repo-api admin keygen` or `openssl rand -hex 32` |
| `STORAGE` | `s3` |
| `S3_PATH_STYLE` | `auto` (virtual-hosted addressing for `*.digitaloceanspaces.com`) |
| `S3_*`, `LITESTREAM_*`, `APP_UID`, `APP_GID` | filled in at step 6 |

Do not put quotes around values, and do not use `$` in a value.

**`REPO_SIGNING_KEY` is the identity of the repository.** Every index entry is
signed with it and the mod pins its public half. If it is lost or changed,
every installed client rejects the repository until it ships a new key. Store
the whole `.env` in a password manager now. It is also the only thing you need,
besides this repository, to rebuild the server.

The public key that the mod pins is printed by `keygen` above and again at the
end of step 8.

## 6. Create the infrastructure

```sh
cd infra/terraform
cp terraform.tfvars.example terraform.tfvars
```

Edit `terraform.tfvars`:

- `admin_ssh_public_key`: the whole line from `~/.ssh/id_ed25519.pub`.
- `ssh_allowed_cidrs`: who may reach port 22.
  - The default in `variables.tf` is `[]`: **port 22 is closed to everyone and
    you cannot log in.** This is deliberate; you must choose.
  - Your address only: `["<your address>/32"]`. Find it with
    `curl -4 https://ifconfig.me`. When your address changes, edit and
    `terraform apply` again.
  - For deploys from GitHub Actions: `["0.0.0.0/0", "::/0"]`. Password logins
    and root logins are disabled and fail2ban is installed, so this is the
    usual key-only SSH exposure.
- `bucket_prefix`: for example `evanwitulski-ecm-repo`. The buckets become
  `<prefix>-packages` and `<prefix>-db`.
- `manage_dns`: `true` only if the zone is hosted at DigitalOcean (step 7).

State is kept in the local file `terraform.tfstate`. It holds no secrets from
this guide, but do not lose it: it is how Terraform knows what it created.
Moving it to a bucket is optional and described under "Remote state" below.

```sh
terraform init
terraform plan
```

Read the plan: one droplet, one SSH key, one reserved IP and its assignment,
one firewall, two buckets (plus a project and a DNS record if you asked for
them). Nothing else, and nothing destroyed.

```sh
terraform apply
terraform output
```

`reserved_ip` is the address of the service. `env_hints` prints lines for the
`.env`:

```sh
terraform output -raw env_hints
```

Copy them over the matching lines in `~/ecm-repo-secrets/.env`.

**Spaces key for the droplet.** Now that the buckets exist:

1. Control panel > Spaces Object Storage > Access Keys > Create Access Key.
2. Access scope **Limited Access**. Select both buckets
   (`<prefix>-packages` and `<prefix>-db`) with permission
   **Read/Write/Delete**. Name `ecm-repo-droplet`.
3. Put the key in `.env` four times:

   ```
   S3_ACCESS_KEY=<key id>
   S3_SECRET_KEY=<secret>
   LITESTREAM_ACCESS_KEY_ID=<key id>
   LITESTREAM_SECRET_ACCESS_KEY=<secret>
   ```

The full-access key from step 3 stays on your machine and never goes to the
droplet.

The droplet is now installing Docker, which takes three to five minutes. Wait
for it:

```sh
ssh deploy@"$(terraform output -raw reserved_ip)" 'cloud-init status --wait && ls -a /opt/ecm-repo'
```

Accept the host key when asked. Expected: `status: done` and a listing with
`docker-compose.yml`, `Caddyfile`, `litestream.yml`, `deploy.sh`,
`backup-check.sh`, `.env.example`, `data` and `.cloud-init-done`. If the
connection is refused or times out, see Troubleshooting.

You always log in as `deploy`. Root login is disabled; `deploy` has `sudo`
without a password and is in the `docker` group.

## 7. DNS

The service needs one record:

```
repo.evanwitulski.com.   A   <reserved_ip>   TTL 300
```

- **Zone hosted at DigitalOcean:** set `manage_dns = true` in
  `terraform.tfvars` and run `terraform apply`. The zone itself must already
  exist under Networking > Domains; Terraform only adds the record.
- **Zone hosted elsewhere:** create the record at that host.
  `terraform output dns_records` prints it.

Do not add an AAAA record: the droplet has no IPv6 address. If the zone has
CAA records, one of them must allow `letsencrypt.org`.

Check, and do not go on until it answers with the reserved IP:

```sh
dig +short repo.evanwitulski.com A
```

Caddy asks Let's Encrypt for the certificate the first time it starts, and
Let's Encrypt must reach the droplet by this name. Starting the stack before
the record resolves only delays the certificate (Caddy retries by itself), but
many failed attempts in a row can hit Let's Encrypt's rate limit, so get DNS
right first.

## 8. First deploy

**Registry access.** If the image is private (the default):

1. GitHub > Settings > Developer settings > Personal access tokens > Tokens
   (classic) > Generate new token. Scope **`read:packages` only**. (At the
   time of writing, fine-grained tokens cannot read packages.)
2. In `.env`: `GHCR_USER=<your GitHub user name>` and
   `GHCR_READ_TOKEN=<the token>`.

Or make the package public (GitHub > Packages > `ecm-repo-backend` > Package
settings > Change visibility) and leave both empty.

**Copy the `.env` and deploy:**

```sh
IP="$(terraform -chdir=infra/terraform output -raw reserved_ip)"

scp ~/ecm-repo-secrets/.env deploy@"$IP":/opt/ecm-repo/.env
ssh deploy@"$IP" 'chmod 600 /opt/ecm-repo/.env'
```

The files in `/opt/ecm-repo` were put there by cloud-init at first boot. If
`infra/docker/` changed since `terraform apply`, copy the current ones first:

```sh
cd infra/docker
scp docker-compose.yml Caddyfile litestream.yml .env.example deploy.sh backup-check.sh deploy@"$IP":/opt/ecm-repo/
```

Deploy:

```sh
ssh deploy@"$IP"
cd /opt/ecm-repo
./deploy.sh
```

`deploy.sh` checks the `.env`, logs in to GHCR, pulls the images, restores the
databases from the bucket if they are missing (on this first deploy there is
nothing to restore and it says so), starts the stack, waits until both
services pass their health check, checks `https://repo.evanwitulski.com/v1/health`
with certificate verification, and prints `docker compose ps`.

Expected at the end: four running containers (`auth-api`, `repo-api`,
`litestream`, `caddy`; the first two marked `healthy`) and `Deployed ghcr.io/ewitulsk/ecm-repo-backend:main`.
A warning that the HTTPS check does not answer yet means the certificate is
still being issued; wait a minute and see step 10.

With a private image, `deploy.sh` runs `docker login`, which leaves the
read-only token in `/home/deploy/.docker/config.json` on the droplet.

**Print the repository's public key:**

```sh
docker compose exec repo-api repo-api admin pubkey
```

Write the printed key down next to the `.env`. It is what the mod pins and
what goes into the game server's configuration in step 11. It must equal the
public key `keygen` printed, if you used it. The same key is in the `repo-api`
log at startup and is the `publisher_key` field of every index
(`curl -s https://repo.evanwitulski.com/r/stable/index.json`).

## 9. Create the first admin account

An account is an admin when its **confirmed** e-mail address is listed in
`ADMIN_EMAILS`. Two ways to make it; both end with the same account.

**A. On the web** (also tests that mail works):

1. Open `https://repo.evanwitulski.com/signup` and sign up with the address
   you put in `ADMIN_EMAILS`. Passwords need at least 10 characters; the
   handle is 3 to 32 of `a-z 0-9 _ -` and is shown as the publisher.
2. Open the verification mail and follow the link. If no mail arrives, see
   Troubleshooting, or use B.
3. Sign in at `https://repo.evanwitulski.com/signin`. On `/account`, create an
   API key; it starts with `ecm_k_` and is shown once.

**B. On the droplet, without mail:**

```sh
cd /opt/ecm-repo
docker compose exec auth-api auth-api admin user-add you@evanwitulski.com evan 'a-long-password'
```

This creates the account with the address already confirmed. It applies the
same rules as the sign-up page and fails if the address or the handle exists.
The password is a command argument, so it is in the shell history and was
briefly in the process list: change it on `/account` afterwards.

Make an API key for it. `key-add` prints only the key, so it can be captured;
use `-T` when capturing:

```sh
KEY=$(docker compose exec -T auth-api auth-api admin key-add evan laptop)
echo "$KEY"
```

The default scopes are `social,publish`; give others as a third argument
(`key-add evan ci publish`).

**Check, either way:**

```sh
docker compose exec auth-api auth-api admin users
```

lists handle, e-mail, `verified=yes` and `admin=yes` for your account. From
your machine:

```sh
curl -s -H "Authorization: Bearer ecm_k_..." https://repo.evanwitulski.com/auth/v1/whoami
curl -s -H "Authorization: Bearer ecm_k_..." https://repo.evanwitulski.com/v1/admin/reports
```

`whoami` must show `"email_verified":true` and `admin` among the scopes; the
second call must answer 200, not 403.

`ADMIN_EMAILS` is read each time a token is looked up, not when the account is
made. To make an existing account an admin later, add its address to
`ADMIN_EMAILS` in `/opt/ecm-repo/.env` and run `./deploy.sh`, which restarts
`auth-api` with the new value. `repo-api` follows within 30 seconds. Removing
an address takes the right away the same way.

## 10. Verify

From your machine:

```sh
# The client's connectivity check: 200 and "ok".
curl -i https://repo.evanwitulski.com/v1/health

# The way the in-game client talks: HTTP/1.1, Connection: close.
# The response must have Content-Length and no Transfer-Encoding.
curl -i --http1.1 -H 'Connection: close' https://repo.evanwitulski.com/r/community/index.json

# The stable channel's index; its publisher_key is the key printed in step 8.
# "packages" is empty until something is promoted.
curl -s https://repo.evanwitulski.com/r/stable/index.json | head -c 400; echo

# The sign-in page: 200, text/html, with the security headers.
curl -sI https://repo.evanwitulski.com/signin

# Plain HTTP only redirects: 301 and Location: https://...
curl -sI http://repo.evanwitulski.com/v1/health

# /internal is not reachable from outside: 404.
curl -s -o /dev/null -w '%{http_code}\n' -X POST https://repo.evanwitulski.com/internal/introspect

# The certificate: issued by Let's Encrypt, ECDSA key.
echo | openssl s_client -connect repo.evanwitulski.com:443 -servername repo.evanwitulski.com 2>/dev/null | openssl x509 -noout -issuer -dates -text | grep -E 'Issuer:|issuer=|notAfter|Public Key Algorithm'
```

On the droplet, prove that the backups can be restored:

```sh
cd /opt/ecm-repo
docker compose logs --tail 20 litestream     # no errors; "snapshot complete" lines
./backup-check.sh
```

`backup-check.sh` restores both databases from the bucket into a temporary
directory, runs `PRAGMA integrity_check` on the copies and ends with
`OK: both databases restore from the bucket and pass the integrity check.`
Run it now and again after any change to the buckets, keys or Litestream.

## 11. Point the game at the repository

The repository has two channels. `community` holds every upload, unreviewed.
`stable` is curated: a version gets there only when an admin promotes it
(`repo-api admin promote <id> <version> stable`, see Day-two operations).

On the Minecraft server, edit `config/evanscomputermod-common.toml`:

```toml
[repo]
url = "https://repo.evanwitulski.com"
key = "<the 64 hex digits from repo-api admin pubkey>"
access = "internet"
allowCommunity = true
allowPublish = true
cacheMiB = 256

[internet]
maxKbps = 0
```

| Setting | Meaning |
|---|---|
| `url` | the repository's address, without a path |
| `key` | the public key from step 8. Computers install official packages only when they are signed with it. Empty: no official repository is set up on computers. |
| `access` | how computers reach the repository; see below |
| `allowCommunity` | also offer the `community` channel |
| `allowPublish` | let players sign in and publish through the server's own link (`access = "open"`). With `internet`, players publish over the in-game network and this does not switch that off. |
| `cacheMiB` | disk the server may use for packages it fetched for computers (16 to 16384) |
| `[internet] maxKbps` | download cap of one connection through an Internet Gateway, in kilobytes per second; 0 is no cap. Read when the server starts. |

`access`:

- `open`: every computer, with no in-game network needed; the server fetches
  for it. Needs the `evanscomputermod-internet` add-on on the server.
- `internet` (the default): a computer needs a route to an Internet Gateway,
  and the server needs the add-on.
- `datacenter`: only repository servers reach it: computers an operator names
  with `/ecm repo grant <pos>`, and those in Tech Villages. They re-serve what
  they choose to other computers with `pkg mirror` and `pkg serve`. Needs the
  add-on.
- `off`: not at all; only repositories inside the game.

Restart the server. Computers get the entries `official` and `community` in
`/etc/pkg/repos` when they boot. In the game, as an operator:

```
/ecm repo status
```

shows what the server is doing. On a computer:

```
pkg online
pkg search <words>
pkg install <id>
pkg login
pkg publish <dir>
```

`pkg online` says which repositories answer from that computer and, if one
does not, why. `pkg login` shows a
code and a link to `https://repo.evanwitulski.com/device`; open it in a
browser, sign in and approve. `pkg publish` then uploads to `community`.

The game's client verifies the Let's Encrypt certificate chain against built-in
public roots and supports ECDSA P-256, ECDSA P-384 and RSA, so the certificate
Caddy requests by default works without further configuration.

A full first test: publish a small package from a computer, see it at
`https://repo.evanwitulski.com/` and in `/r/community/index.json`, promote it
on the droplet, and install it on a second computer.

## 12. Deploys from GitHub Actions

Skip this section to keep deploying by hand (`./deploy.sh <tag>` over SSH).

**Port 22 must be reachable from GitHub's runners:** set
`ssh_allowed_cidrs = ["0.0.0.0/0", "::/0"]` and `terraform apply`.

**A key pair for CI**, separate from your own:

```sh
ssh-keygen -t ed25519 -N '' -C 'github-actions-deploy' -f ~/ecm-repo-secrets/ci_deploy_key
```

Install the public half on the droplet (no rebuild needed):

```sh
ssh deploy@"$IP" 'cat >> ~/.ssh/authorized_keys' < ~/ecm-repo-secrets/ci_deploy_key.pub
```

Also put the same line in `ci_ssh_public_key` in `terraform.tfvars`, so that a
rebuilt droplet gets it again. (This does not change the running droplet.)

**The droplet's host key**, so the workflow can refuse an impostor:

```sh
ssh-keyscan -t ed25519 "$IP" 2>/dev/null
```

Compare the key with the line for that address in your own
`~/.ssh/known_hosts` (`ssh-keygen -F "$IP"`); they must match.

**The environment:** GitHub > the repository > Settings > Environments > New
environment, named exactly `production`.

- Deployment branches and tags: restrict to `main`.
- Required reviewers: add yourself if every deploy should wait for a click.

Add these as **environment secrets** of `production`:

| Secret | Value |
|---|---|
| `DEPLOY_HOST` | the reserved IP |
| `DEPLOY_USER` | `deploy` |
| `DEPLOY_SSH_KEY` | the whole contents of `~/ecm-repo-secrets/ci_deploy_key` (the private key) |
| `DEPLOY_KNOWN_HOSTS` | the `ssh-keyscan` output line |
| `GHCR_READ_USER` | your GitHub user name (omit when the image is public) |
| `GHCR_READ_TOKEN` | the `read:packages` token from step 8 (omit when the image is public) |
| `DEPLOY_ENV_FILE` | optional: the whole `.env`. When set, every deploy overwrites `/opt/ecm-repo/.env` with it. When not set, the `.env` on the droplet is used. |

If you set `DEPLOY_ENV_FILE`, the secret becomes the source of truth: change
values there, not on the droplet. Its `IMAGE_TAG` line does not matter; the
deploy sets it.

What the workflow does:

- Push to `main` or `staging` touching `backend/**`,
  `rust/wasm-programs/pkg/**`, `infra/docker/**` or the workflow: `cargo test`,
  build, push the tags `<sha>` and `<branch>`.
- Push to `main`: then deploy that commit's image (`<sha>`) to the droplet.
  `staging` is built but never deployed.
- Actions > Backend > Run workflow, with an image tag: deploy that tag without
  building. This is also the rollback button.

The deploy job copies `infra/docker/*` to `/opt/ecm-repo` and runs
`./deploy.sh <tag>` over SSH.

Whoever holds the CI key is root on the droplet (the `deploy` user is in the
`docker` group). Keep it only in the environment secret.

---

## Day-two operations

All commands run on the droplet in `/opt/ecm-repo` unless they say otherwise.

### Logs and status

```sh
docker compose ps
docker compose logs -f --tail 100 auth-api repo-api
docker compose logs --tail 100 caddy          # access log (JSON) and certificate messages
docker compose logs --tail 100 litestream
docker stats --no-stream
df -h / && free -m
```

Container logs are rotated (5 files of 10 MB per container). With
`EMAIL_PROVIDER=log`, the mails are in the `auth-api` log.

### Upgrade

With CI: merge to `main`. By hand:

```sh
./deploy.sh <commit sha>      # or: ./deploy.sh main
```

Only containers whose image or configuration changed are recreated. Caddy
retries for a few seconds while a service restarts, so most requests during a
deploy are delayed, not failed.

To update Caddy or Litestream themselves, change the image tag in
`infra/docker/docker-compose.yml`, commit, and deploy. After changing the
Litestream version, read its release notes and run `./backup-check.sh`.

### Roll back

Every build is kept in GHCR under its commit SHA.

```sh
cat .previous-image-tag           # the tag that ran before the last change
./deploy.sh "$(cat .previous-image-tag)"
```

or Actions > Backend > Run workflow with the old SHA. A rollback changes the
code, not the data: if the newer version migrated the database, the older one
must be able to read it.

### Rotate secrets

Edit `.env` (or the `DEPLOY_ENV_FILE` secret), then `./deploy.sh`. The
containers that use a changed value are recreated.

| Secret | How |
|---|---|
| `INTERNAL_SECRET` | new `openssl rand -hex 32` (at least 16 characters); deploy. Both services restart together. |
| `EMAIL_API_KEY` | new key at the provider; deploy; delete the old key. |
| Droplet Spaces key | create a new limited key (step 6), replace all four values, deploy, run `./backup-check.sh`, then delete the old key in the control panel. |
| `GHCR_READ_TOKEN` | new token; update `.env` or the GitHub secret; deploy; delete the old token. |
| CI SSH key | new key pair; add the public half to `~/.ssh/authorized_keys` and remove the old line; update `DEPLOY_SSH_KEY`. |
| DigitalOcean API token, Terraform Spaces key | only on your machine: create new ones, delete the old. |
| `REPO_SIGNING_KEY` | **Do not rotate casually.** A new key needs a mod release that pins the new public key, and every existing `pkg repo add ... --key` stops verifying. Only when the key has leaked. |

User passwords, sessions and API keys are in `auth.db` and are not affected by
any of these.

### Restore onto a new droplet

Use this when the droplet is broken, compromised, or you want a fresh
operating system. Needed: this repository, `terraform.tfstate`, and the
`.env`.

1. If the old droplet still runs, stop it cleanly so the last writes reach
   the bucket:

   ```sh
   docker compose stop caddy auth-api repo-api
   sleep 5
   docker compose stop litestream
   ```

2. Replace the droplet. The reserved IP, firewall, buckets and DNS stay:

   ```sh
   cd infra/terraform
   terraform apply -replace=digitalocean_droplet.this
   ```

3. The new machine has a new host key:

   ```sh
   ssh-keygen -R "$IP"
   ssh deploy@"$IP" 'cloud-init status --wait'
   ```

4. Copy the `.env` back and deploy:

   ```sh
   scp ~/ecm-repo-secrets/.env deploy@"$IP":/opt/ecm-repo/.env
   ssh deploy@"$IP" 'cd /opt/ecm-repo && ./deploy.sh'
   ```

   `data/` is empty, so the `restore-auth` and `restore-repo` steps download
   both databases from the bucket before the services start. If the bucket
   cannot be read, the deploy stops and nothing starts; it never starts with
   empty databases over an existing replica.

5. Verify as in step 10. Sign in with an existing account: it must still
   exist. Update `DEPLOY_KNOWN_HOSTS` (step 12) because the host key changed.

Caddy requests a new certificate on the new droplet. Let's Encrypt allows only
a handful of identical certificates per week, so do not rebuild repeatedly in
a short time.

### Restore to an earlier point in time

Litestream keeps daily snapshots for 7 days and the changes between them. To
look at an older state without touching the live database:

```sh
mkdir -p /tmp/pitr && chmod 777 /tmp/pitr
docker compose run --rm --no-deps -v /tmp/pitr:/restore litestream \
  restore -timestamp 2026-01-31T12:00:00Z -o /restore/repo.db /data/repo.db
sqlite3 /tmp/pitr/repo.db 'PRAGMA integrity_check;'
```

To put that state into service: stop `auth-api`, `repo-api` and `litestream`,
move `data/repo.db`, `data/repo.db-wal`, `data/repo.db-shm` and
`data/.repo.db-litestream` aside, copy the restored file to `data/repo.db`
owned by `10001:10001`, and run `./deploy.sh`. This last procedure has not
been rehearsed; try it on a copy first.

Package files that were deleted or overwritten stay in the `-packages` bucket
as noncurrent versions for 30 days (`packages_noncurrent_days`) and can be
brought back with any S3 client that understands versions.

### Takedowns and other admin actions

On the droplet. The commands work on the same database and bucket as the
running service, which picks the changes up at once:

```sh
docker compose exec repo-api repo-api admin help
docker compose exec repo-api repo-api admin reports
docker compose exec repo-api repo-api admin takedown <id>              # the whole package
docker compose exec repo-api repo-api admin takedown <id> <version>    # one version
docker compose exec repo-api repo-api admin promote <id> <version> stable
docker compose exec repo-api repo-api admin demote <id> <version> stable
docker compose exec repo-api repo-api admin channel <name> "<title>"   # a new channel
docker compose exec auth-api auth-api admin users
```

Or from anywhere, with an admin API key (step 9):

```sh
KEY=ecm_k_...
curl -s -H "Authorization: Bearer $KEY" https://repo.evanwitulski.com/v1/admin/reports

curl -s -X POST -H "Authorization: Bearer $KEY" -H 'Content-Type: application/json' \
  -d '{"id":"com.example.bad","version":null}' \
  https://repo.evanwitulski.com/v1/admin/takedown

curl -s -X POST -H "Authorization: Bearer $KEY" -H 'Content-Type: application/json' \
  -d '{"id":"com.example.good","version":"1.2.0","channel":"stable"}' \
  https://repo.evanwitulski.com/v1/admin/promote
```

A takedown removes the files and the index entries. Clients that already
installed the package keep their copy. A taken-down package keeps its id
(nobody can publish under it again), and a taken-down or yanked version number
cannot be reused.

The admin calls that change something answer `204` with no body.

### Resize

```sh
# infra/terraform/terraform.tfvars
droplet_size = "s-1vcpu-2gb"
```

```sh
terraform apply
```

The droplet is powered off, resized and started again: a few minutes of
downtime, same IP, same disk. Only CPU and memory change, so it can be sized
back down later. Size slugs: `doctl compute size list` or the API docs.

Signs that 1 GB is too little: `free -m` shows swap in constant use, or
`dmesg | grep -i oom` shows killed processes.

### Operating system

Security updates install themselves. To update Docker and everything else:

```sh
sudo apt-get update && sudo apt-get dist-upgrade -y
sudo reboot
```

The stack starts by itself after a reboot.

### Remote state (optional)

`infra/terraform/backend.tf.example` holds an S3 backend block for a separate
Spaces bucket, with the steps as comments at its top. Do it once the
deployment works; local state is fine to start with. There is no state
locking with this backend, so only one person applies at a time.

---

## Teardown

This deletes the service and all its data.

1. Optional: keep a last copy of the data.

   ```sh
   cd /opt/ecm-repo && ./backup-check.sh    # proves the replica is good
   ```

   and download the buckets with an S3 client if you want the files.

2. Destroy. The buckets are versioned and not empty, so allow Terraform to
   empty them first:

   ```sh
   cd infra/terraform
   terraform apply -var bucket_force_destroy=true
   terraform destroy -var bucket_force_destroy=true
   ```

3. By hand, because Terraform did not create them:
   - both Spaces access keys and the API token (control panel > API, and
     Spaces Object Storage > Access Keys);
   - the state bucket, if you made one;
   - the Spaces subscription, if no other bucket needs it (check the billing
     page: it is charged while it is active);
   - the A record, if `manage_dns` was false, and the e-mail provider's
     records;
   - the sending domain and API key at the e-mail provider;
   - the GHCR package, the `production` environment and its secrets, and the
     `read:packages` token on GitHub.

4. Check the DigitalOcean billing page a day later: no droplet, no reserved
   IP, no Spaces.

---

## Troubleshooting

**`ssh` times out.** Port 22 is closed to your address. Check
`ssh_allowed_cidrs` against `curl -4 https://ifconfig.me`, then
`terraform apply`. If you are locked out entirely, use the droplet's Recovery
Console in the control panel, or rebuild (Restore onto a new droplet).

**`ssh` says `Permission denied (publickey)`.** Log in as `deploy`, not
`root`, with the key whose public half is in `admin_ssh_public_key`.

**`ssh` warns that the host key changed.** Expected after a rebuild:
`ssh-keygen -R <ip>`.

**`cloud-init status` says `error`, or `/opt/ecm-repo` is incomplete.**

```sh
sudo cloud-init status --long
sudo tail -n 80 /var/log/cloud-init-output.log
```

Usually a package mirror was briefly unreachable. Rebuilding the droplet
(`terraform apply -replace=digitalocean_droplet.this`) is quicker than
repairing it.

**`terraform apply` fails on a bucket with `BucketAlreadyExists` or
`Conflict`.** The name is taken by someone. Change `bucket_prefix`.

**`terraform apply` fails on a bucket with `403`, `InvalidAccessKeyId` or
asks for `spaces_access_id`.** `SPACES_ACCESS_KEY_ID` and
`SPACES_SECRET_ACCESS_KEY` are not exported in this shell, or the key is a
limited one. Step 3.

**`terraform apply` fails on the DNS record with `404`.** `manage_dns = true`
but the zone `evanwitulski.com` is not in this DigitalOcean account. Add it
under Networking > Domains, or set `manage_dns = false`.

**`terraform plan` wants to replace the droplet.** It should not: changes to
`user_data`, `image` and `ssh_keys` are ignored on purpose. Changing `region`
or `name` does replace it. Read the plan line marked `# forces replacement`.

**`deploy.sh`: `pull access denied` or `unauthorized`.** The image is private
and the droplet is not logged in, or `IMAGE` has upper-case letters, or the tag
does not exist. Check `GHCR_USER`/`GHCR_READ_TOKEN`, then
`docker pull "$(grep '^IMAGE=' .env | cut -d= -f2-):main"`.

**`deploy.sh`: `required variable ... is missing a value`.** That line of
`.env` is empty.

**`deploy.sh`: `restore-auth failed`.** Litestream cannot read the bucket.
Check `LITESTREAM_BUCKET`, `LITESTREAM_ENDPOINT`
(`https://<region>.digitaloceanspaces.com`, with no bucket name in it),
`LITESTREAM_REGION` and the key, and that the limited key covers the `-db`
bucket. On `SignatureDoesNotMatch` the secret is wrong. If the endpoint and key
are right and it still fails to sign, try `LITESTREAM_REGION=us-east-1`, which
S3 clients conventionally use for Spaces.

**The services do not become healthy.**

```sh
docker compose ps --all
docker compose logs --tail 100 auth-api repo-api
```

- `permission denied` on `/data`: `ls -ln data` must show owner `10001`.
  `docker compose run --rm data-init` fixes it; `APP_UID` in `.env` must match
  `app_uid` in Terraform.
- `repo-api` restarts in a loop with `the blob store does not work: s3 bucket
  ...`: it checks the bucket at startup and cannot use it. See the next entry.
- `INTERNAL_SECRET must be at least 16 characters`, `S3_PATH_STYLE must be
  auto, true or false`: fix that line of `.env`.
- `Read-only file system`: the containers run with a read-only root file
  system and may write only to `/data` and `/tmp`.

**`repo-api` cannot use the bucket, or Spaces rejects the signature
(`SignatureDoesNotMatch`, `403`, `400`).** Check `S3_ENDPOINT`
(`https://<region>.digitaloceanspaces.com`, with no bucket name in it),
`S3_BUCKET`, the key, and that the limited key covers the `-packages` bucket
with Read/Write/Delete. If those are right, try, one at a time, in `.env`:

```
S3_REGION=us-east-1
S3_PATH_STYLE=true
```

then `./deploy.sh`. `S3_PATH_STYLE=auto` uses `bucket.host` addresses for
`*.digitaloceanspaces.com`; `true` forces `host/bucket`.

**The browser shows a certificate error, or `curl` says `SSL` / `internal
error`.** No certificate yet.

```sh
docker compose logs --tail 100 caddy | grep -iE 'acme|certificate|error'
dig +short repo.evanwitulski.com A          # must be the reserved IP
```

Causes, most likely first: the A record is missing or wrong; an AAAA record
points somewhere else; a CAA record forbids Let's Encrypt; ports 80 and 443
are blocked (they are open in this firewall); a rate limit after many failed
attempts (wait an hour). Caddy retries by itself; `docker compose restart
caddy` forces a try now.

**`502` or `503` with `{"error":"internal",...}`.** Caddy runs but the service
behind it does not answer. `docker compose ps` and the service's log.

**`413` on publish for a package under 8 MiB.** `MAX_PACKAGE_BYTES` was raised
but `PUBLISH_BODY_LIMIT` (Caddy's limit, a little above it) was not. Set both
in `.env` and deploy.

**Rate limits hit everyone at once, or the logs show one client address for
all requests.** The services are not seeing the real client address. They must
run with `TRUST_PROXY=true` (set in `docker-compose.yml`); they then use the
last entry of `X-Forwarded-For`, which Caddy sets to the address of the
connecting client, discarding whatever the client sent.
If you later put a CDN or load balancer in front of Caddy, add
`trusted_proxies` to the `servers` block of the `Caddyfile`.

**No verification mail.**

```sh
docker compose logs --tail 100 auth-api | grep -i mail
```

Check: the sending domain is verified at the provider; `EMAIL_FROM` is on that
domain; `EMAIL_API_KEY` is right; the provider's activity page (it shows
bounces and rejections); Postmark accounts awaiting approval deliver only to
their own domain. To get in while mail is broken, create the account with
`auth-api admin user-add` (step 9, B), or set `EMAIL_PROVIDER=log`, deploy, and
read the verification link from the `auth-api` log; then set it back. The services never use SMTP, so DigitalOcean's SMTP block is not the
cause.

**Litestream logs errors.**

```sh
docker compose logs --tail 100 litestream
./backup-check.sh
```

`AccessDenied`: the droplet's Spaces key was deleted or does not cover the
`-db` bucket. While Litestream is failing, the databases on the droplet are
the only copy: fix it before doing anything that could lose the droplet.

**The disk is full.**

```sh
df -h /
docker system df
docker image prune -a      # removes images no container uses
```

**The GitHub deploy job fails at "Set up SSH" or times out connecting.** A
secret is missing in the `production` environment, or port 22 is not open to
the internet (step 12). `Host key verification failed`: the droplet was
rebuilt; update `DEPLOY_KNOWN_HOSTS`.

**The GitHub deploy job is skipped.** It runs only for pushes to `main` whose
image was built, and for manual runs. A push to `staging` never deploys.

---

## Build context

`backend/Dockerfile` is built with the repository root as context because the
backend depends on `rust/wasm-programs/pkg` by path, and `pkg` inherits its
dependencies from the workspace manifest `rust/Cargo.toml`.

The root `.dockerignore` is an allow-list. The context is `backend/`,
`rust/Cargo.toml`, `rust/Cargo.lock`, `rust/crates/` and
`rust/wasm-programs/pkg/`, about 10 MB.

What was tested: cargo, building from `backend/` (its own workspace), needs
`rust/Cargo.toml` to exist, because `pkg` uses `workspace = true`; without it
the build stops with `failed to find a workspace root`. It does **not** need
the other workspace members listed in that file: with only the paths above
present, `cargo metadata` and `cargo check` succeed. The `[patch]` section of
`rust/Cargo.toml` is ignored when building from another workspace, so
`rust/crates/rustpython-vm-patch` is not needed either (it is in the context
only because all of `rust/crates/` is).

This stops being true if the backend becomes a member of the `rust/`
workspace or cargo is run from inside `rust/`: then cargo loads every member
and fails on the first missing `Cargo.toml`.

If `pkg` gains a path dependency outside `rust/crates/`, add that path to
`.dockerignore` with a `!` line.

To build the image locally:

```sh
docker build -f backend/Dockerfile -t ecm-repo-backend:dev .
```

## What was and was not tested

Checked: `terraform validate` and an offline `terraform plan`,
`caddy validate`, `docker compose config`, `actionlint`, `shellcheck`.

Run locally with the real backend image (built from `backend/Dockerfile` with
the root `.dockerignore`), this compose file, `Caddyfile`, `litestream.yml`,
`deploy.sh` and `backup-check.sh`, against a local S3 server and Caddy's
internal certificate: the health checks, routing, headers, body limits, the
admin commands of steps 5, 8 and 9, `ADMIN_EMAILS` added later, a restore into
an empty data directory, and `backup-check.sh`.

Not tested, because it needs a real account or the game: `terraform apply`,
cloud-init on a real droplet, Litestream and `repo-api` against Spaces itself
(including which `S3_REGION` and `S3_PATH_STYLE` Spaces accepts), certificate
issuance by Let's Encrypt, sending mail through Resend or Postmark, the GitHub
workflow, and step 11 in a running Minecraft server.
