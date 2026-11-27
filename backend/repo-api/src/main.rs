use repo_api::{admin, build, serve, Config, StorageConfig};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("admin") => match admin::run(&args[1..]).await {
            Ok(text) => print!("{text}"),
            Err(e) => fail(&e),
        },
        Some("healthcheck") => {
            let listen = backend_common::env::var_or("LISTEN", "0.0.0.0:8082");
            std::process::exit(if backend_common::http::healthcheck(&listen) { 0 } else { 1 });
        }
        Some(other) => fail(&format!("unknown command '{other}' (run without arguments to serve; `repo-api admin help`, `repo-api healthcheck`)")),
        None => run().await,
    }
}

async fn run() {
    let cfg = match Config::from_env() {
        Ok(c) => c,
        Err(e) => fail(&e),
    };
    let listen = cfg.listen.clone();
    let storage = match &cfg.storage {
        StorageConfig::Fs { dir } => format!("fs {dir}"),
        StorageConfig::S3(s) => format!("s3 {} bucket {}", s.endpoint, s.bucket),
    };
    println!(
        "repo-api: origin {}, database {}, storage {}, auth at {}, trust proxy: {}",
        cfg.public_origin, cfg.database_path, storage, cfg.auth_internal_url, cfg.trust_proxy
    );
    let app = match build(cfg) {
        Ok(a) => a,
        Err(e) => fail(&e),
    };
    println!("repo-api: repository key {}", app.public_key);
    if let Err(e) = app.storage.check().await {
        fail(&format!("the blob store does not work: {e}"));
    }
    let listener = match tokio::net::TcpListener::bind(&listen).await {
        Ok(l) => l,
        Err(e) => fail(&format!("cannot listen on {listen}: {e}")),
    };
    println!("repo-api: listening on {listen}");
    tokio::select! {
        r = serve(app, listener) => { if let Err(e) = r { fail(&format!("server stopped: {e}")); } }
        _ = stop_signal() => println!("repo-api: stopping"),
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
    eprintln!("repo-api: {}", message.trim_end());
    std::process::exit(1)
}
