//! Modem round trips: modulate -> AWGN channel -> demodulate, with stated
//! performance targets. All noise is seeded, so results are deterministic.

use ecm_dsp::coding::ax25::{Address, UiFrame};
use ecm_dsp::complex::{mean_power, mean_power_real, C32};
use ecm_dsp::fir::{lowpass_kaiser, FirRR};
use ecm_dsp::modem::am::{AmDemod, AmMod};
use ecm_dsp::modem::chirp::{ChirpMod, ChirpParams, ChirpRx};
use ecm_dsp::modem::cw::{morse_decode, morse_keying, wpm_to_baud, OokDemod, OokMod};
use ecm_dsp::modem::fm::{FmDemod, FmMod, FmParams};
use ecm_dsp::modem::fsk::{AfskParams, FskDemod, FskMod, PacketRx, PacketTx};
use ecm_dsp::modem::psk::{slice, PskDemod, PskMod, PskParams};
use ecm_dsp::modem::ssb::{Sideband, SsbDemod, SsbMod, WeaverDemod};
use ecm_dsp::rng::{awgn, awgn_real, Rng};
use std::f64::consts::TAU;

/// SNR (dB) of a tone at `f` Hz in `x`: least-squares fit of `a cos + b sin + c`,
/// residual = noise + distortion.
fn tone_snr(x: &[f32], f: f64, fs: f64) -> (f32, f32) {
    let (mut cc, mut ss, mut cs, mut xc, mut xs, mut sum) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    let n = x.len() as f64;
    for (i, &v) in x.iter().enumerate() {
        let (s, c) = (TAU * f * i as f64 / fs).sin_cos();
        cc += c * c;
        ss += s * s;
        cs += c * s;
        xc += v as f64 * c;
        xs += v as f64 * s;
        sum += v as f64;
    }
    let det = cc * ss - cs * cs;
    let a = (xc * ss - xs * cs) / det;
    let b = (xs * cc - xc * cs) / det;
    let mean = sum / n;
    let mut err = 0.0;
    for (i, &v) in x.iter().enumerate() {
        let (s, c) = (TAU * f * i as f64 / fs).sin_cos();
        let e = v as f64 - a * c - b * s - mean;
        err += e * e;
    }
    let amp = (a * a + b * b).sqrt();
    let sig = amp * amp / 2.0;
    ((10.0 * (sig / (err / n)).log10()) as f32, amp as f32)
}

fn tone(n: usize, f: f64, fs: f64, amp: f32) -> Vec<f32> {
    (0..n).map(|i| amp * (TAU * f * i as f64 / fs).sin() as f32).collect()
}

fn audio_lpf(x: &[f32], cutoff: f32, fs: f32) -> Vec<f32> {
    FirRR::new(lowpass_kaiser(cutoff / fs, 1000.0 / fs, 60.0)).process_vec(x)
}

// ---------------------------------------------------------------- analog

#[test]
fn am_envelope_round_trip() {
    let fs = 48_000.0;
    let mut rng = Rng::new(10);
    let audio = tone(48_000, 1000.0, fs as f64, 0.8);
    let m = AmMod::new(0.8, 0.5);
    let mut rf = m.modulate_vec(&audio);
    // small carrier offset: envelope detection doesn't care
    let mut nco = ecm_dsp::nco::Nco::new(25.0, fs as f64);
    nco.mix_up_inplace(&mut rf);
    let p = mean_power(&rf);
    awgn(&mut rf, p / 1000.0, &mut rng); // 30 dB CNR over the full 48 kHz
    let mut d = AmDemod::new(fs, 0.8);
    let out = audio_lpf(&d.demod_vec(&rf), 4000.0, fs);
    let (snr, amp) = tone_snr(&out[12_000..], 1000.0, fs as f64);
    eprintln!("AM: audio SNR {snr:.1} dB, amplitude {amp:.3}");
    assert!(snr > 25.0, "AM audio SNR {snr}");
    assert!((amp - 0.8).abs() < 0.05, "AM amplitude {amp}");
}

#[test]
fn nbfm_round_trip_audio_snr() {
    let fs = 48_000.0;
    let mut rng = Rng::new(11);
    let audio = tone(48_000, 1000.0, fs as f64, 0.5);
    let p = FmParams::nbfm(fs);
    let mut rf = FmMod::new(p).modulate_vec(&audio);
    awgn(&mut rf, 1.0 / ecm_dsp::complex::from_db(15.0), &mut rng); // CNR 15 dB in 48 kHz
    let out = audio_lpf(&FmDemod::new(p).demod_vec(&rf), 3000.0, fs);
    let (snr, amp) = tone_snr(&out[12_000..], 1000.0, fs as f64);
    eprintln!("NBFM: CNR 15 dB/48 kHz -> audio SNR {snr:.1} dB, amplitude {amp:.3}");
    assert!(snr > 25.0, "NBFM audio SNR {snr}");
    assert!((amp - 0.5).abs() < 0.02);
}

#[test]
fn wbfm_round_trip_with_emphasis() {
    let fs = 240_000.0;
    let mut rng = Rng::new(12);
    let audio = tone(120_000, 1000.0, fs as f64, 0.5);
    let p = FmParams::wbfm(fs);
    let mut rf = FmMod::new(p).modulate_vec(&audio);
    awgn(&mut rf, 1.0 / ecm_dsp::complex::from_db(20.0), &mut rng);
    let out = audio_lpf(&FmDemod::new(p).demod_vec(&rf), 15_000.0, fs);
    let (snr, amp) = tone_snr(&out[30_000..], 1000.0, fs as f64);
    eprintln!("WBFM: CNR 20 dB/240 kHz -> audio SNR {snr:.1} dB, amplitude {amp:.3}");
    assert!(snr > 40.0, "WBFM audio SNR {snr}");
    assert!((amp - 0.5).abs() < 0.02);
}

#[test]
fn ssb_usb_lsb_phasing_and_weaver() {
    let fs = 8000.0;
    let audio = tone(16_000, 1000.0, fs as f64, 1.0);
    for sb in [Sideband::Upper, Sideband::Lower] {
        let rf = SsbMod::new(sb, 127).modulate_vec(&audio);
        // spectrum: the wanted sideband dominates the unwanted one by > 40 dB
        let seg = &rf[4000..4000 + 4096];
        let w = ecm_dsp::window::Window::BlackmanHarris.periodic(4096);
        let mut buf: Vec<C32> = seg.iter().zip(&w).map(|(v, w)| v.scale(*w)).collect();
        ecm_dsp::fft::Fft::new(4096).forward(&mut buf);
        let k = (1000.0 / fs * 4096.0) as usize;
        let near = |c: usize| (c - 3..=c + 3).map(|i| buf[i % 4096].norm_sqr()).fold(0.0, f32::max);
        let (pos, neg) = (near(k), near(4096 - k));
        let ratio = 10.0 * if sb == Sideband::Upper { pos / neg } else { neg / pos }.log10();
        assert!(ratio > 40.0, "{sb:?} sideband suppression {ratio}");

        let out = SsbDemod::new(sb, 127).demod_vec(&rf);
        let (snr, amp) = tone_snr(&out[2000..], 1000.0, fs as f64);
        assert!(snr > 35.0 && (amp - 1.0).abs() < 0.02, "{sb:?} phasing demod snr {snr} amp {amp}");
        let wrong = if sb == Sideband::Upper { Sideband::Lower } else { Sideband::Upper };
        let rej = SsbDemod::new(wrong, 127).demod_vec(&rf);
        assert!(mean_power_real(&rej[2000..]) < 1e-3 * mean_power_real(&out[2000..]), "{sb:?} opposite sideband leaks");

        let out = WeaverDemod::new(sb, fs, 300.0, 2700.0).demod_vec(&rf);
        let (snr, amp) = tone_snr(&out[3000..], 1000.0, fs as f64);
        assert!(snr > 35.0 && (amp - 1.0).abs() < 0.03, "{sb:?} weaver snr {snr} amp {amp}");
        let rej = WeaverDemod::new(wrong, fs, 300.0, 2700.0).demod_vec(&rf);
        assert!(mean_power_real(&rej[3000..]) < 1e-4, "{sb:?} weaver opposite sideband leaks");
    }
}

// ---------------------------------------------------------------- CW / OOK

#[test]
fn cw_morse_round_trip_in_noise() {
    let fs = 8000.0;
    let wpm = 20.0;
    let text = "CQ CQ DE N0CALL K";
    let mut keys = vec![0u8; 10];
    keys.extend(morse_keying(text));
    keys.extend([0u8; 10]);
    let mut m = OokMod::new(fs, wpm_to_baud(wpm), 600.0, 0.005);
    let mut rf = m.modulate_vec(&keys);
    let mut rng = Rng::new(13);
    awgn(&mut rf, 2.0, &mut rng); // -3 dB SNR over 8 kHz (~19 dB in a 50 Hz CW filter)
    let mut d = OokDemod::new(fs, wpm_to_baud(wpm), 600.0);
    let mut units = Vec::new();
    d.demod(&rf, &mut units);
    let got = morse_decode(&units);
    eprintln!("CW: decoded '{got}'");
    assert_eq!(got.trim(), text);
}

#[test]
fn morse_tables_round_trip() {
    let text = "HELLO WORLD 73 ?";
    assert_eq!(morse_decode(&morse_keying(text)), text);
}

// ---------------------------------------------------------------- FSK / AFSK

fn errors_aligned(got: &[u8], want: &[u8], max_shift: usize, skip: usize) -> (usize, usize) {
    // find the shift of `got` that best matches `want`, then count errors after `skip`
    let probe = 200;
    let (shift, _) = (0..max_shift)
        .map(|s| {
            let e = (0..probe).filter(|&i| got.get(s + skip + i) != want.get(skip + i)).count();
            (s, e)
        })
        .min_by_key(|x| x.1)
        .unwrap();
    let n = want.len().min(got.len().saturating_sub(shift)) - skip - 1;
    let e = (skip..skip + n).filter(|&i| got[i + shift] != want[i]).count();
    (e, n)
}

#[test]
fn fsk_baseband_ber() {
    let fs = 48_000.0;
    let mut rng = Rng::new(14);
    let bits: Vec<u8> = (0..20_000).map(|_| rng.bit()).collect();
    let mut rf = FskMod::new(fs, 1200.0, 1200.0).modulate_vec(&bits);
    awgn(&mut rf, 1.0 / ecm_dsp::complex::from_db(10.0), &mut rng);
    let mut out = Vec::new();
    FskDemod::new(fs, 1200.0).demod(&rf, &mut out);
    let (e, n) = errors_aligned(&out, &bits, 10, 50);
    let ber = e as f64 / n as f64;
    eprintln!("2-FSK 1200 bd, dev 1.2 kHz, SNR 10 dB/48 kHz: BER {ber:.2e} ({e}/{n})");
    assert!(ber < 1e-3, "FSK BER {ber}");
}

fn test_frame(i: usize) -> Vec<u8> {
    let info = format!("!4903.50N/07201.75W-ECM packet #{i:03} hello from the radio lane");
    UiFrame::new(Address::parse("APECM").unwrap(), Address::parse("N0CALL-7").unwrap(), info.as_bytes())
        .encode()
        .unwrap()
}

/// AFSK1200 audio for `count` frames separated by silence, plus the clean signal power.
fn afsk_burst(fs: f32, count: usize) -> (Vec<f32>, f32) {
    let mut tx = PacketTx::new(AfskParams::bell202(fs), 0.5);
    let mut audio = vec![0.0f32; (fs * 0.2) as usize];
    let mut sig_p = 0.0;
    for i in 0..count {
        let mut pkt = Vec::new();
        tx.send(&test_frame(i), &mut pkt);
        sig_p = mean_power_real(&pkt);
        audio.extend(pkt);
        audio.extend(vec![0.0f32; (fs * 0.1) as usize]);
    }
    (audio, sig_p)
}

#[test]
fn afsk1200_packets_decode_at_15db() {
    let fs = 48_000.0;
    let n = 10;
    let (mut audio, sig_p) = afsk_burst(fs, n);
    let mut rng = Rng::new(15);
    awgn_real(&mut audio, sig_p / ecm_dsp::complex::from_db(15.0), &mut rng); // 15 dB over 24 kHz
    let mut rx = PacketRx::new(AfskParams::bell202(fs));
    let mut frames = Vec::new();
    for chunk in audio.chunks(4096) {
        frames.extend(rx.push(chunk));
    }
    eprintln!("AFSK1200 @15 dB: {}/{} frames, {} bad FCS", frames.len(), n, rx.bad_fcs());
    assert_eq!(frames.len(), n);
    for (i, f) in frames.iter().enumerate() {
        assert_eq!(f, &test_frame(i));
        let ui = UiFrame::parse(f).unwrap();
        assert_eq!(ui.src.to_string(), "N0CALL-7");
    }
}

#[test]
fn afsk1200_over_nbfm_link() {
    // The real VHF path: AFSK audio -> NBFM (3 kHz dev) -> AWGN -> FM demod -> packet RX.
    let fs = 48_000.0;
    let (audio, _) = afsk_burst(fs, 5);
    let p = FmParams { fs, deviation: 3_000.0, tau: 0.0 };
    let mut rf = FmMod::new(p).modulate_vec(&audio);
    let mut rng = Rng::new(16);
    awgn(&mut rf, 1.0 / ecm_dsp::complex::from_db(8.0), &mut rng); // CNR 8 dB over 48 kHz
    let demod = FmDemod::new(p).demod_vec(&rf);
    let mut rx = PacketRx::new(AfskParams::bell202(fs));
    let frames = rx.push(&demod);
    eprintln!("AFSK1200 over NBFM, CNR 8 dB/48 kHz: {}/5 frames", frames.len());
    assert_eq!(frames.len(), 5);
}

#[test]
fn afsk1200_success_rate_vs_snr() {
    // Informational sweep (asserts only the monotone trend at the extremes).
    let fs = 48_000.0;
    let mut rates = Vec::new();
    for &snr in &[0.0f32, 3.0, 6.0, 10.0] {
        let (mut audio, sig_p) = afsk_burst(fs, 10);
        let mut rng = Rng::new(100 + snr as u64);
        awgn_real(&mut audio, sig_p / ecm_dsp::complex::from_db(snr), &mut rng);
        let n = PacketRx::new(AfskParams::bell202(fs)).push(&audio).len();
        eprintln!("AFSK1200 SNR {snr:4.1} dB/24 kHz: {n}/10 frames");
        rates.push(n);
    }
    assert!(rates[3] == 10, "10 dB must decode everything");
    assert!(rates[0] <= rates[3]);
}

#[test]
fn afsk_pure_noise_decodes_nothing() {
    let fs = 48_000.0;
    let mut rng = Rng::new(17);
    let mut noise = vec![0.0f32; 48_000 * 5];
    awgn_real(&mut noise, 0.1, &mut rng);
    let mut rx = PacketRx::new(AfskParams::bell202(fs));
    assert!(rx.push(&noise).is_empty());
}

// ---------------------------------------------------------------- PSK

fn ebn0_noise(ebn0_db: f32, bits_per_symbol: usize) -> f32 {
    // unit-energy symbols + unit-energy RRC: Es = 1 per symbol, N0 = sigma^2 (complex)
    let eb = 1.0 / bits_per_symbol as f32;
    eb / ecm_dsp::complex::from_db(ebn0_db)
}

/// Ideal coherent receiver (known timing and phase) BER.
fn psk_ideal_ber(p: PskParams, ebn0_db: f32, nbits: usize, seed: u64) -> f64 {
    let mut rng = Rng::new(seed);
    let bits: Vec<u8> = (0..nbits).map(|_| rng.bit()).collect();
    let mut m = PskMod::new(p);
    let mut rf = m.modulate_vec(&bits);
    m.flush(&mut rf);
    awgn(&mut rf, ebn0_noise(ebn0_db, p.bits_per_symbol()), &mut rng);
    let mut mf = ecm_dsp::fir::FirRC::new(ecm_dsp::fir::rrc(p.sps, p.span, p.beta));
    let y = mf.process_vec(&rf);
    let d = p.sps * p.span; // tx + rx group delay
    let mut errs = 0;
    let k = p.bits_per_symbol();
    for (i, c) in bits.chunks(k).enumerate() {
        let idx = slice(p.order, y[d + i * p.sps]);
        errs += (idx & 1 != c[0]) as usize;
        if k == 2 {
            errs += ((idx >> 1) & 1 != c[1]) as usize;
        }
    }
    errs as f64 / nbits as f64
}

/// Full receiver (timing + carrier recovery) through a channel with phase,
/// frequency and timing offsets. Resolves the Costas phase ambiguity with the
/// first symbols (as a known preamble would).
fn psk_sync_ber(p: PskParams, ebn0_db: f32, nbits: usize, seed: u64) -> f64 {
    psk_sync_ber_clock(p, ebn0_db, nbits, seed, None)
}

fn psk_sync_ber_clock(p: PskParams, ebn0_db: f32, nbits: usize, seed: u64, clock: Option<(usize, usize)>) -> f64 {
    let mut rng = Rng::new(seed);
    let bits: Vec<u8> = (0..nbits).map(|_| rng.bit()).collect();
    let mut m = PskMod::new(p);
    let mut rf = vec![C32::ZERO; 13]; // timing offset (not a multiple of sps)
    m.modulate(&bits, &mut rf);
    m.flush(&mut rf);
    // carrier phase and frequency offset (the 13-sample lead is a quarter-symbol timing offset)
    let mut nco = ecm_dsp::nco::Nco::new(0.0002, 1.0); // 2e-4 cycles/sample
    nco.set_phase(1.1);
    let mut ch = match clock {
        Some((l, m)) => ecm_dsp::resample::ResamplerC::new(l, m).process_vec(&rf),
        None => rf,
    };
    nco.mix_up_inplace(&mut ch);
    awgn(&mut ch, ebn0_noise(ebn0_db, p.bits_per_symbol()), &mut rng);
    let mut d = PskDemod::new(p);
    let mut syms = Vec::new();
    for c in ch.chunks(1000) {
        d.demod_symbols(c, &mut syms);
    }
    let k = p.bits_per_symbol();
    let want: Vec<u8> = bits.chunks(k).map(|c| if k == 2 { c[0] | (c[1] << 1) } else { c[0] }).collect();
    // acquisition: skip 2000 symbols, find symbol offset and rotation on 300 symbols
    let skip = 2000;
    let rots = if p.order == 2 { 2 } else { 4 };
    let mut best = (usize::MAX, 0, 0);
    for off in 0..60 {
        for r in 0..rots {
            let rot = C32::expj(std::f32::consts::TAU * r as f32 / rots as f32);
            let e: usize = (0..300)
                .filter(|&i| slice(p.order, syms[off + skip + i] * rot) != want[skip + i])
                .count();
            if e < best.0 {
                best = (e, off, r);
            }
        }
    }
    let (_, off, r) = best;
    let rot = C32::expj(std::f32::consts::TAU * r as f32 / rots as f32);
    let n = (want.len() - skip - 20).min(syms.len() - off - skip);
    let mut errs = 0;
    for i in skip..skip + n {
        let idx = slice(p.order, syms[off + i] * rot);
        errs += ((idx ^ want[i]) & 1) as usize + (((idx ^ want[i]) >> 1) & 1) as usize;
    }
    errs as f64 / (n * k) as f64
}

#[test]
fn bpsk_ber_at_7db() {
    let p = PskParams::bpsk(4);
    let ideal = psk_ideal_ber(p, 7.0, 200_000, 20);
    let synced = psk_sync_ber(p, 7.0, 200_000, 21);
    eprintln!("BPSK Eb/N0 7 dB: ideal BER {ideal:.2e}, full sync BER {synced:.2e} (theory 7.7e-4)");
    assert!(ideal < 1e-3, "ideal BPSK BER {ideal}");
    assert!(synced < 1e-3, "synced BPSK BER {synced}");
}

#[test]
fn qpsk_ber_at_7db() {
    let p = PskParams::qpsk(4);
    let ideal = psk_ideal_ber(p, 7.0, 200_000, 22);
    let synced = psk_sync_ber(p, 7.0, 200_000, 23);
    eprintln!("QPSK Eb/N0 7 dB: ideal BER {ideal:.2e}, full sync BER {synced:.2e} (theory 7.7e-4)");
    assert!(ideal < 1e-3, "ideal QPSK BER {ideal}");
    assert!(synced < 1.5e-3, "synced QPSK BER {synced}");
}

#[test]
fn psk_ber_curve() {
    // Informational: BER vs Eb/N0 for the full BPSK receiver; must fall monotonically.
    let p = PskParams::bpsk(4);
    let mut last = 1.0;
    for &e in &[2.0f32, 4.0, 6.0, 8.0] {
        let b = psk_sync_ber(p, e, 50_000, 30 + e as u64);
        eprintln!("BPSK sync Eb/N0 {e:.0} dB: BER {b:.2e}");
        assert!(b <= last);
        last = b;
    }
}

#[test]
fn differential_psk_needs_no_ambiguity_resolution() {
    for p in [
        PskParams { differential: true, ..PskParams::bpsk(4) },
        PskParams { differential: true, ..PskParams::qpsk(4) },
    ] {
        let mut rng = Rng::new(40);
        let bits: Vec<u8> = (0..20_000).map(|_| rng.bit()).collect();
        let mut m = PskMod::new(p);
        let mut rf = m.modulate_vec(&bits);
        m.flush(&mut rf);
        let mut nco = ecm_dsp::nco::Nco::new(0.0, 1.0);
        nco.set_phase(2.0); // arbitrary carrier phase
        nco.mix_up_inplace(&mut rf);
        awgn(&mut rf, ebn0_noise(12.0, p.bits_per_symbol()), &mut rng);
        let mut d = PskDemod::new(p);
        let mut out = Vec::new();
        d.demod_bits(&rf, &mut out);
        let (e, n) = errors_aligned(&out, &bits, 40, 2000);
        eprintln!("D{}PSK Eb/N0 12 dB: {e}/{n} errors", if p.order == 2 { "B" } else { "Q" });
        assert!((e as f64) < n as f64 * 1e-3, "{e}/{n}");
    }
}

// ---------------------------------------------------------------- chirp

fn chirp_stream(p: ChirpParams, packets: &[Vec<u8>], snr_db: f32, cfo_bins: f64, seed: u64) -> Vec<C32> {
    let mut rng = Rng::new(seed);
    let mut m = ChirpMod::new(p);
    let mut x = Vec::new();
    for pkt in packets {
        let gap = 1000 + rng.below(3 * p.symbol_len() as u64) as usize;
        x.extend(vec![C32::ZERO; gap]);
        m.modulate_packet(pkt, &mut x);
    }
    x.extend(vec![C32::ZERO; 2 * p.symbol_len()]);
    if cfo_bins != 0.0 {
        let f = cfo_bins * p.bw as f64 / p.chips() as f64;
        ecm_dsp::nco::Nco::new(f, p.fs() as f64).mix_up_inplace(&mut x);
    }
    // SNR measured in the chirp bandwidth: noise power in BW = total / os
    let noise = p.os as f32 / ecm_dsp::complex::from_db(snr_db);
    awgn(&mut x, noise, &mut rng);
    x
}

fn chirp_receive(p: ChirpParams, x: &[C32]) -> (Vec<Vec<u8>>, u64) {
    let mut rx = ChirpRx::new(p);
    let mut got = Vec::new();
    for c in x.chunks(1500) {
        got.extend(rx.push(c));
    }
    (got, rx.crc_errors)
}

fn payloads(n: usize, seed: u64) -> Vec<Vec<u8>> {
    let mut rng = Rng::new(seed);
    (0..n)
        .map(|i| {
            let mut v = format!("sensor {i}: ").into_bytes();
            let mut extra = vec![0u8; 4 + (rng.below(12) as usize)];
            rng.fill_bytes(&mut extra);
            v.extend(extra);
            v
        })
        .collect()
}

#[test]
fn chirp_sf7_decodes_at_minus_5db() {
    let p = ChirpParams::new(7, 125_000.0, 1);
    let pk = payloads(20, 50);
    let x = chirp_stream(p, &pk, -5.0, 0.0, 51);
    let (got, crc) = chirp_receive(p, &x);
    eprintln!("CSS SF7 @ -5 dB: {}/{} packets, {crc} CRC errors", got.len(), pk.len());
    assert_eq!(got, pk);
}

#[test]
fn chirp_sf12_decodes_at_minus_15db_and_minus_20db() {
    let p = ChirpParams::new(12, 125_000.0, 1);
    for (snr, seed) in [(-15.0f32, 52u64), (-20.0, 53)] {
        let pk = payloads(3, seed);
        let x = chirp_stream(p, &pk, snr, 0.0, seed + 100);
        let (got, crc) = chirp_receive(p, &x);
        eprintln!("CSS SF12 @ {snr} dB: {}/{} packets, {crc} CRC errors", got.len(), pk.len());
        assert_eq!(got, pk, "SF12 at {snr} dB");
    }
}

#[test]
fn chirp_oversampled_with_timing_and_carrier_offsets() {
    // Random sample-level gaps give arbitrary (half-chip) timing; CFO in bins, incl. fractional.
    for (os, cfo, seed) in [(2usize, 3.0f64, 55u64), (2, -5.3, 56), (4, 0.4, 57), (3, 7.6, 58)] {
        let mut p = ChirpParams::new(9, 62_500.0, os);
        p.preamble = 10;
        let pk = payloads(8, seed);
        let x = chirp_stream(p, &pk, -8.0, cfo, seed + 10);
        let (got, crc) = chirp_receive(p, &x);
        eprintln!("CSS SF9 os={os} CFO {cfo} bins @ -8 dB: {}/{} packets, {crc} CRC errors", got.len(), pk.len());
        assert_eq!(got, pk, "os={os} cfo={cfo}");
    }
}

#[test]
fn chirp_noise_only_decodes_nothing() {
    let p = ChirpParams::new(7, 125_000.0, 1);
    let mut rng = Rng::new(57);
    let mut x = vec![C32::ZERO; 1_000_000];
    awgn(&mut x, 1.0, &mut rng);
    let (got, _) = chirp_receive(p, &x);
    assert!(got.is_empty());
}

#[test]
fn cw_noise_only_decodes_nothing() {
    let fs = 8000.0;
    let mut rf = vec![C32::ZERO; 80_000];
    let mut rng = Rng::new(18);
    awgn(&mut rf, 1.0, &mut rng);
    let mut d = OokDemod::new(fs, wpm_to_baud(20.0), 600.0);
    let mut units = Vec::new();
    d.demod(&rf, &mut units);
    assert!(units.iter().all(|&u| u == 0), "noise keyed the slicer");
    assert_eq!(morse_decode(&units), "");
}

#[test]
fn psk_tracks_sample_clock_offset() {
    // TX clock 0.1% fast relative to RX: the Gardner loop must track the drift.
    let p = PskParams::qpsk(4);
    let ber = psk_sync_ber_clock(p, 10.0, 100_000, 60, Some((1001, 1000)));
    eprintln!("QPSK Eb/N0 10 dB with 1000 ppm clock offset: BER {ber:.2e} (theory 3.9e-6)");
    assert!(ber < 1e-4, "clock-offset BER {ber}");
}

#[test]
fn mueller_muller_timing_recovery() {
    use ecm_dsp::sync::{Decision, SymbolSync, Ted};
    let p = PskParams::bpsk(4);
    let mut rng = Rng::new(61);
    let bits: Vec<u8> = (0..20_000).map(|_| rng.bit()).collect();
    let mut m = PskMod::new(p);
    let mut rf = vec![C32::ZERO; 3]; // 3/4-symbol timing offset
    m.modulate(&bits, &mut rf);
    m.flush(&mut rf);
    awgn(&mut rf, ebn0_noise(10.0, 1), &mut rng);
    let mf = ecm_dsp::fir::FirRC::new(ecm_dsp::fir::rrc(p.sps, p.span, p.beta)).process_vec(&rf);
    let mut ss = SymbolSync::new(Ted::MuellerMuller, Decision::Bpsk, p.sps as f32, 0.005, 0.01);
    let syms = ss.process_vec(&mf);
    let out: Vec<u8> = syms.iter().map(|y| slice(2, *y)).collect();
    let (e, n) = errors_aligned(&out, &bits, 30, 500);
    eprintln!("BPSK + Mueller-Muller, Eb/N0 10 dB: {e}/{n} errors");
    assert!((e as f64) < n as f64 * 1e-3);
}
