//! Redstone by side name, as ComputerCraft programs use it (`rs`/`redstone`):
//! "top", "bottom", "front", "back", "left", "right", relative to the
//! computer (left and right as you face its screen, as for `shell`'s sides).

/// CC's side names, in the order `rs.getSides()` lists them.
pub const SIDES: [&str; 6] = ["top", "bottom", "left", "right", "front", "back"];

/// The host's side number (shell.DOWN 0, UP 1, FRONT 2, BACK 3, LEFT 4, RIGHT 5) of a side name.
pub fn side(name: &str) -> Result<i32, String> {
    Ok(match name {
        "bottom" => 0,
        "top" => 1,
        "front" => 2,
        "back" => 3,
        "left" => 4,
        "right" => 5,
        other => return Err(format!("bad argument #1 (unknown side '{other}')")),
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn side_names() {
        assert_eq!(super::side("top"), Ok(1));
        assert_eq!(super::side("right"), Ok(5));
        assert!(super::side("up").is_err());
    }
}
