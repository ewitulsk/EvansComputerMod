//! The Volume Mixer: `/dev/mixer` (docs/gui/APPS.md "Volume Mixer").
//!
//! Reading it lists the strips, one per line:
//!
//! ```text
//! output left volume=100 mute=0 peak=1200 label=Speaker
//! app dev.ecm.music volume=80 mute=0 route=- on=left peak=5000 ducked=1 role=media streams=1
//! input top gain=50 level=-42
//! ```
//!
//! Writing it changes them (`set <app|master[.side]> <percent>`, `mute <target> [on|off|toggle]`,
//! `route <app> <side|->`, `gain <input> <0-100>`, `reset <app>`). Apps need the `mixer`
//! permission to write; anyone may read. [`strips`] parses the text (host-testable),
//! [`read`] and [`command`] do the I/O.

use std::io::{Read, Write};

pub const DEVICE: &str = "/dev/mixer";
/// Highest app volume, in percent.
pub const MAX_APP_PERCENT: u32 = 150;

/// A speaker's master strip.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Output {
    pub side: String,
    pub volume: u32,
    pub mute: bool,
    /// Peak of the last mix, 0-32767.
    pub peak: u32,
    /// "Speaker", "ePhone speaker", "AirPods".
    pub label: String,
}

/// One app's strip (an app id, or a program's name for programs that aren't apps).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct AppStrip {
    pub app: String,
    pub volume: u32,
    pub mute: bool,
    /// The speaker it is routed to (None = the default one).
    pub route: Option<String>,
    /// The speaker it plays on now (None while it plays nothing).
    pub on: Option<String>,
    pub peak: u32,
    /// A louder voice or phone stream lowers it right now.
    pub ducked: bool,
    pub role: String,
    pub streams: u32,
}

/// A microphone's strip.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Input {
    pub side: String,
    pub gain: u32,
    /// Level in dB (-99 = silence).
    pub level: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Strips {
    pub outputs: Vec<Output>,
    pub apps: Vec<AppStrip>,
    pub inputs: Vec<Input>,
}

impl Strips {
    pub fn app(&self, app: &str) -> Option<&AppStrip> {
        self.apps.iter().find(|a| a.app == app)
    }

    pub fn output(&self, side: &str) -> Option<&Output> {
        self.outputs.iter().find(|o| o.side == side)
    }
}

fn kv(words: &[&str], key: &str) -> Option<String> {
    words.iter().find_map(|w| w.strip_prefix(key).and_then(|r| r.strip_prefix('=')).map(str::to_string))
}

fn num(words: &[&str], key: &str) -> u32 {
    kv(words, key).and_then(|v| v.parse().ok()).unwrap_or(0)
}

fn side(v: Option<String>) -> Option<String> {
    v.filter(|s| !s.is_empty() && s != "-")
}

/// Parse `/dev/mixer`'s text.
pub fn strips(text: &str) -> Strips {
    let mut out = Strips::default();
    for line in text.lines() {
        let w: Vec<&str> = line.split_whitespace().collect();
        if w.len() < 2 {
            continue;
        }
        let rest = &w[2..];
        match w[0] {
            "output" => out.outputs.push(Output {
                side: w[1].to_string(),
                volume: num(rest, "volume"),
                mute: num(rest, "mute") == 1,
                peak: num(rest, "peak"),
                label: kv(rest, "label").unwrap_or_default().replace('_', " "),
            }),
            "app" => out.apps.push(AppStrip {
                app: w[1].to_string(),
                volume: num(rest, "volume"),
                mute: num(rest, "mute") == 1,
                route: side(kv(rest, "route")),
                on: side(kv(rest, "on")),
                peak: num(rest, "peak"),
                ducked: num(rest, "ducked") == 1,
                role: kv(rest, "role").unwrap_or_else(|| "media".into()),
                streams: num(rest, "streams"),
            }),
            "input" => out.inputs.push(Input {
                side: w[1].to_string(),
                gain: num(rest, "gain"),
                level: kv(rest, "level").and_then(|v| v.parse().ok()).unwrap_or(-99),
            }),
            _ => {}
        }
    }
    out
}

/// A peak (0-32767) as a meter fill 0-100, on a dB scale from -60 dBFS.
pub fn meter_percent(peak: u32) -> u32 {
    if peak == 0 {
        return 0;
    }
    let db = 20.0 * (peak as f64 / 32767.0).log10();
    (((db + 60.0) / 60.0).clamp(0.0, 1.0) * 100.0).round() as u32
}

/// The strips now.
pub fn read() -> std::io::Result<Strips> {
    let mut f = std::fs::File::open(DEVICE)?;
    let mut text = String::new();
    f.read_to_string(&mut text)?;
    Ok(strips(&text))
}

/// One mixer command, e.g. `set dev.ecm.music 80`. Errors say why (bad value, no permission).
pub fn command(cmd: &str) -> std::io::Result<()> {
    let mut f = std::fs::OpenOptions::new().write(true).open(DEVICE)?;
    f.write_all(format!("{}\n", cmd.trim()).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_strip() {
        let t = "output left volume=100 mute=0 peak=1200 label=ePhone_speaker\n\
                 app dev.ecm.music volume=80 mute=0 route=- on=left peak=5000 ducked=1 role=media streams=1\n\
                 app aplay volume=100 mute=1 route=right on=- peak=0 ducked=0 role=voice streams=0\n\
                 input top gain=50 level=-42\n";
        let s = strips(t);
        assert_eq!(s.outputs[0], Output { side: "left".into(), volume: 100, mute: false, peak: 1200, label: "ePhone speaker".into() });
        let m = s.app("dev.ecm.music").unwrap();
        assert_eq!((m.volume, m.ducked, m.on.as_deref(), m.route.as_deref()), (80, true, Some("left"), None));
        let a = s.app("aplay").unwrap();
        assert!(a.mute && a.route.as_deref() == Some("right") && a.on.is_none() && a.role == "voice");
        assert_eq!(s.inputs[0], Input { side: "top".into(), gain: 50, level: -42 });
        assert!(strips("garbage\n\napp\n").apps.is_empty());
    }

    #[test]
    fn meters_are_logarithmic() {
        assert_eq!(meter_percent(0), 0);
        assert_eq!(meter_percent(32767), 100);
        assert_eq!(meter_percent(33), 0, "-60 dBFS and below is empty");
        assert!((45..=55).contains(&meter_percent(1036)), "{}", meter_percent(1036)); // -30 dBFS
    }
}
