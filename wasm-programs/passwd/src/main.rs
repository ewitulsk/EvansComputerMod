extern crate ecm_host_abi;

use std::io::{self, Write, BufRead};

fn main() {
    print!("New password: ");
    io::stdout().flush().unwrap();
    let mut password = String::new();
    io::stdin().lock().read_line(&mut password).unwrap();
    let password = password.trim_end();

    if password.is_empty() {
        println!("Password not changed.");
        return;
    }

    print!("Confirm password: ");
    io::stdout().flush().unwrap();
    let mut confirm = String::new();
    io::stdin().lock().read_line(&mut confirm).unwrap();
    let confirm = confirm.trim_end();

    if password == confirm {
        ecm_host_abi::auth::set_password("root", password);
        println!("Password updated.");
    } else {
        println!("Passwords don't match.");
    }
}
