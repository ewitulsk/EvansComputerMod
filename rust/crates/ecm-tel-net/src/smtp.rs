//! SMTP (RFC 5321, trimmed) for MM4 between carriers' MMSCs: the receiving
//! side as a line state machine, and the sending side's dialogue.

/// The server side of one SMTP connection.
#[derive(Debug, Default)]
pub struct Server {
    domain: String,
    state: St,
    from: String,
    rcpt: Vec<String>,
    data: String,
    buf: Vec<u8>,
}

#[derive(Debug, Default, PartialEq)]
enum St {
    #[default]
    Start,
    Ready,
    Data,
    Done,
}

/// What the server produced for some input.
#[derive(Debug, Default, PartialEq)]
pub struct Step {
    /// Lines to send back.
    pub replies: Vec<String>,
    /// A complete message: (envelope from, recipients, text).
    pub message: Option<(String, Vec<String>, String)>,
    pub close: bool,
}

impl Server {
    pub fn new(domain: &str) -> Server {
        Server { domain: domain.into(), ..Default::default() }
    }

    /// The greeting to send on connect.
    pub fn greeting(&self) -> String {
        format!("220 mms.{} ESMTP ecm-mmscd", self.domain)
    }

    /// Bytes from the client. A message waiting for the MMSC's verdict gets its
    /// reply from [`Server::verdict`].
    pub fn push(&mut self, b: &[u8]) -> Step {
        self.buf.extend_from_slice(b);
        let mut step = Step::default();
        while let Some(i) = self.buf.iter().position(|c| *c == b'\n') {
            let raw: Vec<u8> = self.buf.drain(..=i).collect();
            let line = String::from_utf8_lossy(&raw).trim_end_matches(['\r', '\n']).to_string();
            if self.state == St::Data {
                if line == "." {
                    self.state = St::Ready;
                    step.message = Some((std::mem::take(&mut self.from), std::mem::take(&mut self.rcpt), std::mem::take(&mut self.data)));
                    // the reply comes with the verdict
                    return step;
                }
                let l = line.strip_prefix('.').filter(|_| line.starts_with("..")).unwrap_or(&line);
                self.data.push_str(l);
                self.data.push_str("\r\n");
                if self.data.len() > 4 * 1024 * 1024 {
                    step.replies.push("552 message too large".into());
                    step.close = true;
                    self.state = St::Done;
                    return step;
                }
                continue;
            }
            let up = line.to_ascii_uppercase();
            let r = if up.starts_with("EHLO") || up.starts_with("HELO") {
                self.state = St::Ready;
                format!("250 mms.{} hello", self.domain)
            } else if up.starts_with("MAIL FROM:") {
                self.from = line[10..].trim().trim_matches(['<', '>']).to_string();
                self.rcpt.clear();
                "250 ok".into()
            } else if up.starts_with("RCPT TO:") {
                self.rcpt.push(line[8..].trim().trim_matches(['<', '>']).to_string());
                "250 ok".into()
            } else if up == "DATA" {
                if self.rcpt.is_empty() {
                    "503 no recipients".into()
                } else {
                    self.state = St::Data;
                    self.data.clear();
                    "354 end with .".into()
                }
            } else if up == "QUIT" {
                step.close = true;
                self.state = St::Done;
                "221 bye".into()
            } else if up == "RSET" || up == "NOOP" {
                "250 ok".into()
            } else {
                "502 not implemented".into()
            };
            step.replies.push(r);
            if step.close {
                break;
            }
        }
        step
    }

    /// The MMSC's answer to the last message: `Ok` (250) or the error line.
    pub fn verdict(r: &Result<(), String>) -> String {
        match r {
            Ok(()) => "250 accepted".into(),
            Err(e) => e.clone(),
        }
    }
}

/// The lines a client sends for one message (dot-stuffed).
pub fn client_script(helo: &str, from: &str, rcpt: &[String], message: &str) -> Vec<String> {
    let mut v = vec![format!("EHLO {}", helo), format!("MAIL FROM:<{}>", from)];
    for r in rcpt {
        v.push(format!("RCPT TO:<{}>", r));
    }
    v.push("DATA".into());
    let mut body = String::new();
    for l in message.lines() {
        if l.starts_with('.') {
            body.push('.');
        }
        body.push_str(l);
        body.push_str("\r\n");
    }
    body.push('.');
    v.push(body);
    v.push("QUIT".into());
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_through_the_server() {
        let mut s = Server::new("cell-red.ecm");
        assert!(s.greeting().starts_with("220"));
        let script = client_script("mms.lakeline.ecm", "system-user@mms.lakeline.ecm", &["+12145550143@cell-red.ecm/TYPE=PLMN".into()], "X-Mms-Message-Type: MM4_forward.REQ\r\n\r\n.hidden\r\nbody");
        let mut got = None;
        let mut replies = Vec::new();
        for line in &script {
            let st = s.push(format!("{}\r\n", line).as_bytes());
            replies.extend(st.replies);
            if st.message.is_some() {
                got = st.message;
                replies.push(Server::verdict(&Ok(())));
            }
        }
        let (from, rcpt, text) = got.unwrap();
        assert_eq!(from, "system-user@mms.lakeline.ecm");
        assert_eq!(rcpt, vec!["+12145550143@cell-red.ecm/TYPE=PLMN".to_string()]);
        assert!(text.contains("\r\n.hidden\r\n"), "dot-stuffing undone: {:?}", text);
        assert_eq!(replies.iter().filter(|r| r.starts_with("250")).count(), 4, "{:?}", replies);
        assert!(replies.last().unwrap().starts_with("221"));
    }
}
