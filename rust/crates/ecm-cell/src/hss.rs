//! The core's HSS and PCRF interface for the IMS programs (docs/TELEPHONY.md):
//! a small Diameter-like query protocol on TCP [`HSS_PORT`] (3868, Diameter's
//! port). `imsd` (the CSCFs and the TAS), `smscd` and `mmscd` ask `cored` about
//! subscribers, numbers, registrations and bearers here, as a real IMS core asks
//! the HSS over Cx/Sh and the PCRF over Rx.
//!
//! The framing is the admin socket's ([`crate::admin`]): one request line, a
//! reply `ok` / `err <why>`, `key value` body lines, `.` alone. A peer starts
//! with `CER` (capabilities exchange) carrying its proof of the shared secret
//! ([`peer_proof`]); until then every other request is refused. Requests use the
//! Diameter command names:
//!
//! | Request | Interface | Reply body |
//! |---|---|---|
//! | `CER <origin> <proof-hex>` | base | `origin <carrier>` |
//! | `MAR <who>` | Cx multimedia-auth | `imsi`, `msisdn`, `rand`, `xres`, `ck` (IMS AKA vector) |
//! | `SAR <who> register\|deregister <server>` | Cx server-assignment | the profile (as `UDR`) |
//! | `LIR <number>` | Cx location-info | `result local\|unassigned\|foreign`, `imsi`, `ims_ip`, `idle`, `registered` |
//! | `UDR <who>` | Sh user-data | `imsi msisdn label used quota cfu cfb cfnr cfnrc cw clir dnd vm block port` |
//! | `PUR <who> <feature> <value>` | Sh profile-update | `ok` (features as `cored feature`) |
//! | `IPR <ims-ip>` | (P-CSCF) who is at an IMS address | `imsi`, `msisdn` |
//! | `IPD <data-ip>` | (P-GW header enrichment, for the MMSC) who is at a default-bearer address, or at an IMS address (the phone's route to the P-CSCF also carries the MMSC when they share the core's address) | `imsi`, `msisdn` |
//! | `AAR <ue-ip> <ue-port> <remote-ip> <remote-port>` | Rx auth-request: a voice bearer | `bearer <id>`, `sent 0\|1` |
//! | `STR <ue-ip> <bearer>` | Rx session-termination | |
//! | `NUM` | numbering and the IMS setup | `cc`, `domain`, `pcscf`, `ims_pool`, `mmsc`, `carrier`, `block <NPA-NXX>` lines |
//! | `REGS` | registrations held | `reg <imsi> <msisdn> <server>` lines |
//!
//! `<who>` is an IMSI or an E.164 number. Values that are empty are written `-`.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::codec::{fmt_ip, hex, parse_ip};
use crate::core::Core;
use crate::crypto::hmac_sha256_parts;
use crate::simauth::ims_aka;

pub const HSS_PORT: u16 = 3868;

/// `CER`'s proof: `HMAC-SHA256(secret, "ECM-DIAM" || origin)`, hex.
pub fn peer_proof(secret: &[u8], origin: &str) -> String {
    hex(&hmac_sha256_parts(secret, &[b"ECM-DIAM", origin.as_bytes()]))
}

fn dash(s: &str) -> &str {
    if s.is_empty() {
        "-"
    } else {
        s
    }
}

impl Core {
    /// One HSS/PCRF request (after `CER`; the program checks the proof).
    pub fn hss(&mut self, line: &str, now: u64) -> Result<String, String> {
        let w: Vec<&str> = line.split_whitespace().collect();
        let imsi_of = |c: &Core, who: &str| c.imsi_of(who).ok_or_else(|| format!("unknown user {}", who));
        match w.as_slice() {
            ["MAR", who] => {
                let imsi = imsi_of(self, who)?;
                let sub = self.db().subs[&imsi].clone();
                if sub.revoked {
                    return Err("SIM revoked".into());
                }
                if sub.ims.msisdn.is_empty() {
                    return Err(format!("{} has no number (no IMS service)", imsi));
                }
                let rand = self.random16();
                let v = ims_aka(&sub.key, &rand);
                Ok(format!("imsi {}\nmsisdn {}\nrand {}\nxres {}\nck {}\n", imsi, sub.ims.msisdn, hex(&rand), hex(&v.res), hex(&v.ks)))
            }
            ["SAR", who, what, server] => {
                let imsi = imsi_of(self, who)?;
                match *what {
                    "register" => {
                        self.ims_regs.insert(imsi.clone(), server.to_string());
                        self.log_line(format!("ims: {} registered at {}", imsi, server));
                    }
                    "deregister" => {
                        self.ims_regs.remove(&imsi);
                        self.log_line(format!("ims: {} deregistered", imsi));
                    }
                    _ => return Err("SAR register|deregister".into()),
                }
                self.hss(&format!("UDR {}", imsi), now)
            }
            ["LIR", number] => {
                if let Some(imsi) = self.db().by_number(number).map(String::from) {
                    let r = self.reach(&imsi).expect("known");
                    let reg = self.ims_regs.get(&imsi).cloned().unwrap_or_default();
                    return Ok(format!(
                        "result local\nimsi {}\nims_ip {}\nidle {}\nregistered {}\n",
                        imsi,
                        r.ims_ip.map(fmt_ip).unwrap_or_else(|| "-".into()),
                        r.idle as u8,
                        dash(&reg)
                    ));
                }
                if self.db().block_of(number).is_some() {
                    Ok("result unassigned\n".into())
                } else {
                    Ok("result foreign\n".into())
                }
            }
            ["UDR", who] => {
                let imsi = imsi_of(self, who)?;
                let s = &self.db().subs[&imsi];
                let p = &s.ims;
                Ok(format!(
                    "imsi {}\nmsisdn {}\nlabel {}\nused {}\nquota {}\ncfu {}\ncfb {}\ncfnr {}\ncfnrc {}\ncw {}\nclir {}\ndnd {}\nvm {}\nblock {}\nport {}\n",
                    imsi,
                    dash(&p.msisdn),
                    dash(&s.label),
                    s.used,
                    s.quota,
                    dash(&p.cfu),
                    dash(&p.cfb),
                    dash(&p.cfnr),
                    dash(&p.cfnrc),
                    p.cw as u8,
                    p.clir as u8,
                    p.dnd as u8,
                    p.vm as u8,
                    dash(&p.blocked.join(",")),
                    if p.port.is_empty() { "-" } else { "set" }
                ))
            }
            ["PUR", who, feature, value] => {
                let imsi = imsi_of(self, who)?;
                self.admin(&format!("feature {} {} {}", imsi, feature, value), now).map(|_| String::new())
            }
            ["IPR", ip] => {
                let ip = parse_ip(ip).ok_or("bad address")?;
                let imsi = self.imsi_at(ip).ok_or("no IMS session at that address")?.to_string();
                let msisdn = self.db().subs[&imsi].ims.msisdn.clone();
                Ok(format!("imsi {}\nmsisdn {}\n", imsi, dash(&msisdn)))
            }
            ["IPD", ip] => {
                let ip = parse_ip(ip).ok_or("bad address")?;
                // Both are addresses the core gave out, so a phone still can't send as another.
                let imsi = self.imsi_at_data(ip).or_else(|| self.imsi_at(ip)).ok_or("no data session at that address")?.to_string();
                let msisdn = self.db().subs[&imsi].ims.msisdn.clone();
                Ok(format!("imsi {}\nmsisdn {}\n", imsi, dash(&msisdn)))
            }
            ["AAR", ue, ue_port, remote, rport] => {
                let ue = parse_ip(ue).ok_or("bad ue address")?;
                let remote = parse_ip(remote).ok_or("bad remote address")?;
                let up: u16 = ue_port.parse().map_err(|_| "bad port")?;
                let rp: u16 = rport.parse().map_err(|_| "bad port")?;
                let (id, sent) = self.bearer_setup(ue, up, remote, rp)?;
                Ok(format!("bearer {}\nsent {}\n", id, sent as u8))
            }
            ["STR", ue, id] => {
                let ue = parse_ip(ue).ok_or("bad ue address")?;
                let id: u8 = id.parse().map_err(|_| "bad bearer id")?;
                self.bearer_release(ue, id).map(|_| String::new())
            }
            ["NUM"] => {
                let db = self.db();
                let mut s = format!("cc {}\ndomain {}\n", db.cc, db.ims.as_ref().map(|i| i.domain.clone()).unwrap_or_else(|| format!("{}.ecm", db.carrier)));
                if let Some(i) = &db.ims {
                    s.push_str(&format!("pcscf {}\nims_pool {}/{}\nmmsc {}\n", fmt_ip(i.pcscf), fmt_ip(i.pool), i.prefix, i.mmsc.map(fmt_ip).unwrap_or_else(|| "-".into())));
                }
                s.push_str(&format!("carrier {}\n", db.carrier));
                for b in &db.blocks {
                    s.push_str(&format!("block {}\n", b));
                }
                Ok(s)
            }
            ["REGS"] => {
                let mut s = String::new();
                for (imsi, server) in &self.ims_regs {
                    let m = self.db().subs.get(imsi).map(|x| x.ims.msisdn.clone()).unwrap_or_default();
                    s.push_str(&format!("reg {} {} {}\n", imsi, dash(&m), server));
                }
                Ok(s)
            }
            _ => Err(format!("unknown request '{}'", line.trim())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{CoreConfig, Entropy};
    use crate::db::Db;

    struct Fixed;
    impl Entropy for Fixed {
        fn fill(&mut self, b: &mut [u8]) {
            for (i, x) in b.iter_mut().enumerate() {
                *x = i as u8;
            }
        }
    }

    fn core() -> Core {
        let db = Db::parse(
            "version 1\ncarrier cell-red\npool 100.65.2.0/24\ngateway 100.65.2.1\nblock 214-555\nims pool=10.250.0.0/24\n\
             sub 310150000000001 key=00112233445566778899aabbccddeeff msisdn=+12145550142 cfnr=+12145550100\n",
        )
        .unwrap();
        Core::new(db, CoreConfig::default(), alloc::boxed::Box::new(Fixed))
    }

    fn kv<'a>(body: &'a str, k: &str) -> &'a str {
        body.lines().find_map(|l| l.strip_prefix(k).and_then(|r| r.strip_prefix(' '))).unwrap_or("")
    }

    #[test]
    fn ims_aka_vector_matches_the_sim() {
        let mut c = core();
        let b = c.hss("MAR +12145550142", 0).unwrap();
        assert_eq!(kv(&b, "imsi"), "310150000000001");
        // RAND is 00 01 .. 0f with this entropy: the vector in TELEPHONY.md
        assert_eq!(kv(&b, "rand"), "000102030405060708090a0b0c0d0e0f");
        assert_eq!(kv(&b, "xres"), "2f725f20b08127b69131df0afbde3fde");
        assert_eq!(kv(&b, "ck"), "083d54d4fd53b69f710bde18dfe077d66ea6a30c56980343e97b927f3e265081");
        assert!(c.hss("MAR 310150000000009", 0).is_err());
    }

    #[test]
    fn location_profile_and_updates() {
        let mut c = core();
        assert_eq!(kv(&c.hss("LIR +12145550142", 0).unwrap(), "result"), "local");
        assert_eq!(kv(&c.hss("LIR +12145550142", 0).unwrap(), "registered"), "-");
        assert_eq!(kv(&c.hss("LIR +12145550199", 0).unwrap(), "result"), "unassigned");
        assert_eq!(kv(&c.hss("LIR +13125550199", 0).unwrap(), "result"), "foreign");
        let p = c.hss("SAR +12145550142 register scscf.cell-red.ecm", 0).unwrap();
        assert_eq!(kv(&p, "cfnr"), "+12145550100");
        assert_eq!(kv(&p, "dnd"), "0");
        assert_eq!(kv(&c.hss("LIR +12145550142", 0).unwrap(), "registered"), "scscf.cell-red.ecm");
        assert!(c.hss("REGS", 0).unwrap().contains("reg 310150000000001 +12145550142 scscf.cell-red.ecm"));
        c.hss("PUR +12145550142 dnd on", 0).unwrap();
        c.hss("PUR +12145550142 block +12145550199", 0).unwrap();
        let u = c.hss("UDR 310150000000001", 0).unwrap();
        assert_eq!((kv(&u, "dnd"), kv(&u, "block")), ("1", "+12145550199"));
        assert!(c.hss("PUR +12145550142 colour red", 0).is_err());
        c.hss("SAR +12145550142 deregister scscf.cell-red.ecm", 0).unwrap();
        assert_eq!(c.hss("REGS", 0).unwrap(), "");
        let n = c.hss("NUM", 0).unwrap();
        assert!(n.contains("cc 1\n") && n.contains("domain cell-red.ecm\n") && n.contains("block 214-555\n"), "{}", n);
        assert!(c.hss("AAR 10.250.0.9 40000 1.2.3.4 30000", 0).is_err(), "no session there");
        assert!(c.hss("IPR 10.250.0.9", 0).is_err());
        assert!(c.hss("FROB", 0).is_err());
        assert_ne!(peer_proof(b"s", "imsd"), peer_proof(b"s", "smscd"));
    }

    #[test]
    fn enum_key_port_codes_and_ported_numbers() {
        let mut c = core();
        assert!(c.admin("port-code +12145550142", 0).unwrap_err().contains("no enum key"));
        assert!(c.admin("enum-key 0011", 0).is_err());
        c.admin("enum-key 000102030405060708090a0b0c0d0e0f", 0).unwrap();
        // the nonce is 00 01 02 with this entropy; ianad checks the mac with the same key
        let r = c.admin("port-code +12145550142", 0).unwrap();
        assert!(r.ends_with("port-out code for +12145550142 is 000102-16ffd23c"), "{}", r);
        assert_eq!(c.db().subs["310150000000001"].ims.port, "000102-16ffd23c");
        assert!(c.admin("port-code 310150000000001 7Q2K9X", 0).is_ok(), "a code typed in still works");
        // porting in: a number outside the blocks needs `ported`
        c.admin("add 310150000000002 00112233445566778899aabbccddeeff", 0).unwrap();
        assert!(c.admin("number 310150000000002 +19725550123", 0).unwrap_err().contains("outside"));
        assert!(c.admin("number 310150000000002 +19725550123 ported", 0).unwrap().contains("ported in"));
        assert!(c.db().subs["310150000000002"].ims.ported);
        assert!(c.admin("port-code +19725550123", 0).unwrap_err().contains("block's holder"));
        assert_eq!(kv(&c.hss("LIR +19725550123", 0).unwrap(), "result"), "local", "a ported-in number is ours");
        let saved = Db::parse(&c.db().to_text()).unwrap();
        assert_eq!(&saved, c.db(), "enum key and ported round-trip");
        assert!(c.db().to_text().contains("enum key=000102030405060708090a0b0c0d0e0f"));
        c.admin("number-release 310150000000002", 0).unwrap();
        assert!(!c.db().subs["310150000000002"].ims.ported);
    }
}
