//! Command line.

/// TCP port for HTTP (the realm handshake of `--auth local` is on PORT+1).
pub const DEFAULT_PORT: u16 = 7800;

pub const USAGE: &str = "Usage: musicd [PORT] [--name NAME] [--library DIR] [--data DIR]\n\
              [--auth local|HOST:PORT --auth-key FINGERPRINT] [--transcode on|off]\n\
              [--public-browse on|off] [--max-kbps N]\n\
       musicd scan|status|key [--library DIR] [--data DIR]\n\
  The music backend: the MP3s in the library folder, with search, playlists,\n\
  likes and streaming for the Music app, over HTTP on PORT (default 7800).\n\
  DIR for the library defaults to /srv/music, the data (index, accounts,\n\
  playlists, cover art, Data saver cache) to /var/music.\n\
  --auth local      its own accounts (the default); sign-in on PORT+1\n\
  --auth HOST:PORT  trust only the realm with --auth-key FINGERPRINT\n\
  --transcode off   serve Original quality only (no Data saver)\n\
  --public-browse off  browsing and search need a sign-in too\n\
  --max-kbps N      warn about files above N kbps (default 192)\n\
  scan    rescan the library now (a running musicd picks it up)\n\
  status  songs, albums, warnings and the running daemon's state\n\
  key     print this backend's key fingerprint\n\
  Start it with 'musicd &' and run 'disco serve &' so Music apps find it.";

/// Which realm's accounts are accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthMode {
    /// musicd runs its own realm (its backend key is the realm key).
    Local,
    /// Tokens from the realm at `endpoint` whose key has fingerprint `fp`
    /// (`public` too when the operator gave the whole public key).
    Remote { endpoint: String, fp: String, public: Option<[u8; 32]> },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Serve,
    Scan,
    Status,
    Key,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opts {
    pub port: u16,
    pub name: String,
    pub library: String,
    pub data: String,
    pub auth: AuthMode,
    pub transcode: bool,
    pub public_browse: bool,
    pub max_kbps: u32,
    pub command: Command,
}

impl Default for Opts {
    fn default() -> Opts {
        Opts {
            port: DEFAULT_PORT,
            name: "Music".into(),
            library: "/srv/music".into(),
            data: "/var/music".into(),
            auth: AuthMode::Local,
            transcode: true,
            public_browse: true,
            max_kbps: ecm_music::library::DEFAULT_MAX_KBPS,
            command: Command::Serve,
        }
    }
}

fn on_off(flag: &str, v: &str) -> Result<bool, String> {
    match v {
        "on" | "yes" | "true" | "1" => Ok(true),
        "off" | "no" | "false" | "0" => Ok(false),
        _ => Err(format!("{flag} takes on or off, not '{v}'")),
    }
}

/// The options; None for --help.
pub fn parse_args(args: &[String]) -> Result<Option<Opts>, String> {
    let mut o = Opts::default();
    let mut auth: Option<String> = None;
    let mut key: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        let value = |i: usize| args.get(i + 1).cloned().ok_or_else(|| format!("{} needs a value", args[i]));
        let a = args[i].as_str();
        match a {
            "-h" | "--help" => return Ok(None),
            "--name" => o.name = ecm_auth::display_name(&value(i)?).map_err(|e| format!("--name: {e}"))?,
            "--library" => o.library = value(i)?.trim_end_matches('/').to_string(),
            "--data" => o.data = value(i)?.trim_end_matches('/').to_string(),
            "--auth" => auth = Some(value(i)?),
            "--auth-key" => key = Some(value(i)?),
            "--transcode" => o.transcode = on_off(a, &value(i)?)?,
            "--public-browse" => o.public_browse = on_off(a, &value(i)?)?,
            "--max-kbps" => o.max_kbps = value(i)?.parse().ok().filter(|n| *n > 0).ok_or("--max-kbps takes a number")?,
            "scan" => o.command = Command::Scan,
            "status" => o.command = Command::Status,
            "key" => o.command = Command::Key,
            p => match p.parse::<u16>() {
                Ok(n) if n > 0 && n < u16::MAX - 1 => o.port = n,
                _ => return Err(format!("bad argument '{p}'")),
            },
        }
        if !matches!(a, "scan" | "status" | "key") && a.starts_with("--") {
            i += 1;
        }
        i += 1;
    }
    o.auth = match auth.as_deref() {
        None | Some("local") => {
            if key.is_some() {
                return Err("--auth-key goes with --auth HOST:PORT".into());
            }
            AuthMode::Local
        }
        Some(ep) => {
            let endpoint = ecm_auth::endpoint(ep).map_err(|e| format!("--auth: {e}"))?;
            let k = key.ok_or("--auth HOST:PORT needs --auth-key FINGERPRINT (authd key prints it)")?;
            let k = k.trim();
            if let Some(public) = ecm_auth::unhex_n::<32>(k) {
                AuthMode::Remote { endpoint, fp: ecm_auth::fingerprint(&public), public: Some(public) }
            } else {
                let fp = ecm_auth::parse_fingerprint(k).ok_or("--auth-key: not a fingerprint (32 hex digits) or public key (64)")?;
                AuthMode::Remote { endpoint, fp, public: None }
            }
        }
    };
    Ok(Some(o))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn arguments() {
        let o = parse_args(&args("7900 --library /srv/m/ --data /var/m --transcode off --public-browse off --max-kbps 160")).unwrap().unwrap();
        assert_eq!((o.port, o.library.as_str(), o.data.as_str(), o.transcode, o.public_browse, o.max_kbps), (7900, "/srv/m", "/var/m", false, false, 160));
        assert_eq!((o.auth.clone(), o.command), (AuthMode::Local, Command::Serve));
        let o = parse_args(&args("--auth 10.0.8.1 --auth-key 3F:A1:9C:00:11:22:33:44:55:66:77:88:99:AA:BB:CC")).unwrap().unwrap();
        assert_eq!(o.auth, AuthMode::Remote { endpoint: "10.0.8.1:7600".into(), fp: "3fa19c00112233445566778899aabbcc".into(), public: None });
        let pk = "11".repeat(32);
        let o = parse_args(&["--auth".into(), "h:1".into(), "--auth-key".into(), pk.clone()]).unwrap().unwrap();
        assert!(matches!(o.auth, AuthMode::Remote { public: Some(_), .. }));
        assert_eq!(parse_args(&args("scan --data /x")).unwrap().unwrap().command, Command::Scan);
        assert_eq!(parse_args(&args("key")).unwrap().unwrap().command, Command::Key);
        assert_eq!(parse_args(&["--name".into(), "Tech Village Radio".into()]).unwrap().unwrap().name, "Tech Village Radio");
        assert!(parse_args(&args("--help")).unwrap().is_none());
        for bad in ["--auth 1.2.3.4", "--auth-key abc", "--transcode maybe", "0", "--max-kbps x", "--auth h:1 --auth-key zz"] {
            assert!(parse_args(&args(bad)).is_err(), "{bad}");
        }
    }
}
