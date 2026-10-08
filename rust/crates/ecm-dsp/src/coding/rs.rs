//! Reed–Solomon codes over GF(256) (field polynomial 0x11D, generator α = 2,
//! first consecutive root α^0), e.g. RS(255,223) correcting 16 byte errors.
//!
//! Shortened codes are supported: any data length up to `255 - nroots` works,
//! as if the block were padded with leading zeros.

const fn gf_tables() -> ([u8; 512], [u8; 256]) {
    let mut exp = [0u8; 512];
    let mut log = [0u8; 256];
    let mut x: u16 = 1;
    let mut i = 0;
    while i < 255 {
        exp[i] = x as u8;
        log[x as usize] = i as u8;
        x <<= 1;
        if x & 0x100 != 0 {
            x ^= 0x11D;
        }
        i += 1;
    }
    let mut j = 255;
    while j < 512 {
        exp[j] = exp[j - 255];
        j += 1;
    }
    (exp, log)
}

static TABLES: ([u8; 512], [u8; 256]) = gf_tables();

#[inline]
fn gexp(i: usize) -> u8 {
    TABLES.0[i % 255]
}

#[inline]
fn glog(x: u8) -> usize {
    TABLES.1[x as usize] as usize
}

/// GF(256) multiply.
#[inline]
pub fn gf_mul(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        0
    } else {
        TABLES.0[glog(a) + glog(b)]
    }
}

/// GF(256) divide (`b != 0`).
#[inline]
pub fn gf_div(a: u8, b: u8) -> u8 {
    assert!(b != 0, "GF division by zero");
    if a == 0 {
        0
    } else {
        TABLES.0[glog(a) + 255 - glog(b)]
    }
}

#[inline]
fn gf_inv(a: u8) -> u8 {
    gf_div(1, a)
}

/// Why a block could not be decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RsError {
    /// More errors than the code can correct (detected).
    TooManyErrors,
    /// Block longer than 255 or shorter than the parity.
    BadLength,
}

/// A Reed–Solomon codec with `nroots` parity bytes (corrects `nroots/2` errors).
#[derive(Clone, Debug)]
pub struct ReedSolomon {
    nroots: usize,
    gen: Vec<u8>, // monic, highest degree first: gen[0] = 1
}

impl ReedSolomon {
    pub fn new(nroots: usize) -> ReedSolomon {
        assert!(nroots > 0 && nroots < 255);
        let mut gen = vec![1u8];
        for i in 0..nroots {
            // multiply by (x - a^i) == (x + a^i)
            let r = gexp(i);
            let mut ng = vec![0u8; gen.len() + 1];
            for (j, &g) in gen.iter().enumerate() {
                ng[j] ^= g;
                ng[j + 1] ^= gf_mul(g, r);
            }
            gen = ng;
        }
        ReedSolomon { nroots, gen }
    }

    /// The classic RS(255,223): 32 parity bytes, corrects 16 byte errors.
    pub fn rs255_223() -> ReedSolomon {
        ReedSolomon::new(32)
    }

    pub fn nroots(&self) -> usize {
        self.nroots
    }

    /// Maximum data length per block.
    pub fn max_data(&self) -> usize {
        255 - self.nroots
    }

    /// Compute the parity bytes for `data`.
    pub fn parity(&self, data: &[u8]) -> Vec<u8> {
        assert!(data.len() <= self.max_data(), "block too long");
        let mut par = vec![0u8; self.nroots];
        for &d in data {
            let fb = d ^ par[0];
            par.rotate_left(1);
            par[self.nroots - 1] = 0;
            if fb != 0 {
                for j in 0..self.nroots {
                    par[j] ^= gf_mul(fb, self.gen[j + 1]);
                }
            }
        }
        par
    }

    /// `data || parity`.
    pub fn encode(&self, data: &[u8]) -> Vec<u8> {
        let mut v = data.to_vec();
        v.extend(self.parity(data));
        v
    }

    fn syndromes(&self, block: &[u8]) -> Vec<u8> {
        (0..self.nroots)
            .map(|j| {
                let r = gexp(j);
                block.iter().fold(0u8, |acc, &c| gf_mul(acc, r) ^ c)
            })
            .collect()
    }

    /// Correct `block` (data followed by parity) in place. Returns the number
    /// of corrected bytes.
    pub fn decode(&self, block: &mut [u8]) -> Result<usize, RsError> {
        let n = block.len();
        if n > 255 || n <= self.nroots {
            return Err(RsError::BadLength);
        }
        let s = self.syndromes(block);
        if s.iter().all(|&v| v == 0) {
            return Ok(0);
        }
        // Berlekamp–Massey: lambda lowest degree first.
        let mut c = vec![0u8; self.nroots + 1];
        let mut b = vec![0u8; self.nroots + 1];
        c[0] = 1;
        b[0] = 1;
        let (mut l, mut m, mut bb) = (0usize, 1usize, 1u8);
        for i in 0..self.nroots {
            let mut d = s[i];
            for k in 1..=l {
                d ^= gf_mul(c[k], s[i - k]);
            }
            if d == 0 {
                m += 1;
                continue;
            }
            let coef = gf_div(d, bb);
            let t = c.clone();
            for k in 0..=self.nroots {
                if k + m <= self.nroots {
                    c[k + m] ^= gf_mul(coef, b[k]);
                }
            }
            if 2 * l <= i {
                l = i + 1 - l;
                b = t;
                bb = d;
                m = 1;
            } else {
                m += 1;
            }
        }
        if l == 0 || l > self.nroots / 2 {
            return Err(RsError::TooManyErrors);
        }
        let lambda = &c[..=l];
        // Chien search over actual positions. Position i has power p = n-1-i.
        let mut positions = Vec::with_capacity(l);
        for i in 0..n {
            let p = n - 1 - i;
            let xinv = gexp(255 - p % 255);
            let mut v = 0u8;
            let mut xp = 1u8;
            for &coef in lambda {
                v ^= gf_mul(coef, xp);
                xp = gf_mul(xp, xinv);
            }
            if v == 0 {
                positions.push(i);
            }
        }
        if positions.len() != l {
            return Err(RsError::TooManyErrors);
        }
        // Omega = S(x) * Lambda(x) mod x^nroots
        let mut omega = vec![0u8; self.nroots];
        for (i, &si) in s.iter().enumerate() {
            for (k, &lk) in lambda.iter().enumerate() {
                if i + k < self.nroots {
                    omega[i + k] ^= gf_mul(si, lk);
                }
            }
        }
        for &i in &positions {
            let p = n - 1 - i;
            let x = gexp(p);
            let xinv = gf_inv(x);
            let eval = |poly: &[u8]| {
                let mut v = 0u8;
                let mut xp = 1u8;
                for &coef in poly {
                    v ^= gf_mul(coef, xp);
                    xp = gf_mul(xp, xinv);
                }
                v
            };
            let om = eval(&omega);
            // formal derivative: odd-power terms only
            let deriv: Vec<u8> = (1..lambda.len()).map(|k| if k % 2 == 1 { lambda[k] } else { 0 }).collect();
            let dl = eval(&deriv);
            if dl == 0 {
                return Err(RsError::TooManyErrors);
            }
            // e = X^(1 - fcr) * Omega(X^-1) / Lambda'(X^-1), fcr = 0
            block[i] ^= gf_mul(x, gf_div(om, dl));
        }
        if self.syndromes(block).iter().any(|&v| v != 0) {
            return Err(RsError::TooManyErrors);
        }
        Ok(positions.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;

    #[test]
    fn gf_basics() {
        for a in 1..=255u8 {
            assert_eq!(gf_mul(a, gf_inv(a)), 1);
        }
        assert_eq!(gf_mul(0x53, 0xCA), {
            // carry-less multiply mod 0x11D computed bitwise
            let (mut a, mut b, mut p) = (0x53u16, 0xCAu16, 0u16);
            while b != 0 {
                if b & 1 != 0 {
                    p ^= a;
                }
                a <<= 1;
                if a & 0x100 != 0 {
                    a ^= 0x11D;
                }
                b >>= 1;
            }
            p as u8
        });
    }

    #[test]
    fn rs255_223_corrects_up_to_16_errors() {
        let rs = ReedSolomon::rs255_223();
        let mut rng = Rng::new(42);
        for trial in 0..30 {
            let mut data = vec![0u8; 223];
            rng.fill_bytes(&mut data);
            let block = rs.encode(&data);
            assert_eq!(block.len(), 255);
            let nerr = 1 + trial % 16;
            let mut bad = block.clone();
            let mut used = Vec::new();
            while used.len() < nerr {
                let p = rng.below(255) as usize;
                if !used.contains(&p) {
                    used.push(p);
                    bad[p] ^= 1 + rng.below(255) as u8;
                }
            }
            assert_eq!(rs.decode(&mut bad), Ok(nerr), "trial {trial}");
            assert_eq!(bad, block);
        }
        // exactly 16 errors
        let mut data = vec![0u8; 223];
        rng.fill_bytes(&mut data);
        let block = rs.encode(&data);
        let mut bad = block.clone();
        for k in 0..16 {
            bad[k * 15 + 3] ^= 0xA5;
        }
        assert_eq!(rs.decode(&mut bad), Ok(16));
        assert_eq!(bad, block);
    }

    #[test]
    fn rs_detects_17_errors_and_handles_shortened_blocks() {
        let rs = ReedSolomon::rs255_223();
        let mut rng = Rng::new(9);
        let mut failures = 0;
        for _ in 0..10 {
            let mut data = vec![0u8; 223];
            rng.fill_bytes(&mut data);
            let block = rs.encode(&data);
            let mut bad = block.clone();
            for k in 0..17 {
                bad[k * 14] ^= 0x3C;
            }
            match rs.decode(&mut bad) {
                Err(RsError::TooManyErrors) => failures += 1,
                Ok(_) => assert_ne!(bad, block, "17 errors cannot decode to the original"),
                Err(e) => panic!("{e:?}"),
            }
        }
        assert!(failures >= 9);
        // shortened RS(40, 8) style: 32-byte payload, 8 roots
        let rs8 = ReedSolomon::new(8);
        let msg = b"short radio packet with parity!!";
        let block = rs8.encode(msg);
        let mut bad = block.clone();
        bad[0] ^= 1;
        bad[10] ^= 0xFF;
        bad[39] ^= 7;
        bad[20] ^= 0x80;
        assert_eq!(rs8.decode(&mut bad), Ok(4));
        assert_eq!(&bad[..32], msg);
    }
}
