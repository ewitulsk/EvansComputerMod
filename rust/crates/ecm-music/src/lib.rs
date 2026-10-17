//! The music library behind `musicd` (the music backend) and the Music app
//! (`dev.ecm.music`). See `plans/background-apps/MUSIC_AND_AUTH.md`.
//!
//! Everything here is plain computation with no host calls, so it runs the
//! same in the WASM daemon, in the WASM app and in host-side unit tests. The
//! daemon does the file and socket I/O and hands bytes in.
//!
//! - [`id3`]: ID3v2.2/2.3/2.4 and ID3v1 tags, including the cover picture.
//! - [`mp3`]: MPEG audio frame headers, duration, VBR, seek offsets.
//! - [`search`]: the in-memory inverted index with typo-tolerant ranking.
//! - [`library`]: what `musicd scan` keeps: songs, albums, artists, genres.
//! - [`store`]: per-user playlists, likes, recently played and play counts.
//! - [`art`] (feature `art`): cover art to 64x64 and 160x160 PNGs.
//! - [`transcode`]: the Data saver stream format (22 kHz mono IMA ADPCM) and,
//!   with feature `decode`, the Symphonia MP3 decoder ([`Mp3Decoder`]).
//! - [`http`]: request parsing, `Range`, response heads, JSON, query strings.

pub mod http;
pub mod id3;
pub mod library;
pub mod mp3;
pub mod search;
pub mod store;
pub mod transcode;

#[cfg(feature = "art")]
pub mod art;

mod text;

#[cfg(feature = "decode")]
pub use transcode::Mp3Decoder;
