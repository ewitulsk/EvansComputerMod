//! The registrar's web pages (http://nic.ring/, docs/DNS.md "Registering a name"), made
//! for the in-game Browser (docs/gui/BROWSER.md, phase 4): a styled page with labelled
//! fields, a web login kept in a session cookie (so the forms after it only ask for the
//! domain), the account's domains with a form to add records to each, and result pages.
//! The plain-text API (`POST /api`, what the `domain` program uses) is unchanged.

use std::collections::BTreeMap;

/// HTML-escape (`sys::esc` lives in the wasm-only half).
pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The session cookie's name.
pub const COOKIE: &str = "nic_session";

/// Web logins: token -> (handle, password). In memory: a restart logs everyone out.
#[derive(Default, Debug)]
pub struct Sessions {
    map: BTreeMap<String, (String, String)>,
}

impl Sessions {
    pub fn start(&mut self, token: &str, handle: &str, password: &str) {
        self.map.insert(token.to_string(), (handle.to_string(), password.to_string()));
    }

    /// The login a `Cookie:` header carries.
    pub fn get(&self, cookie_header: &str) -> Option<&(String, String)> {
        self.map.get(&token_of(cookie_header)?)
    }

    pub fn end(&mut self, cookie_header: &str) {
        if let Some(t) = token_of(cookie_header) {
            self.map.remove(&t);
        }
    }
}

/// The session token in a `Cookie:` header.
pub fn token_of(cookie_header: &str) -> Option<String> {
    cookie_header.split(';').find_map(|c| {
        let (k, v) = c.split_once('=')?;
        (k.trim() == COOKIE && !v.trim().is_empty()).then(|| v.trim().to_string())
    })
}

/// The `Set-Cookie:` value for a session (empty token: log out).
pub fn set_cookie(token: &str) -> String {
    if token.is_empty() {
        format!("{COOKIE}=; Path=/; Max-Age=0")
    } else {
        format!("{COOKIE}={token}; Path=/")
    }
}

const STYLE: &str = "body { margin: 0 }
header { display: flex; align-items: center; gap: 12px; padding: 8px 16px; background: #2f5f9e; color: #ffffff }
header a { color: #ffffff } header .who { margin-left: auto }
main { padding: 4px 16px 16px }
.card { border: 1px solid #7a8496; border-radius: 6px; padding: 6px 12px; margin: 8px 0 }
.card h3 { margin: 4px 0 }
form { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; margin: 4px 0 }
.ok { color: #2e8b57 } .err { color: #d04040 }
.hint { opacity: 0.75 }
pre { white-space: pre-wrap }";

/// A page: the header bar (with who is logged in), the title and the body.
pub fn page(title: &str, who: Option<&str>, body: &str) -> String {
    let login = match who {
        Some(h) => format!(
            "<span class=\"who\">Logged in as <b>{}</b></span><form method=\"post\" action=\"/web\"><input type=\"hidden\" name=\"op\" value=\"logout\"><button>Log out</button></form>",
            esc(h)
        ),
        None => String::new(),
    };
    format!(
        "<!DOCTYPE html>\n<html><head><title>{t} - nic.ring</title><style>{STYLE}</style></head>\n<body>\n\
         <header><b>nic.ring</b><a href=\"/\">home</a>{login}</header>\n<main>\n<h2>{t}</h2>\n{body}\n</main>\n</body></html>\n",
        t = esc(title)
    )
}

fn field(label: &str, name: &str, kind: &str, value: &str, placeholder: &str) -> String {
    format!(
        "<label>{label} <input name=\"{name}\" type=\"{kind}\" value=\"{}\" placeholder=\"{}\"></label>",
        esc(value),
        esc(placeholder)
    )
}

fn hidden(name: &str, value: &str) -> String {
    format!("<input type=\"hidden\" name=\"{name}\" value=\"{}\">", esc(value))
}

/// The record form for one domain.
fn record_form(d: &str) -> String {
    format!(
        "<form method=\"post\" action=\"/web\">{}{}{}<label>type <select name=\"type\"><option>A</option><option>CNAME</option><option>TXT</option><option>MX</option><option>AAAA</option></select></label>{}{}<button>Add the record</button></form>",
        hidden("op", "set"),
        hidden("domain", d),
        field("name", "name", "text", "www", "www"),
        field("points at", "data", "text", "", "an IPv4 address"),
        field("TTL", "ttl", "text", "300", "300"),
    )
}

/// The home page: logged out, search, create an account, log in; logged in, register and
/// the account's domains.
pub fn home(tld: &str, who: Option<&str>, domains: &[String]) -> String {
    let mut b = format!(
        "<p>The registrar of the Tech Village ring: register a <code>.{tld}</code> name and point it at your own computer, \
         or at your own nameserver. The .{tld} registry runs in another village; this registrar sends it your changes.</p>\n\
         <div class=\"card\"><h3>Is it free?</h3><form method=\"get\" action=\"/search\">{}<button>Search</button></form></div>\n",
        field("name", "domain", "text", "", "a name to look up")
    );
    match who {
        None => {
            b.push_str(&format!(
                "<div class=\"card\"><h3>New here? Create an account</h3><form method=\"post\" action=\"/web\">{}{}{}<button>Create account</button></form></div>\n\
                 <div class=\"card\"><h3>Log in</h3><form method=\"post\" action=\"/web\">{}{}{}<button>Log in</button></form></div>\n",
                hidden("op", "account"),
                field("handle", "handle", "text", "", "pick a handle"),
                field("password", "password", "password", "", "pick a password"),
                hidden("op", "login"),
                field("handle", "handle", "text", "", "your handle"),
                field("password", "password", "password", "", "your password"),
            ));
        }
        Some(_) => {
            b.push_str(&format!(
                "<div class=\"card\"><h3>Register a name</h3><form method=\"post\" action=\"/web\">{}{}<button>Register</button></form></div>\n",
                hidden("op", "register"),
                field("domain", "domain", "text", "", "a name to register"),
            ));
            if domains.is_empty() {
                b.push_str("<p class=\"hint\">No domains yet.</p>\n");
            }
            for d in domains {
                b.push_str(&format!(
                    "<div class=\"card\"><h3>{}</h3>{}<form method=\"post\" action=\"/web\">{}{}<button>Show its records</button></form></div>\n",
                    esc(d),
                    record_form(d),
                    hidden("op", "show"),
                    hidden("domain", d)
                ));
            }
        }
    }
    b.push_str(&format!(
        "<div class=\"card\"><h3>From a terminal</h3><pre>domain account create HANDLE PASSWORD\ndomain register mybase.{tld}\n\
         domain set mybase.{tld} www A 100.70.0.120\ndomain ns mybase.{tld} ns1.mybase.{tld}=10.200.0.53\ndomain show mybase.{tld}\n\
         whois mybase.{tld}</pre></div>\n<p class=\"hint\">Rules: names are 3-63 letters, digits and hyphens; some are reserved \
         (nic, www, root, v1-v10, ...); five per account; TTLs of at least 60 seconds.</p>"
    ));
    page("Register a .ecm name", who, &b)
}

/// The page after a form: the API's answer (`OK ...` / `ERR ...`), in words.
pub fn result(op: &str, out: &str, who: Option<&str>) -> String {
    let (ok, text) = match out.split_once(' ') {
        Some(("OK", t)) => (true, t),
        Some(("ERR", t)) => (false, t),
        _ => (!out.starts_with("ERR"), out),
    };
    let title = match (op, ok) {
        (_, false) => "That didn't work",
        ("register", true) => "Registered",
        ("set", true) => "Record added",
        ("check", true) => "Search",
        ("show", true) => "Records",
        (_, true) => "Done",
    };
    // A search says it in words.
    let said;
    let text = match (op, ok, text.rsplit_once(' ')) {
        ("check", true, Some((d, "available"))) => {
            said = format!("{d} is free. Log in (or create an account), then register it.");
            said.as_str()
        }
        ("check", true, Some((d, "taken"))) => {
            said = format!("{d} is already registered (whois {d} says to whom).");
            said.as_str()
        }
        _ => text,
    };
    let class = if ok { "ok" } else { "err" };
    let body = if text.contains('\n') {
        format!("<pre class=\"{class}\">{}</pre>", esc(text))
    } else {
        format!("<p class=\"{class}\">{}</p>", esc(text))
    };
    page(title, who, &format!("{body}\n<p><a href=\"/\">Back to nic.ring</a></p>"))
}

/// The account's domains from the API's `list` answer (`OK n domain(s)` then one per line).
pub fn domains_of_list(out: &str) -> Vec<String> {
    if !out.starts_with("OK") {
        return Vec::new();
    }
    out.lines().skip(1).map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_and_cookies() {
        let mut s = Sessions::default();
        s.start("abc123", "carol", "pw1234");
        assert_eq!(token_of("other=1; nic_session=abc123"), Some("abc123".into()));
        assert_eq!(s.get("nic_session=abc123"), Some(&("carol".to_string(), "pw1234".to_string())));
        assert!(s.get("nic_session=wrong").is_none());
        assert!(s.get("").is_none());
        s.end("nic_session=abc123");
        assert!(s.get("nic_session=abc123").is_none());
        assert_eq!(set_cookie("t"), "nic_session=t; Path=/");
        assert!(set_cookie("").contains("Max-Age=0"));
    }

    #[test]
    fn pages() {
        let out = home("ecm", None, &[]);
        assert!(out.contains("Create account") && out.contains("Log in") && out.contains("placeholder=\"your handle\""));
        assert!(!out.contains("Register a name"), "registering needs a login");
        let me = home("ecm", Some("carol"), &["mybase.ecm".into()]);
        assert!(me.contains("Logged in as <b>carol</b>") && me.contains("Register a name") && me.contains("<h3>mybase.ecm</h3>"));
        assert!(me.contains("name=\"domain\" value=\"mybase.ecm\""), "the record form names its domain");
        assert!(!me.contains("Create account"));
        let r = result("register", "OK registered mybase.ecm to carol", Some("carol"));
        assert!(r.contains("<title>Registered - nic.ring</title>") && r.contains("class=\"ok\""));
        let e = result("register", "ERR mybase.ecm is already registered", Some("carol"));
        assert!(e.contains("That didn't work") && e.contains("class=\"err\"") && e.contains("already registered"));
        assert!(result("x", "OK <script>", None).contains("&lt;script&gt;"));
        assert!(result("check", "OK mybase.ecm available", None).contains("mybase.ecm is free."));
        assert!(result("check", "OK mybase.ecm taken", None).contains("mybase.ecm is already registered"));
        assert_eq!(domains_of_list("OK 2 domain(s)\nmybase.ecm\nother.ecm"), vec!["mybase.ecm", "other.ecm"]);
        assert!(domains_of_list("ERR wrong").is_empty());
    }
}
