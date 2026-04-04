use std::io::{self, Write};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut no_newline = false;
    let mut interpret_escapes = false;
    let mut text_start = 1;

    // Parse leading flags
    for (i, arg) in args.iter().enumerate().skip(1) {
        match arg.as_str() {
            "-n" => { no_newline = true; text_start = i + 1; }
            "-e" => { interpret_escapes = true; text_start = i + 1; }
            "-ne" | "-en" => { no_newline = true; interpret_escapes = true; text_start = i + 1; }
            _ => break,
        }
    }

    let text = args[text_start..].join(" ");

    if interpret_escapes {
        let mut chars = text.chars();
        let mut output = String::new();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => output.push('\n'),
                    Some('t') => output.push('\t'),
                    Some('r') => output.push('\r'),
                    Some('\\') => output.push('\\'),
                    Some('0') => output.push('\0'),
                    Some(other) => { output.push('\\'); output.push(other); }
                    None => output.push('\\'),
                }
            } else {
                output.push(c);
            }
        }
        print!("{}", output);
    } else {
        print!("{}", text);
    }

    if !no_newline {
        println!();
    }
    let _ = io::stdout().flush();
}
