//! `/etc/celld.toml`: what the phone's owner can set. A small TOML subset
//! (`key = value` lines, `#` comments; strings, booleans, integer arrays),
//! parsed strictly so a typo is reported with its line instead of ignored.
//!
//! ```toml
//! # celld: the phone's cellular daemon
//! modem = "module_1"      # the modem's framed radio (default: the first framed_radio)
//! sim = "sim"             # the SIM peripheral (default: <modem>_sim, else the first `sim`)
//! carrier = "cell-red"    # carrier lock: only a SIM of this carrier is used
//! channels = [1, 3]       # channels to search (default: every channel of the modem)
//! roaming = false         # data roaming (see Config::roaming)
//! autoconnect = true      # attach when celld starts (false: wait for `cell up`)
//! ```

pub const PATH: &str = "/etc/celld.toml";

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub modem: Option<String>,
    pub sim: Option<String>,
    /// A carrier lock: a SIM of another carrier is refused (state `locked`).
    pub carrier: Option<String>,
    pub channels: Vec<u8>,
    /// Data roaming. Cellular v2 cores have no roaming agreements, so a phone
    /// only ever attaches to cells of its SIM's carrier either way; the switch
    /// is kept (and shown) for when cores gain partner networks.
    pub roaming: bool,
    pub autoconnect: bool,
}

impl Default for Config {
    fn default() -> Config {
        Config { modem: None, sim: None, carrier: None, channels: Vec::new(), roaming: false, autoconnect: true }
    }
}

fn unquote(v: &str, line: usize, key: &str) -> Result<String, String> {
    let v = v.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        let inner = &v[1..v.len() - 1];
        if inner.contains('"') || inner.contains('\\') {
            return Err(format!("line {line}: {key}: escapes are not supported"));
        }
        Ok(inner.to_string())
    } else {
        Err(format!("line {line}: {key} must be a quoted string"))
    }
}

fn boolean(v: &str, line: usize, key: &str) -> Result<bool, String> {
    match v.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("line {line}: {key} must be true or false")),
    }
}

fn channels(v: &str, line: usize) -> Result<Vec<u8>, String> {
    let v = v.trim();
    let inner = v.strip_prefix('[').and_then(|r| r.strip_suffix(']')).ok_or_else(|| format!("line {line}: channels must be a list like [1, 3]"))?;
    let mut out = Vec::new();
    for p in inner.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        match p.parse::<u8>() {
            Ok(c) if (1..=16).contains(&c) => out.push(c),
            _ => return Err(format!("line {line}: channels: '{p}' is not a channel number")),
        }
    }
    Ok(out)
}

/// Strip a `#` comment that is not inside a string.
fn strip_comment(l: &str) -> &str {
    let mut quoted = false;
    for (i, c) in l.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '#' if !quoted => return &l[..i],
            _ => {}
        }
    }
    l
}

impl Config {
    pub fn parse(text: &str) -> Result<Config, String> {
        let mut c = Config::default();
        for (i, raw) in text.lines().enumerate() {
            let n = i + 1;
            let l = strip_comment(raw).trim();
            if l.is_empty() {
                continue;
            }
            let Some((k, v)) = l.split_once('=') else {
                return Err(format!("line {n}: expected key = value"));
            };
            let k = k.trim();
            match k {
                "modem" => c.modem = Some(unquote(v, n, k)?),
                "sim" => c.sim = Some(unquote(v, n, k)?),
                "carrier" => {
                    let s = unquote(v, n, k)?;
                    if !ecm_cell::simauth::valid_carrier(&s) {
                        return Err(format!("line {n}: carrier '{s}' is not a carrier id ([a-z0-9_-], up to 24)"));
                    }
                    c.carrier = Some(s);
                }
                "channels" => c.channels = channels(v, n)?,
                "roaming" => c.roaming = boolean(v, n, k)?,
                "autoconnect" => c.autoconnect = boolean(v, n, k)?,
                _ => return Err(format!("line {n}: unknown key '{k}' (modem, sim, carrier, channels, roaming, autoconnect)")),
            }
        }
        Ok(c)
    }
}

/// `text` with `key = value` set (the line replaced, or appended). `value` is
/// written as given (quote strings yourself).
pub fn set(text: &str, key: &str, value: &str) -> String {
    let mut out = String::new();
    let mut done = false;
    for l in text.lines() {
        let k = strip_comment(l).split_once('=').map(|(k, _)| k.trim());
        if !done && k == Some(key) {
            out.push_str(&format!("{key} = {value}\n"));
            done = true;
        } else {
            out.push_str(l);
            out.push('\n');
        }
    }
    if !done {
        out.push_str(&format!("{key} = {value}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_key_and_comments() {
        let c = Config::parse(
            "# celld\nmodem = \"module_1\"   # the modem\nsim = \"sim\"\ncarrier = \"cell-red\"\nchannels = [1, 3]\nroaming = true\nautoconnect = false\n",
        )
        .unwrap();
        assert_eq!(c.modem.as_deref(), Some("module_1"));
        assert_eq!(c.sim.as_deref(), Some("sim"));
        assert_eq!(c.carrier.as_deref(), Some("cell-red"));
        assert_eq!(c.channels, vec![1, 3]);
        assert!(c.roaming);
        assert!(!c.autoconnect);
        assert_eq!(Config::parse("").unwrap(), Config::default());
        assert!(Config::default().autoconnect && !Config::default().roaming);
    }

    #[test]
    fn errors_name_the_line() {
        assert_eq!(Config::parse("\nroaming = yes").unwrap_err(), "line 2: roaming must be true or false");
        assert!(Config::parse("colour = \"red\"").unwrap_err().contains("unknown key 'colour'"));
        assert!(Config::parse("modem = module_1").unwrap_err().contains("quoted string"));
        assert!(Config::parse("channels = [1, x]").unwrap_err().contains("'x'"));
        assert!(Config::parse("channels = 1").unwrap_err().contains("list"));
        assert!(Config::parse("carrier = \"Cell Red\"").unwrap_err().contains("carrier id"));
        assert!(Config::parse("just words").unwrap_err().contains("key = value"));
    }

    #[test]
    fn set_replaces_or_appends() {
        let t = "# mine\nroaming = false # off\nchannels = [1]\n";
        let t2 = set(t, "roaming", "true");
        assert_eq!(t2, "# mine\nroaming = true\nchannels = [1]\n");
        assert!(Config::parse(&t2).unwrap().roaming);
        let t3 = set("", "roaming", "true");
        assert_eq!(t3, "roaming = true\n");
        let t4 = set("modem = \"a#b\"\n", "sim", "\"sim\"");
        assert_eq!(Config::parse(&t4).unwrap().modem.as_deref(), Some("a#b"));
    }
}
