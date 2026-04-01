use std::thread;
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: sleep <milliseconds>");
        std::process::exit(1);
    }
    match args[1].parse::<u64>() {
        Ok(ms) => thread::sleep(Duration::from_millis(ms)),
        Err(_) => {
            eprintln!("Usage: sleep <milliseconds>");
            std::process::exit(1);
        }
    }
}
