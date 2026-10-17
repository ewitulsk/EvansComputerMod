//! `ecm_dns`: the Domain Name System of the Tech Village ring (docs/DNS.md).
//!
//! Real RFC 1035 DNS, built the way `ecm-bgp`/`router` and `ecm-disco`/`disco` are: the
//! protocol and every policy are plain Rust with time passed in, tested on the host, and
//! the programs (`named`, `dig`, `nslookup`, `whois`, `domain`, `registryd`,
//! `registrard`, `ianad`) only move bytes between sockets and these state machines.
//!
//! * [`wire`]: messages, records, name compression.
//! * [`zone`]: master-format zone files.
//! * [`auth`]: authoritative answers, referrals, negative answers.
//! * [`cache`], [`resolver`]: the iterative resolver.
//! * [`xfr`]: TCP framing, AXFR, NOTIFY.
//! * [`conf`], [`server`]: `named`.
//! * [`registry`], [`registrar`], [`iana`]: who may put what in which zone.
//! * [`enum_dns`]: ENUM (RFC 6116) names for phone numbers and the NAPTR rewrite to SIP.

pub mod auth;
pub mod cache;
pub mod conf;
pub mod enum_dns;
pub mod iana;
pub mod name;
pub mod registrar;
pub mod registrar_web;
pub mod registry;
pub mod resolver;
pub mod server;
pub mod wire;
pub mod xfr;
pub mod zone;

#[cfg(target_arch = "wasm32")]
pub mod sys;
