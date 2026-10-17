//! Building SIP requests and responses inside and outside dialogs, the small
//! helpers every endpoint (phone, `imsd`, `smscd`, `mrfd`) shares.

use crate::sip::{new_branch, new_call_id, new_tag, Message, Method, NameAddr, Uri, Via};
use crate::txn::{Addr, Rng};

/// One side's view of a dialog (RFC 3261 12): enough to send in-dialog requests.
#[derive(Clone, Debug, PartialEq)]
pub struct Dialog {
    pub call_id: String,
    /// Our address-of-record with our tag.
    pub local: NameAddr,
    /// The peer's, with its tag once known.
    pub remote: NameAddr,
    pub local_cseq: u32,
    /// Where in-dialog requests go (the peer's Contact).
    pub target: Uri,
    /// The next hop for in-dialog requests.
    pub next_hop: Addr,
}

impl Dialog {
    /// A new dialog we start (UAC).
    pub fn outgoing(rng: &mut Rng, local_uri: Uri, display: Option<&str>, remote_uri: Uri, next_hop: Addr, host: &str) -> Dialog {
        let mut local = NameAddr::new(local_uri);
        local.display = display.map(String::from);
        local.set_tag(&new_tag(rng.next()));
        Dialog { call_id: new_call_id(rng.next(), host), local, remote: NameAddr::new(remote_uri.clone()), local_cseq: 0, target: remote_uri, next_hop }
    }

    /// The dialog a request we answer (UAS) creates; `our_tag` goes in To.
    pub fn incoming(req: &Message, our_tag: &str, from: Addr) -> Option<Dialog> {
        let h = req.headers();
        let mut local = h.to().ok()?;
        if local.tag().is_none() {
            local.set_tag(our_tag);
        }
        let remote = h.from().ok()?;
        let target = match h.contact() {
            Ok(crate::sip::Contact::Addrs(a)) if !a.is_empty() => a[0].uri.clone(),
            _ => remote.uri.clone(),
        };
        let cseq = h.cseq().ok()?.seq;
        Some(Dialog { call_id: h.call_id().ok()?.to_string(), local, remote, local_cseq: cseq.wrapping_add(1000), target, next_hop: from })
    }

    /// A request in (or starting) this dialog, with a fresh Via branch from `via_host:via_port`.
    pub fn request(&mut self, rng: &mut Rng, method: Method, via_host: &str, via_port: u16) -> Message {
        if method != Method::Ack && method != Method::Cancel {
            self.local_cseq += 1;
        }
        let mut m = Message::request(method.clone(), self.target.clone());
        let h = m.headers_mut();
        h.push("Via", Via::new("UDP", via_host, Some(via_port), &new_branch(rng.next())).to_string());
        h.push("Max-Forwards", "70");
        h.push("From", self.local.to_string());
        h.push("To", self.remote.to_string());
        h.push("Call-ID", self.call_id.clone());
        h.push("CSeq", format!("{} {}", self.local_cseq, method.as_str()));
        m
    }

    /// Learn the peer's tag and target from a response (UAC side).
    pub fn learn(&mut self, resp: &Message) {
        let h = resp.headers();
        if let Ok(to) = h.to() {
            if let Some(t) = to.tag() {
                self.remote.set_tag(t);
            }
        }
        if let Ok(crate::sip::Contact::Addrs(a)) = h.contact() {
            if let Some(c) = a.first() {
                self.target = c.uri.clone();
            }
        }
    }
}

/// The ACK for a 2xx: a new transaction in the dialog with the INVITE's CSeq number.
pub fn ack_for(d: &Dialog, rng: &mut Rng, via_host: &str, via_port: u16) -> Message {
    let mut m = Message::request(Method::Ack, d.target.clone());
    let h = m.headers_mut();
    h.push("Via", Via::new("UDP", via_host, Some(via_port), &new_branch(rng.next())).to_string());
    h.push("Max-Forwards", "70");
    h.push("From", d.local.to_string());
    h.push("To", d.remote.to_string());
    h.push("Call-ID", d.call_id.clone());
    h.push("CSeq", format!("{} ACK", d.local_cseq));
    m
}

/// The ACK for a non-2xx final response to `invite`: same branch, To from the response.
pub fn ack_non2xx(invite: &Message, resp: &Message) -> Option<Message> {
    let uri = invite.uri()?.clone();
    let ih = invite.headers();
    let mut m = Message::request(Method::Ack, uri);
    let h = m.headers_mut();
    h.push("Via", ih.get("Via")?.to_string());
    h.push("Max-Forwards", "70");
    h.push("From", ih.get("From")?.to_string());
    h.push("To", resp.headers().get("To")?.to_string());
    h.push("Call-ID", ih.get("Call-ID")?.to_string());
    let seq = ih.cseq().ok()?.seq;
    h.push("CSeq", format!("{} ACK", seq));
    Some(m)
}

/// CANCEL for an INVITE we sent (same branch and CSeq number).
pub fn cancel_for(invite: &Message) -> Option<Message> {
    let uri = invite.uri()?.clone();
    let ih = invite.headers();
    let mut m = Message::request(Method::Cancel, uri);
    let h = m.headers_mut();
    h.push("Via", ih.get("Via")?.to_string());
    h.push("Max-Forwards", "70");
    h.push("From", ih.get("From")?.to_string());
    h.push("To", ih.get("To")?.to_string());
    h.push("Call-ID", ih.get("Call-ID")?.to_string());
    let seq = ih.cseq().ok()?.seq;
    h.push("CSeq", format!("{} CANCEL", seq));
    Some(m)
}

/// Where a response to `req` goes: the top Via's `received`/`rport`, else its sent-by.
pub fn response_target(req: &Message, from: Addr) -> Addr {
    let Ok(v) = req.headers().top_via() else { return from };
    let ip = v.received().and_then(Addr::parse_ip).or_else(|| Addr::parse_ip(&v.host)).unwrap_or(from.ip);
    let port = match v.rport() {
        Some(Some(p)) => p,
        Some(None) => from.port,
        None => v.port.unwrap_or(5060),
    };
    Addr::new(ip, port)
}

/// Stamp `received` (and `rport`) on a request's top Via, as a proxy does.
pub fn stamp_via(req: &mut Message, from: Addr) {
    let h = req.headers_mut();
    let Some(raw) = h.get("Via").map(String::from) else { return };
    // only the first Via value (a header may hold several, comma-separated)
    let vals = crate::sip::split_list(&raw).into_iter().map(String::from).collect::<Vec<_>>();
    let Some(first) = vals.first() else { return };
    let Ok(mut v) = Via::parse(first) else { return };
    if v.host != from.host() {
        v.set_received(&from.host());
    }
    if v.rport().is_some() {
        v.set_rport(from.port);
    }
    let mut out = vec![v.to_string()];
    out.extend(vals.into_iter().skip(1));
    // replace the first Via header's value
    let mut done = false;
    let mut nh = crate::sip::Headers::new();
    for (k, val) in h.iter() {
        if !done && k.eq_ignore_ascii_case("Via") {
            nh.push("Via", out.join(", "));
            done = true;
        } else {
            nh.push(k, val.to_string());
        }
    }
    *h = nh;
}

/// The user part of a URI as a number when it is one (`tel:+1...`, `sip:+1...@...;user=phone`, or digits).
pub fn uri_number(u: &Uri) -> Option<String> {
    let user = u.user()?;
    let n: String = user.split(';').next().unwrap_or("").chars().filter(|c| !matches!(c, '-' | '.' | '(' | ')' | ' ')).collect();
    if n.starts_with('+') && n.len() > 1 && n[1..].bytes().all(|b| b.is_ascii_digit()) {
        return Some(n);
    }
    // service codes: digits, * and #, and a number after them (*72+12145550142)
    if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit() || b == b'*' || b == b'#' || b == b'+') && !n[1..].starts_with('+') {
        return Some(n);
    }
    None
}

/// `<sip:+12145550142@domain;user=phone>`
pub fn number_uri(number: &str, domain: &str) -> Uri {
    let mut u = Uri::sip(Some(number), domain, None);
    if number.starts_with('+') {
        u.set_param("user", Some("phone"));
    }
    u
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialog_round_trip() {
        let mut rng = Rng::new(5);
        let next = Addr::new([10, 0, 0, 1], 5060);
        let mut a = Dialog::outgoing(&mut rng, number_uri("+12145550100", "cell-red.ecm"), Some("Alex"), number_uri("+12145550142", "cell-red.ecm"), next, "10.250.0.2");
        let mut inv = a.request(&mut rng, Method::Invite, "10.250.0.2", 5060);
        inv.headers_mut().push("Contact", "<sip:+12145550100@10.250.0.2:5060>");
        let b = Dialog::incoming(&inv, "btag", Addr::new([10, 250, 0, 2], 5060)).unwrap();
        assert_eq!(b.call_id, a.call_id);
        assert_eq!(b.local.tag(), Some("btag"));
        assert_eq!(b.target.host(), "10.250.0.2");
        let mut ok = inv.response_to(200, Some("btag")).unwrap();
        ok.headers_mut().push("Contact", "<sip:+12145550142@10.250.0.3:5060>");
        a.learn(&ok);
        assert_eq!(a.remote.tag(), Some("btag"));
        assert_eq!(a.target.host(), "10.250.0.3");
        let ack = ack_for(&a, &mut rng, "10.250.0.2", 5060);
        assert_eq!(ack.headers().cseq().unwrap().seq, 1);
        let bye = a.request(&mut rng, Method::Bye, "10.250.0.2", 5060);
        assert_eq!(bye.headers().cseq().unwrap().seq, 2);
        assert!(cancel_for(&inv).unwrap().to_bytes().starts_with(b"CANCEL"));
        let busy = inv.response_to(486, Some("btag")).unwrap();
        let ack2 = ack_non2xx(&inv, &busy).unwrap();
        assert_eq!(crate::txn::branch_of(&ack2), crate::txn::branch_of(&inv));
    }

    #[test]
    fn via_stamping_and_targets() {
        let mut rng = Rng::new(9);
        let mut d = Dialog::outgoing(&mut rng, number_uri("+12145550100", "x"), None, number_uri("+12145550142", "x"), Addr::default(), "h");
        let mut m = d.request(&mut rng, Method::Message, "10.250.0.2", 5060);
        let src = Addr::new([10, 250, 0, 2], 41000);
        assert_eq!(response_target(&m, src), Addr::new([10, 250, 0, 2], 5060));
        stamp_via(&mut m, Addr::new([10, 250, 0, 9], 5060));
        assert_eq!(m.headers().top_via().unwrap().received(), Some("10.250.0.9"));
        assert_eq!(response_target(&m, src), Addr::new([10, 250, 0, 9], 5060));
        assert_eq!(uri_number(&number_uri("+12145550142", "x")).as_deref(), Some("+12145550142"));
        assert_eq!(uri_number(&Uri::parse("tel:+1-214-555-0142").unwrap()).as_deref(), Some("+12145550142"));
        assert_eq!(uri_number(&Uri::parse("sip:*86@x").unwrap()).as_deref(), Some("*86"));
        assert_eq!(uri_number(&Uri::parse("sip:smsc@x").unwrap()), None);
    }
}
