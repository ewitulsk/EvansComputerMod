use std::io::{self, Read};

fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap_or(0);
    let lines = input.lines().count();
    let words = input.split_whitespace().count();
    let bytes = input.len();
    println!("  {} {} {}", lines, words, bytes);
}
