//! `smscd`: the carrier's SMSC (docs/TELEPHONY.md). The machine is `ecm_tel::smsc`.
//!
//!   smscd start [--wait] | stop | status | queue | log [n]
//!   smscd config | config keys | config set <key> <value>...
//!   smscd enable | disable
//!
//! Short codes: a program connects to 127.0.0.1:7730 and sends `register <code>`;
//! texts to that code arrive as `sms <code> <from> <text>` lines and it answers
//! with `reply <code> <to> <text>` (text percent-encoded: `%0A` newline, `%25` %).

#[cfg(target_os = "wasi")]
mod prog;

/// Percent-encoding for the short-code socket's text fields.
pub fn enc(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_graphic() && b != b'%' {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{:02X}", b));
        }
    }
    o
}

pub fn dec(s: &str) -> String {
    let b = s.as_bytes();
    let mut o = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 3 <= b.len() {
            if let Some(x) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                o.push(x);
                i += 3;
                continue;
            }
        }
        o.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&o).into_owned()
}

#[cfg(target_os = "wasi")]
fn main() {
    prog::main();
}

#[cfg(not(target_os = "wasi"))]
fn main() {
    eprintln!("smscd runs on a computer (wasm32-wasip1)");
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    #[test]
    fn percent_round_trip() {
        for t in ["BAL", "hello world", "50% off\nnow", "héllo"] {
            assert_eq!(super::dec(&super::enc(t)), t);
        }
        assert!(!super::enc("a b").contains(' '));
    }
}
