//! `messages` — Messages (dev.ecm.messages): texts and picture messages in a
//! window, over telephonyd's texts-only port (127.0.0.1:7726, the `sms`
//! permission). Needs the GUI.
//!
//!   messages [--to <number[,number]>] [--thread <key>] [--text <text>]

#[cfg(target_os = "wasi")]
fn main() {
    use ecm_tel_ui::messages::{MessagesApp, Recorder};
    use ecm_tel_ui::net::{Fs, Net};
    use ecm_ui::conn::{default_connection, ConnError};

    /// Voice notes from the phone's microphone: 8 kHz, 16-bit mono.
    struct Mic {
        mic: Option<ecm_audio::Mic>,
        pcm: Vec<u8>,
    }
    const RATE: u32 = 8000;
    impl Recorder for Mic {
        fn start(&mut self) -> Result<(), String> {
            let mut m = ecm_audio::Mic::open(None).map_err(|e| format!("no microphone ({e})"))?;
            m.set_format(RATE, 16, 1).map_err(|e| e.to_string())?;
            let _ = m.flush_queue();
            self.mic = Some(m);
            self.pcm.clear();
            Ok(())
        }
        fn pump(&mut self) {
            // 50 ms at a time, so the window keeps answering
            let mut buf = [0u8; (RATE as usize / 20) * 2];
            if let Some(m) = self.mic.as_mut() {
                if let Ok(n) = m.read_bytes(&mut buf) {
                    self.pcm.extend_from_slice(&buf[..n]);
                }
            }
        }
        fn stop(&mut self) -> Vec<u8> {
            self.mic = None;
            let mut w = ecm_audio::wav_header(RATE, 16, 1, self.pcm.len() as u32);
            w.extend_from_slice(&std::mem::take(&mut self.pcm));
            w
        }
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("usage: messages [--to <number[,number]>] [--thread <key>] [--text <text>]");
        return;
    }
    let conn = match default_connection(0) {
        Ok(c) => c,
        Err(ConnError::Console) => {
            println!("messages: Messages needs the GUI (start it with `gui`); in the console use sms");
            std::process::exit(1);
        }
        Err(e) => {
            println!("messages: {e}");
            std::process::exit(1);
        }
    };
    let mut app = match MessagesApp::with_recorder(conn, Net::new(ecm_tel::ctl::SMS_PORT), Fs, Mic { mic: None, pcm: Vec::new() }) {
        Ok(a) => a,
        Err(e) => {
            println!("messages: {e}");
            std::process::exit(1);
        }
    };
    app.model.borrow_mut().apply_args(&args);
    app.run();
}

#[cfg(not(target_os = "wasi"))]
fn main() {
    let _ = ecm_tel_ui::POLL_MS;
    let _ = ecm_ui::conn::default_connection;
    eprintln!("messages runs on a computer (wasm32-wasip1)");
}
