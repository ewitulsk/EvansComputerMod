fn main() {
    match std::env::current_dir() {
        Ok(path) => {
            let p = path.display().to_string();
            if p.is_empty() || p == "." {
                println!("/");
            } else {
                println!("{}", p);
            }
        }
        Err(_) => println!("/"),
    }
}
