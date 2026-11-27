use auth_api::{build, serve, Config};

#[tokio::main]
async fn main() {
    match std::env::args().nth(1).as_deref() {
        None => {}
        Some("admin") => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            match auth_api::admin::run(&args).await {
                Ok(text) => {
                    print!("{text}");
                    return;
                }
                Err(e) => fail(e.trim_end()),
            }
        }
        Some("healthcheck") => {
            let listen = backend_common::env::var_or("LISTEN", "0.0.0.0:8081");
            std::process::exit(if backend_common::http::healthcheck(&listen) { 0 } else { 1 });
        }
        Some(other) => fail(&format!("unknown command '{other}' (run without arguments to serve; `auth-api admin help`, `auth-api healthcheck`)")),
    }
    let cfg = match Config::from_env() {
        Ok(c) => c,
        Err(e) => fail(&e),
    };
    let listen = cfg.listen.clone();
    println!(
        "auth-api: origin {}, database {}, mail via {}, {} admin email(s), trust proxy: {}",
        cfg.public_origin,
        cfg.database_path,
        cfg.email_provider,
        cfg.admin_emails.len(),
        cfg.trust_proxy
    );
    let app = match build(cfg) {
        Ok(a) => a,
        Err(e) => fail(&e),
    };
    let listener = match tokio::net::TcpListener::bind(&listen).await {
        Ok(l) => l,
        Err(e) => fail(&format!("cannot listen on {listen}: {e}")),
    };
    println!("auth-api: listening on {listen}");
    tokio::select! {
        r = serve(app, listener) => { if let Err(e) = r { fail(&format!("server stopped: {e}")); } }
        _ = stop_signal() => println!("auth-api: stopping"),
    }
}

async fn stop_signal() {
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("signal handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}

fn fail(message: &str) -> ! {
    eprintln!("auth-api: {message}");
    std::process::exit(1)
}
