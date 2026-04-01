//! Authentication host function wrappers.

extern "C" {
    fn ipc_auth_set_password(user_ptr: i32, user_len: i32, pass_ptr: i32, pass_len: i32) -> i32;
}

/// Set the password for a user. Returns 0 on success, -1 on error.
pub fn set_password(username: &str, password: &str) -> i32 {
    unsafe {
        ipc_auth_set_password(
            username.as_ptr() as i32, username.len() as i32,
            password.as_ptr() as i32, password.len() as i32,
        )
    }
}
