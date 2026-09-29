//! `peripherals` — list what's attached to this computer.
//!
//!   peripherals          attached peripherals (name and type)
//!   peripherals <name>   one peripheral's methods

use ecm_host_abi::peripheral::{self, Error};

fn describe(e: Error) -> String {
    match e {
        Error::Peripheral(msg) => msg,
        Error::Unavailable => "peripherals are not available".to_string(),
        Error::Malformed(what) => format!("bad reply from the computer: {what}"),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => match peripheral::list() {
            Ok(list) if list.is_empty() => {
                println!("No peripherals attached.");
                println!("Place a peripheral block next to the computer, or install a module");
                println!("(right-click the computer with a Module Expansion Card, then a module).");
            }
            Ok(list) => {
                let width = list.iter().map(|(n, _)| n.len()).max().unwrap_or(4).max(4);
                println!("{:<width$}  TYPE", "NAME");
                for (name, ty) in list {
                    println!("{name:<width$}  {ty}");
                }
            }
            Err(e) => {
                eprintln!("peripherals: {}", describe(e));
                std::process::exit(1);
            }
        },
        [name] => match peripheral::methods(name) {
            Ok((ty, methods)) => {
                println!("{name} ({ty})");
                for m in methods {
                    println!("  {m}");
                }
            }
            Err(e) => {
                eprintln!("peripherals: {}", describe(e));
                std::process::exit(1);
            }
        },
        _ => {
            eprintln!("usage: peripherals [name]");
            std::process::exit(2);
        }
    }
}
