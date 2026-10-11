//! Frequency and rate parsing / formatting.

/// Parse `146.52e6`, `146520000`, `144.39M`, `144.39MHz`, `600k`, `7.1 MHz`,
/// `2.4G`. Suffixes are case-insensitive (`m` means mega here, not milli).
pub fn parse_freq(s: &str) -> Option<f64> {
    let t: String = s.trim().chars().filter(|c| !c.is_whitespace() && *c != '_').collect();
    let lower = t.to_ascii_lowercase();
    let body = lower.strip_suffix("hz").unwrap_or(&lower);
    let (num, mult) = match body.chars().last()? {
        'k' => (&body[..body.len() - 1], 1e3),
        'm' => (&body[..body.len() - 1], 1e6),
        'g' => (&body[..body.len() - 1], 1e9),
        _ => (body, 1.0),
    };
    let v: f64 = num.parse().ok()?;
    let v = v * mult;
    (v.is_finite() && v >= 0.0).then_some(v)
}

/// Parse a sample rate (`48000`, `48k`, `250e3`), rounded to whole samples/s.
pub fn parse_rate(s: &str) -> Option<u32> {
    let v = parse_freq(s)?;
    (v >= 1.0 && v <= 100e6).then(|| v.round() as u32)
}

/// `146520000.0` -> `146.520 MHz`.
pub fn fmt_freq(hz: f64) -> String {
    let a = hz.abs();
    if a >= 1e9 {
        format!("{:.4} GHz", hz / 1e9)
    } else if a >= 1e6 {
        format!("{:.4} MHz", hz / 1e6)
    } else if a >= 1e3 {
        format!("{:.3} kHz", hz / 1e3)
    } else {
        format!("{:.0} Hz", hz)
    }
}

/// Greatest common divisor.
pub fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// Interpolation / decimation factors `(l, m)` for `from -> to`, reduced, and
/// approximated (continued fractions) so neither exceeds `max`.
pub fn ratio(from: f64, to: f64, max: u64) -> (usize, usize) {
    let (fi, ti) = (from.round() as u64, to.round() as u64);
    if fi > 0 && ti > 0 && (from - fi as f64).abs() < 1e-6 && (to - ti as f64).abs() < 1e-6 {
        let g = gcd(fi, ti);
        let (l, m) = (ti / g, fi / g);
        if l <= max && m <= max {
            return (l as usize, m as usize);
        }
    }
    // Best rational approximation of to/from with both terms <= max.
    let x = to / from;
    let (mut h0, mut h1, mut k0, mut k1) = (0u64, 1u64, 1u64, 0u64);
    let mut v = x;
    let mut best = (1usize, 1usize);
    for _ in 0..40 {
        let a = v.floor() as u64;
        let h2 = a.saturating_mul(h1).saturating_add(h0);
        let k2 = a.saturating_mul(k1).saturating_add(k0);
        if h2 > max || k2 > max || h2 == 0 && k2 == 0 {
            break;
        }
        if h2 > 0 && k2 > 0 {
            best = (h2 as usize, k2 as usize);
        }
        h0 = h1;
        h1 = h2;
        k0 = k1;
        k1 = k2;
        let frac = v - a as f64;
        if frac < 1e-12 {
            break;
        }
        v = 1.0 / frac;
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frequencies_parse_with_suffixes() {
        assert_eq!(parse_freq("146.52e6"), Some(146.52e6));
        assert_eq!(parse_freq("144.39M"), Some(144.39e6));
        assert_eq!(parse_freq("144.39MHz"), Some(144.39e6));
        assert_eq!(parse_freq("600k"), Some(600e3));
        assert_eq!(parse_freq("7.1 MHz"), Some(7.1e6));
        assert_eq!(parse_freq("2.4G"), Some(2.4e9));
        assert_eq!(parse_freq("1_000"), Some(1000.0));
        assert_eq!(parse_freq("abc"), None);
        assert_eq!(parse_freq("-5"), None);
        assert_eq!(parse_rate("48k"), Some(48_000));
        assert_eq!(fmt_freq(146.52e6), "146.5200 MHz");
    }

    #[test]
    fn ratios_reduce_and_approximate() {
        assert_eq!(ratio(250_000.0, 48_000.0, 512), (24, 125));
        assert_eq!(ratio(48_000.0, 8_000.0, 512), (1, 6));
        assert_eq!(ratio(44_100.0, 48_000.0, 512), (160, 147));
        let (l, m) = ratio(1_000_000.0, 44_100.0, 64);
        assert!(l <= 64 && m <= 64);
        assert!(((l as f64 / m as f64) - 0.0441).abs() < 0.001, "{l}/{m}");
    }
}
