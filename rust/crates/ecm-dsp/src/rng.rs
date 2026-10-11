//! A tiny deterministic PRNG (xoshiro256**) and AWGN channel helpers.
//!
//! Used for reproducible tests and by the game's noise synthesis; it is *not*
//! cryptographically secure.

use crate::complex::C32;

/// xoshiro256** seeded through splitmix64.
#[derive(Clone, Debug)]
pub struct Rng {
    s: [u64; 4],
    spare: Option<f64>,
}

impl Rng {
    pub fn new(seed: u64) -> Rng {
        let mut z = seed;
        let mut next = || {
            z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut x = z;
            x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            x ^ (x >> 31)
        };
        Rng { s: [next(), next(), next(), next()], spare: None }
    }

    pub fn next_u64(&mut self) -> u64 {
        let result = self.s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        result
    }

    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// Uniform in `[0, 1)`.
    pub fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniform integer in `0..n` (n > 0).
    pub fn below(&mut self, n: u64) -> u64 {
        ((self.next_u64() as u128 * n as u128) >> 64) as u64
    }

    /// A random bit (0 or 1).
    pub fn bit(&mut self) -> u8 {
        (self.next_u64() >> 63) as u8
    }

    /// Standard normal (mean 0, variance 1), Box–Muller.
    pub fn gaussian(&mut self) -> f64 {
        if let Some(v) = self.spare.take() {
            return v;
        }
        loop {
            let u1 = self.uniform();
            if u1 <= f64::MIN_POSITIVE {
                continue;
            }
            let u2 = self.uniform();
            let r = (-2.0 * u1.ln()).sqrt();
            let th = 2.0 * core::f64::consts::PI * u2;
            self.spare = Some(r * th.sin());
            return r * th.cos();
        }
    }

    /// Circular complex Gaussian with total variance `power` (`power/2` per component).
    pub fn cgaussian(&mut self, power: f32) -> C32 {
        let s = (power as f64 / 2.0).sqrt();
        C32::new((self.gaussian() * s) as f32, (self.gaussian() * s) as f32)
    }

    pub fn fill_bytes(&mut self, out: &mut [u8]) {
        for b in out {
            *b = self.next_u64() as u8;
        }
    }
}

/// Add complex white Gaussian noise of total power `noise_power` per sample.
pub fn awgn(x: &mut [C32], noise_power: f32, rng: &mut Rng) {
    for v in x {
        *v += rng.cgaussian(noise_power);
    }
}

/// Add real white Gaussian noise of variance `noise_power`.
pub fn awgn_real(x: &mut [f32], noise_power: f32, rng: &mut Rng) {
    let s = (noise_power as f64).sqrt();
    for v in x {
        *v += (rng.gaussian() * s) as f32;
    }
}

/// Add complex noise so that `signal_power / noise_power = snr_db` (power over the
/// full sample bandwidth). Returns the noise power used.
pub fn awgn_snr(x: &mut [C32], signal_power: f32, snr_db: f32, rng: &mut Rng) -> f32 {
    let np = signal_power / crate::complex::from_db(snr_db);
    awgn(x, np, rng);
    np
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gaussian_moments() {
        let mut r = Rng::new(1);
        let n = 200_000;
        let (mut m, mut v) = (0.0, 0.0);
        for _ in 0..n {
            let g = r.gaussian();
            m += g;
            v += g * g;
        }
        m /= n as f64;
        v /= n as f64;
        assert!(m.abs() < 0.01, "mean {m}");
        assert!((v - 1.0).abs() < 0.02, "var {v}");
    }
}
