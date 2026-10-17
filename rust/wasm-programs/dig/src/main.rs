//! dig: ask DNS servers and show what they say (docs/DNS.md).
//!
//! `dig [@SERVER] NAME [TYPE] [+trace] [+norecurse] [+short] [+tcp] [-p PORT]`
//! `dig @SERVER ZONE AXFR` lists a whole zone (zone transfer over TCP).
//! Without `@SERVER` it asks the first `nameserver` of `/etc/resolv.conf`.
//! `+trace` walks the tree itself from the root, as a resolver does, and prints
//! every referral: root -> TLD -> the zone that has the answer.
//! `dig -t NAPTR <ENUM name>` and `dig -t SRV _sip._udp.<domain>` show the records
//! that route a phone number to SIP (RFC 6116, RFC 3263).
//!
//! [`format`] holds what is printed (host-tested); `app` the sockets (wasm32 only).

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
mod format;

#[cfg(target_arch = "wasm32")]
mod app;

fn main() {
    #[cfg(target_arch = "wasm32")]
    app::main();
    #[cfg(not(target_arch = "wasm32"))]
    eprintln!("dig: runs on an ECM computer (build for wasm32)");
}
