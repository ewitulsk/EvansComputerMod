//! What dig prints, as plain functions of the messages (tested on the host).

use ecm_dns::name::fqdn;
use ecm_dns::wire::{rcode_name, type_name, Message, Record};
use ecm_dns::zone::Zone;

pub const USAGE: &str = "Usage: dig [@SERVER] NAME [TYPE] [+trace] [+norecurse] [+short] [+tcp] [-p PORT] [-t TYPE]
       dig @SERVER ZONE AXFR      (the whole zone, by zone transfer)
Types: A NS CNAME SOA MX TXT PTR SRV NAPTR ANY. Without @SERVER: /etc/resolv.conf.
       dig -t NAPTR 2.4.1.0.5.5.5.4.1.2.1.e164.arpa   (ENUM: a phone number)
       dig -t SRV _sip._udp.carrier.ecm                (SIP servers of a domain)";

/// `;; ANSWER SECTION:` and its records, after a blank line (nothing when empty).
pub fn section(title: &str, records: &[Record]) -> Vec<String> {
    if records.is_empty() {
        return Vec::new();
    }
    let mut out = vec![String::new(), format!(";; {title} SECTION:")];
    out.extend(records.iter().map(|r| r.to_columns()));
    out
}

/// The header, question and the three record sections, as BIND's dig prints them.
pub fn message(m: &Message) -> Vec<String> {
    let h = &m.header;
    let mut out = vec![
        format!(";; ->>HEADER<<- opcode: QUERY, status: {}, id: {}", rcode_name(h.rcode), h.id),
        format!(
            ";; flags: {}; QUERY: {}, ANSWER: {}, AUTHORITY: {}, ADDITIONAL: {}",
            h.flag_text(),
            m.questions.len(),
            m.answers.len(),
            m.authority.len(),
            m.additional.len()
        ),
    ];
    if h.rd && !h.ra {
        out.push(";; WARNING: recursion requested but not available".into());
    }
    out.push(String::new());
    out.push(";; QUESTION SECTION:".into());
    for q in &m.questions {
        out.push(format!(";{:<22}        IN {}", fqdn(&q.name), type_name(q.qtype)));
    }
    out.extend(section("ANSWER", &m.answers));
    out.extend(section("AUTHORITY", &m.authority));
    out.extend(section("ADDITIONAL", &m.additional));
    out
}

/// `+short`: the data of the answer records only.
pub fn short(m: &Message) -> Vec<String> {
    m.answers.iter().map(|r| r.data.to_text()).collect()
}

/// A transferred zone: every record, then the SOA again (as AXFR sends it).
pub fn axfr(z: &Zone) -> Vec<String> {
    let mut out: Vec<String> = z.records.iter().map(|r| r.to_columns()).collect();
    out.push(z.soa_record().to_columns());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecm_dns::wire::*;

    fn naptr() -> Record {
        Record::new(
            "2.4.1.0.5.5.5.4.1.2.1.e164.arpa",
            300,
            RData::Naptr(Naptr {
                order: 100,
                preference: 10,
                flags: "u".into(),
                services: "E2U+sip".into(),
                regexp: "!^.*$!sip:\\1@carrier.ecm!".into(),
                replacement: String::new(),
            }),
        )
    }

    fn srv() -> Record {
        Record::new("_sip._udp.carrier.ecm", 60, RData::Srv(Srv { priority: 10, weight: 60, port: 5060, target: "sip.carrier.ecm".into() }))
    }

    #[test]
    fn naptr_answer() {
        let q = Message::query(7, "2.4.1.0.5.5.5.4.1.2.1.e164.arpa", NAPTR, true);
        let mut r = Message::response_to(&q, NOERROR);
        r.header.aa = true;
        r.header.ra = true;
        r.answers.push(naptr());
        // through the wire, as dig receives it
        let r = Message::decode(&r.encode()).unwrap();
        assert_eq!(
            message(&r),
            vec![
                ";; ->>HEADER<<- opcode: QUERY, status: NOERROR, id: 7",
                ";; flags: qr aa rd ra; QUERY: 1, ANSWER: 1, AUTHORITY: 0, ADDITIONAL: 0",
                "",
                ";; QUESTION SECTION:",
                ";2.4.1.0.5.5.5.4.1.2.1.e164.arpa.        IN NAPTR",
                "",
                ";; ANSWER SECTION:",
                "2.4.1.0.5.5.5.4.1.2.1.e164.arpa.    300 IN NAPTR 100 10 \"u\" \"E2U+sip\" \"!^.*$!sip:\\\\1@carrier.ecm!\" .",
            ]
        );
        assert_eq!(short(&r), vec!["100 10 \"u\" \"E2U+sip\" \"!^.*$!sip:\\\\1@carrier.ecm!\" ."]);
    }

    #[test]
    fn srv_answer_with_additional() {
        let q = Message::query(8, "_sip._udp.carrier.ecm", SRV, false);
        let mut r = Message::response_to(&q, NOERROR);
        r.header.aa = true;
        r.answers.push(srv());
        r.additional.push(Record::new("sip.carrier.ecm", 60, RData::A([100, 70, 0, 5])));
        let r = Message::decode(&r.encode()).unwrap();
        let lines = message(&r);
        assert_eq!(lines[4], ";_sip._udp.carrier.ecm.        IN SRV");
        assert_eq!(&lines[5..], [
            "",
            ";; ANSWER SECTION:",
            "_sip._udp.carrier.ecm.      60 IN SRV   10 60 5060 sip.carrier.ecm.",
            "",
            ";; ADDITIONAL SECTION:",
            "sip.carrier.ecm.            60 IN A     100.70.0.5",
        ]);
        assert_eq!(short(&r), vec!["10 60 5060 sip.carrier.ecm."]);
    }

    #[test]
    fn axfr_lists_them() {
        let z = Zone::parse(
            "$TTL 60\n@ SOA ns1 a 1 1 1 1 1\n@ NS ns1\nns1 A 10.0.0.1\n_sip._udp SRV 10 60 5060 ns1\n@ NAPTR 10 0 \"s\" \"SIP+D2U\" \"\" _sip._udp\n",
            "carrier.ecm",
        )
        .unwrap();
        let l = axfr(&z);
        assert_eq!(l.len(), 6);
        assert_eq!(l[3], "_sip._udp.carrier.ecm.      60 IN SRV   10 60 5060 ns1.carrier.ecm.");
        assert_eq!(l[4], "carrier.ecm.                60 IN NAPTR 10 0 \"s\" \"SIP+D2U\" \"\" _sip._udp.carrier.ecm.");
        assert!(l[5].contains(" IN SOA "));
    }

    #[test]
    fn usage_names_the_types() {
        assert!(USAGE.contains("SRV NAPTR"));
        assert_eq!(type_from_name("naptr"), Some(NAPTR));
    }
}
