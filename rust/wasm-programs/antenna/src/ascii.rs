//! Text plots for the terminal.

use crate::cli::Plane;

/// Top of the SWR axis; worse readings pin to the top row.
pub const SWR_TOP: f64 = 10.0;
/// Dynamic range of a polar plot, dB below the peak at the centre.
pub const POLAR_RANGE_DB: f64 = 30.0;

/// Height fraction of an SWR reading on the log axis: 0 = 1:1, 1 = 10:1 or worse.
pub fn y_frac(swr: f64) -> f64 {
    if swr.is_nan() {
        return 1.0;
    }
    if swr <= 1.0 {
        return 0.0;
    }
    if !swr.is_finite() || swr >= SWR_TOP {
        return 1.0;
    }
    swr.ln() / SWR_TOP.ln()
}

/// Row (0 = top) of an SWR reading in a plot `height` rows tall.
pub fn swr_row(swr: f64, height: usize) -> usize {
    ((1.0 - y_frac(swr)) * (height - 1) as f64).round() as usize
}

/// SWR at `f`, linearly interpolated between sweep points; ∞ outside the
/// sweep or next to an unusable point.
pub fn swr_at(hz: &[f64], swr: &[f64], f: f64) -> f64 {
    if hz.is_empty() || f < hz[0] || f > hz[hz.len() - 1] {
        return f64::INFINITY;
    }
    if hz.len() == 1 {
        return swr[0];
    }
    let mut i = 1;
    while i < hz.len() - 1 && hz[i] < f {
        i += 1;
    }
    let (a, b) = (swr[i - 1], swr[i]);
    if !a.is_finite() || !b.is_finite() {
        return f64::INFINITY;
    }
    let t = if hz[i] > hz[i - 1] { (f - hz[i - 1]) / (hz[i] - hz[i - 1]) } else { 0.0 };
    a + t * (b - a)
}

fn mhz(f: f64) -> String {
    if f >= 1e6 {
        format!("{:.3}", f / 1e6)
    } else {
        format!("{:.1}k", f / 1e3)
    }
}

/// SWR against frequency: `height` rows by `width` columns, a label gutter
/// on the left (the SWR at each row), `-` along the 2:1 row, `*` for the
/// curve, `@` at the sweep's best point and `^` where it is off the top.
/// Columns outside the analysed bands stay blank. Then the frequency axis.
pub fn swr_plot(hz: &[f64], swr: &[f64], width: usize, height: usize) -> Vec<String> {
    let width = width.max(8);
    let height = height.max(4);
    let mut out = Vec::with_capacity(height + 2);
    if hz.len() < 2 {
        out.push("(no sweep)".to_string());
        return out;
    }
    let (f0, f1) = (hz[0], hz[hz.len() - 1]);
    let col_of = |f: f64| (((f - f0) / (f1 - f0)) * (width - 1) as f64).round() as usize;
    let best = crate::min_index(swr).map(|i| (col_of(hz[i]), swr_row(swr[i], height)));
    let row2 = swr_row(2.0, height);
    let mut grid = vec![vec![' '; width]; height];
    for c in 0..width {
        grid[row2][c] = '-';
    }
    for c in 0..width {
        let f = f0 + (f1 - f0) * c as f64 / (width - 1) as f64;
        let s = swr_at(hz, swr, f);
        if s.is_nan() || s == f64::INFINITY {
            continue;
        }
        if s >= SWR_TOP {
            grid[0][c] = '^';
        } else {
            grid[swr_row(s, height)][c] = '*';
        }
    }
    if let Some((c, r)) = best {
        grid[r][c.min(width - 1)] = '@';
    }
    for (r, row) in grid.iter().enumerate() {
        let v = SWR_TOP.powf(1.0 - r as f64 / (height - 1) as f64);
        let label = if r == 0 { " 10+".to_string() } else { format!("{:4.1}", v) };
        out.push(format!("{label} |{}", row.iter().collect::<String>()));
    }
    out.push(format!("     +{}", "-".repeat(width)));
    // Frequency labels: start, middle, end (MHz).
    let mut axis = vec![' '; width + 6];
    let mut put = |text: &str, centre: usize| {
        let start = centre.saturating_sub(text.len() / 2).min(axis.len().saturating_sub(text.len()));
        for (k, ch) in text.chars().enumerate() {
            if start + k < axis.len() {
                axis[start + k] = ch;
            }
        }
    };
    let l0 = mhz(f0);
    let l1 = mhz(f1);
    put(&l0, 6 + l0.len() / 2);
    put(&mhz((f0 + f1) / 2.0), 6 + width / 2);
    put(&l1, 6 + width - 1 - (l1.len() - 1) / 2);
    let unit = if f1 >= 1e6 { " MHz" } else { "" };
    out.push(format!("{}{unit}", axis.iter().collect::<String>().trim_end()));
    out
}

/// Gain at `angle` degrees from a cut sampled every 360/n degrees, linearly
/// interpolated round the circle.
pub fn gain_at(gains: &[f64], angle: f64) -> f64 {
    let n = gains.len();
    if n == 0 {
        return f64::NEG_INFINITY;
    }
    let pos = angle.rem_euclid(360.0) / (360.0 / n as f64);
    let i = pos.floor() as usize % n;
    let j = (i + 1) % n;
    let t = pos - pos.floor();
    gains[i] + t * (gains[j] - gains[i])
}

/// Radius fraction for a gain: 1 at the peak, 0 at `POLAR_RANGE_DB` below.
pub fn polar_rho(gain: f64, peak: f64) -> f64 {
    ((gain - (peak - POLAR_RANGE_DB)) / POLAR_RANGE_DB).clamp(0.0, 1.0)
}

/// Unit (x right, y up) screen direction of a cut angle.
/// Azimuth: compass bearing, north up, east right.
/// Elevation: 0 = horizon to the right (towards the cut's azimuth), 90 = up.
pub fn polar_xy(plane: Plane, angle_deg: f64) -> (f64, f64) {
    let a = angle_deg.to_radians();
    match plane {
        Plane::Azimuth => (a.sin(), a.cos()),
        Plane::Elevation => (a.cos(), a.sin()),
    }
}

/// Peak (finite) gain of a cut.
pub fn peak(gains: &[f64]) -> f64 {
    gains.iter().copied().filter(|g| g.is_finite()).fold(f64::NEG_INFINITY, f64::max)
}

/// A polar plot of a gain cut, `2r+1` rows by `4r+1` columns (terminal
/// cells are about twice as tall as wide): `.` is the peak ring, `*` the
/// pattern (radius linear in dB over 30 dB), `+` the centre. Azimuth plots
/// are labelled N/E/S/W; elevation plots U (zenith), D, `>` (towards the
/// cut's azimuth) and `<`.
pub fn polar_plot(gains: &[f64], plane: Plane, r: usize) -> Vec<String> {
    let r = r.max(3);
    let (rows, cols) = (2 * r + 1, 4 * r + 1);
    let (cx, cy) = (2 * r, r);
    let mut g = vec![vec![' '; cols]; rows];
    for c in 0..cols {
        g[cy][c] = '-';
    }
    for row in g.iter_mut() {
        row[cx] = '|';
    }
    let place = |g: &mut Vec<Vec<char>>, x: f64, y: f64, ch: char| {
        let col = (cx as f64 + 2.0 * r as f64 * x).round();
        let row = (cy as f64 - r as f64 * y).round();
        if col >= 0.0 && row >= 0.0 && (col as usize) < cols && (row as usize) < rows {
            g[row as usize][col as usize] = ch;
        }
    };
    for k in 0..360 {
        let (x, y) = polar_xy(Plane::Azimuth, k as f64);
        place(&mut g, x, y, '.');
    }
    let pk = peak(gains);
    if pk.is_finite() {
        for k in 0..720 {
            let a = k as f64 * 0.5;
            let rho = polar_rho(gain_at(gains, a), pk);
            let (x, y) = polar_xy(plane, a);
            place(&mut g, x * rho, y * rho, '*');
        }
    }
    g[cy][cx] = '+';
    let (top, bottom, left, right) = match plane {
        Plane::Azimuth => ('N', 'S', 'W', 'E'),
        Plane::Elevation => ('U', 'D', '<', '>'),
    };
    g[0][cx] = top;
    g[rows - 1][cx] = bottom;
    g[cy][0] = left;
    g[cy][cols - 1] = right;
    g.into_iter().map(|row| row.into_iter().collect::<String>().trim_end().to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dipole_sweep() -> (Vec<f64>, Vec<f64>) {
        // 6..8 MHz, 21 points, a V-shaped dip to 1.2:1 at 7.1 MHz.
        let hz: Vec<f64> = (0..21).map(|i| 6e6 + i as f64 * 0.1e6).collect();
        let swr = hz.iter().map(|f| 1.2 + ((f - 7.1e6).abs() / 0.1e6) * 0.8).collect();
        (hz, swr)
    }

    #[test]
    fn swr_axis_is_logarithmic() {
        assert_eq!(y_frac(1.0), 0.0);
        assert_eq!(y_frac(10.0), 1.0);
        assert_eq!(y_frac(f64::INFINITY), 1.0);
        assert!((y_frac(10f64.sqrt()) - 0.5).abs() < 1e-12);
        assert_eq!(swr_row(1.0, 12), 11);
        assert_eq!(swr_row(50.0, 12), 0);
    }

    #[test]
    fn interpolation_and_edges() {
        let hz = [1.0, 2.0, 3.0];
        let swr = [3.0, 1.0, f64::INFINITY];
        assert_eq!(swr_at(&hz, &swr, 1.5), 2.0);
        assert_eq!(swr_at(&hz, &swr, 2.5), f64::INFINITY);
        assert_eq!(swr_at(&hz, &swr, 0.5), f64::INFINITY);
        assert_eq!(swr_at(&hz, &swr, 1.0), 3.0);
    }

    #[test]
    fn swr_plot_marks_the_dip_at_resonance() {
        let (hz, swr) = dipole_sweep();
        let lines = swr_plot(&hz, &swr, 41, 12);
        assert_eq!(lines.len(), 14);
        // the best point (7.1 MHz = column 22 of 41) is '@' on the lowest-SWR row
        let best_row = swr_row(1.2, 12);
        let row = &lines[best_row];
        let at = row.find('@').expect("best point marked");
        assert_eq!(at, " 1.2 |".len() + 22);
        // the 2:1 line is drawn
        assert!(lines[swr_row(2.0, 12)].contains("--"));
        // the ends of the band are far worse: pinned to the top or high up
        assert!(lines[0].contains('^') || lines[1].contains('*'));
        assert!(lines[13].starts_with("      6.000"));
        assert!(lines[13].ends_with("8.000 MHz"), "{}", lines[13]);
        assert!(lines[13].contains("7.000"));
    }

    #[test]
    fn swr_plot_leaves_unanalysed_columns_blank() {
        let hz = [1e6, 2e6, 3e6];
        let swr = [f64::INFINITY, f64::INFINITY, f64::INFINITY];
        let lines = swr_plot(&hz, &swr, 20, 6);
        assert!(lines.iter().all(|l| !l.contains('*') && !l.contains('@')));
    }

    #[test]
    fn polar_gain_interpolates_round_the_circle() {
        let g = [0.0, -10.0, -20.0, -10.0];
        assert_eq!(gain_at(&g, 45.0), -5.0);
        assert_eq!(gain_at(&g, 315.0), -5.0);
        assert_eq!(gain_at(&g, 360.0), 0.0);
        assert_eq!(polar_rho(0.0, 0.0), 1.0);
        assert_eq!(polar_rho(-30.0, 0.0), 0.0);
        assert_eq!(polar_rho(-15.0, 0.0), 0.5);
    }

    #[test]
    fn polar_plot_of_a_horizontal_dipole_has_figure_eight_lobes() {
        // East-west wire: broadside north and south, nulls east and west.
        let gains: Vec<f64> = (0..72)
            .map(|i| {
                let a = (i as f64 * 5.0).to_radians();
                let c = a.cos().abs().max(1e-3);
                20.0 * c.log10() + 2.15
            })
            .collect();
        let lines = polar_plot(&gains, Plane::Azimuth, 8);
        assert_eq!(lines.len(), 17);
        assert!(lines[0].contains('N') && lines[16].contains('S'));
        // lobes reach the ring north and south of the centre
        assert!(lines[1].contains('*') && lines[15].contains('*'));
        // ... and the north lobe is wide near the ring but the east/west nulls pull in to the centre
        let north: Vec<char> = lines[1].chars().collect();
        let stars = north.iter().filter(|c| **c == '*').count();
        assert!(stars >= 2, "{}", lines[1]);
        // nothing far out to the east or west on the centre row
        let centre: Vec<char> = lines[8].chars().collect();
        assert!(!centre[..6].contains(&'*') && !centre[27..].contains(&'*'), "{}", lines[8]);
        assert_eq!(centre[0], 'W');
        assert_eq!(*centre.last().unwrap(), 'E');
    }

    #[test]
    fn elevation_plot_is_labelled() {
        let lines = polar_plot(&vec![0.0; 72], Plane::Elevation, 5);
        assert!(lines[0].contains('U'));
        assert!(lines[10].contains('D'));
        assert!(lines[5].starts_with('<') && lines[5].ends_with('>'));
    }
}
