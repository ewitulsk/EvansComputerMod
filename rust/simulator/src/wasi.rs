//! WASI types and errno constants.

pub const ERRNO_SUCCESS: i32 = 0;
pub const ERRNO_2BIG: i32 = 1;
pub const ERRNO_ACCES: i32 = 2;
pub const ERRNO_BADF: i32 = 8;
pub const ERRNO_EXIST: i32 = 20;
pub const ERRNO_INVAL: i32 = 28;
pub const ERRNO_ISDIR: i32 = 31;
pub const ERRNO_NOENT: i32 = 44;
pub const ERRNO_NOSYS: i32 = 52;
pub const ERRNO_NOTDIR: i32 = 54;
pub const ERRNO_NOTEMPTY: i32 = 55;

pub const FILETYPE_UNKNOWN: u8 = 0;
pub const FILETYPE_CHARACTER_DEVICE: u8 = 2;
pub const FILETYPE_DIRECTORY: u8 = 3;
pub const FILETYPE_REGULAR_FILE: u8 = 4;
pub const FILETYPE_SYMBOLIC_LINK: u8 = 7;
pub const PREOPENTYPE_DIR: u8 = 0;

// WASI oflags bits
pub const OFLAG_CREAT: i32 = 1;
pub const OFLAG_DIRECTORY: i32 = 2;
pub const OFLAG_EXCL: i32 = 4;
pub const OFLAG_TRUNC: i32 = 8;

// WASI fdflags bits
pub const FDFLAG_APPEND: i32 = 1;
pub const FDFLAG_DSYNC: i32 = 2;
pub const FDFLAG_NONBLOCK: i32 = 4;
pub const FDFLAG_RSYNC: i32 = 8;
pub const FDFLAG_SYNC: i32 = 16;

// WASI rights bits (selected)
pub const RIGHT_FD_READ: u64 = 1 << 1;
pub const RIGHT_FD_WRITE: u64 = 1 << 6;
