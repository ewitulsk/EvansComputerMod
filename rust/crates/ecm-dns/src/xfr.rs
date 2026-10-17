//! Zone transfers and change notification between a primary and its secondaries:
//! DNS over TCP framing (a two-byte length before every message), AXFR (RFC 5936: the
//! SOA, every record, the SOA again, over as many messages as needed) and NOTIFY
//! (RFC 1996: "the zone changed, come and check the serial").

use crate::name::is_sub;
use crate::wire::*;
use crate::zone::Zone;

/// Records per AXFR message (well under 64 KiB for our zones).
pub const AXFR_CHUNK: usize = 64;

/// One message with its TCP length prefix.
pub fn frame(msg: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(msg.len() + 2);
    b.extend_from_slice(&(msg.len().min(65535) as u16).to_be_bytes());
    b.extend_from_slice(&msg[..msg.len().min(65535)]);
    b
}

/// Splits a TCP byte stream back into messages.
#[derive(Default, Debug)]
pub struct Deframer {
    buf: Vec<u8>,
}

impl Deframer {
    pub fn push(&mut self, data: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(data);
        let mut out = Vec::new();
        loop {
            if self.buf.len() < 2 {
                break;
            }
            let len = u16::from_be_bytes([self.buf[0], self.buf[1]]) as usize;
            if self.buf.len() < 2 + len {
                break;
            }
            out.push(self.buf[2..2 + len].to_vec());
            self.buf.drain(..2 + len);
        }
        out
    }

    pub fn buffered(&self) -> usize {
        self.buf.len()
    }
}

/// The AXFR answer to `q` from `zone`, as messages to frame and send.
pub fn axfr_messages(q: &Message, zone: &Zone) -> Vec<Message> {
    let mut records: Vec<Record> = Vec::with_capacity(zone.records.len() + 1);
    records.extend(zone.records.iter().cloned());
    records.push(zone.soa_record().clone());
    records
        .chunks(AXFR_CHUNK)
        .map(|c| {
            let mut m = Message::response_to(q, NOERROR);
            m.header.aa = true;
            m.answers = c.to_vec();
            m
        })
        .collect()
}

/// Collects an incoming AXFR until the closing SOA.
#[derive(Debug)]
pub struct AxfrClient {
    pub origin: String,
    pub id: u16,
    records: Vec<Record>,
    messages: usize,
}

impl AxfrClient {
    pub fn new(origin: &str, id: u16) -> AxfrClient {
        AxfrClient { origin: origin.to_string(), id, records: Vec::new(), messages: 0 }
    }

    /// The query to send (over TCP, framed).
    pub fn query(&self) -> Message {
        Message::query(self.id, &self.origin, AXFR, false)
    }

    /// Feed one message. `Ok(Some(zone))` when the transfer is complete.
    pub fn feed(&mut self, data: &[u8]) -> Result<Option<Zone>, String> {
        let m = Message::decode(data).map_err(|e| format!("bad message in the transfer: {e:?}"))?;
        if m.header.id != self.id || !m.header.qr {
            return Err("unexpected message in the transfer".into());
        }
        if m.header.rcode != NOERROR {
            return Err(format!("transfer refused: {}", rcode_name(m.header.rcode)));
        }
        self.messages += 1;
        for r in m.answers {
            if !is_sub(&r.name, &self.origin) {
                return Err(format!("record {} outside the zone", r.name));
            }
            if self.records.is_empty() && r.rtype != SOA {
                return Err("transfer does not start with the SOA".into());
            }
            let closing = r.rtype == SOA && !self.records.is_empty();
            if closing {
                let zone = Zone::from_records(&self.origin, std::mem::take(&mut self.records))
                    .map_err(|e| format!("transferred zone is invalid: {}", e.msg))?;
                return Ok(Some(zone));
            }
            self.records.push(r);
            if self.records.len() > 100_000 {
                return Err("transfer too large".into());
            }
        }
        Ok(None)
    }
}

/// A NOTIFY for `zone` (its SOA in the answer section, as BIND sends it).
pub fn notify(zone: &Zone, id: u16) -> Message {
    let mut m = Message::query(id, &zone.origin, SOA, false);
    m.header.opcode = OP_NOTIFY;
    m.header.aa = true;
    m.answers.push(zone.soa_record().clone());
    m
}

/// The acknowledgement a secondary sends back for a NOTIFY.
pub fn notify_ack(q: &Message) -> Message {
    let mut r = Message::response_to(q, NOERROR);
    r.header.aa = true;
    r
}

/// Serial arithmetic (RFC 1982): is `a` newer than `b`?
pub fn serial_newer(a: u32, b: u32) -> bool {
    a != b && a.wrapping_sub(b) < 0x8000_0000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axfr_roundtrip_in_pieces() {
        let mut text = String::from("$TTL 60\n@ SOA ns1 a 9 1 1 1 1\n@ NS ns1\nns1 A 10.0.0.1\n");
        for i in 0..150 {
            text.push_str(&format!("h{i} A 10.0.{}.{}\n", i / 200, i % 200));
        }
        let z = Zone::parse(&text, "big.test").unwrap();
        let c = AxfrClient::new("big.test", 77);
        let msgs = axfr_messages(&c.query(), &z);
        assert_eq!(msgs.len(), 3);
        // through the TCP framing, a byte at a time
        let mut stream = Vec::new();
        for m in &msgs {
            stream.extend(frame(&m.encode()));
        }
        let mut d = Deframer::default();
        let mut c = c;
        let mut got = None;
        for b in stream {
            for m in d.push(&[b]) {
                if let Some(z) = c.feed(&m).unwrap() {
                    got = Some(z);
                }
            }
        }
        assert_eq!(got.unwrap(), z);
    }

    #[test]
    fn axfr_carries_naptr_and_srv() {
        let origin = "5.5.5.4.1.2.1.e164.arpa";
        let z = Zone::parse(crate::zone::tests::ENUM_ZONE, origin).unwrap();
        let mut c = AxfrClient::new(origin, 9);
        let msgs = axfr_messages(&c.query(), &z);
        let mut got = None;
        for m in &msgs {
            got = c.feed(&m.encode()).unwrap();
        }
        let got = got.unwrap();
        assert_eq!(got, z);
        assert_eq!(got.records.iter().filter(|r| r.rtype == NAPTR).count(), 3);
        assert_eq!(got.records.iter().filter(|r| r.rtype == SRV).count(), 2);
    }

    #[test]
    fn axfr_rejects_junk() {
        let z = Zone::parse("$TTL 60\n@ SOA ns1 a 9 1 1 1 1\n@ NS ns1\n", "t").unwrap();
        let mut c = AxfrClient::new("other", 1);
        let m = &axfr_messages(&c.query(), &z)[0];
        assert!(c.feed(&m.encode()).is_err(), "records outside the zone");
        let mut c = AxfrClient::new("t", 2);
        assert!(c.feed(&m.encode()).is_err(), "wrong id");
        let mut c = AxfrClient::new("t", 1);
        let mut r = Message::response_to(&c.query(), REFUSED);
        r.header.id = 1;
        assert!(c.feed(&r.encode()).unwrap_err().contains("REFUSED"));
    }

    #[test]
    fn notify_and_serials() {
        let z = Zone::parse("$TTL 60\n@ SOA ns1 a 9 1 1 1 1\n@ NS ns1\n", "t").unwrap();
        let n = Message::decode(&notify(&z, 5).encode()).unwrap();
        assert_eq!(n.header.opcode, OP_NOTIFY);
        assert_eq!(crate::auth::answer_serial(&n), Some(9));
        assert!(notify_ack(&n).header.qr);
        assert!(serial_newer(10, 9) && !serial_newer(9, 9) && serial_newer(0, u32::MAX));
    }
}
