//! The computer's secret store (abi/child-abi.toml `[imports.keystore]`,
//! docs/REPO.md "Secret store").
//!
//! Secrets live with the server, outside the computer's files, so no program
//! can read them off the disk. Each belongs to the player who stored it and is
//! only handed to the system's own programs (the ones in `/server-bin`): a
//! downloaded program, or a copy of `pkg` dropped into `/bin`, is refused.
//! Signing keys never leave the store at all: a program asks for a signature.
//!
//! Not every host has one (the simulator does not): every call then says
//! [`Error::NoStore`] and the caller falls back to a file.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

#[cfg(target_arch = "wasm32")]
extern "C" {
    fn keystore_get(name_ptr: i32, name_len: i32, out_ptr: i32, cap: i32) -> i32;
    fn keystore_put(name_ptr: i32, name_len: i32, val_ptr: i32, val_len: i32) -> i32;
    fn keystore_delete(name_ptr: i32, name_len: i32) -> i32;
    fn keystore_keygen(name_ptr: i32, name_len: i32, pub_out: i32) -> i32;
    fn keystore_public(name_ptr: i32, name_len: i32, pub_out: i32) -> i32;
    fn keystore_sign(name_ptr: i32, name_len: i32, msg_ptr: i32, msg_len: i32, sig_out: i32) -> i32;
    fn keystore_confirm(text_ptr: i32, text_len: i32, timeout_ms: i32) -> i32;
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// This host has no secret store.
    NoStore,
    /// Nothing is stored under that name (for the player using the computer).
    NotFound,
    /// The calling program is not one of the system's own, or nobody is using the computer.
    NotPermitted,
    /// The player said no, or did not answer.
    Declined,
    Other(i32),
}

impl Error {
    #[allow(dead_code)]
    fn from_code(c: i32) -> Error {
        match c {
            -52 | 52 | -4 => Error::NoStore,
            -2 => Error::NotFound,
            -13 => Error::NotPermitted,
            -10 => Error::Declined,
            c => Error::Other(c),
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Error::NoStore => "this computer has no secret store",
            Error::NotFound => "nothing is stored under that name",
            Error::NotPermitted => "the secret store only answers the system's own programs, with a player at the computer",
            Error::Declined => "not confirmed",
            Error::Other(_) => "the secret store failed",
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use super::*;

    pub fn get(name: &str) -> Result<Vec<u8>, Error> {
        let mut buf = alloc::vec![0u8; 4096];
        let n = unsafe { keystore_get(name.as_ptr() as i32, name.len() as i32, buf.as_mut_ptr() as i32, buf.len() as i32) };
        if n < 0 || n == 52 {
            return Err(Error::from_code(n));
        }
        buf.truncate((n as usize).min(4096));
        Ok(buf)
    }

    pub fn put(name: &str, value: &[u8]) -> Result<(), Error> {
        let rc = unsafe { keystore_put(name.as_ptr() as i32, name.len() as i32, value.as_ptr() as i32, value.len() as i32) };
        if rc == 0 { Ok(()) } else { Err(Error::from_code(rc)) }
    }

    pub fn delete(name: &str) -> Result<(), Error> {
        let rc = unsafe { keystore_delete(name.as_ptr() as i32, name.len() as i32) };
        if rc == 0 { Ok(()) } else { Err(Error::from_code(rc)) }
    }

    pub fn keygen(name: &str) -> Result<[u8; 32], Error> {
        let mut out = [0u8; 32];
        let rc = unsafe { keystore_keygen(name.as_ptr() as i32, name.len() as i32, out.as_mut_ptr() as i32) };
        if rc == 0 { Ok(out) } else { Err(Error::from_code(rc)) }
    }

    pub fn public(name: &str) -> Result<[u8; 32], Error> {
        let mut out = [0u8; 32];
        let rc = unsafe { keystore_public(name.as_ptr() as i32, name.len() as i32, out.as_mut_ptr() as i32) };
        if rc == 0 { Ok(out) } else { Err(Error::from_code(rc)) }
    }

    pub fn sign(name: &str, message: &[u8]) -> Result<[u8; 64], Error> {
        let mut out = [0u8; 64];
        let rc = unsafe { keystore_sign(name.as_ptr() as i32, name.len() as i32, message.as_ptr() as i32, message.len() as i32, out.as_mut_ptr() as i32) };
        if rc == 0 { Ok(out) } else { Err(Error::from_code(rc)) }
    }

    pub fn confirm(text: &str, timeout_ms: i32) -> Result<(), Error> {
        match unsafe { keystore_confirm(text.as_ptr() as i32, text.len() as i32, timeout_ms) } {
            1 => Ok(()),
            0 => Err(Error::Declined),
            c => Err(Error::from_code(c)),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use super::*;

    pub fn get(_: &str) -> Result<Vec<u8>, Error> {
        Err(Error::NoStore)
    }
    pub fn put(_: &str, _: &[u8]) -> Result<(), Error> {
        Err(Error::NoStore)
    }
    pub fn delete(_: &str) -> Result<(), Error> {
        Err(Error::NoStore)
    }
    pub fn keygen(_: &str) -> Result<[u8; 32], Error> {
        Err(Error::NoStore)
    }
    pub fn public(_: &str) -> Result<[u8; 32], Error> {
        Err(Error::NoStore)
    }
    pub fn sign(_: &str, _: &[u8]) -> Result<[u8; 64], Error> {
        Err(Error::NoStore)
    }
    pub fn confirm(_: &str, _: i32) -> Result<(), Error> {
        Err(Error::NoStore)
    }
}

/// The secret stored under `name` by the player using the computer.
pub fn get(name: &str) -> Result<Vec<u8>, Error> {
    imp::get(name)
}

pub fn get_string(name: &str) -> Result<String, Error> {
    get(name).map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// Store (or replace) a secret of at most 4096 bytes for the player using the computer.
pub fn put(name: &str, value: &[u8]) -> Result<(), Error> {
    imp::put(name, value)
}

pub fn delete(name: &str) -> Result<(), Error> {
    imp::delete(name)
}

/// Make an ed25519 signing key under `name` (keeping one that exists) and return its public half.
pub fn keygen(name: &str) -> Result<[u8; 32], Error> {
    imp::keygen(name)
}

/// The public half of the signing key under `name`.
pub fn public(name: &str) -> Result<[u8; 32], Error> {
    imp::public(name)
}

/// Sign `message` with the key under `name`; the key itself never leaves the store.
pub fn sign(name: &str, message: &[u8]) -> Result<[u8; 64], Error> {
    imp::sign(name, message)
}

/// Ask the player at the computer to confirm something that matters (publishing
/// under their name). Only the player's own click answers it: no program can.
/// `Ok` means yes.
pub fn confirm(text: &str, timeout_ms: i32) -> Result<(), Error> {
    imp::confirm(text, timeout_ms)
}
