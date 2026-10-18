# Accounts (`ecm-auth`, `authd`)

Accounts let one player be the same person on every backend that trusts the same realm:
Discord (`discordd`), Music (`musicd`) and anything a player builds. They are made so that a
fake server on the network can never get a login it can use: no password ever crosses the
network, realms are known by a pinned key and not by a name or address, and a token taken
from one backend is useless at another.

- Crate: `rust/crates/ecm-auth` (pure logic: keys, the handshake, tokens, the server state
  machine, pins and saved logins; feature `host` adds sockets and files for programs).
- Program: `authd`, the general-purpose Auth server (`man authd`).
- Scripting: the `auth` module in Python and Lua (`man auth`).
- Apps: Discord and Music use the shared sign-in screen (`ecm_apps_ui::signin`).

## Realms and pinning

A **realm** is an `authd`, or a backend running its own accounts (`--auth local`). Its
identity is its ed25519 **realm key**; its name ("Tech Village") is only a label. The
**fingerprint** is the first 16 bytes of SHA-256 of the public key, written as hex
(`bea5d959...`) or in pairs (`BE:A5:D9:59:...`).

Each app keeps its own logins, one per realm key, in its data folder
(`/var/apps/<app>/accounts/`: `pins`, one file per realm fingerprint, `last`). Discord's logins
are invisible to Music and the other way round. Scripts keep theirs next to the script
(`./.auth/`), the console `chat` command in `~/.chat/accounts/`.

- **First contact**: the sign-in screen shows the realm's name, host and fingerprint. Signing
  in or up pins the key.
- **Same name or address, different key**: refused. The app shows "The identity of <name> has
  changed. This could be someone pretending to be it." with both fingerprints. The saved
  login is never offered to the new key, and there is no "trust the new key" button: the
  player removes the old pin by hand (Discord: Settings > Accounts > Forget realm; Music:
  Settings > Accounts; `chat --forget <fp>`; Python/Lua `auth.unpin(...)`), and only then is
  the new key a first contact.
- **A backend that names a realm**: backends say which realm they trust
  (`AUTH <fp> <host:port|-:port> <name>`); the app goes to that address and checks that the
  key found there has that fingerprint. A pinned realm signs in silently.
- **Tech Villages**: every village computer and Phone Store phone ships with the village realm
  pinned for Discord and Music (the key comes from the world seed, like the app repository's).

## Password to key

The client turns the password into an ed25519 key pair:

```
seed = PBKDF2-HMAC-SHA256(password, salt = "ecm-auth-v1" || realm public key || lowercase(username), N)
```

The realm announces N in its signed handshake (default 50,000; clients refuse fewer than 4,096
or more than 250,000); a phone takes a few tenths of a second. Because the realm key is in the
salt, the same password gives unrelated keys on different realms: reusing a password on a
fake realm leaks nothing usable at the real one. The server stores only the public key.

## The handshake

Length-prefixed frames (u32 little-endian length, at most 64 KiB) over TCP (`authd` port 7600):

1. Client to server: an ephemeral X25519 key `ce`.
2. Server to client: its ephemeral `se`, the realm public key, the KDF iteration count, the
   realm name, and an ed25519 signature by the **realm key** over the transcript hash
   `th = SHA-256("ecm-auth-hs1" || ce || se || realm key || name || N)`. The client checks it
   against the pinned key; a spoof without the realm's private key fails here.
3. Both derive two keys from X25519(ce, se) and `th`; everything after this is
   ChaCha20-Poly1305 with a counter nonce per direction (a replayed, dropped or reordered frame
   closes the connection).
4. Client to server: `LOGIN` (username, session public key, a signature by the account key over
   `th`), `SIGNUP` (also the account public key and the display name), or `REFRESH` (a refresh
   token). The session key is a fresh ed25519 key the client made for this sign-in.
5. Server to client: `WELCOME` (user id, username, display name, a refresh token for 30 days) or
   `FAIL <code>`.

Afterwards, on the same channel: `TOKEN <audience>`, `PASSWD`, `WHOAMI`, `LOGOUT`.

A relay in the middle can pass messages 1 and 2 along, but it cannot read the token in 5 (it
has no session key) and cannot swap in its own `ce` (the account signature covers `th`).
`tcpdump` on the network sees neither the password nor anything derived from it.

## Tokens

```
ecmtok1.<base64url payload>.<base64url signature by the realm key>
payload lines: v=1, realm=<fp>, uid=, user=, name=, aud=<backend fp>, cnf=<session public key>, iat=, exp=   (1 hour)
```

A backend has its own ed25519 key (`backend.key` in its data folder; `discordd key`,
`musicd key`) and accepts a token only if it is signed by the realm key it trusts, its audience
is the backend's own fingerprint, it hasn't expired, and the client proves it holds the
session key by signing the backend's fresh nonce:
`proof = sign(session key, "ecm-auth-proof1" || nonce || audience)`. A token sniffed on the
network, or taken from one backend, is useless anywhere else. Discord sends the nonce in its
`BACKEND` greeting line; Music asks `GET /nonce`.

## Standalone or shared

```
discordd --auth local                                     its own accounts (the default)
discordd --auth 10.0.8.1:7600 --auth-key <fingerprint>    a shared authd: trusts only that key
musicd   --auth local | <host:port> --auth-key <fingerprint>
```

With `--auth local` the backend's realm key is its own key, and it answers the handshake on a
second port (Discord: PORT+2, Music: PORT+1). There is no `--auth none` (Discord requires
accounts) and no realm from discovery on the server side: a backend that picked its realm from
discovery could be made to trust a fake `authd`. The operator gives the key (`authd key`
prints it); a Tech Village sets it up by itself.

## `authd`

```
authd [PORT] [--name REALM] [--data DIR] [--key-file FILE]
authd key | users | disable USER | enable USER | reset USER | revoke USER   [--data DIR]
```

Port 7600, data in `/var/auth` (`users`, `realm.key` made on first start unless `--key-file`
names one). It announces `auth` on discovery, which apps use only to find an address: trust
always comes from the pinned key. Failed sign-ins are rate-limited per username (5) and per
address (10): a 30 s lockout, doubling up to 15 minutes. `reset` clears a user's key (the next
sign-in with that name sets a new password and keeps the account), `revoke` ends all of a
user's refresh tokens, `disable` also revokes them. The running `authd` notices admin changes
within 2 s.

## Scripting

The `auth` module, the same in Python and Lua: `auth.signin(host, user, password)`,
`auth.signup(host, user, password, display)`, `auth.token(audience[, host])`,
`auth.prove(nonce, audience[, host])`, `auth.fingerprint(host)`, `auth.logout([host])`,
`auth.pins()`, `auth.unpin(fp or host)`, `auth.display_fingerprint(fp)`. Host is `host:port`
(default port 7600). Same pinning rules as the apps; a changed key raises an error that names
both fingerprints.

## Tests

`cargo test -p ecm-auth` (KDF vectors, sign-up and sign-in over an in-memory pipe, wrong
passwords and rate limits, a spoofed server and a relay that rewrites `ce` both fail, a pinned
realm with a changed key is refused, tokens for the wrong audience, expired, with a wrong proof
or tampered are refused, the wire never carries the password or the account seed, refresh,
revoke, reset, PASSWD), `cargo test -p authd`, `cargo test -p python -p lua` (the `auth`
module), `-JUnit VillageAuthTest -McVersion 26.1` (the village realm's key matches
`ecm-auth`'s vector).

Scenarios (`/ecm scenario spawn <name>`; GameTests `-GameTests ecm_discord -GameTestOnly auth_pinned_key,auth_shared_realm`):

| Scenario | Shows | Controls |
|---|---|---|
| `auth_pinned_key` | `authd --name Lab` and a `discordd` that trusts it (`--auth-key`, the fingerprint typed from `authd key`); alex signs up in Discord, the server's console with `chat --user base --new`. The operator reinstalls the Auth server (`kill` it, delete `/var/auth/realm.key` and `/var/auth/users`, start it: a new key) and restarts `discordd` with the new fingerprint: Discord shows "The identity of Lab has changed" with both fingerprints and no sign-in button; `chat --user base` refuses and names `chat --forget <old fp>` | the backend sees no sign-in from alex; Discord's Settings > Forget realm makes it a first contact again and sign-up works; `chat --forget` (the pin removed by hand) lets `chat` sign up again |
| `auth_shared_realm` | One `authd` ("Village Accounts") for the Discord backends Lab and Village and for `musicd`: alex signs up once, on Lab, and Village signs in by itself (a refresh token) | Other (`--auth local`, another realm) still asks ("First time here"); Music keeps its own logins: it asks too (first contact for Music) and alex signs in there with the same account |
