//! `mixer`: the Volume Mixer (`/dev/mixer`) from the console.
//!
//!   mixer                              the strips, with text level meters
//!   mixer set <app|master[.side]> <n>  an app 0-150 %, a speaker 0-100 %
//!   mixer mute|unmute <app|master[.side]>
//!   mixer route <app> <side|->         play an app on another speaker
//!   mixer gain <microphone> <0-100>
//!   mixer reset <app>                  forget an app's settings
//!   mixer watch                        live meters until q or Ctrl+C
//!
//! This library holds what is tested on the host: the argument checks
//! ([`parse`]), the table ([`table`]) and the plain-words errors ([`explain`]).

use ecm_audio::mixer::{meter_percent, Strips, MAX_APP_PERCENT};

pub const USAGE: &str = "usage: mixer                              show the strips
       mixer set <app|master[.side]> <0-150>
       mixer mute <app|master[.side]> [on|off|toggle]
       mixer unmute <app|master[.side]>
       mixer route <app> <side|->
       mixer gain <microphone> <0-100>
       mixer reset <app>
       mixer watch                        live meters (q to quit)";

/// What the command line asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cmd {
    Show,
    Watch,
    Help,
    /// One `/dev/mixer` command line, already checked.
    Send(String),
}

/// `master` or `master.<side>`: a speaker's own strip.
pub fn is_master(target: &str) -> bool {
    target == "master" || target.starts_with("master.")
}

fn percent(s: &str) -> Result<u32, String> {
    s.strip_suffix('%').unwrap_or(s).parse::<u32>().map_err(|_| format!("not a number: {s}"))
}

/// Check the arguments (without the program name) and build the device command.
pub fn parse(args: &[String]) -> Result<Cmd, String> {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let usage = |u: &str| Err(format!("usage: mixer {u}"));
    match a.as_slice() {
        [] => Ok(Cmd::Show),
        ["-h" | "--help" | "help", ..] => Ok(Cmd::Help),
        ["watch"] => Ok(Cmd::Watch),
        ["set", target, n] => {
            let v = percent(n)?;
            if is_master(target) {
                if v > 100 {
                    return Err("a speaker's volume is 0-100".into());
                }
            } else if v > MAX_APP_PERCENT {
                return Err(format!("an app's volume is 0-{MAX_APP_PERCENT}"));
            }
            Ok(Cmd::Send(format!("set {target} {v}")))
        }
        ["set", ..] => usage("set <app|master[.side]> <percent>"),
        ["mute", target] => Ok(Cmd::Send(format!("mute {target} on"))),
        ["mute", target, how @ ("on" | "off" | "toggle")] => Ok(Cmd::Send(format!("mute {target} {how}"))),
        ["mute", ..] => usage("mute <app|master[.side]> [on|off|toggle]"),
        ["unmute", target] => Ok(Cmd::Send(format!("unmute {target}"))),
        ["unmute", ..] => usage("unmute <app|master[.side]>"),
        ["route", app, side] => {
            if is_master(app) {
                return Err("route moves an app to another speaker; a speaker's own strip can't be routed".into());
            }
            let side = if matches!(*side, "-" | "default") { "-" } else { side };
            Ok(Cmd::Send(format!("route {app} {side}")))
        }
        ["route", ..] => usage("route <app> <side|->"),
        ["gain", input, n] => {
            let v = percent(n)?;
            if v > 100 {
                return Err("gain is 0-100".into());
            }
            Ok(Cmd::Send(format!("gain {input} {v}")))
        }
        ["gain", ..] => usage("gain <microphone> <0-100>"),
        ["reset", app] => Ok(Cmd::Send(format!("reset {app}"))),
        ["reset", ..] => usage("reset <app>"),
        [other, ..] => Err(format!("unknown command '{other}' (set, mute, unmute, route, gain, reset, watch)")),
    }
}

/// A meter of `cells` characters, `percent` (0-100) of them filled: `||||......`.
pub fn meter(percent: u32, cells: usize) -> String {
    let n = ((percent.min(100) as usize * cells) + 50) / 100;
    format!("{}{}", "|".repeat(n), ".".repeat(cells - n))
}

/// A microphone level (dB, -99 = silence) as a meter fill 0-100, on the same -60 dB scale.
pub fn level_percent(db: i32) -> u32 {
    ((db + 60).clamp(0, 60) * 100 / 60) as u32
}

/// What an app's strip says besides its volume: where it plays, ducking, its role.
fn app_notes(a: &ecm_audio::mixer::AppStrip) -> String {
    let mut n = Vec::new();
    match (&a.route, &a.on) {
        (Some(r), _) => n.push(format!("-> {r}")),
        (None, Some(on)) => n.push(format!("on {on}")),
        (None, None) => n.push("idle".into()),
    }
    if a.ducked {
        n.push("ducked".into());
    }
    if a.role != "media" {
        n.push(a.role.clone());
    }
    n.join(", ")
}

/// The strips as lines for a terminal `width` columns wide.
pub fn table(s: &Strips, width: usize) -> Vec<String> {
    let names: Vec<String> = s
        .outputs
        .iter()
        .map(|o| format!("master.{}", o.side))
        .chain(s.apps.iter().map(|a| a.app.clone()))
        .chain(s.inputs.iter().map(|i| i.side.clone()))
        .collect();
    let nw = names.iter().map(|n| n.chars().count()).max().unwrap_or(0).clamp(8, 28);
    // 2 indent + name + 1 + value (8) + 1 + mute (5) + 1 + meter + 2 + notes
    let cells = width.saturating_sub(nw + 20 + 8).clamp(4, 20);
    let line = |name: &str, value: String, muted: bool, fill: u32, notes: String| {
        let name: String = if name.chars().count() > nw { name.chars().take(nw - 1).chain(['~']).collect() } else { name.to_string() };
        let l = format!("  {name:<nw$} {value:>8} {:<5} {}  {notes}", if muted { "muted" } else { "" }, meter(fill, cells));
        l.trim_end().to_string()
    };
    let mut out = vec!["SPEAKERS".to_string()];
    if s.outputs.is_empty() {
        out.push("  no speaker attached".into());
    }
    for o in &s.outputs {
        out.push(line(&format!("master.{}", o.side), format!("{}%", o.volume), o.mute, meter_percent(o.peak), o.label.clone()));
    }
    out.push("APPS".into());
    if s.apps.is_empty() {
        out.push("  no app has played sound yet".into());
    }
    for a in &s.apps {
        out.push(line(&a.app, format!("{}%", a.volume), a.mute, meter_percent(a.peak), app_notes(a)));
    }
    out.push("MICROPHONES".into());
    if s.inputs.is_empty() {
        out.push("  no microphone attached".into());
    }
    for i in &s.inputs {
        let level = if i.level <= -99 { "silent".to_string() } else { format!("{} dB", i.level) };
        out.push(line(&i.side, format!("gain {}", i.gain), i.gain == 0, level_percent(i.level), level));
    }
    out
}

/// The table line of the strip `cmd` changed (to confirm it), if it has one.
pub fn confirm(cmd: &str, s: &Strips, width: usize) -> String {
    let target = cmd.split_whitespace().nth(1).unwrap_or("");
    let key = if target == "master" { s.outputs.first().map(|o| format!("master.{}", o.side)).unwrap_or_default() } else { target.to_string() };
    let found = table(s, width).into_iter().find(|l| l.trim_start().split_whitespace().next() == Some(key.as_str()));
    match found {
        Some(l) => l.trim_start().to_string(),
        None => format!("{target}: saved (it applies when {target} plays sound)"),
    }
}

/// A refused command in plain words. `strips` (read after the refusal) names
/// what is attached when the target or the speaker doesn't exist.
pub fn explain(cmd: &str, e: &std::io::Error, strips: Option<&Strips>) -> String {
    use std::io::ErrorKind;
    match e.kind() {
        ErrorKind::PermissionDenied => "changing the Volume Mixer needs the 'mixer' permission".into(),
        ErrorKind::NotFound => "this computer has no Volume Mixer (/dev/mixer is missing)".into(),
        ErrorKind::InvalidInput => {
            let w: Vec<&str> = cmd.split_whitespace().collect();
            if let Some(s) = strips {
                let speakers = || s.outputs.iter().map(|o| o.side.as_str()).collect::<Vec<_>>().join(" ");
                let no_speaker = |side: &str| {
                    if s.outputs.is_empty() {
                        "no speaker attached".to_string()
                    } else {
                        format!("no speaker '{side}' (speakers: {})", speakers())
                    }
                };
                match w.as_slice() {
                    [_, t, ..] if is_master(t) => {
                        let side = t.strip_prefix("master.").unwrap_or("");
                        if s.outputs.is_empty() || (!side.is_empty() && s.output(side).is_none()) {
                            return no_speaker(side);
                        }
                    }
                    ["route", _, side] if *side != "-" && s.output(side).is_none() => return no_speaker(side),
                    ["gain", input, _] if !s.inputs.iter().any(|i| i.side == *input) => {
                        let mics: Vec<&str> = s.inputs.iter().map(|i| i.side.as_str()).collect();
                        return if mics.is_empty() {
                            "no microphone attached".into()
                        } else {
                            format!("no microphone '{input}' (microphones: {})", mics.join(" "))
                        };
                    }
                    _ => {}
                }
            }
            format!("the Volume Mixer refused '{cmd}' (a bad name or value)")
        }
        _ => format!("{e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecm_audio::mixer::strips;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    const TEXT: &str = "output left volume=100 mute=0 peak=32767 label=Speaker\n\
                        output right volume=40 mute=1 peak=0 label=Speaker\n\
                        app dev.ecm.music volume=80 mute=0 route=- on=left peak=1036 ducked=1 role=media streams=1\n\
                        app discord volume=150 mute=0 route=right on=right peak=0 ducked=0 role=voice streams=1\n\
                        app aplay volume=100 mute=1 route=- on=- peak=0 ducked=0 role=media streams=0\n\
                        input top gain=50 level=-30\n\
                        input back gain=0 level=-99\n";

    #[test]
    fn the_table_has_every_strip_with_meters() {
        let t = table(&strips(TEXT), 80);
        assert_eq!(
            t,
            vec![
                "SPEAKERS",
                "  master.left       100%       ||||||||||||||||||||  Speaker",
                "  master.right       40% muted ....................  Speaker",
                "APPS",
                "  dev.ecm.music      80%       ||||||||||..........  on left, ducked",
                "  discord           150%       ....................  -> right, voice",
                "  aplay             100% muted ....................  idle",
                "MICROPHONES",
                "  top            gain 50       ||||||||||..........  -30 dB",
                "  back            gain 0 muted ....................  silent",
            ]
        );
        // Narrow (a phone's 30 columns): the meters shrink to 4 cells.
        let narrow = table(&strips(TEXT), 30);
        assert!(narrow[1].contains(" |||| "), "{}", narrow[1]);
        // Nothing attached.
        assert_eq!(table(&Strips::default(), 80), vec!["SPEAKERS", "  no speaker attached", "APPS", "  no app has played sound yet", "MICROPHONES", "  no microphone attached"]);
    }

    #[test]
    fn meters_round_to_cells() {
        assert_eq!(meter(0, 10), "..........");
        assert_eq!(meter(100, 10), "||||||||||");
        assert_eq!(meter(55, 10), "||||||....");
        assert_eq!(meter(250, 4), "||||");
        assert_eq!((level_percent(-99), level_percent(0), level_percent(-30)), (0, 100, 50));
    }

    #[test]
    fn arguments_become_device_commands() {
        let send = |s: &str| parse(&args(s));
        assert_eq!(send(""), Ok(Cmd::Show));
        assert_eq!(send("watch"), Ok(Cmd::Watch));
        assert_eq!(send("-h"), Ok(Cmd::Help));
        assert_eq!(send("set dev.ecm.music 80"), Ok(Cmd::Send("set dev.ecm.music 80".into())));
        assert_eq!(send("set dev.ecm.music 150%"), Ok(Cmd::Send("set dev.ecm.music 150".into())));
        assert_eq!(send("set master.left 70"), Ok(Cmd::Send("set master.left 70".into())));
        assert_eq!(send("set dev.ecm.music 151"), Err("an app's volume is 0-150".into()));
        assert_eq!(send("set master 120"), Err("a speaker's volume is 0-100".into()));
        assert_eq!(send("set aplay loud"), Err("not a number: loud".into()));
        assert_eq!(send("mute aplay"), Ok(Cmd::Send("mute aplay on".into())));
        assert_eq!(send("mute master toggle"), Ok(Cmd::Send("mute master toggle".into())));
        assert_eq!(send("unmute aplay"), Ok(Cmd::Send("unmute aplay".into())));
        assert_eq!(send("route discord right"), Ok(Cmd::Send("route discord right".into())));
        assert_eq!(send("route discord default"), Ok(Cmd::Send("route discord -".into())));
        assert_eq!(send("gain top 60"), Ok(Cmd::Send("gain top 60".into())));
        assert_eq!(send("gain top 101"), Err("gain is 0-100".into()));
        assert_eq!(send("reset aplay"), Ok(Cmd::Send("reset aplay".into())));
        assert!(send("route master right").is_err());
        assert_eq!(send("set aplay"), Err("usage: mixer set <app|master[.side]> <percent>".into()));
        assert!(send("louder").unwrap_err().starts_with("unknown command 'louder'"));
    }

    #[test]
    fn refusals_are_explained() {
        use std::io::{Error, ErrorKind};
        let s = strips(TEXT);
        let inval = Error::from(ErrorKind::InvalidInput);
        assert_eq!(explain("set master.top 50", &inval, Some(&s)), "no speaker 'top' (speakers: left right)");
        assert_eq!(explain("route discord back", &inval, Some(&s)), "no speaker 'back' (speakers: left right)");
        assert_eq!(explain("gain left 50", &inval, Some(&s)), "no microphone 'left' (microphones: top back)");
        assert_eq!(explain("set master 50", &inval, Some(&Strips::default())), "no speaker attached");
        assert_eq!(explain("set a b", &inval, Some(&s)), "the Volume Mixer refused 'set a b' (a bad name or value)");
        assert_eq!(explain("set aplay 5", &Error::from(ErrorKind::PermissionDenied), None), "changing the Volume Mixer needs the 'mixer' permission");
        assert!(explain("set aplay 5", &Error::from(ErrorKind::NotFound), None).contains("no Volume Mixer"));
    }

    #[test]
    fn a_change_is_confirmed_with_its_line() {
        let s = strips(TEXT);
        assert_eq!(confirm("set dev.ecm.music 80", &s, 80), "dev.ecm.music      80%       ||||||||||..........  on left, ducked");
        assert!(confirm("mute master on", &s, 80).starts_with("master.left"));
        assert_eq!(confirm("set dev.new.app 90", &s, 80), "dev.new.app: saved (it applies when dev.new.app plays sound)");
    }
}
