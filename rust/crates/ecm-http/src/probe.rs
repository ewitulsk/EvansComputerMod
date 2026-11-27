//! Is this computer online? A staged answer, the way a person would check:
//! is a cable (or radio) up, is there an address, a way out, does the name
//! resolve, does the server answer. The first step that fails names the fix.

use crate::{Client, Connector, HttpError, Request, Transfer};

/// What the computer's own network settings say (gathered by `sys::net_state`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetState {
    /// (name, link is up, address or empty).
    pub interfaces: Vec<(String, bool, String)>,
    /// Has a default route (0.0.0.0/0)?
    pub default_route: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Online {
    /// The server answered (HTTP status).
    Yes(u16),
    /// No interface has a link: nothing is plugged in or joined.
    NoLink,
    /// A link, but no address on it.
    NoAddress,
    /// An address, but no default route and the server is not on a connected network.
    NoRoute,
    /// The name does not resolve: (host, why).
    NoDns(String, String),
    /// Resolved (or an address was given), but nobody answered there.
    Unreachable(String),
    /// Answered, but the certificate was refused.
    Certificate(String),
    /// The app has no network permission.
    NotAllowed,
    /// Something else went wrong talking to it.
    Failed(String),
}

impl Online {
    pub fn is_online(&self) -> bool {
        matches!(self, Online::Yes(_))
    }

    /// A short word for scripts: `online`, `no-link`, `no-address`, `no-route`, `no-dns`, `unreachable`, `certificate`, `not-allowed`, `failed`.
    pub fn code(&self) -> &'static str {
        match self {
            Online::Yes(_) => "online",
            Online::NoLink => "no-link",
            Online::NoAddress => "no-address",
            Online::NoRoute => "no-route",
            Online::NoDns(..) => "no-dns",
            Online::Unreachable(_) => "unreachable",
            Online::Certificate(_) => "certificate",
            Online::NotAllowed => "not-allowed",
            Online::Failed(_) => "failed",
        }
    }

    /// One sentence saying what is wrong and what to try.
    pub fn explain(&self) -> String {
        match self {
            Online::Yes(_) => "online".into(),
            Online::NoLink => "not connected: plug in a cable, join Wi-Fi or get a cellular signal".into(),
            Online::NoAddress => "connected, but this computer has no address: run dhclient, or set one with ifconfig".into(),
            Online::NoRoute => "no route out of this network: add a default route (ip route add default via <router>)".into(),
            Online::NoDns(h, why) => format!("cannot find {h}: {why} (check /etc/resolv.conf)"),
            Online::Unreachable(a) => format!("{a} did not answer: there may be no way to it from here (try ping and traceroute)"),
            Online::Certificate(m) => m.clone(),
            Online::NotAllowed => "this app is not allowed to use the network (Settings > Apps)".into(),
            Online::Failed(m) => m.clone(),
        }
    }
}

/// Turn a failed request into the step that failed, using the network settings
/// to tell "no cable" from "no address" from "no route".
pub fn diagnose(state: &NetState, error: &HttpError) -> Online {
    let up: Vec<&(String, bool, String)> = state.interfaces.iter().filter(|(n, up, _)| *up && n != "lo").collect();
    let addressed = up.iter().any(|(_, _, ip)| !ip.is_empty() && ip != "0.0.0.0");
    match error {
        HttpError::NotAllowed => Online::NotAllowed,
        HttpError::Tls(e) => Online::Certificate(e.to_string()),
        HttpError::NoNetwork | HttpError::Resolve(..) | HttpError::Connect(_) => {
            if up.is_empty() {
                Online::NoLink
            } else if !addressed {
                Online::NoAddress
            } else {
                match error {
                    HttpError::Resolve(h, why) => Online::NoDns(h.clone(), why.clone()),
                    HttpError::Connect(a) if !state.default_route => {
                        let _ = a;
                        Online::NoRoute
                    }
                    HttpError::Connect(a) => Online::Unreachable(a.clone()),
                    _ => Online::NoRoute,
                }
            }
        }
        other => Online::Failed(other.to_string()),
    }
}

/// Ask `url` (a small page such as a health check) and say how it went.
pub fn check<C: Connector>(client: &mut Client<C>, state: &NetState, url: &str) -> Online {
    let req = match Request::get(url) {
        Ok(r) => r,
        Err(e) => return Online::Failed(e.to_string()),
    };
    let mut t = Transfer { max_body: 64 * 1024, ..Transfer::default() };
    match client.exchange(&req, &mut t) {
        Ok(r) => Online::Yes(r.status),
        Err(e) => diagnose(state, &e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(ifaces: &[(&str, bool, &str)], default_route: bool) -> NetState {
        NetState { interfaces: ifaces.iter().map(|(n, u, i)| (n.to_string(), *u, i.to_string())).collect(), default_route }
    }

    #[test]
    fn the_first_failing_step_is_named() {
        let connect = HttpError::Connect("203.0.113.9:443".into());
        let resolve = HttpError::Resolve("repo.example".into(), "no DNS server configured".into());
        assert_eq!(diagnose(&state(&[("eth0", false, "")], false), &connect), Online::NoLink);
        assert_eq!(diagnose(&state(&[], false), &HttpError::NoNetwork), Online::NoLink);
        assert_eq!(diagnose(&state(&[("eth0", true, "0.0.0.0")], false), &resolve), Online::NoAddress);
        assert_eq!(diagnose(&state(&[("eth0", true, "10.0.0.2")], false), &connect), Online::NoRoute);
        assert_eq!(diagnose(&state(&[("eth0", true, "10.0.0.2")], true), &connect), Online::Unreachable("203.0.113.9:443".into()));
        assert_eq!(diagnose(&state(&[("eth0", true, "10.0.0.2")], true), &resolve).code(), "no-dns");
        assert_eq!(diagnose(&state(&[("eth0", true, "10.0.0.2")], true), &HttpError::NotAllowed), Online::NotAllowed);
        let tls = HttpError::Tls(ecm_tls::TlsError::Expired("x".into()));
        assert_eq!(diagnose(&state(&[], false), &tls).code(), "certificate");
        assert_eq!(diagnose(&state(&[], false), &HttpError::Protocol("x".into())).code(), "failed");
        assert!(Online::Yes(200).is_online());
        assert!(Online::NoRoute.explain().contains("default route"));
    }
}
