//! Sans-IO IPv4 host network stack: Ethernet (802.1Q) → ARP → IPv4 → ICMP/UDP/TCP → DNS.
//!
//! No globals, no I/O, no clock: the owner of a [`Stack`] feeds received frames to
//! [`Stack::handle_frame`], drains outgoing frames with [`Stack::pop_tx`], runs timers
//! with [`Stack::poll`] (which returns the next deadline), and passes the current
//! monotonic time in milliseconds to every call that needs it.
//!
//! Typical driver loop:
//!
//! ```text
//! for (iface, frame) in received { stack.handle_frame(iface, &frame, now); }
//! let deadline = stack.poll(now);           // timers + loopback
//! while let Some((iface, frame)) = stack.pop_tx() { nic_send(iface, &frame); }
//! // sleep until `deadline`, the next frame, or the next socket call
//! ```
//!
//! Socket calls never block: they return `Err(NetError::WouldBlock)` / `Ok(None)` when
//! not ready. After any socket call, drain `pop_tx` and call `poll` for the new deadline.

pub mod arp;
pub mod checksum;
pub mod dhcp;
pub mod dns;
pub mod eth;
pub mod http;
pub mod icmp;
pub mod ipv4;
mod stack;
mod tcb;
pub mod tcp;
pub mod types;
pub mod udp;

pub use stack::{
    DnsHandle, DnsStatus, IfStats, Interface, NeighborState, Route, RouteSource, SocketHandle,
    Stack, StackConfig, StackStats, ARP_MAX_REQUESTS, ARP_PENDING_PER_NEIGHBOR, ARP_REACHABLE_MS,
    ARP_RETRY_MS, DNS_TIMEOUT_MS, DNS_TRIES, ICMP_QUEUE_LEN, LOOPBACK_QUEUE_LEN, MAX_BACKLOG,
    MAX_DNS_QUERIES, MAX_INTERFACES, MAX_NEIGHBORS, MAX_ROUTES, MAX_SOCKETS, TX_QUEUE_LEN,
    UDP_QUEUE_LEN,
};
pub use tcb::{
    FIN_WAIT2_TIMEOUT_MS, MAX_RETRIES as TCP_MAX_RETRIES, MSL_MS, RX_CAP as TCP_RX_BUF,
    TIME_WAIT_MS, TX_CAP as TCP_TX_BUF,
};
pub use tcp::TcpState;
pub use types::{Ipv4Addr, MacAddr, NetError, SocketAddr, MAX_FRAME_SIZE, MTU};
