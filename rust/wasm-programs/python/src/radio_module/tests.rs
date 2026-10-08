//! Host tests of the Python `radio` module: a real RustPython interpreter
//! with `_radio` registered and radio.py loaded exactly as the bootstrap does,
//! driving flowgraphs over synthetic IQ files (no device needed: an "SDR" here
//! is a sample file plus a control file holding sdrctl-style status lines).

use super::*;
use ecm_dsp::iq::SampleFormat;
use ecm_dsp::modem::fm::{FmMod, FmParams};
use rustpython_vm::{compiler::Mode, Interpreter, Settings};
use std::path::PathBuf;

fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ecm-pyradio-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn p(path: &PathBuf) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Run `code` with `radio` importable; `vars` are set as string globals.
/// Panics with the Python traceback on an exception.
fn run_py(code: &str, vars: &[(&str, String)]) {
    let interp = Interpreter::with_init(Settings::default(), |vm| {
        vm.add_native_module("_radio".to_owned(), Box::new(radio_native::make_module));
    });
    interp.enter(|vm| {
        let scope = vm.new_scope_with_builtins();
        let setup = "import sys, _radio\nM = type(sys)('radio')\nexec(compile(_radio._source, '<radio>', 'exec'), M.__dict__)\nsys.modules['radio'] = M\n";
        for (k, v) in vars {
            scope.globals.set_item(*k, vm.ctx.new_str(v.as_str()).into(), vm).unwrap();
        }
        let full = format!("{setup}{code}");
        let code_obj = vm.compile(&full, Mode::Exec, "<test>".to_owned()).expect("test script compiles");
        if let Err(e) = vm.run_code_obj(code_obj, scope) {
            let mut s = String::new();
            vm.write_exception(&mut s, &e).unwrap();
            panic!("python raised:\n{s}");
        }
    });
}

fn goertzel(x: &[f32], f: f64, fs: f64) -> f64 {
    let w = std::f64::consts::TAU * f / fs;
    let (mut re, mut im) = (0.0, 0.0);
    for (n, &v) in x.iter().enumerate() {
        re += v as f64 * (w * n as f64).cos();
        im -= v as f64 * (w * n as f64).sin();
    }
    (re * re + im * im) / (x.len() as f64).powi(2)
}

/// A synthetic 1 kHz-tone NBFM recording in cs16, read through `radio.open`
/// (the SDR path, with the format taken from the control file), demodulated
/// by `sdr >> fm_demod >> lowpass >> wav_sink`: the WAV holds the tone.
#[test]
fn fm_demod_of_synthetic_iq_gives_the_tone() {
    let d = tmpdir("fm");
    let fs = 48_000.0f32;
    let audio: Vec<f32> = (0..48_000).map(|n| 0.8 * (std::f32::consts::TAU * 1000.0 * n as f32 / fs).sin()).collect();
    let iq = FmMod::new(FmParams { fs, deviation: 5_000.0, tau: 0.0 }).modulate_vec(&audio);
    let mut bytes = Vec::new();
    SampleFormat::Cs16.encode(&iq, &mut bytes);
    std::fs::write(d.join("sdr0"), &bytes).unwrap();
    std::fs::write(d.join("sdrctl0"), "tier sdr_standard\nfreq 146520000\nrate 48000\nformat cs16\ntimestamp 0\n").unwrap();
    let out = d.join("out.wav");
    run_py(
        r#"
import radio
sdr = radio.open(data_path=DATA, ctl_path=CTL)
assert sdr.rate == 48000 and sdr.format == "cs16" and sdr.freq == 146520000, (sdr.rate, sdr.format, sdr.freq)
fg = radio.Flowgraph(sdr >> radio.fm_demod(5e3) >> radio.lowpass(3e3) >> radio.wav_sink(OUT))
assert fg.describe() == "fm_demod(5000 Hz) >> lowpass(3000 Hz)", fg.describe()
fg.run(samples=48000)
assert fg.consumed == 48000, fg.consumed
"#,
        &[("DATA", p(&d.join("sdr0"))), ("CTL", p(&d.join("sdrctl0"))), ("OUT", p(&out))],
    );
    let wav = std::fs::read(&out).unwrap();
    let a = ecm_radio::audio::read_audio(&wav, 0);
    assert_eq!(a.rate, 48_000);
    assert_eq!(a.samples.len(), 48_000);
    let x = &a.samples[4800..];
    let (p1, p2, p3) = (goertzel(x, 1000.0, 48e3), goertzel(x, 2000.0, 48e3), goertzel(x, 500.0, 48e3));
    assert!(p1 > 1000.0 * p2 && p1 > 1000.0 * p3, "1 kHz {p1:e}, 2 kHz {p2:e}, 500 Hz {p3:e}");
    // amplitude: 0.8 in, ~0.8 out (sine of amplitude A has Goertzel power A^2/4)
    let amp = (p1 * 4.0).sqrt();
    assert!((amp - 0.8).abs() < 0.05, "amplitude {amp}");
    let _ = std::fs::remove_dir_all(&d);
}

/// AX.25 frames -> afsk1200 -> NBFM -> .cf32 + SigMF (iq_sink), then the
/// recording -> file_source (rate from the meta) -> fm_demod -> afsk1200 ->
/// frames: both frames come back intact.
#[test]
fn afsk1200_round_trip_through_the_radio_api() {
    let d = tmpdir("afsk");
    let iq = d.join("packet.cf32");
    run_py(
        r#"
import radio
msgs = [radio.ax25("APRS", "N0CALL-1", "hello from ecm"), radio.ax25("CQ", "N0CALL-1", "second frame " * 4)]
tx = radio.Flowgraph(radio.frames_source(msgs, rate=48000) >> radio.afsk1200() >> radio.fm_mod(3e3)
                     >> radio.iq_sink(IQ, freq=144.39e6, description="afsk test"))
tx.run()
src = radio.file_source(IQ)
assert src.rate == 48000 and src.freq == 144390000, (src.rate, src.freq)
got = []
rx = radio.Flowgraph(src >> radio.fm_demod(3e3) >> radio.afsk1200() >> radio.frames(got.append, echo=False))
rx.run()
assert len(got) == 2, got
d = radio.parse_ax25(got[0])
assert d["src"] == "N0CALL-1" and d["dest"] == "APRS" and d["text"] == "hello from ecm", d
assert radio.parse_ax25(got[1])["text"] == "second frame " * 4
"#,
        &[("IQ", p(&iq))],
    );
    let meta = std::fs::read_to_string(d.join("packet.sigmf-meta")).unwrap();
    let m = ecm_radio::sigmf::Meta::parse(&meta).unwrap();
    assert_eq!((m.format, m.sample_rate, m.frequency), (SampleFormat::Cf32, 48_000.0, 144.39e6));
    assert_eq!(m.description, "afsk test");
    let _ = std::fs::remove_dir_all(&d);
}

/// Composition rules, sources, conversions and SDR control.
#[test]
fn flowgraph_composition_and_sdr_control() {
    let d = tmpdir("comp");
    std::fs::write(d.join("ctl"), "rate 48000\nformat cf32\nfreq 100000000\ntimestamp 77\n").unwrap();
    std::fs::write(d.join("data"), b"").unwrap();
    run_py(
        r#"
import radio
# >> builds pipelines from blocks, sources and other pipelines
a = radio.tone(1000, rate=8000, seconds=1.0) >> radio.gain(0.5)
b = a >> (radio.real() >> radio.lowpass(2000))
assert len(b.items) == 4, b
fg = radio.Flowgraph(b)
assert fg.describe() == "gain(0.5) >> real >> lowpass(2000 Hz)", fg.describe()
out = fg.run()
s = out.samples
assert s.kind == "real" and len(s) == 8000 and s.rate == 8000, s
# chained resample changes the rate and the length
r = radio.Flowgraph(radio.tone(0, rate=48000, seconds=0.5) >> radio.resample(8000)).run().samples
assert r.rate == 8000 and abs(len(r) - 4000) <= 2, (r.rate, len(r))
# mismatched kinds are rejected when the graph is built
for bad in (radio.tone(1000) >> radio.am_mod(), radio.tone(1000, kind="real") >> radio.fm_demod()):
    try:
        radio.Flowgraph(bad)
        raise AssertionError("accepted %r" % bad)
    except ValueError as e:
        assert "takes a" in str(e), e
try:
    radio.Flowgraph(radio.lowpass(1000) >> radio.collect())
    raise AssertionError("no source accepted")
except radio.RadioError:
    pass
try:
    radio.Flowgraph(radio.tone(100) >> radio.afsk1200() >> radio.frames())
    raise AssertionError("complex into afsk accepted")
except ValueError as e:
    assert "afsk" in str(e), e
# samples <-> python numbers
sm = radio.Samples.from_list([1+2j, -0.5j], 1000)
assert sm.to_list() == [1+2j, -0.5j], sm.to_list()
assert abs(radio.tone(0, amplitude=1.0, seconds=0.01).read(10).power_db()) < 1e-3
# SDR: name resolution, status, control commands
assert radio.parse_freq("144.39M") == 144390000.0
assert _radio.resolve_sdr("sdr_1") == ("/dev/sdr1", "/dev/sdrctl1")
sdr = radio.open(data_path=DATA, ctl_path=CTL)
assert sdr.format == "cf32" and sdr.timestamp() == 77
sdr.tune("146.52M", rate=24000)
assert open(CTL).read().startswith("rate 24000\n"), open(CTL).read()
assert sdr.rate == 24000 and sdr.freq == 146520000
sdr.tx(True, 20)
assert open(CTL).read().startswith("tx on 20\n")
# an SDR sink resamples to its own rate
fg = radio.Flowgraph(radio.tone(500, rate=48000, seconds=0.1) >> sdr)
assert fg.describe() == "resample(24000)", fg.describe()
fg.run()
assert len(open(DATA, "rb").read()) == 2400 * 8
# signals in a block
blk = radio.tone(5000, rate=48000, amplitude=0.5).read(8192)
hits = radio.find_signals(blk, center=1e6)
assert len(hits) == 1 and abs(hits[0][0] - 1005000) < 100, hits
"#,
        &[("DATA", p(&d.join("data"))), ("CTL", p(&d.join("ctl")))],
    );
    let _ = std::fs::remove_dir_all(&d);
}
