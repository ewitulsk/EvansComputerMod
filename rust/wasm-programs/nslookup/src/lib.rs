use ecm_net_tools::NetTools;

pub fn run(args: &[String], nt: &mut dyn NetTools) -> i32 {
    if args.is_empty() {
        nt.err("Usage: nslookup <hostname>\n");
        return 1;
    }

    let name = &args[0];
    match nt.getaddrinfo(name) {
        Some(ip) => {
            nt.out(&format!("Name:    {}\n", name));
            nt.out(&format!("Address: {}.{}.{}.{}\n", ip[0], ip[1], ip[2], ip[3]));
            0
        }
        None => {
            nt.err(&format!("DNS lookup failed for {}\n", name));
            1
        }
    }
}
