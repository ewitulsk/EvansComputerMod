//! Scenario steps: a tiny line-oriented DSL inside `[scenario] steps`.
//!
//! ```text
//! # comment
//! send h1 "ifconfig eth0 10.0.0.1/24"    # type a line + Enter; sets h1's mark
//! type h1 "abc\x14"                       # raw keystrokes (escapes: \n \r \t \\ \" \xNN)
//! ctrl_t h1                               # Ctrl+T (interrupt + IRQ_TERMINATE, like Java)
//! expect h1 /bytes from/ within 5s        # regex over text produced after the
//!                                         # last `send`'s echo line (never the
//!                                         # typed command itself)
//! expect_any h1 /Welcome/                 # regex over the whole transcript
//! expect_not h1 /Unreachable/ for 3s      # must not appear (after the mark)
//! wait_prompt h1 within 10s               # prompt back on a line after the mark
//! wait 2s
//! link down h1:eth0 | link up h1:eth0     # pull / re-plug a cable (or a segment name)
//! link set h1:eth0 drop=10 delay=5ms duplicate=0 reorder=0 reorder_delay=5ms
//! assert_frames link=sw1:eth1 max=100 [min=1] during 10s
//! frames link=sw1:eth1                    # print the counter
//! pcap link=sw1:eth1 file=sw1-eth1.pcap   # start capturing (relative to --out-dir)
//! pcap_stop link=sw1:eth1
//! dump h1                                 # print h1's screen
//! log "any text"
//! ```
//! Regexes use Rust `regex` syntax; `/.../i` for case-insensitive, `\/`
//! for a literal slash. Matching is multi-line (`(?m)`): `^`/`$` anchor
//! at line boundaries of the rendered screen text.

use regex::Regex;

use crate::net::Faults;
use crate::sim::Sim;
use crate::util::{fmt_ms, parse_duration_ms};

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Word(String),
    Str(String),
    Re(String, bool),
}

fn unescape(s: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('e') => out.push('\x1b'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('x') => {
                let h: String = [it.next(), it.next()].iter().flatten().collect();
                let v = u8::from_str_radix(&h, 16).map_err(|_| format!("bad \\x escape '\\x{}'", h))?;
                out.push(v as char);
            }
            Some(o) => return Err(format!("unknown escape '\\{}'", o)),
            None => return Err("trailing backslash".into()),
        }
    }
    Ok(out)
}

fn tokenize(line: &str) -> Result<Vec<Tok>, String> {
    let mut toks = Vec::new();
    let b: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '#' {
            break;
        }
        if c == '"' || c == '/' {
            let q = c;
            let mut j = i + 1;
            let mut raw = String::new();
            let mut closed = false;
            while j < b.len() {
                if b[j] == '\\' && j + 1 < b.len() {
                    if q == '/' && b[j + 1] == '/' {
                        raw.push('/');
                    } else {
                        raw.push(b[j]);
                        raw.push(b[j + 1]);
                    }
                    j += 2;
                    continue;
                }
                if b[j] == q {
                    closed = true;
                    break;
                }
                raw.push(b[j]);
                j += 1;
            }
            if !closed {
                return Err(format!("unterminated {}", if q == '"' { "string" } else { "regex" }));
            }
            j += 1;
            if q == '"' {
                toks.push(Tok::Str(unescape(&raw)?));
            } else {
                let ci = j < b.len() && b[j] == 'i';
                if ci {
                    j += 1;
                }
                toks.push(Tok::Re(raw, ci));
            }
            i = j;
            continue;
        }
        // word, possibly key="quoted value"
        let mut j = i;
        let mut w = String::new();
        while j < b.len() && !b[j].is_whitespace() {
            if b[j] == '"' && w.ends_with('=') {
                let mut k = j + 1;
                let mut raw = String::new();
                while k < b.len() && b[k] != '"' {
                    if b[k] == '\\' && k + 1 < b.len() {
                        raw.push(b[k]);
                        k += 1;
                    }
                    raw.push(b[k]);
                    k += 1;
                }
                if k >= b.len() {
                    return Err("unterminated string".into());
                }
                w.push_str(&unescape(&raw)?);
                j = k + 1;
                continue;
            }
            w.push(b[j]);
            j += 1;
        }
        toks.push(Tok::Word(w));
        i = j;
    }
    Ok(toks)
}

#[derive(Clone, Debug)]
pub enum Step {
    Send { node: String, text: String },
    Type { node: String, text: String },
    CtrlT { node: String },
    Expect { node: String, re: String, ci: bool, within: Option<i64>, any: bool },
    ExpectNot { node: String, re: String, ci: bool, dur: i64 },
    WaitPrompt { node: String, within: Option<i64> },
    Wait { ms: i64 },
    Link { spec: String, up: bool },
    LinkSet { spec: String, kv: Vec<(String, String)> },
    AssertFrames { link: String, max: Option<u64>, min: Option<u64>, during: i64 },
    Frames { link: String },
    Pcap { link: String, file: String },
    PcapStop { link: String },
    Dump { node: String },
    Log { text: String },
}

#[derive(Clone, Debug)]
pub struct Line {
    pub no: usize,
    pub text: String,
    pub step: Step,
}

fn word(t: Option<&Tok>, what: &str) -> Result<String, String> {
    match t {
        Some(Tok::Word(w)) => Ok(w.clone()),
        Some(o) => Err(format!("expected {}, got {:?}", what, o)),
        None => Err(format!("missing {}", what)),
    }
}

fn dur(s: &str) -> Result<i64, String> {
    parse_duration_ms(s).ok_or_else(|| format!("bad duration '{}'", s))
}

fn kv(toks: &[Tok]) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        match &toks[i] {
            Tok::Word(w) if w.contains('=') => {
                let (k, v) = w.split_once('=').unwrap();
                out.push((k.to_string(), v.to_string()));
            }
            Tok::Word(w) if (w == "during" || w == "for" || w == "within") && i + 1 < toks.len() => {
                out.push((w.clone(), word(toks.get(i + 1), "duration")?));
                i += 1;
            }
            o => return Err(format!("unexpected {:?}", o)),
        }
        i += 1;
    }
    Ok(out)
}

fn get<'a>(kv: &'a [(String, String)], k: &str) -> Option<&'a str> {
    kv.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str())
}

fn parse_step(line: &str) -> Result<Option<Step>, String> {
    let toks = tokenize(line)?;
    let Some(first) = toks.first() else { return Ok(None) };
    let Tok::Word(cmd) = first else { return Err("a step starts with a command word".into()) };
    let rest = &toks[1..];
    let opt_within = |i: usize| -> Result<Option<i64>, String> {
        match (rest.get(i), rest.get(i + 1)) {
            (None, _) => Ok(None),
            (Some(Tok::Word(w)), Some(Tok::Word(d))) if w == "within" => Ok(Some(dur(d)?)),
            (o, _) => Err(format!("expected 'within <duration>', got {:?}", o)),
        }
    };
    let s = match cmd.as_str() {
        "send" | "type" => {
            let node = word(rest.first(), "node")?;
            let text = match rest.get(1) {
                Some(Tok::Str(s)) => s.clone(),
                o => return Err(format!("expected a quoted string, got {:?}", o)),
            };
            if rest.len() > 2 {
                return Err("trailing tokens".into());
            }
            if cmd == "send" {
                Step::Send { node, text }
            } else {
                Step::Type { node, text }
            }
        }
        "ctrl_t" => Step::CtrlT { node: word(rest.first(), "node")? },
        "expect" | "expect_output" | "expect_any" => {
            let node = word(rest.first(), "node")?;
            let (re, ci) = match rest.get(1) {
                Some(Tok::Re(r, ci)) => (r.clone(), *ci),
                Some(Tok::Str(s)) => (regex::escape(s), false),
                o => return Err(format!("expected /regex/, got {:?}", o)),
            };
            Step::Expect { node, re, ci, within: opt_within(2)?, any: cmd == "expect_any" }
        }
        "expect_not" => {
            let node = word(rest.first(), "node")?;
            let (re, ci) = match rest.get(1) {
                Some(Tok::Re(r, ci)) => (r.clone(), *ci),
                Some(Tok::Str(s)) => (regex::escape(s), false),
                o => return Err(format!("expected /regex/, got {:?}", o)),
            };
            let d = match (rest.get(2), rest.get(3)) {
                (None, _) => 0,
                (Some(Tok::Word(w)), Some(Tok::Word(d))) if w == "for" => dur(d)?,
                (o, _) => return Err(format!("expected 'for <duration>', got {:?}", o)),
            };
            Step::ExpectNot { node, re, ci, dur: d }
        }
        "wait_prompt" => Step::WaitPrompt { node: word(rest.first(), "node")?, within: opt_within(1)? },
        "wait" | "sleep" => Step::Wait { ms: dur(&word(rest.first(), "duration")?)? },
        "link" => {
            let action = word(rest.first(), "up/down/set")?;
            let spec = word(rest.get(1), "link endpoint")?;
            match action.as_str() {
                "up" => Step::Link { spec, up: true },
                "down" => Step::Link { spec, up: false },
                "set" => Step::LinkSet { spec, kv: kv(&rest[2..])? },
                o => return Err(format!("link {}: expected up, down or set", o)),
            }
        }
        "assert_frames" => {
            let kv = kv(rest)?;
            let link = get(&kv, "link").ok_or("assert_frames needs link=")?.to_string();
            let max = get(&kv, "max").map(|v| v.parse::<u64>().map_err(|_| format!("bad max '{}'", v))).transpose()?;
            let min = get(&kv, "min").map(|v| v.parse::<u64>().map_err(|_| format!("bad min '{}'", v))).transpose()?;
            if max.is_none() && min.is_none() {
                return Err("assert_frames needs max= and/or min=".into());
            }
            let during = dur(get(&kv, "during").ok_or("assert_frames needs 'during <duration>'")?)?;
            Step::AssertFrames { link, max, min, during }
        }
        "frames" => Step::Frames { link: get(&kv(rest)?, "link").ok_or("frames needs link=")?.to_string() },
        "pcap" => {
            let kv = kv(rest)?;
            Step::Pcap {
                link: get(&kv, "link").ok_or("pcap needs link=")?.to_string(),
                file: get(&kv, "file").ok_or("pcap needs file=")?.to_string(),
            }
        }
        "pcap_stop" => Step::PcapStop { link: get(&kv(rest)?, "link").ok_or("pcap_stop needs link=")?.to_string() },
        "dump" => Step::Dump { node: word(rest.first(), "node")? },
        "log" => match rest.first() {
            Some(Tok::Str(s)) => Step::Log { text: s.clone() },
            _ => return Err("log needs a quoted string".into()),
        },
        o => return Err(format!("unknown step '{}'", o)),
    };
    Ok(Some(s))
}

pub fn parse_steps(text: &str) -> Result<Vec<Line>, String> {
    let mut out = Vec::new();
    for (i, l) in text.lines().enumerate() {
        match parse_step(l) {
            Ok(Some(step)) => out.push(Line { no: i + 1, text: l.trim().to_string(), step }),
            Ok(None) => {}
            Err(e) => return Err(format!("steps line {}: {}: {}", i + 1, e, l.trim())),
        }
    }
    Ok(out)
}

// ============================================================ running

pub struct Failure {
    pub line: Line,
    pub reason: String,
    pub node: Option<usize>,
}

fn build_re(re: &str, ci: bool) -> Result<Regex, String> {
    let pat = format!("(?m){}{}", if ci { "(?i)" } else { "" }, re);
    Regex::new(&pat).map_err(|e| format!("bad regex /{}/: {}", re, e))
}

/// The text `expect` looks at: everything after the mark (the echo line
/// of the last `send`), or the whole transcript.
pub fn expect_text(sim: &Sim, node: usize, any: bool) -> String {
    let n = &sim.nodes[node];
    let from = if any { 0 } else { n.mark.map(|m| m + 1).unwrap_or(0) };
    n.kernel.screen.text_from(from)
}

fn parse_faults(kv: &[(String, String)], base: &Faults) -> Result<Faults, String> {
    let mut f = base.clone();
    for (k, v) in kv {
        let pct = || v.trim_end_matches('%').parse::<f64>().map_err(|_| format!("bad {} '{}'", k, v));
        match k.as_str() {
            "drop" => f.drop_pct = pct()?,
            "duplicate" | "dup" => f.dup_pct = pct()?,
            "reorder" => f.reorder_pct = pct()?,
            "delay" => f.delay_ms = dur(v)?,
            "reorder_delay" => f.reorder_ms = dur(v)?,
            o => return Err(format!("unknown link setting '{}'", o)),
        }
    }
    Ok(f)
}

pub fn run_steps(sim: &mut Sim, steps: &[Line], default_timeout: i64, out: &mut dyn FnMut(String)) -> Result<(), Failure> {
    for line in steps {
        let fail = |reason: String, node: Option<usize>| Failure { line: line.clone(), reason, node };
        let nd = |sim: &Sim, n: &str| sim.node(n).map_err(|e| fail(e, None));
        let t0 = sim.sched.elapsed();
        out(format!("[{:>9.3}s] {}", t0 as f64 / 1000.0, line.text));
        match &line.step {
            Step::Send { node, text } => {
                let i = nd(sim, node)?;
                sim.send(i, text, true);
            }
            Step::Type { node, text } => {
                let i = nd(sim, node)?;
                sim.type_raw(i, text.as_bytes());
            }
            Step::CtrlT { node } => {
                let i = nd(sim, node)?;
                sim.ctrl_t(i);
            }
            Step::Expect { node, re, ci, within, any } => {
                let i = nd(sim, node)?;
                let r = build_re(re, *ci).map_err(|e| fail(e, None))?;
                let limit = within.unwrap_or(default_timeout);
                let d = sim.now() + limit;
                let any = *any;
                let ok = sim
                    .run_until(d, |s| s.nodes[i].kernel.faulted.is_some() || r.is_match(&expect_text(s, i, any)))
                    .map_err(|e| fail(e, Some(i)))?;
                if let Some(f) = &sim.nodes[i].kernel.faulted {
                    return Err(fail(format!("kernel faulted: {}", f), Some(i)));
                }
                if !ok {
                    return Err(fail(
                        format!("/{}/ did not appear on {} within {}", re, node, fmt_ms(limit)),
                        Some(i),
                    ));
                }
            }
            Step::ExpectNot { node, re, ci, dur } => {
                let i = nd(sim, node)?;
                let r = build_re(re, *ci).map_err(|e| fail(e, None))?;
                let d = sim.now() + dur;
                let hit = sim.run_until(d, |s| r.is_match(&expect_text(s, i, false))).map_err(|e| fail(e, Some(i)))?;
                if hit {
                    return Err(fail(format!("/{}/ appeared on {} but must not", re, node), Some(i)));
                }
            }
            Step::WaitPrompt { node, within } => {
                let i = nd(sim, node)?;
                let limit = within.unwrap_or(default_timeout);
                let d = sim.now() + limit;
                let ok = sim
                    .run_until(d, |s| s.nodes[i].kernel.faulted.is_some() || s.at_prompt(i))
                    .map_err(|e| fail(e, Some(i)))?;
                if let Some(f) = &sim.nodes[i].kernel.faulted {
                    return Err(fail(format!("kernel faulted: {}", f), Some(i)));
                }
                if !ok {
                    return Err(fail(format!("no prompt on {} within {}", node, fmt_ms(limit)), Some(i)));
                }
            }
            Step::Wait { ms } => sim.run_for(*ms).map_err(|e| fail(e, None))?,
            Step::Link { spec, up } => {
                let what = sim.net.lock().unwrap_or_else(|e| e.into_inner()).set_cable(spec, *up).map_err(|e| fail(e, None))?;
                out(format!("            {} {}", what, if *up { "up" } else { "down (cable pulled)" }));
                sim.sched.notify_event();
            }
            Step::LinkSet { spec, kv } => {
                let mut net = sim.net.lock().unwrap_or_else(|e| e.into_inner());
                let s = net.segment(spec).map_err(|e| fail(e, None))?;
                let f = parse_faults(kv, &net.segs[s].faults).map_err(|e| fail(e, None))?;
                out(format!("            {}: {}", net.segs[s].name, f.describe()));
                net.set_faults(s, f);
            }
            Step::AssertFrames { link, max, min, during } => {
                let seg = sim.net.lock().unwrap_or_else(|e| e.into_inner()).segment(link).map_err(|e| fail(e, None))?;
                let before = sim.net.lock().unwrap_or_else(|e| e.into_inner()).segs[seg].frames;
                sim.run_for(*during).map_err(|e| fail(e, None))?;
                let n = sim.net.lock().unwrap_or_else(|e| e.into_inner()).segs[seg].frames - before;
                out(format!("            {} frames on {} in {}", n, link, fmt_ms(*during)));
                if let Some(m) = max {
                    if n > *m {
                        return Err(fail(format!("{} frames on {} in {} (max {}): storm?", n, link, fmt_ms(*during), m), None));
                    }
                }
                if let Some(m) = min {
                    if n < *m {
                        return Err(fail(format!("only {} frames on {} in {} (min {})", n, link, fmt_ms(*during), m), None));
                    }
                }
            }
            Step::Frames { link } => {
                let net = sim.net.lock().unwrap_or_else(|e| e.into_inner());
                let s = net.segment(link).map_err(|e| fail(e, None))?;
                let sg = &net.segs[s];
                out(format!("            {}: {} frames, {} bytes, {} dropped", sg.name, sg.frames, sg.bytes, sg.dropped));
            }
            Step::Pcap { link, file } => {
                let path = sim.out_path(file);
                let mut net = sim.net.lock().unwrap_or_else(|e| e.into_inner());
                let s = net.segment(link).map_err(|e| fail(e, None))?;
                net.start_pcap(s, &path).map_err(|e| fail(e, None))?;
                out(format!("            capturing {} -> {}", net.segs[s].name, path.display()));
            }
            Step::PcapStop { link } => {
                let mut net = sim.net.lock().unwrap_or_else(|e| e.into_inner());
                let s = net.segment(link).map_err(|e| fail(e, None))?;
                if let Some((p, n)) = net.stop_pcap(s) {
                    out(format!("            {} frames written to {}", n, p.display()));
                }
            }
            Step::Dump { node } => {
                let i = nd(sim, node)?;
                out(format!("----- {} screen -----\n{}\n---------------------", node, sim.nodes[i].kernel.screen.dump()));
            }
            Step::Log { text } => out(format!("            {}", text)),
        }
    }
    Ok(())
}

/// Human-readable failure report.
pub fn failure_report(sim: &Sim, f: &Failure) -> String {
    let mut s = String::new();
    s.push_str(&format!("FAILED at steps line {}: {}\n  {}\n", f.line.no, f.line.text, f.reason));
    s.push_str(&format!("  virtual time: {}\n", fmt_ms(sim.sched.elapsed())));
    if let Some(i) = f.node {
        let n = &sim.nodes[i];
        s.push_str(&format!(
            "\n--- {}: text considered (after the last command's echo line) ---\n{}\n",
            n.name,
            expect_text(sim, i, false)
        ));
    }
    for (i, n) in sim.nodes.iter().enumerate() {
        s.push_str(&format!("\n=== {} screen (cursor {},{}) ===\n{}\n", n.name, n.kernel.screen.cursor.0, n.kernel.screen.cursor.1, n.kernel.screen.dump()));
        if let Some(fl) = &n.kernel.faulted {
            s.push_str(&format!("!!! kernel faulted: {}\n", fl));
        }
        let log = sim.host_log(i);
        if !log.is_empty() {
            s.push_str(&format!("--- {} host log (last {}) ---\n", n.name, log.len().min(25)));
            for l in log.iter().rev().take(25).rev() {
                s.push_str(l);
                s.push('\n');
            }
        }
    }
    {
        let net = sim.net.lock().unwrap_or_else(|e| e.into_inner());
        s.push_str("\n=== segments ===\n");
        for sg in &net.segs {
            let members: Vec<String> = sg.members.iter().map(|&m| net.endpoint_name(m)).collect();
            s.push_str(&format!(
                "{:<28} {:<4} frames={:<8} dropped={:<6} [{}]\n",
                sg.name,
                if sg.up { "up" } else { "DOWN" },
                sg.frames,
                sg.dropped,
                members.join(", ")
            ));
        }
    }
    for w in &sim.warnings {
        s.push_str(&format!("warning: {}\n", w));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_steps() {
        let s = parse_steps(
            r#"
            # comment
            send h1 "ifconfig eth0 10.0.0.1/24"
            expect h1 /bytes from 10\.0\.0\.2/ within 5s
            expect h1 /a\/b/i
            expect_not h1 /Unreachable/ for 2s
            wait 1.5s
            link down h1:eth0
            link set sw1:eth1 drop=10 delay=5ms
            assert_frames link=sw1:eth1 max=100 during 10s
            pcap link=sw1:eth1 file="out dir/x.pcap"
            type h1 "a\x14b"
            wait_prompt h1 within 3s
            "#,
        )
        .unwrap();
        assert_eq!(s.len(), 11);
        match &s[0].step {
            Step::Send { node, text } => assert_eq!((node.as_str(), text.as_str()), ("h1", "ifconfig eth0 10.0.0.1/24")),
            o => panic!("{:?}", o),
        }
        match &s[1].step {
            Step::Expect { re, within, .. } => assert_eq!((re.as_str(), *within), (r"bytes from 10\.0\.0\.2", Some(5000))),
            o => panic!("{:?}", o),
        }
        match &s[2].step {
            Step::Expect { re, ci, .. } => assert_eq!((re.as_str(), *ci), ("a/b", true)),
            o => panic!("{:?}", o),
        }
        match &s[7].step {
            Step::AssertFrames { max, during, .. } => assert_eq!((*max, *during), (Some(100), 10_000)),
            o => panic!("{:?}", o),
        }
        match &s[8].step {
            Step::Pcap { file, .. } => assert_eq!(file, "out dir/x.pcap"),
            o => panic!("{:?}", o),
        }
        match &s[9].step {
            Step::Type { text, .. } => assert_eq!(text.as_bytes(), b"a\x14b"),
            o => panic!("{:?}", o),
        }
    }

    #[test]
    fn rejects_bad_steps() {
        assert!(parse_steps("frobnicate h1").is_err());
        assert!(parse_steps("send h1 unquoted").is_err());
        assert!(parse_steps("expect h1 /unterminated").is_err());
        assert!(parse_steps("assert_frames link=x during 1s").is_err());
    }
}
