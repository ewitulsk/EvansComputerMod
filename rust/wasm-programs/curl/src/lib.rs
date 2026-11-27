//! `curl`'s command line (tested on the host); the program does the I/O.

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Options {
    pub url: String,
    pub method: Option<String>,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
    pub verbose: bool,
    /// `-L`: follow redirects.
    pub follow: bool,
    /// `-k`: accept any certificate.
    pub insecure: bool,
    /// `-I`: HEAD, print the headers.
    pub head: bool,
    /// `-f`: an HTTP error status fails the command without printing the body.
    pub fail: bool,
    /// `-s`: no progress or error messages.
    pub silent: bool,
    /// `-o FILE` (or `-O`: the address's file name).
    pub output: Option<String>,
}

pub const USAGE: &str = "Usage: curl [-v] [-L] [-k] [-I] [-f] [-s] [-o FILE | -O] [-X METHOD] [-d DATA] [-H HEADER] <url>";

/// The file name `-O` saves to: the last path part, or `index.html`.
pub fn remote_name(url: &ecm_http::Url) -> String {
    let name = url.path().rsplit('/').next().unwrap_or("");
    if name.is_empty() {
        "index.html".into()
    } else {
        name.into()
    }
}

pub fn parse(args: &[String]) -> Result<Options, String> {
    let mut o = Options::default();
    let mut remote = false;
    let mut url: Option<String> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |flag: &str| it.next().cloned().ok_or_else(|| format!("curl: {flag} needs a value"));
        match a.as_str() {
            "-v" | "--verbose" => o.verbose = true,
            "-L" | "--location" => o.follow = true,
            "-k" | "--insecure" => o.insecure = true,
            "-I" | "--head" => o.head = true,
            "-f" | "--fail" => o.fail = true,
            "-s" | "--silent" => o.silent = true,
            "-O" | "--remote-name" => remote = true,
            "-o" | "--output" => o.output = Some(value("-o")?),
            "-X" | "--request" => o.method = Some(value("-X")?.to_ascii_uppercase()),
            "-d" | "--data" => o.body = Some(value("-d")?),
            "-H" | "--header" => {
                let h = value("-H")?;
                let (k, v) = h.split_once(':').ok_or_else(|| format!("curl: -H wants Name: value, not {h}"))?;
                o.headers.push((k.trim().to_string(), v.trim().to_string()));
            }
            s if s.starts_with('-') && s.len() > 1 => return Err(format!("curl: unknown option {s}\n{USAGE}")),
            s => url = Some(s.to_string()),
        }
    }
    let url = url.ok_or_else(|| USAGE.to_string())?;
    // A bare host is http://, as it always was.
    o.url = if url.contains("://") { url } else { format!("http://{url}") };
    if remote {
        let u = ecm_http::Url::parse(&o.url)?;
        o.output = Some(remote_name(&u));
    }
    Ok(o)
}

impl Options {
    pub fn method(&self) -> String {
        match (&self.method, self.head, &self.body) {
            (Some(m), _, _) => m.clone(),
            (None, true, _) => "HEAD".into(),
            (None, false, Some(_)) => "POST".into(),
            _ => "GET".into(),
        }
    }

    pub fn request(&self) -> Result<ecm_http::Request, String> {
        let mut r = ecm_http::Request::new(&self.method(), &self.url).map_err(|e| format!("curl: {e}"))?;
        if let Some(b) = &self.body {
            r = r.body("application/x-www-form-urlencoded", b.clone().into_bytes());
        }
        for (k, v) in &self.headers {
            r = r.header(k, v);
        }
        Ok(r)
    }
}

/// The message for a failed request, in curl's words.
pub fn error_message(e: &ecm_http::HttpError, url: &ecm_http::Url) -> String {
    use ecm_http::HttpError as E;
    match e {
        E::Resolve(h, why) => format!("curl: Could not resolve host: {h} ({why})"),
        E::Connect(_) | E::NoNetwork => format!("curl: Failed to connect to {}:{}", url.host, url.port),
        other => format!("curl: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn options() {
        let o = parse(&args("-v -L -k -f -s -o out.bin https://repo.example/r/stable/index.json")).unwrap();
        assert!(o.verbose && o.follow && o.insecure && o.fail && o.silent);
        assert_eq!((o.output.as_deref(), o.method().as_str()), (Some("out.bin"), "GET"));
        // A bare host is plain http.
        assert_eq!(parse(&args("10.0.0.5:8080/x")).unwrap().url, "http://10.0.0.5:8080/x");
        // -d makes it a POST with the form type unless -H says otherwise.
        let p = parse(&["-d".into(), "op=list".into(), "-H".into(), "Content-Type: text/plain".into(), "http://h/api".into()]).unwrap();
        let r = p.request().unwrap();
        assert_eq!(r.method, "POST");
        let sent = String::from_utf8(r.encode("curl/ecm")).unwrap();
        assert!(sent.contains("Content-Type: text/plain\r\n") && !sent.contains("urlencoded") && sent.ends_with("\r\n\r\nop=list"), "{sent}");
        assert_eq!(parse(&args("-I http://h/")).unwrap().method(), "HEAD");
        assert_eq!(parse(&args("-X delete http://h/")).unwrap().method(), "DELETE");
        assert_eq!(parse(&args("-O http://h/files/a.ecmpkg")).unwrap().output.as_deref(), Some("a.ecmpkg"));
        assert_eq!(parse(&args("-O http://h/")).unwrap().output.as_deref(), Some("index.html"));
        assert!(parse(&args("-o")).unwrap_err().contains("needs a value"));
        assert!(parse(&args("--bogus http://h/")).unwrap_err().contains("unknown option"));
        assert_eq!(parse(&[]).unwrap_err(), USAGE);
    }

    #[test]
    fn messages_keep_their_wording() {
        let u = ecm_http::Url::parse("http://10.0.0.1:9999/").unwrap();
        assert_eq!(error_message(&ecm_http::HttpError::Connect("10.0.0.1:9999".into()), &u), "curl: Failed to connect to 10.0.0.1:9999");
        assert_eq!(error_message(&ecm_http::HttpError::Resolve("web".into(), "name not found".into()), &u), "curl: Could not resolve host: web (name not found)");
    }
}
