use super::*;

/// Programs that are tests or internals, not commands a player looks up.
const NO_PAGE: &[&str] = &["apptest", "crypto_test", "fd_test", "hello", "phonetest", "rng_test", "test-shell", "tty_test", "uppercase"];

#[test]
fn every_page_is_well_formed_and_listed_once() {
    let mut listed: Vec<&str> = CATEGORIES.iter().flat_map(|(_, n)| n.iter().copied()).collect();
    for (name, text) in PAGES {
        assert!(summary(text).is_some(), "{name}: NAME must be 'name - summary'");
        let first = text.lines().nth(1).unwrap_or("").trim();
        assert!(first.starts_with(&format!("{name} - ")), "{name}: NAME line names another page: {first}");
        for (i, l) in text.lines().enumerate() {
            assert!(l.is_ascii(), "{name}:{}: not ASCII", i + 1);
            assert!(l.trim_end().len() <= 76, "{name}:{}: longer than 76", i + 1);
        }
        assert_eq!(listed.iter().filter(|n| *n == name).count(), 1, "{name} must be in exactly one help category");
        listed.retain(|n| n != name);
    }
    assert!(listed.is_empty(), "categories list pages that don't exist: {listed:?}");
}

#[test]
fn every_command_has_a_page() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../wasm-programs");
    let mut missing = Vec::new();
    for e in std::fs::read_dir(dir).unwrap() {
        let d = e.unwrap().file_name().to_string_lossy().into_owned();
        let page = match d.as_str() {
            "ssh-client" => "ssh",
            "terminal" => "terminal-app",
            other => other,
        };
        if !NO_PAGE.contains(&page) && builtin(page).is_none() {
            missing.push(page.to_string());
        }
    }
    for b in ["cd", "exit", "ps", "kill", "jobs", "fg", "bg", "gfxtest", "switch", "router", "history"] {
        if builtin(b).is_none() {
            missing.push(b.to_string());
        }
    }
    assert!(missing.is_empty(), "commands without a manual page: {missing:?}");
}

#[test]
fn see_also_points_at_pages_that_exist() {
    let mut bad = Vec::new();
    for (name, text) in PAGES {
        if let Some(lines) = section(text, "SEE ALSO") {
            for w in lines.join(" ").split(',') {
                let w = w.trim().trim_end_matches('.');
                if !w.is_empty() && builtin(w).is_none() {
                    bad.push(format!("{name} -> {w}"));
                }
            }
        }
    }
    assert!(bad.is_empty(), "SEE ALSO names missing pages: {bad:?}");
}

const SAMPLE: &str = "NAME
    demo - a demo page

SYNOPSIS
    demo [-v] file

    demo --list

DESCRIPTION
    This paragraph is written over
    two lines and gets joined.

OPTIONS
    -v
        Verbose output, which is a longer line of text to wrap.

EXAMPLES
      demo -v a.txt
";

#[test]
fn render_rewraps_paragraphs_and_keeps_verbatim_lines() {
    let r = render(SAMPLE, 80, false);
    assert!(r.contains(&"    This paragraph is written over two lines and gets joined.".to_string()), "{r:#?}");
    assert!(r.contains(&"    demo [-v] file".to_string()));
    assert!(r.contains(&"    demo --list".to_string()), "synopsis lines stay apart");
    assert!(r.contains(&"      demo -v a.txt".to_string()));
    // 30 columns (a phone): paragraphs wrap, nothing is wider than the screen.
    let r = render(SAMPLE, 30, false);
    assert!(r.iter().all(|l| l.len() <= 30), "{r:#?}");
    assert!(r.contains(&"        Verbose output, which".to_string()), "{r:#?}");
    // Headings bold with ANSI.
    assert!(render(SAMPLE, 80, true).contains(&"\x1b[1mOPTIONS\x1b[0m".to_string()));
}

#[test]
fn help_index_topic_and_apropos() {
    let idx = help_index(80, false, &["mytool".into()]);
    assert!(idx.contains(&"NETWORKING".to_string()));
    assert!(idx.iter().any(|l| l.contains("ifconfig") && l.starts_with("  ")));
    assert_eq!(idx.last().unwrap().trim(), "mytool");
    let t = help_topic("demo", SAMPLE, 80, false);
    assert_eq!(t[0], "demo - a demo page");
    assert!(t.contains(&"  demo [-v] file".to_string()));
    assert!(t.contains(&"This paragraph is written over two lines and gets joined.".to_string()));
    assert_eq!(t.last().unwrap(), "More: man demo");
    let hits = apropos("DNS", PAGES.iter().map(|(n, t)| (n.to_string(), *t)));
    assert!(hits.iter().any(|(n, _)| n == "dig"), "{hits:?}");
}

#[test]
fn pager_moves_searches_and_quits() {
    let lines: Vec<String> = (1..=100).map(|i| format!("line {i}")).collect();
    let mut p = Pager::new("test", lines);
    let h = 25;
    p.key(Key::Char(b' '), h);
    assert_eq!(p.top, 24);
    p.key(Key::Char(b'b'), h);
    assert_eq!(p.top, 0);
    p.key(Key::End, h);
    assert_eq!(p.top, 76);
    assert!(p.draw(80, h).contains("(END)"));
    p.key(Key::Home, h);
    for k in KeyReader::default().feed(b"/line 50\r") {
        p.key(k, h);
    }
    assert_eq!(p.top, 49);
    assert!(p.draw(80, h).contains("\x1b[7mline 50\x1b[0m"));
    for k in KeyReader::default().feed(b"/nothing\r") {
        p.key(k, h);
    }
    assert!(p.draw(80, h).contains("Pattern not found: nothing"));
    assert_eq!(p.key(Key::Escape, h), Action::Redraw, "Esc doesn't quit");
    assert_eq!(p.key(Key::CtrlC, h), Action::Quit);
    assert_eq!(p.key(Key::Char(b'q'), h), Action::Quit);
    let mut r = KeyReader::default();
    assert_eq!(r.feed(b"\x1b"), vec![Key::Escape]);
    assert_eq!(r.feed(b"\x1b[5~j"), vec![Key::PageUp, Key::Char(b'j')]);
    assert!(r.feed(b"x\x1b[").len() == 1);
    assert_eq!(r.feed(b"B"), vec![Key::Down]);
}

#[test]
fn help_heading_lists_its_commands_with_summaries() {
    let (title, names) = category("networking").unwrap();
    assert_eq!(title, "Networking");
    assert_eq!(category("NAMES").unwrap().0, "Names and the web");
    assert_eq!(category("names and the web").unwrap().0, "Names and the web");
    assert!(category("nothing").is_none());
    let out = help_category(title, names, 80, false, &|n| builtin(n).map(str::to_string));
    assert_eq!(out[0], "NETWORKING");
    assert!(out.iter().any(|l| l.starts_with("  ping") && l.contains("check that another computer answers")), "{out:#?}");
    let narrow = help_category(title, names, 30, false, &|n| builtin(n).map(str::to_string));
    assert!(narrow.iter().all(|l| l.len() <= 30), "{narrow:#?}");
}
