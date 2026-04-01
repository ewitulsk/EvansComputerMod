extern crate ecm_host_abi;
use ecm_host_abi::peripheral;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() <= 1 {
        // List all peripherals
        match peripheral::list_raw() {
            Some(json) if !json.trim().is_empty() && json != "[]" => {
                println!();
                println!("Connected peripherals:");
                println!();
                // Parse JSON array of peripherals
                for entry in json.split('{').skip(1) {
                    let name = extract_json_str(entry, "name").unwrap_or_default();
                    let ptype = extract_json_str(entry, "type").unwrap_or_default();
                    let side = extract_json_str(entry, "side").unwrap_or_default();
                    println!("  {} ({}) - {}", name, ptype, side);
                }
            }
            _ => {
                println!();
                println!("No peripherals connected.");
                println!("Place CC:Tweaked peripheral blocks adjacent to this terminal.");
            }
        }
    } else {
        // Show methods for a specific peripheral
        let name = &args[1];
        match peripheral::get_methods_raw(name) {
            Some(json) if !json.trim().is_empty() && json != "[]" => {
                println!();
                println!("Methods for {}:", name);
                println!();
                // Parse JSON array of method strings
                for method in json.split('"').enumerate().filter_map(|(i, s)| {
                    if i % 2 == 1 && !s.is_empty() { Some(s) } else { None }
                }) {
                    println!("  {}", method);
                }
            }
            Some(_) => {
                println!();
                println!("Methods for {}:", name);
                println!();
                println!("  (no methods)");
            }
            None => {
                eprintln!("Error: peripheral '{}' not found", name);
            }
        }
    }
}

fn extract_json_str(json: &str, key: &str) -> Option<String> {
    let search = format!("\"{}\":\"", key);
    let start = json.find(&search)? + search.len();
    let rest = &json[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}
