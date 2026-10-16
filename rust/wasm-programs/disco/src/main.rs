//! `disco`: service discovery (docs/DISCOVERY.md).
//!
//!   disco [TYPE] [--wait MS]   search the network (every type, or chat, pkg, ...)
//!   disco serve [-v] &         answer for this computer's services (/etc/disco.conf and
//!                              what servers announce in /run/disco); relay for routers

use std::io::Write;

const USAGE: &str = "usage: disco [TYPE] [--wait MS]   find services on the network (chat, pkg, ... ; default every type)
       disco serve [-v] &         answer for this computer's services (-v: log every query)
Servers announce themselves (chatd, pkg serve); /etc/disco.conf adds more:
  service <type> <port> [name]          on this computer
  record <type> <host> <port> [name]    elsewhere (routers answer for the network)
  relay gateway | relay <address>       pass queries on to the next router";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["-h"] | ["--help"] | ["help"] => println!("{USAGE}"),
        ["serve", rest @ ..] => {
            let verbose = rest.contains(&"-v");
            let mut log = |l: &str| {
                println!("{l}");
                let _ = std::io::stdout().flush();
            };
            if let Err(e) = ecm_disco::sys::serve(verbose, &mut log) {
                println!("disco: {e}");
                std::process::exit(1);
            }
        }
        rest => {
            let mut kind = "*".to_string();
            let mut wait = ecm_disco::CLIENT_WAIT_MS;
            let mut i = 0;
            while i < rest.len() {
                match rest[i] {
                    "--wait" => {
                        i += 1;
                        match rest.get(i).and_then(|w| w.parse().ok()) {
                            Some(w) => wait = w,
                            None => {
                                println!("{USAGE}");
                                std::process::exit(2);
                            }
                        }
                    }
                    t if ecm_disco::valid_type(t) || t == "*" => kind = t.to_string(),
                    _ => {
                        println!("{USAGE}");
                        std::process::exit(2);
                    }
                }
                i += 1;
            }
            match ecm_disco::sys::search(&kind, wait) {
                Ok(found) if found.is_empty() => {
                    println!("disco: nothing found{}.", if kind == "*" { String::new() } else { format!(" for {kind}") });
                    println!("  Servers need 'disco serve &' on their computer; across routers, the routers need it with 'relay gateway'.");
                    std::process::exit(1);
                }
                Ok(found) => print!("{}", ecm_disco::format_results(&found)),
                Err(e) => {
                    println!("disco: {e}");
                    std::process::exit(1);
                }
            }
        }
    }
}
