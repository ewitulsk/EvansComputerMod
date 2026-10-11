//! The `antenna` program's pure parts: argument parsing, the text plots
//! (SWR against frequency, polar gain patterns) and the raster plots drawn
//! on a display. `main.rs` talks to the `antenna` peripheral (a feed point
//! next to the computer) and prints / draws what these produce.

pub mod ascii;
pub mod cli;
pub mod raster;

/// `7100000.0` -> `7.100 MHz` (kHz below 1 MHz, GHz from 1 GHz).
pub fn fmt_hz(hz: f64) -> String {
    if !hz.is_finite() {
        return "?".to_string();
    }
    let a = hz.abs();
    if a >= 1e9 {
        format!("{:.3} GHz", hz / 1e9)
    } else if a >= 1e6 {
        format!("{:.3} MHz", hz / 1e6)
    } else {
        format!("{:.1} kHz", hz / 1e3)
    }
}

/// `1.234` -> `1.23:1`; unusable readings (∞, NaN, > 99) -> `>99:1`.
pub fn fmt_swr(swr: f64) -> String {
    if !swr.is_finite() || swr > 99.0 {
        ">99:1".to_string()
    } else {
        format!("{:.2}:1", swr)
    }
}

/// `640.0` -> `640 W`, `2000.0` -> `2.0 kW`.
pub fn fmt_watts(w: f64) -> String {
    if !w.is_finite() {
        "unlimited".to_string()
    } else if w >= 1e6 {
        format!("{:.1} MW", w / 1e6)
    } else if w >= 1e4 {
        format!("{:.0} kW", w / 1e3)
    } else if w >= 1e3 {
        format!("{:.1} kW", w / 1e3)
    } else if w >= 10.0 {
        format!("{:.0} W", w)
    } else {
        format!("{:.1} W", w)
    }
}

/// Index of the lowest finite SWR, if any.
pub fn min_index(swr: &[f64]) -> Option<usize> {
    let mut best: Option<usize> = None;
    for (i, s) in swr.iter().enumerate() {
        if !s.is_finite() {
            continue;
        }
        if best.map_or(true, |b| *s < swr[b]) {
            best = Some(i);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(fmt_hz(7.1e6), "7.100 MHz");
        assert_eq!(fmt_hz(146.52e6), "146.520 MHz");
        assert_eq!(fmt_hz(600e3), "600.0 kHz");
        assert_eq!(fmt_hz(2.4e9), "2.400 GHz");
        assert_eq!(fmt_swr(1.234), "1.23:1");
        assert_eq!(fmt_swr(f64::INFINITY), ">99:1");
        assert_eq!(fmt_swr(f64::NAN), ">99:1");
        assert_eq!(fmt_watts(640.0), "640 W");
        assert_eq!(fmt_watts(2000.0), "2.0 kW");
        assert_eq!(fmt_watts(5.0), "5.0 W");
    }

    #[test]
    fn min_skips_unusable() {
        assert_eq!(min_index(&[f64::INFINITY, 3.0, 1.5, 2.0, f64::NAN]), Some(2));
        assert_eq!(min_index(&[f64::INFINITY, f64::NAN]), None);
        assert_eq!(min_index(&[]), None);
    }
}
