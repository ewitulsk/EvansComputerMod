fn main() {
    // Test 1: std::fs::write (same as edit)
    let content = "Hello from fd_test!\nLine 2\n";
    match std::fs::write("fd_test_output.txt", content) {
        Ok(_) => println!("std::fs::write: OK"),
        Err(e) => println!("std::fs::write: FAIL: {}", e),
    }

    // Test 2: Read back
    match std::fs::read_to_string("fd_test_output.txt") {
        Ok(s) => {
            println!("Read back {} bytes: [{}]", s.len(), s.trim());
        }
        Err(e) => println!("Read back: FAIL: {}", e),
    }

    // Test 3: Overwrite with shorter content (tests truncation)
    match std::fs::write("fd_test_output.txt", "short") {
        Ok(_) => println!("Overwrite: OK"),
        Err(e) => println!("Overwrite: FAIL: {}", e),
    }

    match std::fs::read_to_string("fd_test_output.txt") {
        Ok(s) => println!("After overwrite: [{}] ({} bytes)", s.trim(), s.len()),
        Err(e) => println!("After overwrite read: FAIL: {}", e),
    }
}
