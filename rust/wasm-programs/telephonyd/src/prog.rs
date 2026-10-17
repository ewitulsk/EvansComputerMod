//! The program (WASI only): the phone's IMS stack around `ecm_tel::ua::Ua`.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::time::{Duration, Instant};

use ecm_cell::admin::{reply, Lines};
use ecm_host_abi::peripheral::{self, Value};
use ecm_host_abi::socket;
use ecm_tel::ctl::{self, dec, enc, CallLine, Event, MsgLine};
use ecm_tel::mmsua;
use ecm_tel::txn::Addr;
use ecm_tel::ua::{Bearer, CallState, RegState, Ua, UaConfig, UaEvent, UaOut};
use ecm_tel::vcard;
use ecm_tel_net::sys::{self, Log};

use crate::store::{self, key, Store};

const PATH: &str = "/etc/telephonyd.toml";
const KEYS: &[(&str, &str)] = &[("sim", ""), ("mms", "auto"), ("wideband", "true"), ("speaker", ""), ("mic", ""), ("ringtone", "on")];

fn fail(m: impl AsRef<str>) -> ! {
    println!("telephonyd: {}", m.as_ref());
    std::process::exit(1);
}

fn get_str(m: &Value, key: &str) -> Option<String> {
    match m {
        Value::Map(e) => e.iter().find(|(k, _)| matches!(k, Value::Str(s) if s == key)).and_then(|(_, v)| if let Value::Str(s) = v { Some(s.clone()) } else { None }),
        _ => None,
    }
}

/// The SIM: its peripheral (the phone's tray, or a Terminal modem's slot).
fn sim_name(wanted: &str) -> Option<String> {
    let list = peripheral::list().ok()?;
    if !wanted.is_empty() {
        return list.iter().find(|(n, _)| n == wanted).map(|(n, _)| n.clone());
    }
    list.iter().find(|(_, t)| t == "sim").map(|(n, _)| n.clone())
}

fn ims_aka(sim: &str, rand: &[u8; 16]) -> Option<[u8; 16]> {
    let hex: String = rand.iter().map(|b| format!("{:02x}", b)).collect();
    let v = peripheral::call(sim, "respond_ims", &[Value::Str(hex)]).ok()?;
    let r = ecm_tel::util::unhex(&get_str(&v, "res")?)?;
    let mut o = [0u8; 16];
    if r.len() != 16 {
        return None;
    }
    o.copy_from_slice(&r);
    Some(o)
}

/// One line to `/dev/uictl` (the phone shell's status, notifications, ringing, the wake lock).
fn uictl(cmd: &str) -> String {
    let mut answer = String::new();
    if let Ok(mut f) = std::fs::OpenOptions::new().read(true).write(true).open("/dev/uictl") {
        if f.write_all(cmd.as_bytes()).is_ok() {
            let _ = f.read_to_string(&mut answer);
        }
    }
    answer
}

/// The contact name for a number (Contacts' vCard file), else the number written NANP style.
fn name_of(number: &str) -> String {
    if let Ok(t) = std::fs::read_to_string("/home/.contacts/contacts.vcf") {
        if let Ok(cs) = vcard::parse(&t) {
            let digits = |s: &str| s.chars().filter(|c| c.is_ascii_digit()).collect::<String>();
            let d = digits(number);
            for c in cs {
                if c.tels.iter().any(|x| {
                    let e = digits(&x.number);
                    !e.is_empty() && (e == d || d.ends_with(&e) || e.ends_with(&d))
                }) {
                    return c.name;
                }
            }
        }
    }
    ecm_tel::number::E164::parse(number).map(|n| n.display("1")).unwrap_or_else(|| number.to_string())
}

struct Client {
    fd: i32,
    lines: Lines,
    watch: bool,
    /// Connected on the texts-only port (ctl::SMS_PORT).
    sms_only: bool,
}

struct Audio {
    call: Option<ecm_audio::Device>,
    ring: Option<ecm_audio::Device>,
    mic: Option<ecm_audio::Mic>,
    speaker_on: bool,
    /// A WAV playing to the far end instead of the microphone: (call, samples).
    play: Option<(u32, Vec<i16>)>,
    /// Recording the far end: (call, until ms, path, samples).
    record: Option<(u32, u64, String, Vec<i16>)>,
}

pub fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let quiet = args.iter().any(|a| a == "--quiet");
    let phone = args.iter().any(|a| a == "--phone");
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("usage: telephonyd [--phone] [--no-mms] [--quiet]");
        println!("  the phone's IMS client (VoLTE calls, SMS, MMS): needs celld attached with an IMS bearer.");
        println!("  Talk to it with tel, sms, the Phone / Messages apps or the phone module (127.0.0.1:7720).");
        return;
    }
    let c = sys::load_conf(PATH, KEYS).unwrap_or_else(|e| fail(e));
    let g = |k: &str| c.get(k).cloned().unwrap_or_default();
    let mut log = Log::new("telephonyd", !quiet);
    let listener = ecm_cell_net::tcp_listen(ecm_cell_net::LOOPBACK, ctl::PORT).unwrap_or_else(|e| fail(format!("control socket: {e} (is telephonyd already running?)")));
    let sms_listener = ecm_cell_net::tcp_listen(ecm_cell_net::LOOPBACK, ctl::SMS_PORT).unwrap_or_else(|e| fail(format!("texts socket: {e}")));
    let sip = sys::udp_bind(ecm_tel::ua::SIP_PORT).unwrap_or_else(|e| fail(e));
    // a text or a call for a sleeping phone wakes it (Background apps' wake sockets)
    socket::set_wake(sip, true);
    let Some(simp) = sim_name(&g("sim")) else { fail("no SIM peripheral (a phone's SIM tray or a modem's slot)") };
    let imsi = peripheral::call(&simp, "identity", &[]).ok().and_then(|v| get_str(&v, "imsi")).unwrap_or_default();
    if imsi.is_empty() {
        fail("no SIM card");
    }
    let mut cfg = UaConfig::new(&imsi);
    cfg.mms = g("mms") != "off" && !args.iter().any(|a| a == "--no-mms");
    cfg.wideband = g("wideband") != "false";
    let seed = u64::from_le_bytes(ecm_cell_net::random::<8>());
    let mut ua = Ua::new(cfg.clone(), seed);
    let _ = std::fs::create_dir_all(store::DIR);
    let mut st = Store::load(&std::fs::read_to_string(store::MESSAGES).unwrap_or_default(), &std::fs::read_to_string(store::CALLS).unwrap_or_default());
    log.line(&format!("telephonyd: SIM {imsi}, {} messages, {} calls in the log", st.msgs.len(), st.calls.len()));
    let mut clients: Vec<Client> = Vec::new();
    let mut rtp: BTreeMap<u16, i32> = BTreeMap::new();
    let mut audio = Audio { call: None, ring: None, mic: None, speaker_on: false, play: None, record: None };
    let clock = Instant::now();
    let ms = || clock.elapsed().as_millis() as u64;
    let mut last_celld = 0u64;
    let mut last_frame = ms();
    let mut volte_reported: Option<(bool, String)> = None;
    let mut wakelock_at = 0u64;
    let mut voicemail_waiting = 0u32;
    let mut mms_jobs: Vec<(String, String, String)> = Vec::new();
    // incoming calls being rung: id -> (number, start)
    let mut ringing: BTreeMap<u32, (String, u64, bool)> = BTreeMap::new();
    let mut started: BTreeMap<u32, (u64, bool, String)> = BTreeMap::new();
    // our outgoing SMS ids -> store ids
    let mut sms_ids: BTreeMap<u32, u64> = BTreeMap::new();
    let mut buf = vec![0u8; 65536];
    loop {
        let now = ms();
        // the IMS bearer, from celld
        if now.saturating_sub(last_celld) >= 1000 {
            last_celld = now;
            match celld::host::query("status", 300) {
                Ok(t) => {
                    let s = celld::ctl::Status::parse(&t);
                    let ip = s.ims_ip.split('/').next().and_then(Addr::parse_ip);
                    match (ip, Addr::parse_ip(&s.pcscf)) {
                        (Some(ip), Some(p)) if s.on_cell() => ua.bearer_up(Bearer { ip, pcscf: Addr::new(p, 5060), domain: s.ims_domain.clone(), msisdn: s.msisdn.clone() }, now),
                        _ => {
                            if ua.bearer().is_some() && !s.on_cell() {
                                ua.bearer_down(now);
                            }
                        }
                    }
                }
                Err(_) => {
                    if ua.bearer().is_some() {
                        ua.bearer_down(now);
                    }
                }
            }
        }
        let mut fds = vec![sip, listener, sms_listener];
        fds.extend(clients.iter().map(|c| c.fd));
        let ports: Vec<u16> = rtp.keys().copied().collect();
        fds.extend(ports.iter().map(|p| rtp[p]));
        let ready = sys::readable(&fds, 5);
        let now = ms();
        if ready[0] {
            for (from, d) in sys::udp_drain(sip, &mut buf, 32) {
                ua.on_sip(from, &d, now);
            }
        }
        for (i, p) in ports.iter().enumerate() {
            if ready[3 + clients.len() + i] {
                for (from, d) in sys::udp_drain(rtp[p], &mut buf, 32) {
                    ua.on_rtp(*p, from, &d, now);
                }
            }
        }
        for (k, l) in [(1usize, listener), (2, sms_listener)] {
            if ready[k] {
                let mut a = socket::SockAddrIn::default();
                let fd = socket::accept(l, &mut a);
                if fd >= 0 {
                    socket::set_wake(fd, true);
                    clients.push(Client { fd, lines: Lines::default(), watch: false, sms_only: k == 2 });
                }
            }
        }
        // control requests
        let mut gone = Vec::new();
        let mut requests: Vec<(i32, String)> = Vec::new();
        for (i, cl) in clients.iter_mut().enumerate() {
            if !ready[3 + i] {
                continue;
            }
            match sys::read_now(cl.fd, &mut buf) {
                Some(d) if d.is_empty() => {}
                Some(d) => match cl.lines.push(&d) {
                    Ok(ls) => {
                        for l in ls {
                            if cl.sms_only && !ctl::sms_port_allows(&l) {
                                sys::send_all(cl.fd, reply(&Err("the texts port takes only messages (calls need the phone port, 7720)".to_string())).as_bytes());
                                continue;
                            }
                            if l.trim() == "watch" {
                                cl.watch = true;
                                sys::send_all(cl.fd, reply(&Ok(String::new())).as_bytes());
                            } else {
                                requests.push((cl.fd, l));
                            }
                        }
                    }
                    Err(_) => gone.push(cl.fd),
                },
                None => gone.push(cl.fd),
            }
        }
        for (fd, l) in requests {
            let r = handle(&l, &mut ua, &mut st, &mut audio, &mut sms_ids, voicemail_waiting, now, &mut log);
            if !sys::send_all(fd, reply(&r).as_bytes()) {
                gone.push(fd);
            }
        }
        for fd in gone {
            socket::close(fd);
            clients.retain(|c| c.fd != fd);
        }
        ua.tick(now);
        // media every 20 ms: microphone to the call, the call (and tones) to the speaker, the ringtone
        if now.saturating_sub(last_frame) >= 20 {
            last_frame += 20;
            if now.saturating_sub(last_frame) > 200 {
                last_frame = now;
            }
            media(&mut ua, &mut audio, &g, now, &mut log);
        }
        let mut events: Vec<Event> = Vec::new();
        for o in ua.take_out() {
            match o {
                UaOut::Sip { to, data } => sys::udp_send(sip, to, &data),
                UaOut::Rtp { port, to, data } => {
                    if let Some(fd) = rtp.get(&port) {
                        sys::udp_send(*fd, to, &data);
                    }
                }
                UaOut::OpenRtp(p) => match sys::udp_bind(p) {
                    Ok(fd) => {
                        socket::set_wake(fd, true);
                        rtp.insert(p, fd);
                    }
                    Err(e) => log.line(&format!("RTP port {p}: {e}")),
                },
                UaOut::CloseRtp(p) => {
                    if let Some(fd) = rtp.remove(&p) {
                        socket::close(fd);
                    }
                }
                UaOut::Aka { rand } => {
                    let r = ims_aka(&simp, &rand);
                    if r.is_none() {
                        log.line("the SIM didn't answer the IMS challenge (respond_ims)");
                    }
                    ua.on_aka(r, now);
                }
                UaOut::Log(l) => log.line(&l),
                UaOut::Event(e) => on_event(e, &mut ua, &mut st, &mut events, &mut ringing, &mut started, &mut sms_ids, &mut mms_jobs, &mut voicemail_waiting, now, phone, &mut log),
            }
        }
        // MMS fetches (over the data bearer, HTTP)
        for (location, tid, from) in std::mem::take(&mut mms_jobs) {
            match sys::http_request("GET", &location, None, &[], Duration::from_secs(20)) {
                Ok(r) if r.status == 200 => match mmsua::retrieved(&r.body) {
                    Ok(m) => {
                        let me = ua.msisdn().to_string();
                        let others: Vec<String> = std::iter::once(m.from.clone()).chain(m.to.iter().cloned()).collect();
                        let k = key(&me, &others);
                        let mut files = Vec::new();
                        let nid = st.msgs.len() as u64 + 1;
                        let dir = format!("{}/{}-{}", store::PARTS, now, nid);
                        for (_ct, name, data) in &m.files {
                            let safe: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '_' }).collect();
                            let p = format!("{dir}/{safe}");
                            if sys::write_file(&p, data).is_ok() {
                                files.push(p);
                            }
                        }
                        let id = st.add(&k, MsgLine { incoming: true, from: m.from.clone(), time_ms: now, status: "received".into(), mms: true, text: m.text.clone().unwrap_or_default(), files: files.clone(), ..Default::default() });
                        let _ = sys::http_request("POST", &ua.mmsc_url.clone(), Some(ecm_tel::mms::MMS_CONTENT_TYPE), &mmsua::notify_resp(&tid, true).encode(), Duration::from_secs(10));
                        let who = name_of(&m.from);
                        let preview = m.text.clone().filter(|t| !t.is_empty()).unwrap_or_else(|| if files.iter().any(|f| f.ends_with(".wav")) { "Voice note".into() } else { "Picture".into() });
                        notify("dev.ecm.messages", &format!("sms:{k}"), &who, &preview);
                        events.push(Event::new("mms", &[("id", &id.to_string()), ("from", &m.from), ("thread", &k), ("text", &m.text.unwrap_or_default()), ("files", &files.join(","))]));
                        log.line(&format!("MMS from {} ({} file(s)) saved", m.from, files.len()));
                    }
                    Err(e) => log.line(&format!("MMS from {from}: {e}")),
                },
                Ok(r) => log.line(&format!("MMS fetch {location}: HTTP {}", r.status)),
                Err(e) => log.line(&format!("MMS fetch {location}: {e}")),
            }
        }
        // the store
        if !st.pending_msgs.is_empty() {
            append(store::MESSAGES, &std::mem::take(&mut st.pending_msgs));
        }
        if !st.pending_calls.is_empty() {
            append(store::CALLS, &std::mem::take(&mut st.pending_calls));
        }
        // events to watchers
        if !events.is_empty() {
            let mut dead = Vec::new();
            for cl in clients.iter().filter(|c| c.watch) {
                for e in events.iter().filter(|e| !cl.sms_only || ctl::sms_event(&e.name)) {
                    if !sys::send_all(cl.fd, format!("{}\n", e.line()).as_bytes()) {
                        dead.push(cl.fd);
                        break;
                    }
                }
            }
            for fd in dead {
                socket::close(fd);
                clients.retain(|c| c.fd != fd);
            }
        }
        // the status bar: VoLTE and the number
        let volte = (ua.registration() == RegState::Registered, ua.msisdn().to_string());
        if phone && volte_reported.as_ref() != Some(&volte) {
            uictl(&format!("volte {} {}", volte.0 as u8, if volte.1.is_empty() { "-" } else { &volte.1 }));
            volte_reported = Some(volte);
        }
        // a ringing or live call keeps a sleeping phone's computer awake (the platform's wake
        // lock: each renewal lasts at most 10 s, so renew every 5 s; release when the calls end)
        let in_call = ua.calls().iter().any(|c| c.state != CallState::Ended);
        if phone && in_call && now.saturating_sub(wakelock_at) >= 5000 {
            wakelock_at = now;
            uictl("wakelock 10000");
        } else if phone && !in_call && wakelock_at != 0 {
            wakelock_at = 0;
            uictl("wakelock 0");
        }
    }
}

fn append(path: &str, lines: &[String]) {
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        for l in lines {
            let _ = writeln!(f, "{}", l);
        }
    }
}

fn notify(app: &str, tag: &str, title: &str, body: &str) {
    uictl(&format!("notify {} {} {} {}", app, enc(tag), enc(title), enc(body)));
}

fn media(ua: &mut Ua, audio: &mut Audio, g: &dyn Fn(&str) -> String, now: u64, log: &mut Log) {
    let live = ua.calls().iter().any(|c| matches!(c.state, CallState::Active | CallState::Alerting | CallState::Dialing) || c.state == CallState::Ended);
    if live {
        if audio.call.is_none() {
            let side = g("speaker");
            match ecm_audio::Device::open(if side.is_empty() { None } else { Some(side.as_str()) }) {
                Ok(mut d) => {
                    let _ = d.set_format(16_000, 16, 1);
                    let _ = d.set_role(ecm_audio::Role::Phone);
                    let _ = d.set_latency_ms(80);
                    let _ = d.set_volume(if audio.speaker_on { 100 } else { 45 });
                    audio.call = Some(d);
                }
                Err(e) => log.line(&format!("no speaker for the call: {e}")),
            }
        }
        if audio.mic.is_none() {
            let side = g("mic");
            match ecm_audio::Mic::open(if side.is_empty() { None } else { Some(side.as_str()) }) {
                Ok(mut m) => {
                    let _ = m.set_format(16_000, 16, 1);
                    let _ = m.flush_queue();
                    audio.mic = Some(m);
                }
                Err(e) => log.line(&format!("no microphone for the call: {e}")),
            }
        }
        // what the far end hears: a file being played, or the microphone
        let mut frame: Vec<i16> = Vec::new();
        if let Some((_, samples)) = audio.play.as_mut() {
            let n = 320.min(samples.len());
            frame = samples.drain(..n).collect();
            if samples.is_empty() {
                audio.play = None;
            }
            if let Some(m) = audio.mic.as_mut() {
                let _ = m.flush_queue();
            }
        } else if let Some(m) = audio.mic.as_mut() {
            if m.stat("buffered_ms").ok().flatten().unwrap_or(0) >= 20 {
                frame = m.read_samples(320).unwrap_or_default();
            }
        }
        if frame.is_empty() {
            frame = vec![0; 320];
        }
        frame.resize(320, 0);
        ua.mic(&frame, now);
        let out = ua.speaker(320);
        if let Some((_, until, path, rec)) = audio.record.as_mut() {
            rec.extend_from_slice(&out);
            if now >= *until {
                let _ = sys::write_file(path, &ecm_rtp::wav::write(16_000, rec));
                log.line(&format!("recorded {} ms to {}", rec.len() / 16, path));
                audio.record = None;
            }
        }
        if let Some(d) = audio.call.as_mut() {
            let _ = d.write_samples(&out);
        }
    } else {
        audio.call = None;
        audio.mic = None;
        audio.play = None;
    }
    match ua.ring(320) {
        Some(r) if g("ringtone") != "off" => {
            if audio.ring.is_none() {
                if let Ok(mut d) = ecm_audio::Device::open(None) {
                    let _ = d.set_format(16_000, 16, 1);
                    let _ = d.set_role(ecm_audio::Role::Phone);
                    let _ = d.set_latency_ms(80);
                    let _ = d.set_volume(100);
                    audio.ring = Some(d);
                }
            }
            if let Some(d) = audio.ring.as_mut() {
                let _ = d.write_samples(&r);
            }
        }
        _ => audio.ring = None,
    }
}

#[allow(clippy::too_many_arguments)]
fn on_event(
    e: UaEvent,
    ua: &mut Ua,
    st: &mut Store,
    events: &mut Vec<Event>,
    ringing: &mut BTreeMap<u32, (String, u64, bool)>,
    started: &mut BTreeMap<u32, (u64, bool, String)>,
    sms_ids: &mut BTreeMap<u32, u64>,
    mms_jobs: &mut Vec<(String, String, String)>,
    vm: &mut u32,
    now: u64,
    phone: bool,
    log: &mut Log,
) {
    match e {
        UaEvent::Registration { state, msisdn, why } => {
            log.line(&format!("registration {}{}", state.name(), if why.is_empty() { String::new() } else { format!(": {why}") }));
            events.push(Event::new("registration", &[("state", state.name()), ("msisdn", &msisdn)]));
        }
        UaEvent::Incoming { call, from, hidden, waiting } => {
            let who = if hidden { "No Caller ID".to_string() } else { name_of(&from) };
            ringing.insert(call, (from.clone(), now, false));
            started.insert(call, (now, false, from.clone()));
            if phone {
                uictl(&format!("ring 1 {}", enc(&who)));
                // the Phone app shows the incoming call (full screen)
                let _ = ecm_host_abi::apps::ctl(&["launch", "dev.ecm.dialer", "--incoming", &call.to_string()]);
            }
            events.push(Event::new("incoming", &[("call", &call.to_string()), ("from", &from), ("hidden", if hidden { "1" } else { "0" }), ("waiting", if waiting { "1" } else { "0" })]));
        }
        UaEvent::CallState { call, state, reason } => {
            if state == CallState::Active {
                if let Some(r) = ringing.get_mut(&call) {
                    r.2 = true;
                }
                if let Some(s) = started.get_mut(&call) {
                    s.1 = true;
                    s.0 = now;
                }
            }
            if matches!(state, CallState::Dialing) {
                let number = ua.calls().iter().find(|c| c.id == call).map(|c| c.remote.clone()).unwrap_or_default();
                started.insert(call, (now, false, number));
            }
            if state == CallState::Ended {
                let was_incoming = ringing.remove(&call);
                let (t0, answered, number) = started.remove(&call).unwrap_or((now, false, String::new()));
                let secs = if answered { now.saturating_sub(t0) / 1000 } else { 0 };
                match was_incoming {
                    Some((from, _, answered)) if !answered => {
                        st.log_call("missed", &from, t0, 0, &reason);
                        if phone {
                            notify("dev.ecm.dialer", "call:missed", "Missed call", &name_of(&from));
                        }
                    }
                    Some((from, _, _)) => st.log_call("in", &from, t0, secs, &reason),
                    None => st.log_call("out", &number, t0, secs, &reason),
                }
                if phone && ringing.is_empty() {
                    uictl("ring 0 -");
                }
            }
            events.push(Event::new("call", &[("call", &call.to_string()), ("state", state.name()), ("reason", &reason)]));
        }
        UaEvent::Dtmf { call, digit } => events.push(Event::new("dtmf", &[("call", &call.to_string()), ("digit", &digit.to_string())])),
        UaEvent::SmsSent { id } => {
            if let Some(m) = sms_ids.get(&id) {
                st.set_status(*m, "sent");
                events.push(Event::new("sent", &[("id", &m.to_string())]));
            }
        }
        UaEvent::SmsDelivered { id } => {
            if let Some(m) = sms_ids.get(&id) {
                st.set_status(*m, "delivered");
                events.push(Event::new("delivered", &[("id", &m.to_string())]));
            }
        }
        UaEvent::SmsFailed { id, why } => {
            if let Some(m) = sms_ids.get(&id) {
                st.set_status(*m, &format!("failed:{why}"));
                if phone {
                    notify("dev.ecm.messages", "sms:failed", "Message not sent", &why);
                }
                events.push(Event::new("failed", &[("id", &m.to_string()), ("why", &why)]));
            }
        }
        UaEvent::Sms { from, text, timestamp_ms: _ } => {
            let me = ua.msisdn().to_string();
            let k = key(&me, &[from.clone()]);
            let id = st.add(&k, MsgLine { incoming: true, from: from.clone(), time_ms: now, status: "received".into(), text: text.clone(), ..Default::default() });
            if phone {
                notify("dev.ecm.messages", &format!("sms:{k}"), &name_of(&from), &text);
                uictl("chime message");
            }
            events.push(Event::new("sms", &[("id", &id.to_string()), ("from", &from), ("thread", &k), ("text", &text)]));
        }
        UaEvent::MmsNotice { from, location, transaction, size, subject: _ } => {
            log.line(&format!("MMS waiting from {from} ({size} bytes)"));
            mms_jobs.push((location, transaction, from));
        }
        UaEvent::MmsDelivered { message_id, to, status } => {
            if let Some(m) = st.by_network(&format!("mms:{message_id}")) {
                st.set_status(m, if status == "delivered" { "delivered" } else { "failed:not delivered" });
                events.push(Event::new("delivered", &[("id", &m.to_string()), ("to", &to)]));
            }
        }
        UaEvent::BinarySms { from, port, .. } => log.line(&format!("binary text from {from} to port {port} (ignored)")),
        UaEvent::Voicemail { waiting } => {
            if waiting > *vm && phone {
                notify("dev.ecm.dialer", "vm", "New voicemail", &format!("{waiting} waiting"));
            }
            *vm = waiting;
            events.push(Event::new("voicemail", &[("waiting", &waiting.to_string())]));
        }
    }
}

/// One control request.
#[allow(clippy::too_many_arguments)]
fn handle(line: &str, ua: &mut Ua, st: &mut Store, audio: &mut Audio, sms_ids: &mut BTreeMap<u32, u64>, vm: u32, now: u64, log: &mut Log) -> Result<String, String> {
    let w: Vec<&str> = line.split_whitespace().collect();
    let id = |s: &str| s.parse::<u32>().map_err(|_| format!("bad call id '{s}'"));
    let cc = ua.cfg.cc.clone();
    let npa = ua.home_npa();
    let number = |s: &str| -> Result<String, String> {
        let s = dec(s);
        if s.starts_with('*') || s == "611" || (s.len() <= 6 && s.bytes().all(|b| b.is_ascii_digit())) {
            return Ok(s);
        }
        ecm_tel::number::normalize(&s, &cc, npa.as_deref()).map(|n| n.to_string()).ok_or_else(|| format!("'{s}' is not a phone number"))
    };
    match w.as_slice() {
        ["status"] => {
            let calls = ua.calls();
            Ok(format!(
                "registration {}\nmsisdn {}\ndomain {}\nvolte {}\nhd {}\nvoicemail {}\nunread {}\nmissed {}\ncalls {}\nmmsc {}\nspeaker {}\nnow {}\n",
                ua.registration().name(),
                if ua.msisdn().is_empty() { "-" } else { ua.msisdn() },
                ua.bearer().map(|b| b.domain.clone()).filter(|d| !d.is_empty()).unwrap_or_else(|| "-".into()),
                (ua.registration() == RegState::Registered) as u8,
                calls.iter().any(|c| c.hd) as u8,
                vm,
                st.unread(),
                st.missed(),
                calls.len(),
                if ua.mmsc_url.is_empty() { "-" } else { &ua.mmsc_url },
                audio.speaker_on as u8,
                now
            ))
        }
        ["calls"] => Ok(ua
            .calls()
            .iter()
            .map(|c| {
                CallLine {
                    id: c.id,
                    state: c.state.name().into(),
                    number: c.remote.clone(),
                    hidden: c.hidden,
                    outgoing: c.outgoing,
                    seconds: c.answered_ms.map(|a| now.saturating_sub(a) / 1000).unwrap_or(0),
                    muted: c.muted,
                    hd: c.hd,
                    codec: c.codec.clone(),
                    loss: c.loss,
                    merged: c.merged,
                }
                .line()
            })
            .collect::<Vec<_>>()
            .join("\n")),
        ["dial", n] | ["dial", n, "hide"] => {
            let hide = w.len() == 3;
            let n = number(n)?;
            let (n, hide) = match n.strip_prefix("*67") {
                Some(r) => (number(r)?, true),
                None => (n, hide),
            };
            ua.dial(&n, hide, now).map(|i| format!("call {i}"))
        }
        ["answer", i] => ua.answer(id(i)?, now).map(|_| String::new()),
        ["hangup", i] | ["decline", i] => ua.hangup(id(i)?, now).map(|_| String::new()),
        ["hold", i, v] => ua.hold(id(i)?, *v == "on", now).map(|_| String::new()),
        ["swap"] => ua.swap(now).map(|_| String::new()),
        ["merge"] => ua.merge(now).map(|_| String::new()),
        ["mute", i, v] => ua.mute(id(i)?, *v == "on").map(|_| String::new()),
        ["dtmf", i, d] => ua.send_dtmf(id(i)?, d, now).map(|_| String::new()),
        ["speaker", v] => {
            audio.speaker_on = *v == "on";
            if let Some(d) = audio.call.as_mut() {
                let _ = d.set_volume(if audio.speaker_on { 100 } else { 45 });
            }
            Ok(String::new())
        }
        ["play", i, path] => {
            let call = id(i)?;
            let path = telephonyd::user_path(&dec(path))?;
            let data = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
            let w = ecm_rtp::wav::read_16k(&data).ok_or("not a WAV file")?;
            audio.play = Some((call, w));
            Ok(String::new())
        }
        ["record", i, secs, path] => {
            let call = id(i)?;
            let s: u64 = secs.parse().map_err(|_| "seconds")?;
            let path = telephonyd::user_path(&dec(path))?;
            audio.record = Some((call, now + s.clamp(1, 300) * 1000, path, Vec::new()));
            Ok(String::new())
        }
        ["sms", to, text] => {
            let to = number(to)?;
            let text = dec(text);
            let sid = ua.send_sms(&to, &text, now)?;
            let me = ua.msisdn().to_string();
            let k = key(&me, &[to.clone()]);
            let mid = st.add(&k, MsgLine { incoming: false, from: me, time_ms: now, status: "sending".into(), text, ..Default::default() });
            st.link(mid, &format!("sms:{sid}"));
            sms_ids.insert(sid, mid);
            Ok(format!("sms {mid}"))
        }
        ["mms", to, text, files] => {
            let to: Vec<String> = to.split(',').map(number).collect::<Result<_, _>>()?;
            let text = dec(text);
            let mut fv = Vec::new();
            if *files != "-" {
                for f in files.split(',').map(dec) {
                    let f = telephonyd::user_path(&f)?;
                    let data = std::fs::read(&f).map_err(|e| format!("{f}: {e}"))?;
                    let ct = if f.ends_with(".png") {
                        "image/png"
                    } else if f.ends_with(".wav") {
                        "audio/wav"
                    } else if f.ends_with(".vcf") {
                        "text/x-vcard"
                    } else {
                        "application/octet-stream"
                    };
                    let name = f.rsplit('/').next().unwrap_or("file").to_string();
                    fv.push((ct.to_string(), name, data));
                }
            }
            if ua.mmsc_url.is_empty() {
                return Err("the network gave no MMSC (not registered?)".into());
            }
            let o = mmsua::Outgoing { to: to.clone(), subject: None, text: if text.is_empty() { None } else { Some(text.clone()) }, files: fv };
            let pdu = mmsua::send_req(&format!("t{now}"), &o).encode();
            let me = ua.msisdn().to_string();
            let k = key(&me, &to);
            let mid = st.add(&k, MsgLine { incoming: false, from: me, time_ms: now, status: "sending".into(), mms: true, text, files: if *files == "-" { vec![] } else { files.split(',').map(dec).collect() }, ..Default::default() });
            let url = ua.mmsc_url.clone();
            match sys::http_request("POST", &url, Some(ecm_tel::mms::MMS_CONTENT_TYPE), &pdu, Duration::from_secs(30)) {
                Ok(r) if r.status == 200 => match mmsua::send_conf(&r.body) {
                    Ok(net) => {
                        st.set_status(mid, "sent");
                        st.link(mid, &format!("mms:{net}"));
                        log.line(&format!("MMS {mid} sent to {}", to.join(",")));
                        Ok(format!("mms {mid}"))
                    }
                    Err(e) => {
                        st.set_status(mid, &format!("failed:{e}"));
                        Err(e)
                    }
                },
                Ok(r) => {
                    st.set_status(mid, &format!("failed:HTTP {}", r.status));
                    Err(format!("the MMSC answered HTTP {}", r.status))
                }
                Err(e) => {
                    st.set_status(mid, &format!("failed:{e}"));
                    Err(e)
                }
            }
        }
        ["threads"] => Ok(st.threads().iter().map(|t| t.line()).collect::<Vec<_>>().join("\n")),
        ["messages", k] | ["messages", k, _] => {
            let n = w.get(2).and_then(|x| x.parse().ok()).unwrap_or(200);
            Ok(st.messages(k, n).iter().map(|m| m.line()).collect::<Vec<_>>().join("\n"))
        }
        ["read", k] => Ok(format!("read {}", st.mark_read(k))),
        ["delete", i] => {
            if st.delete(i.parse().map_err(|_| "bad id")?) {
                Ok(String::new())
            } else {
                Err("no such message".into())
            }
        }
        ["delete-thread", k] => {
            st.delete_thread(k);
            Ok(String::new())
        }
        ["log"] | ["log", _] => {
            let n = w.get(1).and_then(|x| x.parse().ok()).unwrap_or(100);
            let skip = st.calls.len().saturating_sub(n);
            Ok(st.calls.iter().skip(skip).map(|c| c.line()).collect::<Vec<_>>().join("\n"))
        }
        ["voicemail"] => {
            let r = sys::http_request("GET", &ua.vvm_url.clone(), None, &[], Duration::from_secs(10))?;
            if r.status != 200 {
                return Err(format!("visual voicemail: HTTP {}", r.status));
            }
            Ok(String::from_utf8_lossy(&r.body).lines().map(|l| l.replacen("msg ", "vm ", 1)).collect::<Vec<_>>().join("\n"))
        }
        ["voicemail-fetch", i, path] => {
            let r = sys::http_request("GET", &format!("{}/{}.wav", ua.vvm_url, i), None, &[], Duration::from_secs(20))?;
            if r.status != 200 {
                return Err(format!("visual voicemail: HTTP {}", r.status));
            }
            sys::write_file(&telephonyd::user_path(&dec(path))?, &r.body)?;
            Ok(String::new())
        }
        ["voicemail-heard", i] => sys::http_request("POST", &format!("{}/{}/heard", ua.vvm_url, i), None, &[], Duration::from_secs(10)).map(|_| String::new()),
        ["voicemail-delete", i] => sys::http_request("DELETE", &format!("{}/{}", ua.vvm_url, i), None, &[], Duration::from_secs(10)).map(|_| String::new()),
        ["forward", "all", n] => {
            let n = number(n)?;
            ua.dial(&format!("*72{n}"), false, now).map(|_| String::new())
        }
        ["forward", "off"] => ua.dial("*73", false, now).map(|_| String::new()),
        _ => Err(format!("unknown request '{}'", line.trim())),
    }
}
