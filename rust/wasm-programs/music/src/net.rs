//! What the app needs from the computer, as traits: the network (TCP to
//! backends and realms, the discovery search, the kind of link) and the
//! speaker. `main.rs` has the real ones; `app/fake.rs` has a scripted world
//! (real musicd and a real realm behind in-memory connections).

use ecm_auth::{AuthError, Stream};

pub trait Net {
    /// Open a TCP connection; a handle. `Err` says why (shown to the player).
    fn connect(&mut self, host: &str, port: u16) -> Result<i32, String>;
    fn send(&mut self, h: i32, data: &[u8]) -> Result<(), String>;
    /// What arrived since the last call (never blocks); `Err` once it closed.
    fn recv(&mut self, h: i32) -> Result<Vec<u8>, String>;
    fn close(&mut self, h: i32);
    /// Wait up to `ms` for data on `h` (API calls wait for their answer).
    fn wait(&mut self, _h: i32, _ms: u64) {}
    /// A clock in milliseconds.
    fn now_ms(&mut self) -> u64;
    /// Music backends on the network (docs/DISCOVERY.md); blocks for the search.
    fn discover(&mut self) -> Vec<ecm_disco::Found> {
        Vec::new()
    }
    /// A connection to the accounts server (realm) at `host:port`, for ecm_auth.
    fn realm(&mut self, host: &str) -> Result<Box<dyn Stream>, AuthError>;
    /// The default route goes over the cellular link (celld's `wwan0`).
    fn cellular(&mut self) -> bool {
        false
    }
}

/// The speaker, mono at the rate given to `open`.
pub trait Speaker {
    fn open(&mut self, rate: u32) -> Result<(), String>;
    /// Queue samples (the engine never queues more than the latency).
    fn write(&mut self, pcm: &[i16]);
    /// Milliseconds queued and not yet heard.
    fn buffered_ms(&mut self) -> i64;
    /// Drop what is queued (pause, seek, a new song).
    fn flush(&mut self);
    fn close(&mut self);
}

/// Whether the default route of `routes_json` (net_config::route_list) goes
/// out of an interface named like celld's cellular link (`wwan*`), given the
/// interfaces as (name, ip, prefix).
pub fn default_route_is_cellular(routes_json: &str, ifaces: &[(String, [u8; 4], u8)]) -> bool {
    let Some(gw) = ecm_disco::default_gateway(routes_json) else { return false };
    let g = u32::from_be_bytes(gw);
    ifaces.iter().any(|(name, ip, prefix)| {
        let mask = if *prefix == 0 { 0 } else { u32::MAX << (32 - (*prefix).min(32) as u32) };
        *prefix > 0 && (u32::from_be_bytes(*ip) & mask) == (g & mask) && (name.starts_with("wwan") || name.starts_with("radio"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cellular_is_the_link_of_the_default_route() {
        let routes = r#"[{"dest":"100.65.2.0","prefix_len":24,"gateway":"0.0.0.0","iface_idx":2},{"dest":"0.0.0.0","prefix_len":0,"gateway":"100.65.2.1","iface_idx":2}]"#;
        let cell = vec![("eth0".to_string(), [10, 0, 0, 5], 24), ("wwan0".to_string(), [100, 65, 2, 2], 24)];
        assert!(default_route_is_cellular(routes, &cell));
        let wifi = r#"[{"dest":"0.0.0.0","prefix_len":0,"gateway":"10.0.0.1","iface_idx":1}]"#;
        assert!(!default_route_is_cellular(wifi, &cell), "Wi-Fi or a cable: Original");
        assert!(!default_route_is_cellular("[]", &cell));
    }
}
