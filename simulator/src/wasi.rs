//! WASI types and errno constants.

pub const ERRNO_SUCCESS: i32 = 0;
pub const ERRNO_BADF: i32 = 8;
pub const ERRNO_INVAL: i32 = 28;
pub const ERRNO_NOENT: i32 = 44;
pub const ERRNO_NOSYS: i32 = 52;
pub const FILETYPE_CHARACTER_DEVICE: u8 = 2;
pub const FILETYPE_DIRECTORY: u8 = 3;
pub const FILETYPE_REGULAR_FILE: u8 = 4;
pub const PREOPENTYPE_DIR: u8 = 0;
