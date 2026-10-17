//! Host tests of the Python `audio` module (audio.py, loaded as the bootstrap
//! does): the microphone side reads PCM and status from files standing in for
//! /dev/mic and /dev/micctl, with /etc/sound.conf choosing the default device.

use rustpython_vm::{compiler::Mode, Interpreter, Settings};
use std::path::PathBuf;

const AUDIO_PY: &str = include_str!("audio.py");

fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ecm-pyaudio-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Run `code` with `audio` importable and builtins.open / os.stat redirecting /dev/ and /etc/ into DIR.
fn run_py(code: &str, dir: &str) {
    let interp = Interpreter::with_init(Settings::default(), |_| {});
    interp.enter(|vm| {
        let scope = vm.new_scope_with_builtins();
        scope.globals.set_item("DIR", vm.ctx.new_str(dir).into(), vm).unwrap();
        scope.globals.set_item("AUDIO_SRC", vm.ctx.new_str(AUDIO_PY).into(), vm).unwrap();
        let setup = r#"
import sys, builtins
_real_open = builtins.open
def _redirect(p):
    if isinstance(p, str):
        if p.startswith("/dev/"):
            return DIR + "/" + p[5:]
        if p.startswith("/etc/"):
            return DIR + "/" + p[5:]
    return p
builtins.open = lambda p, *a, **k: _real_open(_redirect(p), *a, **k)
M = type(sys)('audio')
exec(compile(AUDIO_SRC, '<audio>', 'exec'), M.__dict__)
sys.modules['audio'] = M
"#;
        let full = format!("{setup}{code}");
        let code_obj = vm.compile(&full, Mode::Exec, "<test>".to_owned()).expect("test script compiles");
        if let Err(e) = vm.run_code_obj(code_obj, scope) {
            let mut s = String::new();
            vm.write_exception(&mut s, &e).unwrap();
            panic!("python raised:\n{s}");
        }
    });
}

#[test]
fn python_microphone_records_and_reads_status() {
    let d = tmpdir("mic");
    let mut pcm = Vec::new();
    for i in 0..80i16 {
        let v: i16 = if i == 7 { 16384 } else { i * 10 };
        pcm.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(d.join("mic.top"), &pcm).unwrap();
    std::fs::write(d.join("micctl.top"), "rate 8000\nbits 16\nchannels 1\ngain 50\nlevel_db -6\noverruns 0\n").unwrap();
    std::fs::write(d.join("sound.conf"), "output left\ninput top\n").unwrap();
    let dir = d.to_string_lossy().replace('\\', "/");
    run_py(
        r#"
import audio
assert audio.default_device("input") == "top" and audio.default_device("output") == "left"
assert audio.default_device("other") is None
mic = audio.open_mic("top")
assert mic.device == "/dev/mic.top", mic.device
assert (mic.rate, mic.bits, mic.channels) == (8000, 16, 1)
pcm = mic.record(0.01)
assert len(pcm) == 160, len(pcm)
assert audio.peak_db(pcm) == -6, audio.peak_db(pcm)
assert audio.peak_db(bytes(10)) == -99
assert audio.peak_db(b"\x00\x80") == 0
assert mic.level() == -6 and mic.status()["overruns"] == 0
mic.close()
audio.save_wav(DIR + "/memo.wav", pcm, 8000)
"#,
        &dir,
    );
    let wav = std::fs::read(d.join("memo.wav")).unwrap();
    assert_eq!(&wav[0..4], b"RIFF");
    assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 8000, "rate");
    assert_eq!(u16::from_le_bytes(wav[34..36].try_into().unwrap()), 16, "bits");
    assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 160, "data length");
    assert_eq!(&wav[44..], &pcm[..]);
    let _ = std::fs::remove_dir_all(&d);
}
