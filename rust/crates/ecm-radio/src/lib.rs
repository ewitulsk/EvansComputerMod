//! `ecm-radio`: the pieces the SDR programs (`rx_fm`, `waterfall`, `radiod`,
//! ...) and the Python `radio` module share. Pure Rust over `std::fs`, so it
//! is tested on the host with synthetic IQ files.
//!
//! - [`units`]: frequency / rate parsing (`144.39M`, `146.52e6`, `600k`).
//! - [`device`]: `/dev/sdr<N>` + `/dev/sdrctl<N>` (name resolution, control
//!   commands, status, sample I/O, transmit pacing).
//! - [`blocks`]: streaming flowgraph blocks (demods, filters, resampler, AGC,
//!   squelch, modulators, packet modems) and the [`blocks::Chain`] that runs them.
//! - [`modem`]: framed packet modems (AFSK1200/AX.25 over NBFM, FSK, BPSK, chirp).
//! - [`audio`]: PCM / WAV conversion.
//! - [`sigmf`]: SigMF `.sigmf-meta` writer and reader (with a tiny JSON parser).
//! - [`link`]: the `radio0` link layer: Ethernet <-> AX.25 UI (+ segmentation,
//!   MSS clamping) <-> KISS.
//! - [`spectrum`]: waterfall rendering and band scanning.
//! - [`cli`]: argument parsing shared by the programs.
//! - [`run`]: the programs' main loops.

pub mod audio;
pub mod blocks;
pub mod cli;
pub mod device;
pub mod link;
pub mod modem;
pub mod run;
pub mod sigmf;
pub mod spectrum;
pub mod units;

pub use ecm_dsp::C32;
