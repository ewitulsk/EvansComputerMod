//! `tel`: calls from a terminal, through `telephonyd` (docs/TELEPHONY.md).
//!
//!   tel [status]                        registration, number, calls
//!   tel dial <number> [--hide]          call (also *86 voicemail, 611 support, *72<number> / *73)
//!   tel answer [id] | hangup [id] | decline [id]
//!   tel hold [id] on|off | swap | merge | mute [id] on|off | speaker on|off
//!   tel dtmf <digits> [id]              keypad tones into the call
//!   tel play <wav> [id] | record <seconds> <wav> [id]
//!   tel calls | log [n] | voicemail     live calls, the call log, visual voicemail
//!   tel forward <number> | forward off  call forwarding (*72 / *73)
//!   tel watch                           print calls as they happen (Ctrl+C stops)

#[cfg(target_os = "wasi")]
fn main() {
    use ecm_tel::ctl::{self, enc, CallLine, Event, LogLine};
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let call = |line: &str| -> Result<Vec<String>, String> {
        ecm_cell_net::admin_call(ctl::PORT, line).map_err(|e| if e == "not running" { "telephonyd is not running (an ePhone starts it; on a Terminal: telephonyd &)".into() } else { e })
    };
    let fail = |e: String| -> ! {
        eprintln!("tel: {e}");
        std::process::exit(1);
    };
    // the call a command means: the one given, else the ringing one, else the active one
    let current = |want: Option<&str>| -> String {
        if let Some(w) = want {
            return w.to_string();
        }
        let calls: Vec<CallLine> = call("calls").unwrap_or_default().iter().filter_map(|l| CallLine::parse(l)).collect();
        calls
            .iter()
            .find(|c| c.state == "incoming")
            .or_else(|| calls.iter().find(|c| c.state == "active"))
            .or_else(|| calls.first())
            .map(|c| c.id.to_string())
            .unwrap_or_else(|| fail("no call".into()))
    };
    let show_calls = |b: &[String]| {
        for l in b {
            if let Some(c) = CallLine::parse(l) {
                let who = if c.hidden { "No Caller ID".to_string() } else { ecm_tel::number::E164::parse(&c.number).map(|n| n.display("1")).unwrap_or(c.number.clone()) };
                println!(
                    "  call {}  {:<9} {} {}  {}:{:02}{}{}{}",
                    c.id,
                    c.state,
                    if c.outgoing { "to" } else { "from" },
                    who,
                    c.seconds / 60,
                    c.seconds % 60,
                    if c.hd { "  HD" } else { "" },
                    if c.muted { "  muted" } else { "" },
                    if c.merged { "  3-way" } else { "" }
                );
            }
        }
    };
    let simple = |line: String| match call(&line) {
        Ok(b) => b.iter().for_each(|l| println!("{l}")),
        Err(e) => fail(e),
    };
    match a.as_slice() {
        [] | ["status"] => match call("status") {
            Ok(b) => {
                let v = |k| ctl::value(&b, k).to_string();
                let num = v("msisdn");
                let shown = ecm_tel::number::E164::parse(&num).map(|n| n.display("1")).unwrap_or(num.clone());
                println!("telephonyd: {}{}", v("registration"), if v("volte") == "1" { " (VoLTE)" } else { "" });
                println!("  number    {}", if num == "-" { "none yet".into() } else { shown });
                println!("  network   {}", v("domain"));
                println!("  voicemail {} new   missed calls {}   unread texts {}", v("voicemail"), v("missed"), v("unread"));
                show_calls(&call("calls").unwrap_or_default());
            }
            Err(e) => fail(e),
        },
        ["dial", n, rest @ ..] => match call(&format!("dial {}{}", enc(n), if rest.contains(&"--hide") { " hide" } else { "" })) {
            Ok(b) => println!("tel: calling {} ({})", n, b.first().cloned().unwrap_or_default()),
            Err(e) => fail(e),
        },
        ["answer"] | ["answer", _] => simple(format!("answer {}", current(a.get(1).copied()))),
        ["hangup"] | ["hangup", _] => simple(format!("hangup {}", current(a.get(1).copied()))),
        ["decline"] | ["decline", _] => simple(format!("decline {}", current(a.get(1).copied()))),
        ["hold", v @ ("on" | "off")] => simple(format!("hold {} {v}", current(None))),
        ["hold", id, v @ ("on" | "off")] => simple(format!("hold {id} {v}")),
        ["swap"] => simple("swap".into()),
        ["merge"] => simple("merge".into()),
        ["mute", v @ ("on" | "off")] => simple(format!("mute {} {v}", current(None))),
        ["mute", id, v @ ("on" | "off")] => simple(format!("mute {id} {v}")),
        ["speaker", v @ ("on" | "off")] => simple(format!("speaker {v}")),
        ["dtmf", d] | ["dtmf", d, _] => simple(format!("dtmf {} {d}", current(a.get(2).copied()))),
        ["play", f] | ["play", f, _] => simple(format!("play {} {}", current(a.get(2).copied()), enc(f))),
        ["record", s, f] | ["record", s, f, _] => simple(format!("record {} {s} {}", current(a.get(3).copied()), enc(f))),
        ["calls"] => match call("calls") {
            Ok(b) if b.is_empty() => println!("tel: no calls"),
            Ok(b) => show_calls(&b),
            Err(e) => fail(e),
        },
        ["log"] | ["log", _] => match call(&format!("log {}", a.get(1).unwrap_or(&"20"))) {
            Ok(b) => {
                for l in b.iter().filter_map(|l| LogLine::parse(l)) {
                    let who = ecm_tel::number::E164::parse(&l.number).map(|n| n.display("1")).unwrap_or(l.number.clone());
                    println!("  {:<6} {}  {}:{:02}  {}", l.dir, who, l.seconds / 60, l.seconds % 60, l.reason);
                }
            }
            Err(e) => fail(e),
        },
        ["voicemail"] => simple("voicemail".into()),
        ["forward", "off"] => simple("forward off".into()),
        ["forward", n] => simple(format!("forward all {}", enc(n))),
        ["watch"] => {
            // the watch stream: a connection reading event lines
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
                                if matches!(e.name.as_str(), "incoming" | "call" | "dtmf" | "voicemail" | "registration") {
                                    println!("{} {}", e.name, e.fields.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(" "));
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
            eprintln!("usage: tel [status] | dial <number> [--hide] | answer | hangup | decline | hold on|off | swap | merge");
            eprintln!("       tel mute on|off | speaker on|off | dtmf <digits> | play <wav> | record <s> <wav>");
            eprintln!("       tel calls | log [n] | voicemail | forward <number>|off | watch   (man tel)");
            std::process::exit(2);
        }
    }
}

#[cfg(not(target_os = "wasi"))]
fn main() {
    eprintln!("tel runs on an ECM computer (wasm32-wasip1)");
    std::process::exit(1);
}
