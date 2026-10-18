//! The microphone's voice-activity gate (from Discord's voice `Sender`): a frame
//! whose peak reaches [`VoiceGate::threshold_db`] opens the gate, and it stays open
//! for [`VoiceGate::hang_frames`] quieter frames after the last loud one, so word
//! endings and short pauses aren't clipped. Muted, nothing passes and the hang time
//! is dropped.
//!
//! The gate judges whole frames; the caller cuts the microphone into frames (Discord
//! uses 40 ms, VoLTE 20 ms) and sends only the frames [`VoiceGate::admit`] lets through.

/// The default opening level, dBFS peak.
pub const DEFAULT_THRESHOLD_DB: i32 = -45;
/// The default hang time, frames.
pub const DEFAULT_HANG_FRAMES: u32 = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoiceGate {
    /// Frames with a peak at or above this (dBFS) open the gate.
    pub threshold_db: i32,
    /// Frames still let through after the last loud one.
    pub hang_frames: u32,
    pub muted: bool,
    hang: u32,
}

impl Default for VoiceGate {
    fn default() -> Self {
        VoiceGate {
            threshold_db: DEFAULT_THRESHOLD_DB,
            hang_frames: DEFAULT_HANG_FRAMES,
            muted: false,
            hang: 0,
        }
    }
}

impl VoiceGate {
    pub fn new() -> VoiceGate {
        VoiceGate::default()
    }

    pub fn with(threshold_db: i32, hang_frames: u32) -> VoiceGate {
        VoiceGate {
            threshold_db,
            hang_frames,
            ..VoiceGate::default()
        }
    }

    /// Judge one frame: true if it is to be sent.
    pub fn admit(&mut self, frame: &[i16]) -> bool {
        if self.muted {
            self.hang = 0;
            return false;
        }
        if ecm_audio::peak_dbfs(frame) >= self.threshold_db {
            self.hang = self.hang_frames;
        } else if self.hang > 0 {
            self.hang -= 1;
        } else {
            return false;
        }
        true
    }

    /// True while the gate is open (we are being heard).
    pub fn talking(&self) -> bool {
        self.hang > 0 && !self.muted
    }

    /// Close the gate now (end of a call, push-to-talk released).
    pub fn reset(&mut self) {
        self.hang = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::sine;

    const F: usize = 640;

    #[test]
    fn opens_on_voice_not_hiss() {
        let mut g = VoiceGate::new();
        let hiss = sine(F, 440.0, 6.0); // about -75 dBFS
        let voice = sine(F, 440.0, 8000.0);
        assert!(!g.admit(&hiss));
        assert!(!g.talking());
        assert!(g.admit(&voice));
        assert!(g.talking());
    }

    #[test]
    fn hangs_over_pauses_then_closes() {
        let mut g = VoiceGate::new();
        let hiss = sine(F, 440.0, 6.0);
        assert!(g.admit(&sine(F, 440.0, 8000.0)));
        let passed = (0..20).filter(|_| g.admit(&hiss)).count();
        assert_eq!(passed, DEFAULT_HANG_FRAMES as usize);
        assert!(!g.talking());
        // A loud frame in the hang time restarts it.
        let mut g = VoiceGate::with(-45, 2);
        g.admit(&sine(F, 440.0, 8000.0));
        assert!(g.admit(&hiss));
        assert!(g.admit(&sine(F, 440.0, 8000.0)));
        assert!(g.admit(&hiss) && g.admit(&hiss) && !g.admit(&hiss));
    }

    #[test]
    fn threshold_is_the_peak_level() {
        let mut g = VoiceGate::with(-20, 0);
        assert!(!g.admit(&[3000]), "-21 dBFS");
        assert!(g.admit(&[3300]), "-20 dBFS");
        assert!(!g.talking(), "no hang time: open only for loud frames");
    }

    #[test]
    fn muted_passes_nothing_and_drops_the_hang() {
        let mut g = VoiceGate::new();
        g.admit(&sine(F, 440.0, 8000.0));
        g.muted = true;
        assert!(!g.talking());
        assert!(!g.admit(&sine(F, 440.0, 8000.0)));
        g.muted = false;
        assert!(!g.admit(&sine(F, 440.0, 6.0)), "the hang time was dropped");
        g.admit(&sine(F, 440.0, 8000.0));
        g.reset();
        assert!(!g.talking());
    }
}
