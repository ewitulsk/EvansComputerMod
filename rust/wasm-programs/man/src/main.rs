//! `man`: show a manual page in the pager (`ecm-man`). `man -k word`
//! searches, `man -l` lists every page, `-P` prints without the pager.

fn usage() -> ! {
    eprintln!("Usage: man <name> | man -k <word> | man -l   (-P: no pager; try: help, man intro)");
    std::process::exit(1);
}

fn all_pages() -> Vec<(String, String)> {
    let mut pages: Vec<(String, String)> = ecm_man::local_names()
        .into_iter()
        .filter_map(|n| ecm_man::load(&n).map(|t| (n, t)))
        .collect();
    for n in ecm_man::names() {
        if !pages.iter().any(|(p, _)| p == n) {
            pages.push((n.to_string(), ecm_man::builtin(n).unwrap_or_default().to_string()));
        }
    }
    pages.sort();
    pages
}

fn list(hits: &[(String, String)]) -> Vec<String> {
    let w = hits.iter().map(|(n, _)| n.len()).max().unwrap_or(0);
    hits.iter().map(|(n, s)| format!("{n:<w$}  {s}")).collect()
}

fn main() {
    let mut plain = false;
    let mut search = false;
    let mut all = false;
    let mut name = None;
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "-P" | "--no-pager" => plain = true,
            "-k" => search = true,
            "-l" => all = true,
            "-h" | "--help" => usage(),
            _ if name.is_none() => name = Some(a),
            _ => usage(),
        }
    }
    let (w, _) = ecm_man::tty::size();
    let ansi = !plain;
    if all {
        let pages = all_pages();
        let hits: Vec<(String, String)> = pages.iter().map(|(n, t)| (n.clone(), ecm_man::summary(t).unwrap_or_default())).collect();
        ecm_man::tty::show("pages", list(&hits), plain);
        return;
    }
    let Some(name) = name else { usage() };
    if search {
        let pages = all_pages();
        let hits = ecm_man::apropos(&name, pages.iter().map(|(n, t)| (n.clone(), t.as_str())));
        if hits.is_empty() {
            println!("{name}: nothing appropriate.");
            std::process::exit(1);
        }
        ecm_man::tty::show(&format!("man -k {name}"), list(&hits), plain);
        return;
    }
    match ecm_man::load(&name) {
        Some(text) => ecm_man::tty::show(&format!("man {name}"), ecm_man::render(&text, w, ansi), plain),
        None => {
            println!("No manual entry for {name}.");
            let pages = all_pages();
            let near = ecm_man::apropos(&name, pages.iter().map(|(n, t)| (n.clone(), t.as_str())));
            if !near.is_empty() {
                let names: Vec<&str> = near.iter().take(8).map(|(n, _)| n.as_str()).collect();
                println!("Related pages: {}", names.join(", "));
            }
            println!("Type help for the list of pages.");
            std::process::exit(1);
        }
    }
}
