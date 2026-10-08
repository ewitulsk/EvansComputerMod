//! Framed packet modems: a frame body goes in, samples come out, and the
//! matching receiver turns samples back into frame bodies (FCS checked and
//! stripped). All but chirp carry HDLC framing (flags, bit stuffing, CRC-16).
//!
//! - AFSK1200: Bell 202 audio (AX.25 / APRS). [`FmPacketTx`] / [`FmPacketRx`]
//!   put it on an NBFM carrier for the SDR, as a VHF packet radio does.
//! - FSK: binary FSK at complex baseband, NRZI so polarity doesn't matter.
//! - BPSK: differential BPSK, RRC shaped.
//! - Chirp: LoRa-style CSS with its own header and CRC.

use ecm_dsp::coding::hdlc::{encode_frame, HdlcDecoder, Nrzi};
use ecm_dsp::modem::chirp::{ChirpMod, ChirpParams, ChirpRx};
use ecm_dsp::modem::fm::{FmDemod, FmMod, FmParams};
use ecm_dsp::modem::fsk::{AfskParams, FskDemod, FskMod, PacketRx, PacketTx};
use ecm_dsp::modem::psk::{PskDemod, PskMod, PskParams};
use ecm_dsp::C32;

/// Largest frame body the HDLC receivers accept.
pub const MAX_FRAME: usize = 330;

/// NBFM deviation used for AFSK1200 packet on VHF.
pub const PACKET_DEVIATION: f32 = 3_000.0;

/// AFSK1200 audio -> NBFM complex baseband transmitter.
pub struct FmPacketTx {
    tx: PacketTx,
    fm: FmMod,
    audio: Vec<f32>,
}

impl FmPacketTx {
    pub fn new(fs: f32) -> FmPacketTx {
        FmPacketTx {
            tx: PacketTx::new(AfskParams::bell202(fs), 0.9),
            fm: FmMod::new(FmParams { fs, deviation: PACKET_DEVIATION, tau: 0.0 }),
            audio: Vec::new(),
        }
    }

    /// Leading HDLC flags (TX delay), ~6.7 ms each.
    pub fn set_preamble_flags(&mut self, n: usize) {
        self.tx.preamble_flags = n.max(1);
    }

    /// One frame body (FCS added) as baseband samples.
    pub fn send(&mut self, frame: &[u8]) -> Vec<C32> {
        self.audio.clear();
        self.tx.send(frame, &mut self.audio);
        self.fm.modulate_vec(&self.audio)
    }
}

/// NBFM complex baseband -> AFSK1200 frames.
pub struct FmPacketRx {
    fm: FmDemod,
    rx: PacketRx,
    audio: Vec<f32>,
}

impl FmPacketRx {
    pub fn new(fs: f32) -> FmPacketRx {
        FmPacketRx {
            fm: FmDemod::new(FmParams { fs, deviation: PACKET_DEVIATION, tau: 0.0 }),
            rx: PacketRx::new(AfskParams::bell202(fs)),
            audio: Vec::new(),
        }
    }

    pub fn push(&mut self, x: &[C32]) -> Vec<Vec<u8>> {
        self.audio.clear();
        self.audio.extend(x.iter().map(|&s| self.fm.demod_sample(s)));
        // The discriminator output is in units of the deviation; AFSK tones sit
        // well inside it. Keep the scale near +-1 for the AFSK demodulator.
        self.rx.push(&self.audio)
    }

    pub fn bad_fcs(&self) -> u64 {
        self.rx.bad_fcs()
    }
}

/// Which framed modem, with its parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ModemKind {
    /// Bell 202 audio (real samples).
    Afsk1200,
    /// Binary FSK at baseband.
    Fsk { baud: f32, deviation: f32 },
    /// Differential BPSK.
    Bpsk { baud: f32 },
    /// LoRa-style chirp.
    Chirp { sf: u8, bw: f32 },
}

impl ModemKind {
    /// Whether the modem's samples are complex baseband (else real audio).
    pub fn complex(&self) -> bool {
        !matches!(self, ModemKind::Afsk1200)
    }
}

/// Framed modulator.
pub enum PacketMod {
    Afsk(PacketTx),
    Fsk { m: FskMod, nrzi: Nrzi },
    Bpsk(PskMod),
    Chirp(ChirpMod),
}

/// Samples from a [`PacketMod`].
pub enum Samples {
    Real(Vec<f32>),
    Complex(Vec<C32>),
}

fn psk_params(fs: f64, baud: f32) -> Result<PskParams, String> {
    let sps = fs / baud as f64;
    if sps < 2.0 || (sps - sps.round()).abs() > 1e-6 {
        return Err(format!("bpsk needs an integer >= 2 samples per symbol (rate {fs} / baud {baud})"));
    }
    Ok(PskParams { differential: true, ..PskParams::bpsk(sps.round() as usize) })
}

fn chirp_params(fs: f64, sf: u8, bw: f32) -> Result<ChirpParams, String> {
    if !(6..=12).contains(&sf) {
        return Err(format!("chirp spreading factor must be 6..12, got {sf}"));
    }
    let os = fs / bw as f64;
    if os < 1.0 || (os - os.round()).abs() > 1e-6 {
        return Err(format!("chirp needs the sample rate ({fs}) to be a whole multiple of the bandwidth ({bw})"));
    }
    Ok(ChirpParams::new(sf, bw, os.round() as usize))
}

impl PacketMod {
    pub fn new(kind: ModemKind, fs: f64) -> Result<PacketMod, String> {
        Ok(match kind {
            ModemKind::Afsk1200 => {
                if fs < 6_000.0 {
                    return Err(format!("afsk1200 needs at least 6000 samples/s, got {fs}"));
                }
                PacketMod::Afsk(PacketTx::new(AfskParams::bell202(fs as f32), 0.9))
            }
            ModemKind::Fsk { baud, deviation } => {
                if fs < 4.0 * baud as f64 || deviation * 2.0 >= fs as f32 {
                    return Err(format!("fsk needs rate >= 4 x baud and deviation < rate/2 (rate {fs})"));
                }
                PacketMod::Fsk { m: FskMod::new(fs as f32, baud, deviation), nrzi: Nrzi::new() }
            }
            ModemKind::Bpsk { baud } => PacketMod::Bpsk(PskMod::new(psk_params(fs, baud)?)),
            ModemKind::Chirp { sf, bw } => PacketMod::Chirp(ChirpMod::new(chirp_params(fs, sf, bw)?)),
        })
    }

    pub fn send(&mut self, frame: &[u8]) -> Samples {
        match self {
            PacketMod::Afsk(tx) => {
                let mut v = Vec::new();
                tx.send(frame, &mut v);
                Samples::Real(v)
            }
            PacketMod::Fsk { m, nrzi } => {
                let bits = encode_frame(frame, 16, 4);
                Samples::Complex(m.modulate_vec(&nrzi.encode_block(&bits)))
            }
            PacketMod::Bpsk(m) => {
                let bits = encode_frame(frame, 16, 4);
                let mut v = Vec::new();
                m.modulate(&bits, &mut v);
                m.flush(&mut v);
                Samples::Complex(v)
            }
            PacketMod::Chirp(m) => {
                let mut v = Vec::new();
                m.modulate_packet(frame, &mut v);
                Samples::Complex(v)
            }
        }
    }
}

/// Framed demodulator.
pub enum PacketDemod {
    Afsk(PacketRx),
    Fsk { d: FskDemod, nrzi: Nrzi, hdlc: HdlcDecoder, bits: Vec<u8> },
    Bpsk { d: PskDemod, hdlc: HdlcDecoder, bits: Vec<u8> },
    Chirp(ChirpRx),
}

impl PacketDemod {
    pub fn new(kind: ModemKind, fs: f64) -> Result<PacketDemod, String> {
        // Validate parameters the same way as the modulator.
        PacketMod::new(kind, fs)?;
        Ok(match kind {
            ModemKind::Afsk1200 => PacketDemod::Afsk(PacketRx::new(AfskParams::bell202(fs as f32))),
            ModemKind::Fsk { baud, .. } => PacketDemod::Fsk {
                d: FskDemod::new(fs as f32, baud),
                nrzi: Nrzi::new(),
                hdlc: HdlcDecoder::new(4, MAX_FRAME),
                bits: Vec::new(),
            },
            ModemKind::Bpsk { baud } => PacketDemod::Bpsk {
                d: PskDemod::new(psk_params(fs, baud)?),
                hdlc: HdlcDecoder::new(4, MAX_FRAME),
                bits: Vec::new(),
            },
            ModemKind::Chirp { sf, bw } => PacketDemod::Chirp(ChirpRx::new(chirp_params(fs, sf, bw)?)),
        })
    }

    /// Real audio in (AFSK only).
    pub fn push_real(&mut self, x: &[f32]) -> Result<Vec<Vec<u8>>, String> {
        match self {
            PacketDemod::Afsk(rx) => Ok(rx.push(x)),
            _ => Err("this modem takes complex samples".into()),
        }
    }

    /// Complex baseband in (FSK, BPSK, chirp).
    pub fn push_complex(&mut self, x: &[C32]) -> Result<Vec<Vec<u8>>, String> {
        let mut frames = Vec::new();
        match self {
            PacketDemod::Afsk(_) => return Err("afsk1200 takes real audio (put fm_demod in front)".into()),
            PacketDemod::Fsk { d, nrzi, hdlc, bits } => {
                bits.clear();
                d.demod(x, bits);
                for &l in bits.iter() {
                    if let Some(f) = hdlc.push_bit(nrzi.decode(l)) {
                        frames.push(f);
                    }
                }
            }
            PacketDemod::Bpsk { d, hdlc, bits } => {
                bits.clear();
                d.demod_bits(x, bits);
                for &b in bits.iter() {
                    if let Some(f) = hdlc.push_bit(b) {
                        frames.push(f);
                    }
                }
            }
            PacketDemod::Chirp(rx) => frames = rx.push(x),
        }
        Ok(frames)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecm_dsp::rng::{awgn, Rng};

    fn frame(i: u8) -> Vec<u8> {
        (0..40u8).map(|k| k.wrapping_mul(7).wrapping_add(i)).collect()
    }

    #[test]
    fn afsk_over_nbfm_round_trip() {
        let mut tx = FmPacketTx::new(48_000.0);
        let mut x = vec![C32::ZERO; 3000];
        for i in 0..3 {
            x.extend(tx.send(&frame(i)));
            x.extend(vec![C32::new(1.0, 0.0); 2000]);
        }
        let mut rng = Rng::new(3);
        awgn(&mut x, 0.05, &mut rng);
        let mut rx = FmPacketRx::new(48_000.0);
        let mut got = Vec::new();
        for c in x.chunks(4096) {
            got.extend(rx.push(c));
        }
        assert_eq!(got, vec![frame(0), frame(1), frame(2)]);
    }

    #[test]
    fn baseband_modems_round_trip() {
        let fs = 48_000.0;
        for kind in [
            ModemKind::Fsk { baud: 1200.0, deviation: 2400.0 },
            ModemKind::Bpsk { baud: 4800.0 },
            ModemKind::Chirp { sf: 7, bw: 12_000.0 },
        ] {
            let mut m = PacketMod::new(kind, fs).unwrap();
            let mut x = vec![C32::ZERO; 500];
            for i in 0..2 {
                match m.send(&frame(i)) {
                    Samples::Complex(v) => x.extend(v),
                    Samples::Real(_) => panic!("complex expected"),
                }
                x.extend(vec![C32::ZERO; 2000]);
            }
            let mut rng = Rng::new(9);
            awgn(&mut x, 0.01, &mut rng);
            let mut d = PacketDemod::new(kind, fs).unwrap();
            let mut got = Vec::new();
            for c in x.chunks(1000) {
                got.extend(d.push_complex(c).unwrap());
            }
            assert_eq!(got, vec![frame(0), frame(1)], "{kind:?}");
        }
    }

    #[test]
    fn bad_parameters_are_errors() {
        assert!(PacketMod::new(ModemKind::Bpsk { baud: 7000.0 }, 48_000.0).is_err());
        assert!(PacketMod::new(ModemKind::Chirp { sf: 7, bw: 7000.0 }, 48_000.0).is_err());
        assert!(PacketMod::new(ModemKind::Afsk1200, 4000.0).is_err());
        assert!(PacketDemod::new(ModemKind::Afsk1200, 48_000.0).unwrap().push_complex(&[]).is_err());
    }
}
