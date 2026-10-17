//! Card schemes and formats (docs/cards/CARDS.md, docs/cards/NFC_MIX.md).
//!
//! | Module | What |
//! |---|---|
//! | [`nfc`] | the Sector Lattice `Mix`, the factory key, the 3-pass sector auth values, the `NF-106` air frames |
//! | [`bank`] | the Keyed Ledger Response TAG, the PIN hash, tag verification |
//! | [`design`] | the `.card` design file (palette-indexed faces, fields, stripe) with a 3×5 font |
//! | [`hex`] | hex encode / decode |
//! | [`json`] | the small JSON value the design file is written with |
//!
//! Everything here is a toy by design (CARDS.md): it must hold inside the
//! game and resemble no real card protocol. The vectors in NFC_MIX.md are the
//! contract shared with the Java applets and the Python / Lua `cards` modules.

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod bank;
pub mod design;
pub mod hex;
pub mod json;
pub mod nfc;
