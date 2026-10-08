//! Command line of `antenna`.

use ecm_radio::units::parse_freq;

pub const USAGE: &str = "[summary]                     resonance, SWR band, ratings
       antenna swr <f1> <f2> [points]       SWR plot across f1..f2 (default 41 points)
       antenna z <f>                        impedance, SWR and efficiency at f
       antenna polar <f> [az|el] [--step D] polar gain plot (az: horizontal cut, el: vertical)
       antenna limits [--amp W]             power limit, weakest link, transmitter verdict
  options: --name P (peripheral; default the first antenna), --wait S (for the solver, default 20)
           --gfx (plot on the terminal display) | --screen (plot on the attached Screen), --seconds S
  frequencies: 7.1M, 7100k, 7.1e6, 146.52MHz";

/// Where a plot goes besides the text printed on the terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    Text,
    Terminal,
    Screen,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plane {
    Azimuth,
    Elevation,
}

impl Plane {
    pub fn name(self) -> &'static str {
        match self {
            Plane::Azimuth => "azimuth",
            Plane::Elevation => "elevation",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    Summary,
    Swr { f0: f64, f1: f64, points: usize },
    Z { f: f64 },
    Polar { f: f64, plane: Plane, step: f64 },
    Limits { amp_w: Option<f64> },
    Help,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Args {
    pub cmd: Cmd,
    /// Peripheral name (None: the first `antenna` peripheral).
    pub name: Option<String>,
    /// Seconds to wait for a pending solve.
    pub wait_s: f64,
    pub output: Output,
    /// Seconds to hold a graphics plot on the display.
    pub seconds: f64,
}

pub const DEFAULT_POINTS: usize = 41;
pub const MAX_POINTS: usize = 401;

fn freq(s: &str) -> Result<f64, String> {
    match parse_freq(s) {
        Some(f) if f > 0.0 => Ok(f),
        _ => Err(format!("bad frequency {s:?}")),
    }
}

fn number(opt: &str, s: Option<&String>) -> Result<f64, String> {
    let s = s.ok_or_else(|| format!("{opt} needs a value"))?;
    parse_freq(s).ok_or_else(|| format!("bad value for {opt}: {s:?}"))
}

pub fn parse(args: &[String]) -> Result<Args, String> {
    let mut pos: Vec<&str> = Vec::new();
    let mut name = None;
    let mut wait_s = 20.0;
    let mut output = Output::Text;
    let mut seconds = 15.0;
    let mut step = 5.0;
    let mut amp_w = None;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-h" | "--help" | "help" if pos.is_empty() => return Ok(Args { cmd: Cmd::Help, name, wait_s, output, seconds }),
            "--name" => {
                name = Some(args.get(i + 1).ok_or("--name needs a peripheral name")?.clone());
                i += 1;
            }
            "--wait" => {
                wait_s = number(a, args.get(i + 1))?;
                i += 1;
            }
            "--seconds" => {
                seconds = number(a, args.get(i + 1))?;
                i += 1;
            }
            "--step" => {
                step = number(a, args.get(i + 1))?;
                if !(1.0..=90.0).contains(&step) {
                    return Err("--step must be 1..90 degrees".into());
                }
                i += 1;
            }
            "--amp" => {
                let w = number(a, args.get(i + 1))?;
                if !(w > 0.0) {
                    return Err("--amp must be a positive number of watts".into());
                }
                amp_w = Some(w);
                i += 1;
            }
            "--gfx" => output = Output::Terminal,
            "--screen" => output = Output::Screen,
            "--text" => output = Output::Text,
            _ if a.starts_with("--") => return Err(format!("unknown option {a}")),
            _ => pos.push(a),
        }
        i += 1;
    }
    let cmd = match pos.as_slice() {
        [] | ["summary"] => Cmd::Summary,
        ["swr", f0, f1, rest @ ..] => {
            let (f0, f1) = (freq(f0)?, freq(f1)?);
            if f1 <= f0 {
                return Err("the stop frequency must be above the start".into());
            }
            let points = match rest {
                [] => DEFAULT_POINTS,
                [p] => p.parse::<usize>().map_err(|_| format!("bad point count {p:?}"))?,
                _ => return Err("too many arguments".into()),
            };
            if !(2..=MAX_POINTS).contains(&points) {
                return Err(format!("points must be 2..{MAX_POINTS}"));
            }
            Cmd::Swr { f0, f1, points }
        }
        ["swr", ..] => return Err("swr needs a start and a stop frequency".into()),
        ["z", f] => Cmd::Z { f: freq(f)? },
        ["z", ..] => return Err("z needs one frequency".into()),
        ["polar", f, rest @ ..] => {
            let plane = match rest {
                [] | ["az"] | ["azimuth"] => Plane::Azimuth,
                ["el"] | ["elevation"] => Plane::Elevation,
                [p] => return Err(format!("plane must be az or el, not {p:?}")),
                _ => return Err("too many arguments".into()),
            };
            Cmd::Polar { f: freq(f)?, plane, step }
        }
        ["polar"] => return Err("polar needs a frequency".into()),
        ["limits"] => Cmd::Limits { amp_w },
        [other, ..] => return Err(format!("unknown command {other:?}")),
    };
    Ok(Args { cmd, name, wait_s, output, seconds })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Result<Args, String> {
        parse(&s.split_whitespace().map(String::from).collect::<Vec<_>>())
    }

    #[test]
    fn summary_is_the_default() {
        let a = p("").unwrap();
        assert_eq!(a.cmd, Cmd::Summary);
        assert_eq!(a.output, Output::Text);
        assert_eq!(a.wait_s, 20.0);
        assert_eq!(p("summary --name left").unwrap().name.as_deref(), Some("left"));
    }

    #[test]
    fn swr_parses_frequencies_and_points() {
        assert_eq!(p("swr 6e6 8e6 21").unwrap().cmd, Cmd::Swr { f0: 6e6, f1: 8e6, points: 21 });
        assert_eq!(p("swr 7M 7.3MHz").unwrap().cmd, Cmd::Swr { f0: 7e6, f1: 7.3e6, points: DEFAULT_POINTS });
        let g = p("swr 6M 8M --screen --seconds 5").unwrap();
        assert_eq!(g.output, Output::Screen);
        assert_eq!(g.seconds, 5.0);
        assert_eq!(p("swr 6M 8M --gfx").unwrap().output, Output::Terminal);
    }

    #[test]
    fn swr_rejects_bad_input() {
        assert!(p("swr 8M 6M").is_err());
        assert!(p("swr 6M").is_err());
        assert!(p("swr 6M 8M 1").is_err());
        assert!(p("swr 6M 8M 1000").is_err());
        assert!(p("swr x 8M").is_err());
        assert!(p("swr 6M 8M 21 9").is_err());
    }

    #[test]
    fn z_polar_limits() {
        assert_eq!(p("z 7.1M").unwrap().cmd, Cmd::Z { f: 7.1e6 });
        assert!(p("z").is_err());
        assert_eq!(p("polar 7.1M").unwrap().cmd, Cmd::Polar { f: 7.1e6, plane: Plane::Azimuth, step: 5.0 });
        assert_eq!(p("polar 7.1M el --step 10").unwrap().cmd, Cmd::Polar { f: 7.1e6, plane: Plane::Elevation, step: 10.0 });
        assert!(p("polar 7.1M up").is_err());
        assert!(p("polar 7.1M az --step 0").is_err());
        assert_eq!(p("limits").unwrap().cmd, Cmd::Limits { amp_w: None });
        assert_eq!(p("limits --amp 1k").unwrap().cmd, Cmd::Limits { amp_w: Some(1000.0) });
        assert!(p("limits --amp").is_err());
    }

    #[test]
    fn help_and_unknowns() {
        assert_eq!(p("--help").unwrap().cmd, Cmd::Help);
        assert_eq!(p("help").unwrap().cmd, Cmd::Help);
        assert!(p("frobnicate").is_err());
        assert!(p("swr 6M 8M --bogus").is_err());
    }
}
