//! `simtool` — write and inspect SIM cards in a Card Reader/Writer next to this computer.
//!
//!   simtool [info]                                   the card in the reader
//!   simtool write <imsi> <carrier> [--key <hex>|--random]
//!                                                    write a blank SIM; prints the key once
//!   simtool auth <challenge-hex>                     the SIM's answer to a 16-byte challenge
//!   simtool eject                                    done with the card (Auto Eject Sim pushes it out)
//!   simtool wait [seconds]                           wait for the next card_inserted / card_removed event
//!
//! `--reader <name>` picks a reader when there are several (default: the first).
//! The key is 16 bytes (32 hex digits). `--random` (the default) draws it from the
//! computer's random source. It is printed once, for the carrier's subscriber
//! database: the card itself never gives it back.
//!
//! This is also the Rust example for the `card_reader` peripheral: everything below
//! is `peripheral::call(reader, method, args)`.

use ecm_host_abi::peripheral::{self, Error, Value};

const TYPE: &str = "card_reader";

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
        Some(Value::Bool(b)) => b.to_string(),
        _ => "-".to_string(),
    }
}

fn flag(m: &Value, key: &str) -> bool {
    matches!(get(m, key), Some(Value::Bool(true)))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|c| c.is_ascii_hexdigit())
}

fn fail(msg: impl AsRef<str>) -> ! {
    eprintln!("simtool: {}", msg.as_ref());
    std::process::exit(1);
}

fn usage() -> ! {
    eprintln!("usage: simtool [--reader <name>] [info]");
    eprintln!("       simtool [--reader <name>] write <imsi> <carrier> [--key <hex>|--random]");
    eprintln!("       simtool [--reader <name>] auth <challenge-hex>");
    eprintln!("       simtool [--reader <name>] eject");
    eprintln!("       simtool [--reader <name>] wait [seconds]");
    std::process::exit(2);
}

fn reader(wanted: Option<&str>) -> String {
    let list = peripheral::list().unwrap_or_else(|e| fail(describe(e)));
    let readers: Vec<String> = list.into_iter().filter(|(_, t)| t == TYPE).map(|(n, _)| n).collect();
    match wanted {
        Some(w) if readers.iter().any(|n| n == w) => w.to_string(),
        Some(w) => fail(format!("no Card Reader/Writer called '{w}' (readers: {})", if readers.is_empty() { "none".into() } else { readers.join(", ") })),
        None => readers.into_iter().next().unwrap_or_else(|| {
            eprintln!("simtool: no Card Reader/Writer next to this computer.");
            eprintln!("Place a Card Reader/Writer against any side of the computer.");
            std::process::exit(1);
        }),
    }
}

fn call(name: &str, method: &str, args: &[Value]) -> Value {
    peripheral::call(name, method, args).unwrap_or_else(|e| fail(describe(e)))
}

fn info(name: &str) {
    let st = call(name, "status", &[]);
    if !flag(&st, "present") {
        println!("{name}: no card (auto eject: {})", text(&st, "auto_eject"));
        return;
    }
    let id = call(name, "identity", &[]);
    if text(&id, "type") != "sim" {
        println!("{name}: a {} card (not a SIM)", text(&id, "type"));
    } else if flag(&id, "blank") {
        println!("{name}: blank SIM");
    } else {
        println!("{name}: SIM IMSI {} carrier {}", text(&id, "imsi"), text(&id, "carrier"));
    }
    println!("  {}, auto eject: {}", if flag(&st, "released") { "released" } else { "held for this computer" }, text(&st, "auto_eject"));
}

fn write(name: &str, args: &[String]) {
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
    let key = key.unwrap_or_else(|| {
        let mut k = [0u8; 16];
        // WASI random_get (the host's SecureRandom-backed entropy).
        if getrandom::getrandom(&mut k).is_err() || k.iter().all(|&b| b == 0) {
            fail("no random source");
        }
        hex(&k)
    });
    let r = call(name, "write", &[Value::Str(imsi), Value::Str(carrier), Value::Str(key.clone())]);
    println!("wrote SIM IMSI {} carrier {}", text(&r, "imsi"), text(&r, "carrier"));
    println!("key {key}");
    println!("  (shown once: put it in the carrier's subscriber database; the card never gives it back)");
    if flag(&r, "ejected") {
        println!("card ejected into an adjacent inventory");
    } else {
        println!("card released: waiting to be taken or pulled out");
    }
}

fn auth(name: &str, args: &[String]) {
    let [c] = args else { usage() };
    if !is_hex(c, 32) {
        fail("the challenge is 32 hex digits (16 bytes)");
    }
    let r = call(name, "respond", &[Value::Str(c.to_ascii_lowercase())]);
    println!("res {}", text(&r, "res"));
    println!("ks  {}", text(&r, "ks"));
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
            println!("event card_inserted {} blank={}", txt(2), txt(3));
        } else {
            println!("event {}", txt(0));
        }
        return;
    }
    fail("no card event");
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut wanted = None;
    if let Some(i) = args.iter().position(|a| a == "--reader") {
        if i + 1 >= args.len() {
            usage();
        }
        wanted = Some(args.remove(i + 1));
        args.remove(i);
    }
    let name = reader(wanted.as_deref());
    match args.first().map(String::as_str) {
        None | Some("info") => info(&name),
        Some("write") => write(&name, &args[1..]),
        Some("auth") => auth(&name, &args[1..]),
        Some("wait") => wait(&name, &args[1..]),
        Some("eject") => {
            let out = call(&name, "eject", &[]);
            if matches!(out, Value::Bool(true)) {
                println!("card ejected into an adjacent inventory");
            } else {
                println!("card released: waiting to be taken or pulled out");
            }
        }
        _ => usage(),
    }
}
