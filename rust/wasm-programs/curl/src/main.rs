use curl::{error_message, parse};
use ecm_http::{Check, Transfer};
use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let o = match parse(&args) {
        Ok(o) => o,
        Err(m) => {
            eprintln!("{m}");
            std::process::exit(2);
        }
    };
    let req = match o.request() {
        Ok(r) => r,
        Err(m) => {
            eprintln!("{m}");
            std::process::exit(3);
        }
    };
    let mut client = ecm_http::sys::client("curl/ecm");
    if o.insecure {
        client.check = Check::None;
    }
    if o.verbose {
        println!("> {} {} HTTP/1.1", req.method, req.url.target);
        println!("> Host: {}", req.url.authority());
        for (k, v) in &req.headers {
            println!("> {k}: {v}");
        }
        println!(">");
    }
    // A download goes to a temporary name first, so a failed one leaves no half file.
    let part = o.output.as_ref().map(|f| format!("{f}.part"));
    let mut file = match &part {
        Some(p) => match std::fs::File::create(p) {
            Ok(f) => Some(f),
            Err(e) => {
                eprintln!("curl: cannot write {}: {e}", o.output.as_deref().unwrap_or(""));
                std::process::exit(23);
            }
        },
        None => None,
    };
    let show = !o.silent && o.output.is_some();
    let mut last = 0u64;
    let mut progress = |done: u64, total: Option<u64>| {
        if show && (done - last >= 64 * 1024 || Some(done) == total) {
            last = done;
            match total {
                Some(t) if t > 0 => println!("  {} of {} KiB ({}%)", done / 1024, t.div_ceil(1024), done * 100 / t),
                _ => println!("  {} KiB", done / 1024),
            }
        }
    };
    let result = {
        let mut t = Transfer { sink: file.as_mut().map(|f| f as &mut dyn Write), progress: Some(&mut progress), max_body: if o.output.is_some() { 256 << 20 } else { ecm_http::DEFAULT_MAX_BODY } };
        if o.follow {
            client.send(&req, &mut t)
        } else {
            client.exchange(&req, &mut t)
        }
    };
    drop(file);
    let r = match result {
        Ok(r) => r,
        Err(e) => {
            if let Some(p) = &part {
                let _ = std::fs::remove_file(p);
            }
            if !o.silent {
                eprintln!("{}", error_message(&e, &req.url));
            }
            std::process::exit(match e {
                ecm_http::HttpError::Resolve(..) => 6,
                ecm_http::HttpError::Connect(_) | ecm_http::HttpError::NoNetwork => 7,
                ecm_http::HttpError::Tls(_) => 60,
                _ => 1,
            });
        }
    };
    if o.verbose || o.head {
        let mark = if o.verbose { "< " } else { "" };
        if let (true, Some(v)) = (o.verbose, r.tls) {
            println!("* {v} connection to {}", r.url.host);
        }
        println!("{mark}{} {} {}", r.version, r.status, r.reason);
        for (k, v) in &r.headers {
            println!("{mark}{k}: {v}");
        }
        println!("{}", mark.trim_end());
    }
    let failed = o.fail && r.status >= 400;
    if failed {
        if !o.silent {
            eprintln!("curl: The requested URL returned error: {}", r.status);
        }
        if let Some(p) = &part {
            let _ = std::fs::remove_file(p);
        }
        std::process::exit(22);
    }
    match (&o.output, &part) {
        (Some(out), Some(p)) if r.ok() => {
            if let Err(e) = std::fs::rename(p, out) {
                eprintln!("curl: cannot write {out}: {e}");
                std::process::exit(23);
            }
            if !o.silent {
                println!("Saved {out} ({} bytes)", r.length);
            }
        }
        (Some(_), Some(p)) => {
            let _ = std::fs::remove_file(p);
            eprintln!("curl: HTTP {} {}; nothing saved", r.status, r.reason);
            std::process::exit(22);
        }
        _ => {
            let mut out = std::io::stdout();
            let _ = out.write_all(&r.body);
            if !r.body.is_empty() && !r.body.ends_with(b"\n") {
                let _ = out.write_all(b"\n");
            }
        }
    }
}
