//! `ecm-dsp`: the pure signal-processing library behind EvansComputerMod's
//! radios (the in-game GNU Radio / liquid-dsp).
//!
//! Everything here is plain Rust with no I/O and no host calls, so it builds
//! for the host (tests), `wasm32-wasip1` (programs) and `wasm32-unknown-unknown`.
//!
//! Layout:
//! - [`complex`], [`rng`]: the `C32` sample type and a deterministic PRNG / AWGN.
//! - [`fft`], [`window`], [`fir`], [`iir`], [`resample`], [`nco`]: basics.
//! - [`sync`]: AGC, squelch, PLL, Costas, symbol timing, frequency offset.
//! - [`modem`]: AM, FM, SSB, CW/OOK, FSK/AFSK, BPSK/QPSK, LoRa-style chirp.
//! - [`coding`]: CRCs, Hamming(7,4), convolutional + Viterbi, Reed–Solomon,
//!   HDLC/AX.25, KISS.
//! - [`analysis`]: Welch PSD, waterfall rows, SNR estimation.
//! - [`iq`]: `/dev/sdr` sample formats and conversions.
//!
//! Streaming blocks keep their state across calls, so a program can feed
//! samples in whatever block sizes the device returns.

#![forbid(unsafe_code)]
// Index loops read more clearly than iterator chains in filter kernels.
#![allow(clippy::needless_range_loop)]

pub mod analysis;
pub mod coding;
pub mod complex;
pub mod fft;
pub mod fir;
pub mod iir;
pub mod iq;
pub mod modem;
pub mod nco;
pub mod resample;
pub mod rng;
pub mod sync;
pub mod window;

pub use complex::C32;
