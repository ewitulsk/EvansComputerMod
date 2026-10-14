//! `wifi` — see `lib.rs`. The steps run in `wifi::system` (shared with the
//! GUI Settings app); this file only prints.

use ecm_host_abi::peripheral::{self, Value};
use ::wifi::system::{self, Computer};
use ::wifi::{format_scan, format_status, usage, Problem};

fn die(p: Problem) -> ! {
    eprint!("{}", p.render());
    std::process::exit(1);
}

fn ready(c: &mut Computer) -> [u8; 6] {
    system::ready(c).unwrap_or_else(|p| die(p))
}

fn connect(ssid: &str, password: Option<&str>) {
    let r = system::connect(ssid, password, &mut |l: &str| println!("{}", l), &mut |w: &str| eprintln!("{}", w));
    match r {
        Ok(done) => println!("{}", done),
        Err(p) => die(p),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    match a.as_slice() {
        [] | ["status"] => {
            let mut c = Computer;
            ready(&mut c);
            let st = system::status_text().unwrap_or_default();
            print!("{}", format_status(&st, system::address().as_deref()));
        }
        ["scan"] => {
            print!("{}", format_scan(&system::scan_networks().unwrap_or_else(|p| die(p))));
        }
        ["connect", ssid] => connect(ssid, None),
        ["connect", ssid, password] => connect(ssid, Some(password)),
        ["connect", ssid, rest @ ..] if !rest.is_empty() => {
            // An unquoted password with spaces: the shell split it.
            connect(ssid, Some(&rest.join(" ")))
        }
        ["disconnect"] => {
            system::disconnect(&mut |l: &str| println!("{}", l)).unwrap_or_else(|p| die(p));
            println!("wlan0: disconnected");
        }
        ["mode", m @ ("wifi" | "controller")] => {
            let Ok(list) = peripheral::list() else { die(Problem { what: "can't list this computer's peripherals".into(), fix: vec![] }) };
            let Some((name, _)) = list.iter().find(|(_, t)| t == "wifi") else {
                die(Problem {
                    what: "this computer has no Wi-Fi Module".into(),
                    fix: vec![
                        "Hold a Module Expansion Card and right-click a side of the computer (not the screen) to fit a bay.".into(),
                        "Hold a Wi-Fi Module and right-click the same side to put it in the bay.".into(),
                    ],
                })
            };
            match peripheral::call(name, "set_mode", &[Value::Str(m.to_string())]) {
                Ok(_) => println!("Wi-Fi Module ({}) is now in {} mode", name, m),
                Err(e) => die(Problem { what: format!("couldn't switch the Wi-Fi Module: {:?}", e), fix: vec![] }),
            }
        }
        ["help"] | ["-h"] | ["--help"] => print!("{}", usage()),
        _ => {
            eprint!("{}", usage());
            std::process::exit(2);
        }
    }
}
