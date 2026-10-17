//! `cardtool` — cards in the readers, NFC readers and printers next to this computer
//! (docs/cards/CARDS.md). It replaces `simtool`: `cardtool sim …` keeps its verbs and output.
//!
//!   cardtool [info]                                   the card in the Card Reader/Writer
//!   cardtool stripe read                              the three stripe tracks
//!   cardtool stripe write <t1> <t2> <t3>              write them ("" for an empty track)
//!   cardtool bank write <issuer> <pan> [--key <hex>|--random] [--pin <digits>] [--pin-required]
//!                                                     write a blank bank chip; prints the key once
//!   cardtool bank pin <pin>                           open a PIN session on the card
//!   cardtool bank sign <nonce-hex> [amount]           {ctr, tag} for an 8-byte nonce
//!   cardtool nfc poll                                 UIDs in the NFC reader's field
//!   cardtool nfc select <uid-hex>                     address one card
//!   cardtool nfc auth <sector> <key-hex>              open a sector session (16-byte key)
//!   cardtool nfc read <sector>                        64 bytes of the sector (hex)
//!   cardtool nfc write <sector> <hex>                 write them
//!   cardtool nfc setkey <sector> <key-hex>            rekey the sector
//!   cardtool nfc field on|off | raw <hex> | rssi      the field, a raw frame, the last RSSI
//!   cardtool print <file.card>                        print a design on the card in the printer
//!   cardtool cost <file.card>                         the dyes it would take
//!   cardtool eject                                    done with the card (Auto Eject pushes it out)
//!   cardtool wait [seconds]                           wait for the next card_inserted / card_removed
//!   cardtool sim [info|write|auth|eject|wait] …       the SIM verbs, exactly as simtool had them
//!
//! `--reader <name>`, `--nfc <name>` and `--printer <name>` pick a device when there are
//! several (default: the first of each type). The `nfc` verbs use an NFC Reader when one is
//! attached and otherwise the Card Reader/Writer's contact fallback (`nfc_*` methods).
//! Bytes are hex strings. A bank key is 32 bytes (64 hex digits), a SIM key 16; `--random`
//! (the default) draws it from the computer's random source and prints it once: the card
//! never gives it back.
//!
//! This is also the Rust example for the card peripherals: everything below is
//! `peripheral::call(name, method, args)`; the schemes themselves live in `ecm-cards`.

use ecm_cards::design::{face_size, Design};
use ecm_cards::{bank, hex};
use ecm_host_abi::peripheral::{self, Error, Value};

const READER: &str = "card_reader";
const NFC: &str = "nfc_reader";
const PRINTER: &str = "card_printer";

fn describe(e: Error) -> String {
    match e {
        Error::Peripheral(msg) => msg,
        Error::Unavailable => "peripherals are not available".to_string(),
        Error::Malformed(what) => format!("bad reply from the computer: {what}"),
    }
}

fn get<'a>(m: &'a Value, key: &str) -> Option<&'a Value> {
    match m {
        Value::Map(entries) => entries.iter().find(|(k, _)| matches!(k, Value::Str(s) if s == key)).map(|(_, v)| v),
        _ => None,
    }
}

fn text(m: &Value, key: &str) -> String {
    match get(m, key) {
        Some(Value::Str(s)) => s.clone(),
        Some(Value::Int(i)) => i.to_string(),
        Some(Value::Float(f)) => f.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Bytes(b)) => hex::encode(b),
        _ => "-".to_string(),
    }
}

fn flag(m: &Value, key: &str) -> bool {
    matches!(get(m, key), Some(Value::Bool(true)))
}

fn int(m: &Value, key: &str) -> Option<i64> {
    match get(m, key) {
        Some(Value::Int(i)) => Some(*i),
        _ => None,
    }
}

fn str_list(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::List(items)) => items
            .iter()
            .map(|i| match i {
                Value::Str(s) => s.clone(),
                Value::Int(n) => n.to_string(),
                other => value_text(other),
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// A value as one line of text (hex for bytes, `k=v` pairs for maps).
fn value_text(v: &Value) -> String {
    match v {
        Value::Nil => "nil".into(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Str(s) => s.clone(),
        Value::Bytes(b) => hex::encode(b),
        Value::List(items) => format!("[{}]", items.iter().map(value_text).collect::<Vec<_>>().join(", ")),
        Value::Map(entries) => entries.iter().map(|(k, v)| format!("{}={}", value_text(k), value_text(v))).collect::<Vec<_>>().join(" "),
    }
}

fn s(v: impl Into<String>) -> Value {
    Value::Str(v.into())
}

fn fail(msg: impl AsRef<str>) -> ! {
    eprintln!("cardtool: {}", msg.as_ref());
    std::process::exit(1);
}

fn usage() -> ! {
    eprintln!("usage: cardtool [--reader <name>] [--nfc <name>] [--printer <name>] <verb> ...");
    eprintln!("  info");
    eprintln!("  stripe read | stripe write <t1> <t2> <t3>");
    eprintln!("  bank write <issuer> <pan> [--key <hex>|--random] [--pin <digits>] [--pin-required]");
    eprintln!("  bank pin <pin> | bank sign <nonce-hex> [amount]");
    eprintln!("  nfc poll | select <uid> | auth <sector> <key-hex> | read <sector> | write <sector> <hex> | setkey <sector> <key-hex>");
    eprintln!("  nfc field on|off | raw <hex> | rssi");
    eprintln!("  print <file.card> | cost <file.card>");
    eprintln!("  eject | wait [seconds]");
    eprintln!("  sim [info] | sim write <imsi> <carrier> [--key <hex>|--random] | sim auth <challenge-hex> | sim eject | sim wait [seconds]");
    std::process::exit(2);
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|c| c.is_ascii_hexdigit())
}

fn random_key(n: usize) -> String {
    let mut k = vec![0u8; n];
    // WASI random_get (the host's SecureRandom-backed entropy).
    if getrandom::getrandom(&mut k).is_err() || k.iter().all(|&b| b == 0) {
        fail("no random source");
    }
    hex::encode(&k)
}

fn call(name: &str, method: &str, args: &[Value]) -> Value {
    peripheral::call(name, method, args).unwrap_or_else(|e| fail(describe(e)))
}

// ---------------------------------------------------------------- devices

struct Devices {
    reader: Option<String>,
    nfc: Option<String>,
    printer: Option<String>,
}

fn of_type(ty: &str) -> Vec<String> {
    let list = peripheral::list().unwrap_or_else(|e| fail(describe(e)));
    list.into_iter().filter(|(_, t)| t == ty).map(|(n, _)| n).collect()
}

fn pick(ty: &str, wanted: Option<&str>, what: &str, hint: &str) -> Option<String> {
    let names = of_type(ty);
    match wanted {
        Some(w) if names.iter().any(|n| n == w) => Some(w.to_string()),
        Some(w) => fail(format!("no {what} called '{w}' (have: {})", if names.is_empty() { "none".into() } else { names.join(", ") })),
        None => {
            let first = names.into_iter().next();
            if first.is_none() {
                eprintln!("cardtool: no {what} next to this computer.");
                eprintln!("{hint}");
            }
            first
        }
    }
}

impl Devices {
    fn reader(&self) -> String {
        pick(READER, self.reader.as_deref(), "Card Reader/Writer", "Place a Card Reader/Writer against any side of the computer, or fit a Card Reader Module in a bay.")
            .unwrap_or_else(|| std::process::exit(1))
    }
    fn nfc(&self) -> Option<String> {
        if self.nfc.is_some() {
            return pick(NFC, self.nfc.as_deref(), "NFC Reader", "");
        }
        of_type(NFC).into_iter().next()
    }
    fn printer(&self) -> String {
        pick(PRINTER, self.printer.as_deref(), "Card Printer", "Place a Card Printer against any side of the computer.").unwrap_or_else(|| std::process::exit(1))
    }
}

// ---------------------------------------------------------------- info

/// The applet maps of an `identity()` result: the new shape nests them
/// (`identity.sim = {imsi, carrier, blank}`); the Cellular v2 reader answered
/// `{type: "sim", imsi, carrier, blank}` flat. Both are read.
fn applet<'a>(id: &'a Value, name: &str) -> Option<&'a Value> {
    match get(id, name) {
        Some(v @ Value::Map(_)) => Some(v),
        _ if text(id, "type") == name || text(id, "kind") == name => Some(id),
        _ => None,
    }
}

fn kind_of(id: &Value) -> String {
    match get(id, "kind") {
        Some(Value::Str(k)) => k.clone(),
        _ => text(id, "type"),
    }
}

fn held_line(st: &Value) -> String {
    format!("  {}, auto eject: {}", if flag(st, "released") { "released" } else { "held for this computer" }, text(st, "auto_eject"))
}

/// `cardtool sim [info]`: simtool's lines, unchanged.
fn sim_info(name: &str) {
    let st = call(name, "status", &[]);
    if !flag(&st, "present") {
        println!("{name}: no card (auto eject: {})", text(&st, "auto_eject"));
        return;
    }
    let id = call(name, "identity", &[]);
    match applet(&id, "sim") {
        None => println!("{name}: a {} card (not a SIM)", kind_of(&id)),
        Some(sim) if flag(sim, "blank") => println!("{name}: blank SIM"),
        Some(sim) => println!("{name}: SIM IMSI {} carrier {}", text(sim, "imsi"), text(sim, "carrier")),
    }
    println!("{}", held_line(&st));
}

/// `cardtool info`: every applet of the card.
fn info(name: &str) {
    let st = call(name, "status", &[]);
    if !flag(&st, "present") {
        println!("{name}: no card (auto eject: {})", text(&st, "auto_eject"));
        return;
    }
    let id = call(name, "identity", &[]);
    let kind = kind_of(&id);
    if kind == "sim" {
        match applet(&id, "sim") {
            Some(sim) if flag(sim, "blank") => println!("{name}: blank SIM"),
            Some(sim) => println!("{name}: SIM IMSI {} carrier {}", text(sim, "imsi"), text(sim, "carrier")),
            None => println!("{name}: sim card"),
        }
    } else {
        println!("{name}: {kind} card");
    }
    if let Some(Value::Str(uid)) = get(&id, "uid") {
        println!("  uid {uid}");
    }
    let applets = str_list(get(&id, "applets"));
    if !applets.is_empty() {
        println!("  applets: {}", applets.join(", "));
    }
    if let Some(b) = applet(&id, "bank") {
        if flag(b, "blank") {
            println!("  bank: blank");
        } else {
            println!(
                "  bank: issuer {} pan {} ctr {}{}",
                text(b, "issuer"),
                text(b, "pan"),
                text(b, "ctr"),
                if flag(b, "locked") { " LOCKED" } else { "" }
            );
        }
    }
    if let Some(n) = applet(&id, "nfc") {
        let sectors = match get(n, "sectors") {
            Some(Value::List(items)) => items
                .iter()
                .enumerate()
                .filter(|(_, s)| flag(s, "written") || matches!(s, Value::Bool(true)))
                .map(|(i, _)| i.to_string())
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        println!("  nfc: {}", if sectors.is_empty() { "blank".to_string() } else { format!("sectors written {}", sectors.join(",")) });
    }
    if kind != "sim" {
        if let Some(sim) = applet(&id, "sim") {
            if flag(sim, "blank") {
                println!("  sim: blank");
            } else {
                println!("  sim: IMSI {} carrier {}", text(sim, "imsi"), text(sim, "carrier"));
            }
        }
    }
    if let Some(Value::Map(fields)) = get(&id, "print") {
        if !fields.is_empty() {
            println!("  fields: {}", value_text(&Value::Map(fields.clone())));
        }
    }
    if let Some(Value::Map(_)) = get(&id, "stripe") {
        let st = get(&id, "stripe").unwrap();
        println!("  stripe: {}{}", stripe_text(st), if flag(st, "corrupted") { " (corrupted)" } else { "" });
    }
    println!("{}", held_line(&st));
}

fn ejected_line(r: &Value) {
    let ejected = match r {
        Value::Bool(b) => *b,
        other => flag(other, "ejected"),
    };
    if ejected {
        println!("card ejected into an adjacent inventory");
    } else {
        println!("card released: waiting to be taken or pulled out");
    }
}

// ---------------------------------------------------------------- stripe

fn stripe_text(st: &Value) -> String {
    format!("t1 \"{}\" t2 \"{}\" t3 \"{}\"", text(st, "t1"), text(st, "t2"), text(st, "t3"))
}

fn stripe(name: &str, args: &[String]) {
    match args {
        [v] if v == "read" => {
            let r = call(name, "read_stripe", &[]);
            if matches!(r, Value::Nil) {
                println!("no stripe on this card");
                return;
            }
            println!("t1 {}", text(&r, "t1"));
            println!("t2 {}", text(&r, "t2"));
            println!("t3 {}", text(&r, "t3"));
            if flag(&r, "corrupted") {
                println!("corrupted (the stripe was in a strong field)");
            }
        }
        [v, t1, t2, t3] if v == "write" => {
            call(name, "write_stripe", &[s(t1.as_str()), s(t2.as_str()), s(t3.as_str())]);
            println!("wrote stripe t1 \"{t1}\" t2 \"{t2}\" t3 \"{t3}\"");
        }
        _ => usage(),
    }
}

// ---------------------------------------------------------------- bank

fn bank_cmd(name: &str, args: &[String]) {
    match args.first().map(String::as_str) {
        Some("write") => bank_write(name, &args[1..]),
        Some("pin") => {
            let [_, pin] = args else { usage() };
            if !bank::valid_pin(pin) {
                fail("the PIN is 4 to 8 digits");
            }
            let r = call(name, "verify_pin", &[s(pin.as_str())]);
            let ok = match &r {
                Value::Bool(b) => *b,
                other => flag(other, "ok"),
            };
            if ok {
                println!("pin ok: session open until the card leaves the reader");
            } else {
                let left = int(&r, "tries_left").map(|n| format!(" ({n} tries left)")).unwrap_or_default();
                println!("pin wrong{left}");
                std::process::exit(1);
            }
        }
        Some("sign") => {
            let (nonce, amount) = match &args[1..] {
                [n] => (n, 0i64),
                [n, a] => (n, a.parse().unwrap_or_else(|_| fail("the amount is a whole number"))),
                _ => usage(),
            };
            if !is_hex(nonce, 16) {
                fail("the nonce is 16 hex digits (8 bytes)");
            }
            let r = call(name, "sign", &[s(nonce.to_ascii_lowercase()), Value::Int(amount)]);
            println!("ctr {}", text(&r, "ctr"));
            println!("tag {}", text(&r, "tag"));
        }
        Some("blank") => {
            let r = call(name, "bank_blank", &[]);
            println!("{}", if matches!(r, Value::Bool(true)) { "blank bank chip" } else { "bank chip written" });
        }
        _ => usage(),
    }
}

fn bank_write(name: &str, args: &[String]) {
    let [issuer, pan, rest @ ..] = args else { usage() };
    if !bank::valid_issuer(issuer) {
        fail("the issuer is 1 to 24 of a-z, 0-9, '_' and '-'");
    }
    if !bank::valid_pan(pan) {
        fail("the PAN is 8 to 19 digits");
    }
    let mut key: Option<String> = None;
    let mut pin = String::new();
    let mut pin_required = false;
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--random" => key = None,
            "--key" => {
                let k = rest.get(i + 1).unwrap_or_else(|| usage());
                if !is_hex(k, 64) {
                    fail("the key is 64 hex digits (32 bytes)");
                }
                key = Some(k.to_ascii_lowercase());
                i += 1;
            }
            "--pin" => {
                let p = rest.get(i + 1).unwrap_or_else(|| usage());
                if !bank::valid_pin(p) {
                    fail("the PIN is 4 to 8 digits");
                }
                pin = p.clone();
                i += 1;
            }
            "--pin-required" => pin_required = true,
            _ => usage(),
        }
        i += 1;
    }
    if pin_required && pin.is_empty() {
        fail("--pin-required needs --pin <digits>");
    }
    let key = key.unwrap_or_else(|| random_key(32));
    let r = call(name, "write_bank", &[s(issuer.as_str()), s(pan.as_str()), s(key.clone()), s(pin.clone()), Value::Bool(pin_required)]);
    println!("wrote bank card issuer {issuer} pan {pan}{}", if pin_required { " (PIN required)" } else { "" });
    println!("key {key}");
    println!("  (shown once: put it in the issuer's database; the card never gives it back)");
    if get(&r, "ejected").is_some() {
        ejected_line(&r);
    }
}

// ---------------------------------------------------------------- nfc

fn sector_arg(a: &str) -> Value {
    let n: i64 = a.parse().unwrap_or_else(|_| fail("the sector is 0 to 7"));
    if !(0..8).contains(&n) {
        fail("the sector is 0 to 7");
    }
    Value::Int(n)
}

fn nfc_cmd(dev: &Devices, args: &[String]) {
    let nfc = dev.nfc();
    // Over the air through an NFC Reader, or by contact through the Card Reader/Writer.
    let (name, prefix) = match &nfc {
        Some(n) => (n.clone(), ""),
        None => (dev.reader(), "nfc_"),
    };
    let m = |method: &str| format!("{prefix}{method}");
    let air_only = |verb: &str| {
        if nfc.is_none() {
            fail(format!("'nfc {verb}' needs an NFC Reader (the Card Reader/Writer only talks to the card in its slot)"));
        }
    };
    match args.first().map(String::as_str) {
        Some("poll") => {
            air_only("poll");
            let r = call(&name, "poll", &[]);
            let cards = match &r {
                Value::List(items) => items.clone(),
                _ => Vec::new(),
            };
            if cards.is_empty() {
                println!("no cards in the field");
            }
            for c in &cards {
                println!("uid {} kinds {}", text(c, "uid"), str_list(get(c, "kinds")).join(","));
            }
        }
        Some("select") => {
            air_only("select");
            let [_, uid] = args else { usage() };
            if !is_hex(uid, 16) {
                fail("the UID is 16 hex digits (8 bytes)");
            }
            call(&name, "select", &[s(uid.to_ascii_lowercase())]);
            println!("selected {uid}");
        }
        Some("auth") => {
            let [_, sector, key] = args else { usage() };
            if !is_hex(key, 32) {
                fail("the sector key is 32 hex digits (16 bytes)");
            }
            let r = call(&name, &m("auth"), &[sector_arg(sector), s(key.to_ascii_lowercase())]);
            if matches!(r, Value::Bool(false)) {
                println!("auth failed for sector {sector}");
                std::process::exit(1);
            }
            println!("sector {sector} open");
        }
        Some("read") => {
            let [_, sector] = args else { usage() };
            let r = call(&name, &m("read"), &[sector_arg(sector)]);
            println!("{}", value_text(&r));
        }
        Some("write") => {
            let [_, sector, data] = args else { usage() };
            let bytes = hex::decode(data).unwrap_or_else(|| fail("the data is hex"));
            if bytes.len() > ecm_cards::nfc::SECTOR_BYTES {
                fail("a sector holds 64 bytes (128 hex digits)");
            }
            let mut padded = bytes;
            padded.resize(ecm_cards::nfc::SECTOR_BYTES, 0);
            call(&name, &m("write"), &[sector_arg(sector), s(hex::encode(&padded))]);
            println!("wrote sector {sector}");
        }
        Some("setkey") => {
            let [_, sector, key] = args else { usage() };
            if !is_hex(key, 32) {
                fail("the sector key is 32 hex digits (16 bytes)");
            }
            call(&name, &m("set_key"), &[sector_arg(sector), s(key.to_ascii_lowercase())]);
            println!("sector {sector} rekeyed");
        }
        Some("field") => {
            air_only("field");
            match args.get(1).map(String::as_str) {
                Some("on") | Some("off") => {
                    call(&name, "set_field", &[Value::Bool(args[1] == "on")]);
                    println!("field {}", args[1]);
                }
                None => println!("field {}", if matches!(call(&name, "field", &[]), Value::Bool(true)) { "on" } else { "off" }),
                _ => usage(),
            }
        }
        Some("raw") => {
            air_only("raw");
            let [_, frame] = args else { usage() };
            if hex::decode(frame).is_none() {
                fail("the frame is hex");
            }
            let r = call(&name, "raw", &[s(frame.to_ascii_lowercase())]);
            println!("{}", value_text(&r));
        }
        Some("rssi") => {
            air_only("rssi");
            println!("rssi {}", value_text(&call(&name, "rssi", &[])));
        }
        _ => usage(),
    }
}

// ---------------------------------------------------------------- printer

fn design_value(d: &Design) -> Value {
    let mut entries = vec![
        (s("palette"), Value::List(d.palette.iter().map(|c| Value::List(c.iter().map(|v| Value::Int(*v as i64)).collect())).collect())),
        (s("front"), s(hex::encode(&d.front))),
        (s("back"), s(hex::encode(&d.back))),
        (s("fields"), Value::Map(d.fields.iter().map(|(k, v)| (s(k.as_str()), s(v.as_str()))).collect())),
    ];
    if let Some((t1, t2, t3)) = &d.stripe {
        entries.push((s("stripe"), Value::Map(vec![(s("t1"), s(t1.as_str())), (s("t2"), s(t2.as_str())), (s("t3"), s(t3.as_str()))])));
    }
    Value::Map(entries)
}

fn load_design(path: &str) -> Design {
    let d = Design::load(path).unwrap_or_else(|e| fail(e));
    d.validate().unwrap_or_else(|e| fail(format!("{path}: {e}")));
    d
}

fn dyes_text(v: &Value) -> String {
    match v {
        Value::Map(entries) => entries.iter().map(|(k, v)| format!("{} x{}", value_text(k), value_text(v))).collect::<Vec<_>>().join(", "),
        Value::List(items) => items.iter().map(value_text).collect::<Vec<_>>().join(", "),
        other => value_text(other),
    }
}

fn cost(printer: &str, path: &str) {
    let d = load_design(path);
    let r = call(printer, "cost", &[design_value(&d)]);
    let dyes = get(&r, "dyes").unwrap_or(&r);
    let (w, h) = face_size(&d.kind).unwrap_or((0, 0));
    println!("{path}: {} card {w}x{h}, {} colours, {} fields{}", d.kind, d.palette.len(), d.fields.len(), if d.stripe.is_some() { ", stripe" } else { "" });
    println!("dyes: {}", dyes_text(dyes));
}

fn print(printer: &str, path: &str) {
    let d = load_design(path);
    let r = call(printer, "print", &[design_value(&d)]);
    let dyes = get(&r, "dyes").map(dyes_text).unwrap_or_default();
    println!("printing {path} on {printer}{}", if dyes.is_empty() { String::new() } else { format!(" (dyes: {dyes})") });
    // Wait for the printer's verdict (it prints over ~2 s).
    let mut waited = 0;
    while waited < 10_000 {
        let ev = peripheral::wait_event(None, 500).unwrap_or_else(|e| fail(describe(e)));
        waited += 500;
        let Some(ev) = ev else { continue };
        let txt = |i: usize| match ev.get(i) {
            Some(Value::Str(s)) => s.clone(),
            Some(other) => value_text(other),
            None => String::new(),
        };
        if txt(1) != printer {
            continue;
        }
        match txt(0).as_str() {
            "print_done" => {
                println!("printed");
                let st = call(printer, "status", &[]);
                if get(&st, "ejected").is_some() || get(&st, "released").is_some() {
                    ejected_line(&st);
                }
                return;
            }
            "print_failed" => fail(format!("print failed: {}", txt(2))),
            _ => {}
        }
    }
    println!("print requested (no print_done event within 10 s)");
}

// ---------------------------------------------------------------- sim (simtool's verbs)

fn sim_write(name: &str, args: &[String]) {
    let (imsi, carrier) = match args {
        [i, c, ..] => (i.clone(), c.clone()),
        _ => usage(),
    };
    let key = match &args[2..] {
        [] => None,
        [f] if f == "--random" => None,
        [f, k] if f == "--key" => {
            if !is_hex(k, 32) {
                fail("the key is 32 hex digits (16 bytes)");
            }
            Some(k.to_ascii_lowercase())
        }
        _ => usage(),
    };
    let key = key.unwrap_or_else(|| random_key(16));
    let r = call(name, "write", &[Value::Str(imsi), Value::Str(carrier), Value::Str(key.clone())]);
    println!("wrote SIM IMSI {} carrier {}", text(&r, "imsi"), text(&r, "carrier"));
    println!("key {key}");
    println!("  (shown once: put it in the carrier's subscriber database; the card never gives it back)");
    ejected_line(&r);
}

fn sim_auth(name: &str, args: &[String]) {
    let [c] = args else { usage() };
    if !is_hex(c, 32) {
        fail("the challenge is 32 hex digits (16 bytes)");
    }
    let r = call(name, "respond", &[Value::Str(c.to_ascii_lowercase())]);
    println!("res {}", text(&r, "res"));
    println!("ks  {}", text(&r, "ks"));
}

/// Whether the card just inserted is blank, for the `card_inserted` line: the
/// event's third argument when it is a bool (Cellular v2), else from `identity()`.
fn inserted_blank(name: &str, ev: &[Value]) -> String {
    if let Some(Value::Bool(b)) = ev.get(3) {
        return b.to_string();
    }
    let id = match peripheral::call(name, "identity", &[]) {
        Ok(id) => id,
        Err(_) => return "-".into(),
    };
    let blank_of = |a: &str| applet(&id, a).map(|m| flag(m, "blank"));
    let kind = kind_of(&id);
    let blank = match kind.as_str() {
        "sim" => blank_of("sim"),
        "bank" => blank_of("bank"),
        "nfc" => applet(&id, "nfc").map(|n| match get(n, "sectors") {
            Some(Value::List(items)) => items.iter().all(|s| !(flag(s, "written") || matches!(s, Value::Bool(true)))),
            _ => true,
        }),
        "hybrid" => Some(blank_of("bank").unwrap_or(true) && blank_of("nfc").unwrap_or(true)),
        _ => None,
    };
    match blank {
        Some(b) => b.to_string(),
        None => "-".into(),
    }
}

/// Print the next card event of this reader (older queued events are skipped).
fn wait(name: &str, args: &[String]) {
    let secs: i32 = match args {
        [] => 30,
        [s] => s.parse().unwrap_or_else(|_| usage()),
        _ => usage(),
    };
    for _ in 0..1000 {
        match peripheral::wait_event(None, 0) {
            Ok(Some(_)) => continue,
            _ => break,
        }
    }
    println!("waiting for a card ({secs} s)");
    let deadline = secs.max(0) as i64 * 1000;
    let mut waited = 0i64;
    while waited <= deadline {
        let ev = peripheral::wait_event(None, 500).unwrap_or_else(|e| fail(describe(e)));
        waited += 500;
        let Some(ev) = ev else { continue };
        let txt = |i: usize| match ev.get(i) {
            Some(Value::Str(s)) => s.clone(),
            Some(Value::Bool(b)) => b.to_string(),
            _ => String::new(),
        };
        if txt(1) != name || !txt(0).starts_with("card_") {
            continue;
        }
        if txt(0) == "card_inserted" {
            println!("event card_inserted {} blank={}", txt(2), inserted_blank(name, &ev));
        } else {
            println!("event {}", txt(0));
        }
        return;
    }
    fail("no card event");
}

fn sim_cmd(dev: &Devices, args: &[String]) {
    let name = dev.reader();
    match args.first().map(String::as_str) {
        None | Some("info") => sim_info(&name),
        Some("write") => sim_write(&name, &args[1..]),
        Some("auth") => sim_auth(&name, &args[1..]),
        Some("wait") => wait(&name, &args[1..]),
        Some("eject") => ejected_line(&call(&name, "eject", &[])),
        _ => usage(),
    }
}

// ---------------------------------------------------------------- main

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut dev = Devices { reader: None, nfc: None, printer: None };
    for (flag_name, slot) in [("--reader", &mut dev.reader), ("--nfc", &mut dev.nfc), ("--printer", &mut dev.printer)] {
        if let Some(i) = args.iter().position(|a| a == flag_name) {
            if i + 1 >= args.len() {
                usage();
            }
            *slot = Some(args.remove(i + 1));
            args.remove(i);
        }
    }
    match args.first().map(String::as_str) {
        None | Some("info") => info(&dev.reader()),
        Some("stripe") => stripe(&dev.reader(), &args[1..]),
        Some("bank") => bank_cmd(&dev.reader(), &args[1..]),
        Some("nfc") => nfc_cmd(&dev, &args[1..]),
        Some("print") => {
            let [_, path] = args.as_slice() else { usage() };
            print(&dev.printer(), path);
        }
        Some("cost") => {
            let [_, path] = args.as_slice() else { usage() };
            cost(&dev.printer(), path);
        }
        Some("eject") => ejected_line(&call(&dev.reader(), "eject", &[])),
        Some("wait") => wait(&dev.reader(), &args[1..]),
        Some("sim") => sim_cmd(&dev, &args[1..]),
        Some("help") | Some("--help") | Some("-h") => usage(),
        _ => usage(),
    }
}
