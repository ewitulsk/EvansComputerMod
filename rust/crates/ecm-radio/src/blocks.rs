//! Streaming flowgraph blocks and the chain that runs them.
//!
//! A block is described by a [`Spec`] (what the Python `radio.fm_demod(5e3)`
//! object holds) and built once the input sample rate and stream kind are
//! known, so `radio.lowpass(3e3)` works after any source. Blocks keep their
//! state between calls, so a chain can be fed whatever buffer sizes the
//! device returns.
//!
//! Stream kinds: complex baseband ([`Buf::C`]), real audio ([`Buf::R`]) and
//! frames ([`Buf::F`], packet bodies). Packet modem blocks pick their
//! direction from the input: frames in = modulate, samples in = demodulate.

use crate::modem::{ModemKind, PacketDemod, PacketMod, Samples};
use crate::units::ratio;
use ecm_dsp::fir::{lowpass, FirRC, FirRR};
use ecm_dsp::iir::DcBlocker;
use ecm_dsp::modem::am::{AmDemod, AmMod};
use ecm_dsp::modem::fm::{FmDemod, FmMod, FmParams};
use ecm_dsp::modem::ssb::{Sideband, SsbMod, WeaverDemod};
use ecm_dsp::nco::Nco;
use ecm_dsp::resample::{Decimator, Resampler};
use ecm_dsp::sync::{Agc, Squelch};
use ecm_dsp::window::Window;
use ecm_dsp::C32;
use std::collections::BTreeMap;

/// A buffer of one stream kind.
#[derive(Clone, Debug, PartialEq)]
pub enum Buf {
    C(Vec<C32>),
    R(Vec<f32>),
    F(Vec<Vec<u8>>),
}

impl Buf {
    pub fn kind(&self) -> Kind {
        match self {
            Buf::C(_) => Kind::Complex,
            Buf::R(_) => Kind::Real,
            Buf::F(_) => Kind::Frames,
        }
    }
    pub fn len(&self) -> usize {
        match self {
            Buf::C(v) => v.len(),
            Buf::R(v) => v.len(),
            Buf::F(v) => v.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Stream kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Complex,
    Real,
    Frames,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Complex => "complex",
            Kind::Real => "real",
            Kind::Frames => "frames",
        }
    }
    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "complex" | "c" => Some(Kind::Complex),
            "real" | "r" => Some(Kind::Real),
            "frames" | "f" => Some(Kind::Frames),
            _ => None,
        }
    }
}

/// A block parameter as handed over from Python or the command line.
#[derive(Clone, Debug, PartialEq)]
pub enum Param {
    Num(f64),
    Str(String),
}

/// What a block does, before it knows its input rate.
#[derive(Clone, Debug, PartialEq)]
pub enum Spec {
    /// Quadrature FM discriminator; `tau` = de-emphasis (0 = none).
    FmDemod { deviation: f32, tau: f32 },
    /// FM modulator (real -> complex, unit amplitude).
    FmMod { deviation: f32, tau: f32 },
    /// Envelope detector, carrier removed and level-normalised.
    AmDemod,
    /// AM with carrier (real -> complex), modulation `index`.
    AmMod { index: f32 },
    /// SSB (Weaver) demodulator, 300-2700 Hz voice passband.
    SsbDemod { upper: bool },
    /// SSB (phasing) modulator.
    SsbMod { upper: bool },
    /// Low-pass FIR (real or complex); `cutoff` Hz.
    Lowpass { cutoff: f32 },
    /// Rational resampler to `rate` samples/s.
    Resample { rate: f64 },
    /// Integer decimator.
    Decimate { factor: usize },
    /// Automatic gain control towards `target` amplitude.
    Agc { target: f32 },
    /// Power squelch: zeroes the stream below `threshold_db` dBFS.
    Squelch { threshold_db: f32 },
    /// Frequency shift by `hz` (complex).
    Shift { hz: f64 },
    /// Multiply by a constant.
    Gain { gain: f32 },
    /// DC blocker.
    DcBlock,
    /// Real part of a complex stream.
    Real,
    /// Real stream as complex (imaginary 0).
    Complex,
    /// Framed packet modem (direction from the input kind).
    Modem(ModemKind),
}

fn num(p: &BTreeMap<String, Param>, k: &str, default: f64) -> Result<f64, String> {
    match p.get(k) {
        None => Ok(default),
        Some(Param::Num(v)) if v.is_finite() => Ok(*v),
        Some(Param::Str(s)) => crate::units::parse_freq(s).ok_or_else(|| format!("{k}: bad number {s:?}")),
        Some(_) => Err(format!("{k}: not a finite number")),
    }
}

fn need(p: &BTreeMap<String, Param>, k: &str) -> Result<f64, String> {
    if !p.contains_key(k) {
        return Err(format!("missing parameter {k}"));
    }
    num(p, k, 0.0)
}

fn sideband(p: &BTreeMap<String, Param>) -> Result<bool, String> {
    match p.get("mode") {
        None => Ok(true),
        Some(Param::Str(s)) => match s.to_ascii_lowercase().as_str() {
            "usb" | "upper" => Ok(true),
            "lsb" | "lower" => Ok(false),
            _ => Err(format!("ssb mode must be usb or lsb, got {s:?}")),
        },
        Some(Param::Num(_)) => Err("ssb mode must be 'usb' or 'lsb'".into()),
    }
}

impl Spec {
    /// Build a spec from a block name and named parameters (the Python /
    /// command-line form). Unknown names and bad values are errors.
    pub fn from_params(name: &str, p: &BTreeMap<String, Param>) -> Result<Spec, String> {
        Ok(match name {
            "fm_demod" => Spec::FmDemod { deviation: num(p, "deviation", 5e3)? as f32, tau: num(p, "tau", 0.0)? as f32 },
            "wbfm_demod" => Spec::FmDemod { deviation: num(p, "deviation", 75e3)? as f32, tau: num(p, "tau", 75e-6)? as f32 },
            "fm_mod" => Spec::FmMod { deviation: num(p, "deviation", 5e3)? as f32, tau: num(p, "tau", 0.0)? as f32 },
            "am_demod" => Spec::AmDemod,
            "am_mod" => Spec::AmMod { index: num(p, "index", 0.8)? as f32 },
            "ssb_demod" => Spec::SsbDemod { upper: sideband(p)? },
            "ssb_mod" => Spec::SsbMod { upper: sideband(p)? },
            "lowpass" => Spec::Lowpass { cutoff: need(p, "cutoff")? as f32 },
            "resample" => Spec::Resample { rate: need(p, "rate")? },
            "decimate" => {
                let f = need(p, "factor")?;
                if f < 1.0 || f.fract() != 0.0 {
                    return Err(format!("decimate factor must be a whole number >= 1, got {f}"));
                }
                Spec::Decimate { factor: f as usize }
            }
            "agc" => Spec::Agc { target: num(p, "target", 0.5)? as f32 },
            "squelch" => Spec::Squelch { threshold_db: num(p, "threshold", -40.0)? as f32 },
            "shift" => Spec::Shift { hz: need(p, "hz")? },
            "gain" => Spec::Gain { gain: need(p, "gain")? as f32 },
            "dc_block" => Spec::DcBlock,
            "real" => Spec::Real,
            "complex" => Spec::Complex,
            "afsk1200" => Spec::Modem(ModemKind::Afsk1200),
            "fsk" => Spec::Modem(ModemKind::Fsk {
                baud: num(p, "baud", 1200.0)? as f32,
                deviation: num(p, "deviation", 2400.0)? as f32,
            }),
            "bpsk" => Spec::Modem(ModemKind::Bpsk { baud: num(p, "baud", 2400.0)? as f32 }),
            "chirp" => {
                let sf = num(p, "sf", 7.0)?;
                Spec::Modem(ModemKind::Chirp { sf: sf as u8, bw: num(p, "bw", 12_000.0)? as f32 })
            }
            other => return Err(format!("unknown block {other:?}")),
        })
    }

    /// Short description, e.g. `fm_demod(5000 Hz)`.
    pub fn describe(&self) -> String {
        match self {
            Spec::FmDemod { deviation, .. } => format!("fm_demod({deviation} Hz)"),
            Spec::FmMod { deviation, .. } => format!("fm_mod({deviation} Hz)"),
            Spec::AmDemod => "am_demod".into(),
            Spec::AmMod { index } => format!("am_mod({index})"),
            Spec::SsbDemod { upper } => format!("ssb_demod({})", if *upper { "usb" } else { "lsb" }),
            Spec::SsbMod { upper } => format!("ssb_mod({})", if *upper { "usb" } else { "lsb" }),
            Spec::Lowpass { cutoff } => format!("lowpass({cutoff} Hz)"),
            Spec::Resample { rate } => format!("resample({rate})"),
            Spec::Decimate { factor } => format!("decimate({factor})"),
            Spec::Agc { target } => format!("agc({target})"),
            Spec::Squelch { threshold_db } => format!("squelch({threshold_db} dB)"),
            Spec::Shift { hz } => format!("shift({hz} Hz)"),
            Spec::Gain { gain } => format!("gain({gain})"),
            Spec::DcBlock => "dc_block".into(),
            Spec::Real => "real".into(),
            Spec::Complex => "complex".into(),
            Spec::Modem(k) => format!("{k:?}").to_ascii_lowercase(),
        }
    }
}

enum Op {
    FmDemod(FmDemod),
    FmMod(FmMod),
    AmDemod(AmDemod),
    AmMod(AmMod),
    SsbDemod(WeaverDemod),
    SsbMod(SsbMod),
    LowpassC(FirRC),
    LowpassR(FirRR),
    ResampleC(Resampler<C32>),
    ResampleR(Resampler<f32>),
    DecimateC(Decimator<C32>),
    DecimateR(Decimator<f32>),
    AgcC(Agc),
    AgcR(Agc),
    Squelch(Squelch),
    Shift(Nco),
    Gain(f32),
    DcC(DcBlocker<C32>),
    DcR(DcBlocker<f32>),
    Real,
    Complex,
    ModTx(PacketMod),
    ModRx(PacketDemod),
    /// Identity (e.g. resampling to the same rate).
    Pass,
}

/// One built block.
pub struct Node {
    op: Op,
    pub spec: Spec,
    pub in_kind: Kind,
    pub out_kind: Kind,
    pub in_rate: f64,
    pub out_rate: f64,
}

fn lowpass_taps(cutoff: f32, fs: f64) -> Result<Vec<f32>, String> {
    let fs = fs as f32;
    if !(cutoff > 0.0 && cutoff < fs / 2.0) {
        return Err(format!("lowpass cutoff {cutoff} Hz must be between 0 and half the rate ({fs})"));
    }
    let transition = (cutoff * 0.3).max(fs * 0.004).min(fs / 2.0 - cutoff).max(fs * 0.002);
    let n = ((3.3 * fs / transition) as usize).clamp(15, 511) | 1;
    Ok(lowpass(n, cutoff / fs, Window::Hamming))
}

fn want(spec: &Spec, got: Kind, want: Kind) -> Result<(), String> {
    if got == want {
        Ok(())
    } else {
        Err(format!("{} takes a {} stream, got {}", spec.describe(), want.name(), got.name()))
    }
}

impl Node {
    /// Build `spec` for an input of `kind` at `rate` samples/s.
    pub fn build(spec: Spec, kind: Kind, rate: f64) -> Result<Node, String> {
        if !(rate > 0.0 && rate.is_finite()) {
            return Err(format!("bad sample rate {rate}"));
        }
        let fs = rate as f32;
        let mut out_rate = rate;
        let (op, out_kind) = match &spec {
            Spec::FmDemod { deviation, tau } => {
                want(&spec, kind, Kind::Complex)?;
                if *deviation <= 0.0 {
                    return Err("fm deviation must be > 0".into());
                }
                (Op::FmDemod(FmDemod::new(FmParams { fs, deviation: *deviation, tau: *tau })), Kind::Real)
            }
            Spec::FmMod { deviation, tau } => {
                want(&spec, kind, Kind::Real)?;
                if *deviation <= 0.0 || *deviation >= fs / 2.0 {
                    return Err(format!("fm deviation must be between 0 and half the rate ({fs})"));
                }
                (Op::FmMod(FmMod::new(FmParams { fs, deviation: *deviation, tau: *tau })), Kind::Complex)
            }
            Spec::AmDemod => {
                want(&spec, kind, Kind::Complex)?;
                (Op::AmDemod(AmDemod::new(fs, 1.0)), Kind::Real)
            }
            Spec::AmMod { index } => {
                want(&spec, kind, Kind::Real)?;
                (Op::AmMod(AmMod::new(*index, 0.5)), Kind::Complex)
            }
            Spec::SsbDemod { upper } => {
                want(&spec, kind, Kind::Complex)?;
                if fs < 6000.0 {
                    return Err("ssb_demod needs at least 6000 samples/s".into());
                }
                let sb = if *upper { Sideband::Upper } else { Sideband::Lower };
                (Op::SsbDemod(WeaverDemod::new(sb, fs, 300.0, 2700.0)), Kind::Real)
            }
            Spec::SsbMod { upper } => {
                want(&spec, kind, Kind::Real)?;
                let sb = if *upper { Sideband::Upper } else { Sideband::Lower };
                (Op::SsbMod(SsbMod::new(sb, 127)), Kind::Complex)
            }
            Spec::Lowpass { cutoff } => {
                let taps = lowpass_taps(*cutoff, rate)?;
                match kind {
                    Kind::Complex => (Op::LowpassC(FirRC::new(taps)), kind),
                    Kind::Real => (Op::LowpassR(FirRR::new(taps)), kind),
                    Kind::Frames => return Err("lowpass takes samples, got frames".into()),
                }
            }
            Spec::Resample { rate: to } => {
                if !(*to >= 1.0 && to.is_finite()) {
                    return Err(format!("bad resample rate {to}"));
                }
                let (l, m) = ratio(rate, *to, 512);
                out_rate = rate * l as f64 / m as f64;
                if l == m {
                    (Op::Pass, kind)
                } else {
                    match kind {
                        Kind::Complex => (Op::ResampleC(Resampler::new(l, m)), kind),
                        Kind::Real => (Op::ResampleR(Resampler::new(l, m)), kind),
                        Kind::Frames => return Err("resample takes samples, got frames".into()),
                    }
                }
            }
            Spec::Decimate { factor } => {
                out_rate = rate / *factor as f64;
                match kind {
                    Kind::Complex => (Op::DecimateC(Decimator::new(*factor)), kind),
                    Kind::Real => (Op::DecimateR(Decimator::new(*factor)), kind),
                    Kind::Frames => return Err("decimate takes samples, got frames".into()),
                }
            }
            Spec::Agc { target } => {
                let mut a = Agc::new(*target, (50.0 / fs).min(1.0), (5.0 / fs).min(1.0));
                a.max_gain = 1e4;
                match kind {
                    Kind::Complex => (Op::AgcC(a), kind),
                    Kind::Real => (Op::AgcR(a), kind),
                    Kind::Frames => return Err("agc takes samples, got frames".into()),
                }
            }
            Spec::Squelch { threshold_db } => {
                want(&spec, kind, Kind::Complex)?;
                (Op::Squelch(Squelch::new(*threshold_db, 3.0, (200.0 / fs).min(1.0))), kind)
            }
            Spec::Shift { hz } => {
                want(&spec, kind, Kind::Complex)?;
                (Op::Shift(Nco::new(*hz, rate)), kind)
            }
            Spec::Gain { gain } => match kind {
                Kind::Frames => return Err("gain takes samples, got frames".into()),
                _ => (Op::Gain(*gain), kind),
            },
            Spec::DcBlock => match kind {
                Kind::Complex => (Op::DcC(DcBlocker::with_corner(10.0, fs)), kind),
                Kind::Real => (Op::DcR(DcBlocker::with_corner(10.0, fs)), kind),
                Kind::Frames => return Err("dc_block takes samples, got frames".into()),
            },
            Spec::Real => {
                want(&spec, kind, Kind::Complex)?;
                (Op::Real, Kind::Real)
            }
            Spec::Complex => {
                want(&spec, kind, Kind::Real)?;
                (Op::Complex, Kind::Complex)
            }
            Spec::Modem(mk) => match kind {
                Kind::Frames => {
                    let m = PacketMod::new(*mk, rate)?;
                    (Op::ModTx(m), if mk.complex() { Kind::Complex } else { Kind::Real })
                }
                k => {
                    let samples = if mk.complex() { Kind::Complex } else { Kind::Real };
                    if k != samples {
                        return Err(format!(
                            "{} demodulates {} samples, got {}",
                            spec.describe(),
                            samples.name(),
                            k.name()
                        ));
                    }
                    (Op::ModRx(PacketDemod::new(*mk, rate)?), Kind::Frames)
                }
            },
        };
        Ok(Node { op, spec, in_kind: kind, out_kind, in_rate: rate, out_rate })
    }

    /// Run one buffer through the block.
    pub fn process(&mut self, b: Buf) -> Result<Buf, String> {
        if b.kind() != self.in_kind {
            return Err(format!(
                "{} expects {} input, got {}",
                self.spec.describe(),
                self.in_kind.name(),
                b.kind().name()
            ));
        }
        Ok(match (&mut self.op, b) {
            (Op::FmDemod(d), Buf::C(x)) => Buf::R(d.demod_vec(&x)),
            (Op::FmMod(m), Buf::R(x)) => Buf::C(m.modulate_vec(&x)),
            (Op::AmDemod(d), Buf::C(x)) => Buf::R(d.demod_vec(&x)),
            (Op::AmMod(m), Buf::R(x)) => Buf::C(m.modulate_vec(&x)),
            (Op::SsbDemod(d), Buf::C(x)) => Buf::R(d.demod_vec(&x)),
            (Op::SsbMod(m), Buf::R(x)) => Buf::C(m.modulate_vec(&x)),
            (Op::LowpassC(f), Buf::C(x)) => Buf::C(f.process_vec(&x)),
            (Op::LowpassR(f), Buf::R(x)) => Buf::R(f.process_vec(&x)),
            (Op::ResampleC(r), Buf::C(x)) => Buf::C(r.process_vec(&x)),
            (Op::ResampleR(r), Buf::R(x)) => Buf::R(r.process_vec(&x)),
            (Op::DecimateC(d), Buf::C(x)) => Buf::C(d.process_vec(&x)),
            (Op::DecimateR(d), Buf::R(x)) => Buf::R(d.process_vec(&x)),
            (Op::AgcC(a), Buf::C(mut x)) => {
                a.process_block(&mut x);
                Buf::C(x)
            }
            (Op::AgcR(a), Buf::R(mut x)) => {
                a.process_block_real(&mut x);
                Buf::R(x)
            }
            (Op::Squelch(s), Buf::C(mut x)) => {
                s.process_block(&mut x);
                Buf::C(x)
            }
            (Op::Shift(n), Buf::C(mut x)) => {
                n.mix_up_inplace(&mut x);
                Buf::C(x)
            }
            (Op::Gain(g), Buf::C(x)) => Buf::C(x.into_iter().map(|v| v.scale(*g)).collect()),
            (Op::Gain(g), Buf::R(x)) => Buf::R(x.into_iter().map(|v| v * *g).collect()),
            (Op::DcC(d), Buf::C(mut x)) => {
                d.process_block(&mut x);
                Buf::C(x)
            }
            (Op::DcR(d), Buf::R(mut x)) => {
                d.process_block(&mut x);
                Buf::R(x)
            }
            (Op::Real, Buf::C(x)) => Buf::R(x.into_iter().map(|v| v.re).collect()),
            (Op::Complex, Buf::R(x)) => Buf::C(x.into_iter().map(|v| C32::new(v, 0.0)).collect()),
            (Op::ModTx(m), Buf::F(frames)) => {
                let (mut c, mut r) = (Vec::new(), Vec::new());
                for f in &frames {
                    match m.send(f) {
                        Samples::Complex(v) => c.extend(v),
                        Samples::Real(v) => r.extend(v),
                    }
                }
                if self.out_kind == Kind::Complex {
                    Buf::C(c)
                } else {
                    Buf::R(r)
                }
            }
            (Op::ModRx(d), Buf::C(x)) => Buf::F(d.push_complex(&x)?),
            (Op::ModRx(d), Buf::R(x)) => Buf::F(d.push_real(&x)?),
            (Op::Pass, b) => b,
            (_, b) => return Err(format!("{}: unexpected {} input", self.spec.describe(), b.kind().name())),
        })
    }
}

/// Blocks connected in series.
pub struct Chain {
    pub nodes: Vec<Node>,
    pub in_kind: Kind,
    pub in_rate: f64,
}

impl Chain {
    /// Build every spec in order, threading kind and rate through.
    pub fn build(specs: Vec<Spec>, kind: Kind, rate: f64) -> Result<Chain, String> {
        let (mut k, mut r) = (kind, rate);
        let mut nodes = Vec::with_capacity(specs.len());
        for (i, s) in specs.into_iter().enumerate() {
            let n = Node::build(s, k, r).map_err(|e| format!("block {} : {e}", i + 1))?;
            k = n.out_kind;
            r = n.out_rate;
            nodes.push(n);
        }
        Ok(Chain { nodes, in_kind: kind, in_rate: rate })
    }

    pub fn out_kind(&self) -> Kind {
        self.nodes.last().map(|n| n.out_kind).unwrap_or(self.in_kind)
    }

    pub fn out_rate(&self) -> f64 {
        self.nodes.last().map(|n| n.out_rate).unwrap_or(self.in_rate)
    }

    pub fn process(&mut self, mut b: Buf) -> Result<Buf, String> {
        for n in &mut self.nodes {
            b = n.process(b)?;
        }
        Ok(b)
    }

    /// `fm_demod(5000 Hz) >> lowpass(3000 Hz)`.
    pub fn describe(&self) -> String {
        self.nodes.iter().map(|n| n.spec.describe()).collect::<Vec<_>>().join(" >> ")
    }
}

/// Complex tone generator: `amplitude * e^{j 2 pi hz t}`, phase-continuous.
pub struct Tone {
    nco: Nco,
    amplitude: f32,
}

impl Tone {
    pub fn new(hz: f64, rate: f64, amplitude: f32) -> Tone {
        Tone { nco: Nco::new(hz, rate), amplitude }
    }
    pub fn complex(&mut self, n: usize) -> Vec<C32> {
        (0..n).map(|_| self.nco.next().scale(self.amplitude)).collect()
    }
    pub fn real(&mut self, n: usize) -> Vec<f32> {
        (0..n).map(|_| self.nco.next().re * self.amplitude).collect()
    }
}

/// Audio-receiver chain for a mode: channel filter, demodulator, audio
/// low-pass, then resampled to `audio_rate`.
pub fn receiver(mode: &str, rate: f64, audio_rate: f64, squelch_db: Option<f32>) -> Result<Chain, String> {
    let half = rate as f32 / 2.0;
    let mut specs = Vec::new();
    if let Some(db) = squelch_db {
        specs.push(Spec::Squelch { threshold_db: db });
    }
    match mode {
        "fm" | "nbfm" => {
            specs.push(Spec::Lowpass { cutoff: (8_000.0f32).min(half * 0.9) });
            specs.push(Spec::FmDemod { deviation: 5_000.0, tau: 0.0 });
            specs.push(Spec::Lowpass { cutoff: (3_500.0f32).min(half * 0.9) });
            specs.push(Spec::Gain { gain: 0.8 });
        }
        "wbfm" => {
            if rate < 150_000.0 {
                return Err(format!("wbfm needs a sample rate of at least 150 kS/s, got {rate}"));
            }
            specs.push(Spec::FmDemod { deviation: 75_000.0, tau: 75e-6 });
            specs.push(Spec::Lowpass { cutoff: 15_000.0 });
        }
        "am" => {
            specs.push(Spec::Lowpass { cutoff: (5_000.0f32).min(half * 0.9) });
            specs.push(Spec::AmDemod);
            specs.push(Spec::Lowpass { cutoff: (4_500.0f32).min(half * 0.9) });
            specs.push(Spec::Gain { gain: 0.8 });
        }
        "usb" | "lsb" => {
            specs.push(Spec::SsbDemod { upper: mode == "usb" });
            specs.push(Spec::Agc { target: 0.3 });
        }
        other => return Err(format!("unknown mode {other:?} (fm, wbfm, am, usb, lsb)")),
    }
    specs.push(Spec::Resample { rate: audio_rate });
    Chain::build(specs, Kind::Complex, rate)
}

/// Transmitter chain for a broadcast mode: real audio at `audio_rate` ->
/// complex baseband at `rate`.
pub fn transmitter(mode: &str, audio_rate: f64, rate: f64) -> Result<Chain, String> {
    let mut specs = vec![Spec::Resample { rate }];
    match mode {
        "fm" | "nbfm" => {
            specs.push(Spec::Lowpass { cutoff: (3_500.0f32).min(rate as f32 * 0.45) });
            specs.push(Spec::FmMod { deviation: 5_000.0, tau: 0.0 });
        }
        "wbfm" => {
            if rate < 150_000.0 {
                return Err(format!("wbfm needs a sample rate of at least 150 kS/s, got {rate}"));
            }
            specs.push(Spec::Lowpass { cutoff: 15_000.0 });
            specs.push(Spec::FmMod { deviation: 75_000.0, tau: 75e-6 });
        }
        "am" => {
            specs.push(Spec::Lowpass { cutoff: (4_500.0f32).min(rate as f32 * 0.45) });
            // Carrier 0.5 x 1.1 = 0.55: a full-scale (1.0) audio peak reaches
            // 0.55 x (1 + 0.8) = 0.99, just inside the SDR's +-1 sample range.
            specs.push(Spec::AmMod { index: 0.8 });
            specs.push(Spec::Gain { gain: 1.1 });
        }
        "usb" | "lsb" => {
            specs.push(Spec::Lowpass { cutoff: (2_700.0f32).min(rate as f32 * 0.45) });
            specs.push(Spec::SsbMod { upper: mode == "usb" });
        }
        other => return Err(format!("unknown mode {other:?} (fm, wbfm, am, usb, lsb)")),
    }
    Chain::build(specs, Kind::Real, audio_rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(kv: &[(&str, Param)]) -> BTreeMap<String, Param> {
        kv.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn specs_parse_and_reject() {
        assert_eq!(
            Spec::from_params("fm_demod", &params(&[("deviation", Param::Num(3e3))])).unwrap(),
            Spec::FmDemod { deviation: 3e3, tau: 0.0 }
        );
        assert_eq!(
            Spec::from_params("ssb_demod", &params(&[("mode", Param::Str("lsb".into()))])).unwrap(),
            Spec::SsbDemod { upper: false }
        );
        assert!(Spec::from_params("lowpass", &params(&[])).is_err());
        assert!(Spec::from_params("nope", &params(&[])).is_err());
        assert!(Spec::from_params("decimate", &params(&[("factor", Param::Num(2.5))])).is_err());
        assert_eq!(
            Spec::from_params("shift", &params(&[("hz", Param::Str("1.5k".into()))])).unwrap(),
            Spec::Shift { hz: 1500.0 }
        );
    }

    #[test]
    fn chain_threads_kind_and_rate_and_rejects_mismatches() {
        let c = Chain::build(
            vec![Spec::FmDemod { deviation: 5e3, tau: 0.0 }, Spec::Lowpass { cutoff: 3e3 }, Spec::Resample { rate: 8000.0 }],
            Kind::Complex,
            48_000.0,
        )
        .unwrap();
        assert_eq!(c.out_kind(), Kind::Real);
        assert_eq!(c.out_rate(), 8000.0);
        assert_eq!(c.describe(), "fm_demod(5000 Hz) >> lowpass(3000 Hz) >> resample(8000)");
        let e = Chain::build(vec![Spec::AmDemod, Spec::AmDemod], Kind::Complex, 48_000.0).err().unwrap();
        assert!(e.contains("block 2"), "{e}");
        assert!(Chain::build(vec![Spec::Lowpass { cutoff: 30e3 }], Kind::Real, 48_000.0).is_err());
        assert!(Chain::build(vec![Spec::Modem(ModemKind::Afsk1200)], Kind::Complex, 48_000.0).is_err());
    }

    #[test]
    fn fm_receiver_recovers_tone() {
        // 1 kHz tone, 5 kHz deviation, offset carrier phase, mild noise.
        let fs = 48_000.0;
        let tone: Vec<f32> = Tone::new(1000.0, fs, 0.8).real(48_000);
        let mut tx = transmitter("fm", fs, fs).unwrap();
        let Buf::C(mut iq) = tx.process(Buf::R(tone)).unwrap() else { panic!() };
        let mut rng = ecm_dsp::rng::Rng::new(1);
        ecm_dsp::rng::awgn(&mut iq, 0.01, &mut rng);
        let mut rx = receiver("fm", fs, 8000.0, None).unwrap();
        let mut audio = Vec::new();
        for c in iq.chunks(1000) {
            let Buf::R(a) = rx.process(Buf::C(c.to_vec())).unwrap() else { panic!() };
            audio.extend(a);
        }
        assert!((audio.len() as i64 - 8000).abs() < 20, "{}", audio.len());
        let a = &audio[2000..];
        let p1 = goertzel(a, 1000.0, 8000.0);
        let p3 = goertzel(a, 3000.0, 8000.0);
        assert!(p1 > 100.0 * p3, "tone {p1} vs {p3}");
    }

    pub(crate) fn goertzel(x: &[f32], f: f64, fs: f64) -> f64 {
        let w = std::f64::consts::TAU * f / fs;
        let (mut re, mut im) = (0.0, 0.0);
        for (n, &v) in x.iter().enumerate() {
            re += v as f64 * (w * n as f64).cos();
            im -= v as f64 * (w * n as f64).sin();
        }
        (re * re + im * im) / (x.len() as f64).powi(2)
    }

    #[test]
    fn am_and_ssb_receivers_recover_tone() {
        let fs = 48_000.0;
        for mode in ["am", "usb", "lsb"] {
            let tone = Tone::new(1000.0, fs, 0.7).real(48_000);
            let mut tx = transmitter(mode, fs, fs).unwrap();
            let Buf::C(iq) = tx.process(Buf::R(tone)).unwrap() else { panic!() };
            let mut rx = receiver(mode, fs, 8000.0, None).unwrap();
            let Buf::R(a) = rx.process(Buf::C(iq)).unwrap() else { panic!() };
            let a = &a[3000..];
            let p1 = goertzel(a, 1000.0, 8000.0);
            let p2 = goertzel(a, 2300.0, 8000.0);
            assert!(p1 > 50.0 * p2, "{mode}: {p1} vs {p2}");
        }
        // The wrong sideband hears (almost) nothing.
        let tone = Tone::new(1000.0, fs, 0.7).real(48_000);
        let Buf::C(iq) = transmitter("usb", fs, fs).unwrap().process(Buf::R(tone)).unwrap() else { panic!() };
        let demod = |upper: bool| {
            let mut c = Chain::build(vec![Spec::SsbDemod { upper }], Kind::Complex, fs).unwrap();
            let Buf::R(a) = c.process(Buf::C(iq.clone())).unwrap() else { panic!() };
            goertzel(&a[18_000..], 1000.0, fs)
        };
        let (pr, pw) = (demod(true), demod(false));
        assert!(pw * 100.0 < pr, "right {pr} wrong {pw}");
    }

    /// AM transmit: a full-scale tone keeps the envelope inside the SDR's
    /// cs16 range (+-1, clamped beyond), with the modulation depth intact.
    #[test]
    fn am_transmitter_peaks_fit_full_scale() {
        let fs = 48_000.0;
        let tone = Tone::new(1000.0, 8000.0, 1.0).real(8000);
        let Buf::C(iq) = transmitter("am", 8000.0, fs).unwrap().process(Buf::R(tone)).unwrap() else { panic!() };
        let env: Vec<f32> = iq[4800..].iter().map(|c| c.abs()).collect();
        let (lo, hi) = env.iter().fold((f32::MAX, 0.0f32), |(l, h), &v| (l.min(v), h.max(v)));
        assert!(hi <= 1.0, "envelope peak {hi} clips at cs16 full scale");
        let depth = (hi - lo) / (hi + lo);
        assert!((depth - 0.8).abs() < 0.05, "modulation depth {depth}");
        assert!(lo > 0.05, "carrier never drops out ({lo})");
    }

    #[test]
    fn squelch_mutes_noise_and_opens_on_signal() {
        let fs = 48_000.0;
        let mut rx = receiver("fm", fs, 48_000.0, Some(-20.0)).unwrap();
        let mut rng = ecm_dsp::rng::Rng::new(5);
        let mut noise = vec![C32::ZERO; 9600];
        ecm_dsp::rng::awgn(&mut noise, 1e-4, &mut rng);
        let Buf::R(quiet) = rx.process(Buf::C(noise)).unwrap() else { panic!() };
        assert!(quiet[2000..].iter().all(|v| v.abs() < 1e-3));
        let sig = Tone::new(500.0, fs, 0.5).complex(9600);
        let Buf::R(loud) = rx.process(Buf::C(sig)).unwrap() else { panic!() };
        assert!(loud[4000..].iter().any(|v| v.abs() > 0.01));
    }

    #[test]
    fn packet_modem_block_switches_direction_by_input() {
        let fs = 48_000.0;
        let frames = vec![b"hello radio world, this is a frame".to_vec(), b"second".repeat(5)];
        let mut tx = Chain::build(
            vec![Spec::Modem(ModemKind::Afsk1200), Spec::FmMod { deviation: 3e3, tau: 0.0 }],
            Kind::Frames,
            fs,
        )
        .unwrap();
        let Buf::C(mut iq) = tx.process(Buf::F(frames.clone())).unwrap() else { panic!() };
        iq.extend(vec![C32::new(1.0, 0.0); 4800]);
        let mut rx = Chain::build(
            vec![Spec::FmDemod { deviation: 3e3, tau: 0.0 }, Spec::Modem(ModemKind::Afsk1200)],
            Kind::Complex,
            fs,
        )
        .unwrap();
        let mut got = Vec::new();
        for c in iq.chunks(2048) {
            let Buf::F(f) = rx.process(Buf::C(c.to_vec())).unwrap() else { panic!() };
            got.extend(f);
        }
        assert_eq!(got, frames);
    }
}
