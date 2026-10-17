//! Host-function wrappers for the upload port (abi/child-abi.toml `[imports.upload]`):
//! files from the player's real computer.
//!
//! A program asks for a file with [`begin`]; the player using the computer
//! picks one in their system's file picker (in their game) and its bytes come
//! over the network into the given directory. [`poll`] reports the progress
//! and, at the end, where the file landed. This needs the optional
//! `evanscomputermod-upload` add-on on the server and in the player's game
//! ([`supported`]).

extern crate alloc;

use alloc::string::String;
use alloc::vec;

extern "C" {
    fn upload_supported() -> i32;
    fn upload_begin(dir_ptr: i32, dir_len: i32) -> i32;
    fn upload_poll(ticket: i32, out_ptr: i32, cap: i32) -> i32;
    fn upload_cancel(ticket: i32) -> i32;
}

/// Whether the player using this computer can upload to it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Support {
    /// The server has no upload add-on.
    NotInstalled,
    /// Ready.
    Ready,
    /// The add-on isn't in the game of the player using the computer (or nobody is using it).
    NoClient,
    /// The server doesn't let that player upload.
    NotPermitted,
}

impl Support {
    /// What to tell the player when it isn't [`Support::Ready`].
    pub fn message(self) -> &'static str {
        match self {
            Support::NotInstalled => "file upload isn't installed on this server (it needs the Evans Computer Mod upload add-on, evanscomputermod-upload)",
            Support::Ready => "ready",
            Support::NoClient => "your game doesn't have the upload add-on (evanscomputermod-upload); install it to upload files",
            Support::NotPermitted => "this server doesn't let you upload files",
        }
    }
}

pub fn supported() -> Support {
    match unsafe { upload_supported() } {
        1 => Support::Ready,
        2 => Support::NoClient,
        3 => Support::NotPermitted,
        _ => Support::NotInstalled,
    }
}

/// Why [`begin`] or an upload failed.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The directory doesn't exist.
    NotADirectory,
    /// Another upload into this computer is running.
    Busy,
    /// No upload add-on on the server.
    NotInstalled,
    /// This program may not write there, or the server doesn't let the player upload.
    NotPermitted,
    /// The program was interrupted (Ctrl+T).
    Interrupted,
    /// The player closed the file picker (or the upload was cancelled).
    Cancelled,
    /// The file is larger than the server takes.
    TooLarge,
    /// The computer's storage would go over the server's limit.
    StorageFull,
    /// The transfer broke (the player left or moved away, the file couldn't be read).
    Failed,
    /// Nobody with the add-on in their game is using the computer.
    NoClient,
    /// Any other code.
    Other(i32),
}

impl Error {
    fn from_code(code: i32) -> Error {
        match code {
            -1 => Error::NotADirectory,
            -2 => Error::Busy,
            -4 => Error::NotInstalled,
            -5 => Error::Interrupted,
            -10 => Error::Cancelled,
            -11 => Error::TooLarge,
            -12 => Error::StorageFull,
            -13 => Error::NotPermitted,
            -14 => Error::Failed,
            -15 => Error::NoClient,
            c => Error::Other(c),
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Error::NotADirectory => "no such directory",
            Error::Busy => "another upload to this computer is running",
            Error::NotInstalled => Support::NotInstalled.message(),
            Error::NotPermitted => "not permitted",
            Error::Interrupted => "interrupted",
            Error::Cancelled => "cancelled",
            Error::TooLarge => "the file is larger than this server allows",
            Error::StorageFull => "the computer's storage is full",
            Error::Failed => "the transfer failed",
            Error::NoClient => Support::NoClient.message(),
            Error::Other(_) => "upload error",
        }
    }
}

/// One upload, from [`begin`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Ticket(pub i32);

/// Ask the player using this computer for a file to put in `dir` (for example `/home`).
pub fn begin(dir: &str) -> Result<Ticket, Error> {
    let rc = unsafe { upload_begin(dir.as_ptr() as i32, dir.len() as i32) };
    if rc > 0 { Ok(Ticket(rc)) } else { Err(Error::from_code(rc)) }
}

/// Where an upload is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// The player is picking a file.
    Picking,
    /// `received` of `total` bytes are in.
    Receiving { received: u64, total: u64 },
    /// The file is on the computer at this path.
    Done(String),
    /// It ended without a file.
    Failed(Error),
}

/// The state of `t`.
pub fn poll(t: Ticket) -> State {
    let mut buf = vec![0u8; 20 + 512];
    let rc = unsafe { upload_poll(t.0, buf.as_mut_ptr() as i32, buf.len() as i32) };
    let u64_at = |o: usize| u64::from_le_bytes(buf[o..o + 8].try_into().unwrap());
    match rc {
        0 => State::Picking,
        1 => State::Receiving { received: u64_at(0), total: u64_at(8) },
        2 => {
            let n = (u32::from_le_bytes(buf[16..20].try_into().unwrap()) as usize).min(buf.len() - 20);
            State::Done(String::from_utf8_lossy(&buf[20..20 + n]).into_owned())
        }
        c => State::Failed(Error::from_code(c)),
    }
}

/// Stop waiting for `t` (the player's game stops sending).
pub fn cancel(t: Ticket) {
    unsafe {
        upload_cancel(t.0);
    }
}
