use ecm_net_tools::NetTools;

pub fn run(args: &[String], nt: &mut dyn NetTools) -> i32 {
    if args.is_empty() {
        nt.err("Usage: resolvectl status | query <hostname>\n");
        return 1;
    }

    match args[0].as_str() {
        "status" => {
            nt.out("DNS resolver status:\n");
            nt.out("  (Use 'resolvectl query <hostname>' to test resolution)\n");
            0
        }
        "query" => {
            if args.len() < 2 {
                nt.err("Usage: resolvectl query <hostname>\n");
                return 1;
            }
            let name = &args[1];
            nt.out(&format!("Resolving {}...\n", name));
            match nt.getaddrinfo(name) {
                Some(ip) => {
                    nt.out(&format!(
                        "{} -> {}.{}.{}.{}\n",
                        name, ip[0], ip[1], ip[2], ip[3]
                    ));
                    0
                }
                None => {
                    nt.err(&format!("Resolution failed for {}\n", name));
                    1
                }
            }
        }
        _ => {
            nt.err("Usage: resolvectl status | query <hostname>\n");
            1
        }
    }
}
