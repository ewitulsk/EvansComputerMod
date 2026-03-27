fn main() {
    println!("Hello from WASI!");
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        println!("Args: {:?}", &args[1..]);
    }
}
