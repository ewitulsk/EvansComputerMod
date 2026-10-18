//! The loudspeaker echo guard (from the Discord app): a microphone next to a
//! loudspeaker hears the far end, and sending that back is an echo. Like a
//! speakerphone, the guard makes the call half duplex on a loudspeaker: while
//! far-end audio is playing (louder than [`EchoGuard::far_end_db`]) and for
//! [`EchoGuard::hold_ms`] after, the microphone is dropped ([`EchoGuard::blocks_mic`]).
//! With headphones there is nothing to hear back, so the call is full duplex.
//!
//! [`EchoGuard::duck_db`] turns the hard mute into attenuation instead (the near end
//! is still heard, quieter, while the far end talks): [`EchoGuard::process`].

/// Discord's hold time after the far end played, ms.
pub const DEFAULT_HOLD_MS: u64 = 300;
/// Far-end audio at or below this (dBFS peak) doesn't count as playing.
pub const DEFAULT_FAR_END_DB: i32 = -60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EchoGuard {
    /// The microphone stays closed this long after far-end audio played, ms.
    pub hold_ms: u64,
    /// Far-end peak level (dBFS) above which it counts as playing.
    pub far_end_db: i32,
    /// None: drop the microphone (Discord). Some(db): attenuate it by this much instead.
    pub duck_db: Option<u32>,
    /// When far-end audio last played (0 = never).
    last_played: u64,
}

impl Default for EchoGuard {
    fn default() -> Self {
        EchoGuard {
            hold_ms: DEFAULT_HOLD_MS,
            far_end_db: DEFAULT_FAR_END_DB,
            duck_db: None,
            last_played: 0,
        }
    }
}

impl EchoGuard {
    pub fn new() -> EchoGuard {
        EchoGuard::default()
    }

    pub fn with_hold(hold_ms: u64) -> EchoGuard {
        EchoGuard {
            hold_ms,
            ..EchoGuard::default()
        }
    }

    /// Far-end samples were handed to the speaker at `now` (ms).
    pub fn played(&mut self, now: u64, pcm: &[i16]) {
        if !pcm.is_empty() && ecm_audio::peak_dbfs(pcm) > self.far_end_db {
            self.last_played = now;
        }
    }

    /// When far-end audio last played (0 = never).
    pub fn last_played(&self) -> u64 {
        self.last_played
    }

    /// True if the microphone is to be dropped at `now`: on a loudspeaker, while the far
    /// end plays and `hold_ms` after.
    pub fn blocks_mic(&self, now: u64, headphones: bool) -> bool {
        !headphones && now.saturating_sub(self.last_played) < self.hold_ms && self.last_played != 0
    }

    /// Apply the guard to captured samples: false if they are to be dropped (mute mode
    /// while blocked); in duck mode they are attenuated in place and always kept.
    pub fn process(&self, now: u64, headphones: bool, mic: &mut [i16]) -> bool {
        if !self.blocks_mic(now, headphones) {
            return true;
        }
        match self.duck_db {
            None => false,
            Some(db) => {
                let gain = 10f32.powf(-(db as f32) / 20.0);
                mic.iter_mut()
                    .for_each(|s| *s = (*s as f32 * gain).round() as i16);
                true
            }
        }
    }

    /// Forget the far end (a new call).
    pub fn reset(&mut self) {
        self.last_played = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loudspeaker_is_half_duplex_with_a_hold() {
        let mut e = EchoGuard::new();
        assert!(!e.blocks_mic(1000, false), "nothing played yet");
        e.played(1000, &[8000; 10]);
        assert!(e.blocks_mic(1000, false));
        assert!(e.blocks_mic(1299, false));
        assert!(!e.blocks_mic(1300, false), "300 ms after");
        assert!(!e.blocks_mic(1100, true), "headphones: full duplex");
        assert!(!EchoGuard::with_hold(50).blocks_mic(0, false));
    }

    #[test]
    fn only_audible_far_end_counts() {
        let mut e = EchoGuard::new();
        e.played(500, &[]);
        e.played(500, &[20; 10]); // about -64 dBFS
        assert_eq!(e.last_played(), 0);
        assert!(!e.blocks_mic(500, false));
        e.played(500, &[100; 10]);
        assert_eq!(e.last_played(), 500);
        e.reset();
        assert!(!e.blocks_mic(500, false));
    }

    #[test]
    fn duck_mode_attenuates_instead() {
        let mut e = EchoGuard::new();
        e.played(10, &[8000]);
        let mut mic = vec![10000i16; 4];
        assert!(!e.process(20, false, &mut mic.clone()), "mute mode drops");
        e.duck_db = Some(20);
        assert!(e.process(20, false, &mut mic));
        assert_eq!(mic, vec![1000; 4]);
        let mut m2 = vec![10000i16; 2];
        assert!(e.process(20, true, &mut m2));
        assert_eq!(m2, vec![10000; 2], "headphones: untouched");
    }
}
