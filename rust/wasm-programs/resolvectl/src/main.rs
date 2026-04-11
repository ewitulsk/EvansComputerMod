use ecm_net_tools::wasi::WasiNetTools;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut nt = WasiNetTools::new();
    let code = resolvectl::run(&args, &mut nt);
    if code != 0 {
        std::process::exit(code);
    }
}
