//! whois: who holds a domain (docs/DNS.md). `whois NAME.ecm [-h SERVER]` asks the
//! registrar's whois server (whois.nic.ring, TCP 43) and prints its answer.

use ecm_dns::sys;
use ecm_host_abi::socket::{self, SockAddrIn, AF_INET, SOCK_STREAM, SOL_SOCKET, SO_RCVTIMEO};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut server = "whois.nic.ring".to_string();
    let mut query = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" => {
                i += 1;
                server = args.get(i).cloned().unwrap_or(server);
            }
            "--help" => {
                println!("Usage: whois NAME.ecm [-h SERVER]   (default whois.nic.ring, port 43)");
                return;
            }
            q => query = Some(q.to_string()),
        }
        i += 1;
    }
    let Some(q) = query else {
        println!("Usage: whois NAME.ecm [-h SERVER]");
        std::process::exit(1);
    };
    let ip = match sys::server_addr(&server) {
        Ok(ip) => ip,
        Err(e) => {
            eprintln!("whois: {e}");
            std::process::exit(1);
        }
    };
    let fd = socket::socket(AF_INET, SOCK_STREAM, 0);
    socket::setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &5000i32.to_le_bytes());
    if fd < 0 || socket::connect(fd, &SockAddrIn::from_ip_port(ip, 43)) != 0 {
        eprintln!("whois: cannot connect to {server} port 43");
        std::process::exit(1);
    }
    sys::send_all(fd, format!("{q}\r\n").as_bytes());
    let out = sys::read_all(fd, 8000).unwrap_or_default();
    socket::close(fd);
    if out.is_empty() {
        eprintln!("whois: {server} sent nothing");
        std::process::exit(1);
    }
    print!("{}", out.replace("\r\n", "\n"));
}
