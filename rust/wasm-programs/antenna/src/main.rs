//! `antenna` — read the antenna on a feed point next to the computer.
//!
//!   antenna                       summary: resonance, 2:1 SWR band, ratings
//!   antenna swr <f1> <f2> [n]     SWR plot across f1..f2
//!   antenna z <f>                 impedance, SWR and efficiency at f
//!   antenna polar <f> [az|el]     polar gain plot
//!   antenna limits [--amp W]      power limit, weakest link, transmitter verdict
//!
//! Plots print as text; `--gfx` also draws them on the terminal's display
//! and `--screen` on the attached Screen, for `--seconds` (default 15).
//! Reads come from the server's antenna cache through the `antenna`
//! peripheral; while the solver runs (`pending`) the program waits up to
//! `--wait` seconds, then uses the estimate.

use antenna::ascii::{polar_plot, swr_plot};
use antenna::cli::{parse, Args, Cmd, Output, Plane, USAGE};
use antenna::raster::{draw_polar, draw_swr, Canvas};
use antenna::{fmt_hz, fmt_swr, fmt_watts, min_index};
use ecm_host_abi::gfx_child::{self, FORMAT_INDEXED8};
use ecm_host_abi::peripheral::{self, Error, Value};
use ecm_host_abi::video::Target;
use std::time::{Duration, Instant};

fn describe(e: Error) -> String {
    match e {
        Error::Peripheral(msg) => msg,
        Error::Unavailable => "peripherals are not available".to_string(),
        Error::Malformed(what) => format!("bad reply from the computer: {what}"),
    }
}

fn get<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Map(entries) => entries.iter().find(|(k, _)| matches!(k, Value::Str(s) if s == key)).map(|(_, v)| v),
        _ => None,
    }
}

fn num(v: &Value, key: &str) -> f64 {
    match get(v, key) {
        Some(Value::Float(f)) => *f,
        Some(Value::Int(i)) => *i as f64,
        _ => f64::NAN,
    }
}

fn text(v: &Value, key: &str) -> String {
    match get(v, key) {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    }
}

fn flag(v: &Value, key: &str) -> bool {
    matches!(get(v, key), Some(Value::Bool(true)))
}

fn as_f64(v: &Value) -> f64 {
    match v {
        Value::Float(f) => *f,
        Value::Int(i) => *i as f64,
        _ => f64::NAN,
    }
}

struct Ant {
    name: String,
}

impl Ant {
    fn call(&self, method: &str, args: &[Value]) -> Result<Value, String> {
        peripheral::call(&self.name, method, args).map_err(describe)
    }
}

fn find(name: &Option<String>) -> Result<Ant, String> {
    let list = peripheral::list().map_err(describe)?;
    let found = match name {
        Some(n) => list.iter().find(|(pn, _)| pn == n).map(|(pn, ty)| (pn.clone(), ty.clone())),
        None => list.iter().find(|(_, ty)| ty == "antenna").map(|(pn, ty)| (pn.clone(), ty.clone())),
    };
    match found {
        Some((pn, ty)) if ty == "antenna" => Ok(Ant { name: pn }),
        Some((pn, ty)) => Err(format!("{pn} is a {ty}, not an antenna")),
        None => Err("no antenna attached (place the computer next to a feed point)".to_string()),
    }
}

/// The antenna's status, waiting up to `wait_s` for a pending solve.
fn status(ant: &Ant, wait_s: f64) -> Result<Value, String> {
    let start = Instant::now();
    let mut said = false;
    loop {
        let st = ant.call("status", &[])?;
        if !flag(&st, "pending") || start.elapsed().as_secs_f64() >= wait_s {
            if flag(&st, "pending") {
                println!("antenna: the solver is still running; showing the estimate");
            }
            return Ok(st);
        }
        if !said {
            println!("antenna: solving...");
            said = true;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let a = match parse(&raw) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("antenna: {e}");
            eprintln!("usage: antenna {USAGE}");
            std::process::exit(2);
        }
    };
    if a.cmd == Cmd::Help {
        println!("usage: antenna {USAGE}");
        return;
    }
    if let Err(e) = run(&a) {
        eprintln!("antenna: {e}");
        std::process::exit(1);
    }
}

fn run(a: &Args) -> Result<(), String> {
    let ant = find(&a.name)?;
    let st = status(&ant, a.wait_s)?;
    if !flag(&st, "present") {
        // "No antenna: ..." or "Antenna can't be analysed: ..."
        println!("{}", text(&st, "summary"));
        if a.cmd == Cmd::Summary {
            return Ok(());
        }
        return Err("nothing to measure".to_string());
    }
    match &a.cmd {
        Cmd::Summary | Cmd::Help => summary(&st),
        Cmd::Swr { f0, f1, points } => swr(&ant, &st, a, *f0, *f1, *points),
        Cmd::Z { f } => z(&ant, *f),
        Cmd::Polar { f, plane, step } => polar(&ant, a, *f, *plane, *step),
        Cmd::Limits { amp_w } => limits(&ant, *amp_w),
    }
}

fn summary(st: &Value) -> Result<(), String> {
    println!("{}", text(st, "summary"));
    let details = text(st, "details");
    if !details.is_empty() {
        println!("{details}");
    }
    if let Some(feed) = get(st, "feed") {
        println!(
            "feed point at ({}, {}, {}) - {}",
            num(feed, "x"),
            num(feed, "y"),
            num(feed, "z"),
            if flag(st, "solved") { "solved" } else { "estimate" }
        );
    }
    Ok(())
}

fn swr(ant: &Ant, st: &Value, a: &Args, f0: f64, f1: f64, points: usize) -> Result<(), String> {
    let list = ant.call("sweep", &[Value::Float(f0), Value::Float(f1), Value::Int(points as i64)])?;
    let Value::List(items) = list else { return Err("bad sweep reply".to_string()) };
    let hz: Vec<f64> = items.iter().map(|p| num(p, "f")).collect();
    let swr: Vec<f64> = items.iter().map(|p| num(p, "swr")).collect();
    println!("SWR {} .. {} ({} points, 50 ohms)", fmt_hz(f0), fmt_hz(f1), hz.len());
    for line in swr_plot(&hz, &swr, 61, 12) {
        println!("{line}");
    }
    let res = num(st, "resonant_hz");
    let res_txt = if res.is_finite() { format!("; resonant at {}", fmt_hz(res)) } else { String::new() };
    let (lo, hi) = (num(st, "band_low_hz"), num(st, "band_high_hz"));
    let band = if lo.is_finite() && hi.is_finite() { format!("; 2:1 band {} - {}", fmt_hz(lo), fmt_hz(hi)) } else { String::new() };
    match min_index(&swr) {
        Some(i) => println!("antenna: min SWR {} at {}{res_txt}{band}", fmt_swr(swr[i]), fmt_hz(hz[i])),
        None => println!("antenna: no usable SWR in this range (outside the analysed bands){res_txt}"),
    }
    if a.output != Output::Text {
        show(a, |c| draw_swr(c, &hz, &swr))?;
    }
    Ok(())
}

fn z(ant: &Ant, f: f64) -> Result<(), String> {
    let v = ant.call("impedance", &[Value::Float(f)])?;
    if !flag(&v, "in_band") {
        println!("{}: outside the analysed bands (no reading)", fmt_hz(f));
        return Ok(());
    }
    let (r, x) = (num(&v, "r"), num(&v, "x"));
    println!(
        "{}: Z = {:.1} {} j{:.1} ohms, SWR {}, efficiency {:.0}%",
        fmt_hz(f),
        r,
        if x < 0.0 { "-" } else { "+" },
        x.abs(),
        fmt_swr(num(&v, "swr")),
        num(&v, "efficiency") * 100.0
    );
    Ok(())
}

fn polar(ant: &Ant, a: &Args, f: f64, plane: Plane, step: f64) -> Result<(), String> {
    let pk = ant.call("peak", &[Value::Float(f)])?;
    let (paz, pel) = (num(&pk, "az"), num(&pk, "el"));
    let angle = match plane {
        Plane::Azimuth => pel,
        Plane::Elevation => paz,
    };
    let cut = ant.call(
        "pattern",
        &[Value::Float(f), Value::Str(plane.name().to_string()), Value::Float(step), Value::Float(angle)],
    )?;
    let Value::List(items) = cut else { return Err("bad pattern reply".to_string()) };
    let gains: Vec<f64> = items.iter().map(as_f64).collect();
    match plane {
        Plane::Azimuth => println!("{} azimuth pattern at {:.0} deg elevation (N up, 30 dB scale)", fmt_hz(f), angle),
        Plane::Elevation => println!("{} elevation pattern through bearing {:.0} deg (> towards it, U up, 30 dB scale)", fmt_hz(f), angle),
    }
    for line in polar_plot(&gains, plane, 9) {
        println!("{line}");
    }
    let pol = ant.call("polarization", &[Value::Float(f)])?;
    println!(
        "antenna: peak {:.1} dBi at bearing {:.0} deg, elevation {:.0} deg; {} polarization (tilt {:.0} deg)",
        num(&pk, "gain_dbi"),
        paz,
        pel,
        text(&pol, "sense"),
        num(&pol, "tilt_deg")
    );
    if a.output != Output::Text {
        show(a, |c| draw_polar(c, &gains, plane))?;
    }
    Ok(())
}

fn limits(ant: &Ant, amp_w: Option<f64>) -> Result<(), String> {
    let args = match amp_w {
        Some(w) => vec![Value::Float(w)],
        None => vec![],
    };
    let v = ant.call("power_limit", &args)?;
    println!("{}", text(&v, "text"));
    if let Some(w) = get(&v, "weakest") {
        println!("weakest link: {} ({})", text(w, "part"), text(w, "block"));
    }
    println!(
        "wire limit {}, insulation limit {}",
        fmt_watts(num(&v, "wire_watts")),
        if num(&v, "voltage_watts").is_nan() { "not computed (estimate)".to_string() } else { fmt_watts(num(&v, "voltage_watts")) }
    );
    if let Some(tx) = get(&v, "transmitter") {
        let verdict = text(&v, "verdict");
        println!(
            "transmitter: {} {} -> {}",
            text(tx, "name"),
            fmt_watts(num(tx, "watts")),
            if verdict == "ok" { "within rating" } else { verdict.as_str() }
        );
    } else {
        println!("no transmitter found on the feedline (antenna limits --amp W to check one)");
    }
    Ok(())
}

/// Draw on the terminal display (`--gfx`) or the Screen (`--screen`) and
/// hold it for `--seconds`.
fn show(a: &Args, draw: impl Fn(&mut Canvas)) -> Result<(), String> {
    let target = if a.output == Output::Screen { Target::Screen } else { Target::Terminal };
    let (w, h) = if a.output == Output::Screen {
        gfx_child::set_screen_power(true);
        let (w, h) = gfx_child::screen_dims().ok_or("no Screen attached to this computer")?;
        (w as usize, h as usize)
    } else {
        (256, 160)
    };
    gfx_child::init2(target, w as i32, h as i32, FORMAT_INDEXED8, 0).map_err(|e| format!("can't open the display: {e:?}"))?;
    let mut c = Canvas::new(w, h);
    draw(&mut c);
    let r = gfx_child::blit_rect(target, 0, 0, w as i32, h as i32, &c.px, FORMAT_INDEXED8);
    if r.is_ok() {
        println!("antenna: plot on the {} for {:.0} s [Ctrl+T stops]", if a.output == Output::Screen { "Screen" } else { "display" }, a.seconds);
        std::thread::sleep(Duration::from_secs_f64(a.seconds.max(0.0)));
    }
    let _ = gfx_child::set_mode(target, 0);
    r.map_err(|_| "display lost".to_string())
}
