//! `imsd`'s configuration and status text (the machine is `ecm_tel::ims`).
//!
//! `/etc/imsd.toml` (every key optional; on the core's computer it needs nothing):
//!
//! ```toml
//! hss = 127.0.0.1:3868        # cored's HSS port
//! hss_secret = ""             # cored's hss_secret (another computer)
//! ims_ip = ""                 # this computer's address towards the phones (default: the HSS's P-CSCF)
//! public_ip = ""              # towards other carriers (default: ims_ip)
//! smsc = ""                   # smscd (default ims_ip:5062); mrf = mrfd (ims_ip:5064); mmsc = mmscd (ims_ip:5063)
//! relay_ports = 30000-31000   # the media relay's UDP ports
//! no_answer = 30              # seconds before voicemail / no-answer forwarding
//! max_expires = 3600          # longest registration
//! peers = ""                  # other carriers: domain@ip[:port]/secret, ... (domain@srv/secret: the border from DNS)
//! public_peering = false      # also take calls from carriers that are not listed (no secret)
//! admin_port = 7721
//! ```

use ecm_tel::ims::{ImsConfig, ImsStatus, Peer};
use ecm_tel::txn::Addr;

pub const PATH: &str = "/etc/imsd.toml";
pub const ADMIN_PORT: u16 = 7721;

pub const KEYS: &[(&str, &str)] = &[
    ("hss", "127.0.0.1:3868"),
    ("hss_secret", ""),
    ("ims_ip", ""),
    ("public_ip", ""),
    ("smsc", ""),
    ("mrf", ""),
    ("mmsc", ""),
    ("relay_ports", "30000-31000"),
    ("no_answer", "30"),
    ("max_expires", "3600"),
    ("peers", ""),
    ("public_peering", "false"),
    ("admin_port", "7721"),
];

pub const HELP: &[(&str, &str)] = &[
    ("hss", "cored's HSS port (ip:port)"),
    ("hss_secret", "cored's hss_secret, when cored runs on another computer"),
    ("ims_ip", "this computer's address towards the phones (default: the P-CSCF the HSS names)"),
    ("public_ip", "this computer's address towards other carriers (default: ims_ip)"),
    ("smsc", "smscd's address (default ims_ip:5062)"),
    ("mrf", "mrfd's address (default ims_ip:5064)"),
    ("mmsc", "mmscd's SIP address (default ims_ip:5063)"),
    ("relay_ports", "the media relay's UDP ports, from-to"),
    ("no_answer", "seconds a phone rings before voicemail"),
    ("max_expires", "the longest registration, seconds"),
    ("peers", "other carriers: domain@ip[:port]/secret, ... (domain@srv/secret finds the border in DNS)"),
    ("public_peering", "true: also carriers that publish ENUM and SRV, without a secret"),
    ("admin_port", "the admin socket (127.0.0.1)"),
];

/// `domain@ip[:port]/secret, ...`
pub fn parse_peers(s: &str) -> Result<Vec<Peer>, String> {
    let mut v = Vec::new();
    for item in s.split(',').map(str::trim).filter(|x| !x.is_empty()) {
        let (domain, rest) = item.split_once('@').ok_or_else(|| format!("peer '{item}': domain@address/secret"))?;
        let (addr, secret) = rest.split_once('/').ok_or_else(|| format!("peer '{item}': no /secret"))?;
        if domain.is_empty() || secret.is_empty() {
            return Err(format!("peer '{item}': empty domain or secret"));
        }
        let addr = if addr == "srv" { None } else { Some(Addr::parse(addr, 5060).ok_or_else(|| format!("peer '{item}': bad address '{addr}'"))?) };
        v.push(Peer { domain: domain.to_ascii_lowercase(), addr, secret: secret.to_string() });
    }
    Ok(v)
}

pub fn format_peers(p: &[Peer]) -> String {
    p.iter().map(|x| format!("{}@{}/{}", x.domain, x.addr.map(|a| a.to_string()).unwrap_or_else(|| "srv".into()), x.secret)).collect::<Vec<_>>().join(", ")
}

/// What the HSS's `NUM` says (domain, P-CSCF, pool, carrier, cc).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Numbering {
    pub cc: String,
    pub domain: String,
    pub carrier: String,
    pub pcscf: Option<[u8; 4]>,
    pub pool: Option<([u8; 4], u8)>,
    pub mmsc: Option<[u8; 4]>,
}

pub fn parse_num(body: &str) -> Numbering {
    let kv = |k: &str| ecm_tel::ims::kv(body, k);
    let pool = kv("ims_pool");
    Numbering {
        cc: kv("cc"),
        domain: kv("domain"),
        carrier: kv("carrier"),
        pcscf: Addr::parse_ip(&kv("pcscf")),
        pool: pool.split_once('/').and_then(|(a, l)| Some((Addr::parse_ip(a)?, l.parse().ok()?))),
        mmsc: Addr::parse_ip(&kv("mmsc")),
    }
}

/// The machine's configuration from the file and the HSS's answer.
pub fn config(c: &std::collections::BTreeMap<String, String>, n: &Numbering) -> Result<ImsConfig, String> {
    let g = |k: &str| c.get(k).cloned().unwrap_or_default();
    let ims_ip = match g("ims_ip").as_str() {
        "" => n.pcscf.ok_or("the HSS has no IMS set up (cored ims pool=...) and imsd.toml has no ims_ip")?,
        s => Addr::parse_ip(s).ok_or_else(|| format!("bad ims_ip '{s}'"))?,
    };
    let domain = if n.domain.is_empty() { "ims.ecm".to_string() } else { n.domain.clone() };
    let mut cfg = ImsConfig::new(&domain, &n.carrier, ims_ip);
    if !n.cc.is_empty() {
        cfg.cc = n.cc.clone();
    }
    if let Some(p) = n.pool {
        cfg.ims_pool = p;
    }
    if !g("public_ip").is_empty() {
        cfg.public_ip = Addr::parse_ip(&g("public_ip")).ok_or("bad public_ip")?;
    }
    let addr = |k: &str, port: u16| -> Result<Option<Addr>, String> {
        match g(k).as_str() {
            "" => Ok(None),
            s => Addr::parse(s, port).map(Some).ok_or_else(|| format!("bad {k} '{s}'")),
        }
    };
    if let Some(a) = addr("smsc", 5062)? {
        cfg.smsc = a;
    }
    if let Some(a) = addr("mrf", 5064)? {
        cfg.mrf = a;
    }
    if let Some(a) = addr("mmsc", 5063)? {
        cfg.mmsc = Some(a);
    }
    let (lo, hi) = g("relay_ports").split_once('-').and_then(|(a, b)| Some((a.trim().parse::<u16>().ok()?, b.trim().parse::<u16>().ok()?))).ok_or("relay_ports is from-to")?;
    if lo >= hi || hi - lo < 4 {
        return Err("relay_ports: at least 4 ports".into());
    }
    cfg.relay_ports = (lo, hi);
    cfg.no_answer_ms = g("no_answer").parse::<u64>().map_err(|_| "no_answer is seconds")?.clamp(5, 120) * 1000;
    cfg.max_expires = g("max_expires").parse::<u32>().map_err(|_| "max_expires is seconds")?.clamp(60, 86400);
    // carrier settings for the phones: where the MMSC and visual voicemail answer
    let mmsc_ip = cfg.mmsc.map(|a| a.ip).or(n.mmsc).unwrap_or(cfg.ims_ip);
    cfg.mmsc_url = format!("http://{}:8002/mms", Addr::new(mmsc_ip, 0).host());
    cfg.vvm_url = format!("http://{}:8003/vm", Addr::new(cfg.mrf.ip, 0).host());
    cfg.peers = parse_peers(&g("peers"))?;
    cfg.public_peering = g("public_peering") == "true";
    Ok(cfg)
}

/// `imsd status`'s body.
pub fn status_text(cfg: &ImsConfig, st: &ImsStatus, hss_ok: bool, hss_err: &str, now_ms: u64) -> String {
    let mut s = String::new();
    s.push_str(&format!("domain {}\ncarrier {}\nims_ip {}\npublic_ip {}\nhss {}\n", cfg.domain, cfg.carrier, Addr::new(cfg.ims_ip, 0).host(), Addr::new(cfg.public_ip, 0).host(), if hss_ok { "up".into() } else { format!("down {hss_err}") }));
    s.push_str(&format!("registrations {}\ncalls {}\nrelays {}\nretransmits {}\npeers {}\n", st.registrations.len(), st.calls.len(), st.relays, st.retransmits, cfg.peers.len()));
    for r in &st.registrations {
        s.push_str(&format!("reg {} {} {} {} {}\n", r.msisdn, r.imsi, r.contact, r.expires_ms.saturating_sub(now_ms) / 1000, r.calls));
    }
    for (from, to, state, start) in &st.calls {
        s.push_str(&format!("call {} {} {} {}\n", from, to, state, now_ms.saturating_sub(*start) / 1000));
    }
    for p in &cfg.peers {
        s.push_str(&format!("peer {} {}\n", p.domain, p.addr.map(|a| a.to_string()).unwrap_or_else(|| "srv".into())));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peers_and_config() {
        let p = parse_peers("lakeline.ecm@100.66.0.9:5060/s3cret, far.ecm@srv/x").unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].addr, Some(Addr::new([100, 66, 0, 9], 5060)));
        assert_eq!(p[1].addr, None);
        assert_eq!(parse_peers(&format_peers(&p)).unwrap(), p);
        assert!(parse_peers("nosecret@1.2.3.4").is_err());
        let n = parse_num("cc 1\ndomain cell-red.ecm\npcscf 100.65.2.1\nims_pool 10.250.0.0/16\ncarrier cell-red\n");
        let c: std::collections::BTreeMap<String, String> = KEYS.iter().map(|(k, d)| (k.to_string(), d.to_string())).collect();
        let cfg = config(&c, &n).unwrap();
        assert_eq!((cfg.ims_ip, cfg.ims_pool, cfg.domain.as_str()), ([100, 65, 2, 1], ([10, 250, 0, 0], 16), "cell-red.ecm"));
        assert_eq!(cfg.smsc, Addr::new([100, 65, 2, 1], 5062));
        assert!(config(&c, &Numbering::default()).is_err(), "no IMS at the HSS and no ims_ip");
    }
}
