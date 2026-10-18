//! `cored` — the carrier core (Cellular v2, docs/gui/CELLULAR_V2.md).
//!
//!   cored init <carrier> <pool> [--gateway IP] [--dns IP]   a new subscriber database
//!   cored start                       run the core (background it: cored start &)
//!   cored stop                        stop the running core
//!   cored status | sites | subs | log [n] | handovers
//!   cored site add <id>               a site with a new credential (prints it for the site's config)
//!   cored site remove <id>
//!   cored sim new [--label L] [--quota BYTES]   write the blank SIM in the Card Reader/Writer
//!   cored add <imsi> <key> [quota] | revoke | unrevoke | quota <imsi> <bytes> | remove | label <imsi> <l> | release
//!   cored block add|remove <NPA-NXX> | blocks | numbers | number <who> [+number [ported]]   numbering
//!   cored enum-key <hex> | port-code <who>    the ENUM key from IANA; a port-out code for a subscriber
//!   cored ims pool=<cidr> [pcscf=..] [domain=..] [mmsc=..] | ims off        the IMS bearer (VoLTE, SMS, MMS)
//!   cored feature <who> <key> <value> | port-code <who> <code>             the TAS profile (HSS)
//!   cored config | config set <key> <value> | config keys
//!   cored enable | disable            start at boot (services.cfg)
//!
//! With the core running, commands go to it over its admin socket (127.0.0.1:7711, as the
//! Carrier Console does); without it they edit the database directly.

#[cfg(target_os = "wasi")]
mod prog;

#[cfg(target_os = "wasi")]
fn main() {
    prog::main();
}

#[cfg(not(target_os = "wasi"))]
fn main() {
    eprintln!("cored runs on a computer (wasm32-wasip1)");
    std::process::exit(1);
}
