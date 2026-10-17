//! The computer's clocks, as ComputerCraft programs read them (`os.time`,
//! `os.day`, `os.epoch`): in-game time from the world's day time (ticks, 0 =
//! 06:00 of the first day, 24000 ticks a day) or real time in UTC.
//!
//! "local" has no time zone to use inside the computer, so it is UTC too.

/// Which clock `time`/`day`/`epoch` read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Locale {
    Ingame,
    Utc,
    Local,
}

impl Locale {
    /// Parse a locale name (case-insensitive); None means in-game.
    pub fn parse(name: Option<&str>) -> Result<Locale, String> {
        match name.map(|s| s.to_ascii_lowercase()) {
            None => Ok(Locale::Ingame),
            Some(s) if s == "ingame" => Ok(Locale::Ingame),
            Some(s) if s == "utc" => Ok(Locale::Utc),
            Some(s) if s == "local" => Ok(Locale::Local),
            Some(s) => Err(format!("Unsupported operation: {s}")),
        }
    }
}

const TICKS_PER_DAY: i64 = 24000;
const MS_PER_DAY: i64 = 86_400_000;
/// In-game milliseconds per tick (a 24000-tick day is 24 in-game hours).
pub const INGAME_MS_PER_TICK: i64 = MS_PER_DAY / TICKS_PER_DAY;

/// What the clocks are read from: the world's day time (None when the
/// computer isn't in a world) and the real time.
#[derive(Clone, Copy, Debug)]
pub struct Now {
    pub day_ticks: Option<i64>,
    pub unix_ms: i64,
}

/// In-game ticks since midnight before the world's first morning.
fn ingame_ticks(now: &Now) -> i64 {
    now.day_ticks.unwrap_or(0).max(0) + 6000
}

/// Hours since midnight, 0 <= time < 24.
pub fn time(now: &Now, locale: Locale) -> f64 {
    match locale {
        Locale::Ingame => (ingame_ticks(now) % TICKS_PER_DAY) as f64 / 1000.0,
        _ => now.unix_ms.rem_euclid(MS_PER_DAY) as f64 / 3_600_000.0,
    }
}

/// In-game: the day number, 1 on the world's first day. Real: days since 1970-01-01.
pub fn day(now: &Now, locale: Locale) -> i64 {
    match locale {
        Locale::Ingame => ingame_ticks(now) / TICKS_PER_DAY + 1,
        _ => now.unix_ms.div_euclid(MS_PER_DAY),
    }
}

/// Milliseconds: in-game ones since the start of the world's first day, or
/// real ones since the Unix epoch.
pub fn epoch(now: &Now, locale: Locale) -> i64 {
    match locale {
        Locale::Ingame => ingame_ticks(now) * INGAME_MS_PER_TICK,
        _ => now.unix_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ingame_clock_follows_the_world() {
        let start = Now { day_ticks: Some(0), unix_ms: 0 };
        assert_eq!(time(&start, Locale::Ingame), 6.0, "a world starts at 06:00");
        assert_eq!(day(&start, Locale::Ingame), 1);
        let noon2 = Now { day_ticks: Some(24000 + 6000), unix_ms: 0 };
        assert_eq!(time(&noon2, Locale::Ingame), 12.0);
        assert_eq!(day(&noon2, Locale::Ingame), 2);
        // Midnight after the first day: day 2 begins.
        let midnight = Now { day_ticks: Some(18000), unix_ms: 0 };
        assert_eq!((time(&midnight, Locale::Ingame), day(&midnight, Locale::Ingame)), (0.0, 2));
        assert_eq!(epoch(&midnight, Locale::Ingame), 86_400_000, "one in-game day of ms");
        // One real second (20 ticks) is 72000 in-game ms.
        let a = Now { day_ticks: Some(100), unix_ms: 0 };
        let b = Now { day_ticks: Some(120), unix_ms: 0 };
        assert_eq!(epoch(&b, Locale::Ingame) - epoch(&a, Locale::Ingame), 72_000);
    }

    #[test]
    fn real_clock_is_utc() {
        let t = Now { day_ticks: None, unix_ms: 3 * MS_PER_DAY + 90 * 60 * 1000 };
        assert_eq!(time(&t, Locale::Utc), 1.5);
        assert_eq!(day(&t, Locale::Utc), 3);
        assert_eq!(epoch(&t, Locale::Local), t.unix_ms);
        assert_eq!(Locale::parse(Some("UTC")), Ok(Locale::Utc));
        assert!(Locale::parse(Some("mars")).is_err());
    }
}
