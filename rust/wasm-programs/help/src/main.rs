//! `help`: every command and topic under headings; `help <heading>`: the
//! heading's commands with what each does; `help <name>`: a command's
//! summary, usage and first paragraph (from its manual page).

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (w, _) = ecm_man::tty::size();
    match args.first() {
        None => {
            let extra: Vec<String> = ecm_man::local_names().into_iter().filter(|n| ecm_man::builtin(n).is_none()).collect();
            ecm_man::tty::show("help", ecm_man::help_index(w, true, &extra), false);
        }
        Some(name) if ecm_man::category(name).is_some() => {
            let (title, names) = ecm_man::category(name).unwrap_or_default();
            let mut lines = ecm_man::help_category(title, names, w, true, &|n| ecm_man::load(n));
            // A heading that is also a topic page ("help networking").
            if ecm_man::load(name).is_some() {
                lines.push(format!("The {name} topic itself: man {name}"));
            }
            ecm_man::tty::show(&format!("help {name}"), lines, false);
        }
        Some(name) => match ecm_man::load(name) {
            Some(text) => {
                for l in ecm_man::help_topic(name, &text, w, true) {
                    println!("{l}");
                }
            }
            None => {
                println!("help: no help for {name}.");
                let hits = ecm_man::apropos(name, ecm_man::PAGES.iter().map(|(n, t)| (n.to_string(), *t)));
                if !hits.is_empty() {
                    let names: Vec<&str> = hits.iter().take(8).map(|(n, _)| n.as_str()).collect();
                    println!("Related: {}", names.join(", "));
                }
                println!("Type help for the list.");
                std::process::exit(1);
            }
        },
    }
}
