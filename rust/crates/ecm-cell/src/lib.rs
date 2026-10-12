//! Cellular v2 protocol (docs/gui/CELLULAR_V2.md, "As built: ecm-cell protocol").
//!
//! Three sans-IO state machines and everything they speak:
//!
//! | Module | What |
//! |---|---|
//! | [`ue`] | the phone (`celld`): cell search, attach, SIM challenge, handover, idle |
//! | [`site`] | the cell site (`celld-site`): beacons, scheduler, relay, handover |
//! | [`core`] | the carrier core (`cored`): subscribers, keys, addresses, tunnels |
//! | [`air`] | air messages (plain and protected items) |
//! | [`backhaul`] | site <-> core control (TCP) and tunnels (UDP) |
//! | [`simauth`], [`crypto`] | SimAuth v1, key derivation, ChaCha20-Poly1305 air protection |
//! | [`phy`] | the framed PHY record API the machines drive, [`phy::FramedPhy`] |
//! | [`db`] | the core's subscriber database file |
//! | [`sched`] | the site's batched uplink scheduler |
//! | [`seg`], [`ip`] | segmentation, dedup, tun/ARP helpers |
//! | [`medium`] | (std) a simulated framed radio medium for tests and the simulator |
//! | [`admin`] | the daemons' admin socket framing and config-file lines |
//!
//! Every machine takes inputs (`on_*`, `tick`) with the time as [`Now`] and
//! queues outputs that the driver drains with `take_out()`.

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod admin;
pub mod air;
pub mod backhaul;
pub mod codec;
pub mod core;
pub mod crypto;
pub mod db;
pub mod ip;
#[cfg(feature = "std")]
pub mod medium;
pub mod phy;
pub mod sched;
pub mod seg;
pub mod simauth;
pub mod site;
pub mod ue;

/// The time, as a program sees it: milliseconds (any monotonic origin) and
/// the radio's current slot on the airtime clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Now {
    pub ms: u64,
    pub slot: u64,
}
