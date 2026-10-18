//! `sms`: texts from a terminal, through `telephonyd` (docs/TELEPHONY.md).
//!
//!   sms send <number> <text...>                      a text (long ones go as several parts)
//!   sms mms <number[,number...]> [--text T] [--file F]...   a picture, voice note or group text
//!   sms list                                         conversations, newest first
//!   sms read <number|thread>                         a conversation (marks it read)
//!   sms delete <id> | sms watch

#[cfg(target_os = "wasi")]
fn main() {
    use ecm_tel::ctl::{self, enc, Event, MsgLine, ThreadLine};
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let call = |line: &str| -> Result<Vec<String>, String> {
        ecm_cell_net::admin_call(ctl::PORT, line).map_err(|e| if e == "not running" { "telephonyd is not running (an ePhone starts it; on a Terminal: telephonyd &)".into() } else { e })
    };
    let fail = |e: String| -> ! {
        eprintln!("sms: {e}");
        std::process::exit(1);
    };
    let pretty = |n: &str| ecm_tel::number::E164::parse(n).map(|x| x.display("1")).unwrap_or(n.to_string());
    match a.as_slice() {
        ["send", to, words @ ..] if !words.is_empty() => {
            let text = words.join(" ");
            let (_, parts, _) = ecm_tel::sms::parts_needed(&text);
            match call(&format!("sms {} {}", enc(to), enc(&text))) {
                Ok(b) => println!("sms: sending to {} ({} message{}) id {}", pretty(to), parts, if parts == 1 { "" } else { "s" }, b.first().map(|s| s.trim_start_matches("sms ")).unwrap_or("?")),
                Err(e) => fail(e),
            }
        }
        ["mms", to, rest @ ..] => {
            let (mut text, mut files) = (String::new(), Vec::new());
            let mut i = 0;
            while i < rest.len() {
                match rest[i] {
                    "--text" if i + 1 < rest.len() => {
                        text = rest[i + 1].to_string();
                        i += 1;
                    }
                    "--file" if i + 1 < rest.len() => {
                        files.push(rest[i + 1].to_string());
                        i += 1;
                    }
                    w => {
                        if !text.is_empty() {
                            text.push(' ');
                        }
                        text.push_str(w);
                    }
                }
                i += 1;
            }
            let f = if files.is_empty() { "-".to_string() } else { files.iter().map(|x| enc(x)).collect::<Vec<_>>().join(",") };
            match call(&format!("mms {} {} {}", to.split(',').map(enc).collect::<Vec<_>>().join(","), enc(&text), f)) {
                Ok(_) => println!("sms: MMS sent to {}", to.split(',').map(pretty).collect::<Vec<_>>().join(", ")),
                Err(e) => fail(e),
            }
        }
        ["list"] | [] => match call("threads") {
            Ok(b) if b.is_empty() => println!("sms: no messages"),
            Ok(b) => {
                for t in b.iter().filter_map(|l| ThreadLine::parse(l)) {
                    let who = t.members().iter().map(|m| pretty(m)).collect::<Vec<_>>().join(", ");
                    println!("{} {}{}  {}", if t.unread > 0 { "*" } else { " " }, who, if t.members().len() > 1 { " (group)" } else { "" }, t.preview.chars().take(50).collect::<String>());
                }
            }
            Err(e) => fail(e),
        },
        ["read", who] => {
            // a number (any form) or a thread key
            let key = if who.contains(',') {
                who.to_string()
            } else {
                ecm_tel::number::normalize(who, "1", None).map(|n| n.to_string()).unwrap_or(who.to_string())
            };
            match call(&format!("messages {}", key)) {
                Ok(b) => {
                    for m in b.iter().filter_map(|l| MsgLine::parse(l)) {
                        let who = if m.incoming { pretty(&m.from) } else { "me".into() };
                        let files = if m.files.is_empty() { String::new() } else { format!("  [{}]", m.files.join(", ")) };
                        let st = if m.incoming { String::new() } else { format!("  ({})", m.status.replace("failed:", "failed: ")) };
                        println!("{:>16}: {}{}{}", who, m.text, files, st);
                    }
                    let _ = call(&format!("read {}", key));
                }
                Err(e) => fail(e),
            }
        }
        ["delete", id] => match call(&format!("delete {id}")) {
            Ok(_) => println!("sms: deleted"),
            Err(e) => fail(e),
        },
        ["watch"] => {
            let fd = ecm_cell_net::tcp_connect(ecm_cell_net::LOOPBACK, ctl::PORT).unwrap_or_else(|| fail("telephonyd is not running".into()));
            ecm_cell_net::send_all(fd, b"watch\n");
            let mut buf = [0u8; 4096];
            let mut pending = String::new();
            loop {
                ecm_cell_net::readable(&[fd], 1000);
                match ecm_cell_net::read_now(fd, &mut buf) {
                    ecm_cell_net::Read::Data(d) => {
                        pending.push_str(&String::from_utf8_lossy(&d));
                        while let Some(i) = pending.find('\n') {
                            let l: String = pending.drain(..=i).collect();
                            if let Some(e) = Event::parse(l.trim()) {
                                match e.name.as_str() {
                                    "sms" => println!("{}: {}", pretty(e.get("from")), e.get("text")),
                                    "mms" => println!("{} (MMS): {} {}", pretty(e.get("from")), e.get("text"), e.get("files")),
                                    "sent" | "delivered" | "failed" => println!("message {} {}{}", e.get("id"), e.name, if e.get("why").is_empty() { String::new() } else { format!(": {}", e.get("why")) }),
                                    _ => {}
                                }
                            }
                        }
                    }
                    ecm_cell_net::Read::Nothing => {}
                    ecm_cell_net::Read::Closed => break,
                }
            }
        }
        _ => {
            eprintln!("usage: sms send <number> <text...> | mms <numbers> [--text T] [--file F]... | list | read <number> | delete <id> | watch   (man sms)");
            std::process::exit(2);
        }
    }
}

#[cfg(not(target_os = "wasi"))]
fn main() {
    eprintln!("sms runs on an ECM computer (wasm32-wasip1)");
    std::process::exit(1);
}
