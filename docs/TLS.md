# TLS in the game

`https://` works in `curl`, the Browser, `pkg`, the App Store, and `net.http_get` /
`net.http_post` in Python and Lua; `httpd --tls` and `pkg serve --tls` serve it. Players have
`man tls`, `man cert` and `man cad`. This page is for people working on the mod.

## What runs where

TLS is done by the program on the computer, not by Java: `rust/crates/ecm-tls` (rustls 0.23
with the pure-Rust crypto provider) on top of the computer's ordinary sockets. Java only
carries the bytes. `rust/crates/ecm-http` is the HTTP client the programs share; TLS is an
opt-in of the client (`Client::new`), so a program that only uses `Client::plain` links none
of it (that keeps `cad` and `cert` at 0.3 MiB).

## Whom a computer trusts

Decided by the peer's address:

| The server is | Checked against |
|---|---|
| on the real internet (a public address) | the public web authorities built into the program (`webpki-roots`) |
| inside the game (any other address) | the authorities in `/etc/ssl/certs`: `*.anchor` (`ed25519 <key> <name>`) and `*.pem` |

An in-game authority can therefore never vouch for a real website, and a real authority's
certificate is not accepted for an address in the game.

## Authorities in the game

Any computer can be one: `cad init <name>`, then `cad &` (port 8555, plain HTTP).

| Step | Who | What |
|---|---|---|
| `cert trust <authority>` | the client | fetches `/ca` and saves the anchor |
| `cert request <name>` | the server-to-be | `POST /order`; the authority answers with a token |
| | | the computer serves the token at `http://<name>/.well-known/ecm-challenge/<token>` (port 80) |
| | the authority | fetches it from the name, and refuses names and addresses on the real internet |
| | | `POST /finalize` returns the certificate: ed25519, 90 days, `/etc/ssl/server.crt` with the key in `/etc/ssl/private/server.key` |
| `cert renew` | | the same again, once the certificate is close to its end (`--force`: now) |

The Tech Village ring has one, Tech-Village-Authority, on its package server; generated
computers trust it from the start (docs/TECH_VILLAGE_NETWORK.md, section 8a).

## Failures a player sees

`certificate not trusted`, `certificate is for another name`, `certificate is out of date`,
each with what to do (`man tls`). `curl -k` skips the check and says so.

## Tests

* `cargo test -p ecm-tls -p ecm-http -p ecm-ca`; `public_site_over_https` (ignored: it needs
  the real internet) checks real sites and deliberately broken ones.
* Simulator: `50_tls_certificates.toml`, `51_pkg_kinds_mirror_https.toml`.
* In the game: `/ecm scenario spawn tls_web` (an authority, a web server with a certificate,
  a client; controls for an untrusted authority and a wrong name), and `repo_mirror` for
  `pkg serve --tls`.

## Cost

A program is compiled once per server and its parsed code stays in memory: about 30 to
40 MB of heap for each different TLS-carrying program that has been run. The GameTest
server's heap was raised to 3 GB for that reason (`build.gradle`).
