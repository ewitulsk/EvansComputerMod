//! CRCs: CRC-16/X-25 (the AX.25 / HDLC FCS), CRC-16/CCITT-FALSE and CRC-32 (IEEE).

const fn table_reflected16(poly: u16) -> [u16; 256] {
    let mut t = [0u16; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u16;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { (c >> 1) ^ poly } else { c >> 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

const fn table_msb16(poly: u16) -> [u16; 256] {
    let mut t = [0u16; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = (i as u16) << 8;
        let mut k = 0;
        while k < 8 {
            c = if c & 0x8000 != 0 { (c << 1) ^ poly } else { c << 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

const fn table32(poly: u32) -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { (c >> 1) ^ poly } else { c >> 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

static X25: [u16; 256] = table_reflected16(0x8408);
static CCITT: [u16; 256] = table_msb16(0x1021);
static CRC32: [u32; 256] = table32(0xEDB8_8320);

/// Incremental CRC-16/X-25 (reflected 0x1021, init 0xFFFF, xorout 0xFFFF).
/// This is the AX.25 / HDLC frame check sequence, transmitted low byte first.
#[derive(Clone, Copy, Debug)]
pub struct Crc16X25(u16);

impl Default for Crc16X25 {
    fn default() -> Self {
        Self::new()
    }
}

impl Crc16X25 {
    pub fn new() -> Self {
        Crc16X25(0xFFFF)
    }
    pub fn update(&mut self, data: &[u8]) {
        for &b in data {
            self.0 = (self.0 >> 8) ^ X25[((self.0 ^ b as u16) & 0xFF) as usize];
        }
    }
    pub fn finish(self) -> u16 {
        self.0 ^ 0xFFFF
    }
}

/// CRC-16/X-25 of `data` (check value for "123456789" is 0x906E).
pub fn crc16_x25(data: &[u8]) -> u16 {
    let mut c = Crc16X25::new();
    c.update(data);
    c.finish()
}

/// CRC-16/CCITT-FALSE (MSB-first 0x1021, init 0xFFFF, no xorout); check 0x29B1.
pub fn crc16_ccitt_false(data: &[u8]) -> u16 {
    let mut c: u16 = 0xFFFF;
    for &b in data {
        c = (c << 8) ^ CCITT[((c >> 8) ^ b as u16) as usize];
    }
    c
}

/// Incremental CRC-32 (IEEE 802.3 / zlib).
#[derive(Clone, Copy, Debug)]
pub struct Crc32(u32);

impl Default for Crc32 {
    fn default() -> Self {
        Self::new()
    }
}

impl Crc32 {
    pub fn new() -> Self {
        Crc32(0xFFFF_FFFF)
    }
    pub fn update(&mut self, data: &[u8]) {
        for &b in data {
            self.0 = (self.0 >> 8) ^ CRC32[((self.0 ^ b as u32) & 0xFF) as usize];
        }
    }
    pub fn finish(self) -> u32 {
        self.0 ^ 0xFFFF_FFFF
    }
}

/// CRC-32 of `data` (check value for "123456789" is 0xCBF43926).
pub fn crc32(data: &[u8]) -> u32 {
    let mut c = Crc32::new();
    c.update(data);
    c.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        assert_eq!(crc16_x25(b"123456789"), 0x906E);
        assert_eq!(crc16_ccitt_false(b"123456789"), 0x29B1);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"The quick brown fox jumps over the lazy dog"), 0x414F_A339);
        let mut c = Crc32::new();
        c.update(b"1234");
        c.update(b"56789");
        assert_eq!(c.finish(), 0xCBF4_3926);
    }

    #[test]
    fn x25_residue() {
        // Appending the FCS (low byte first) gives the constant good-frame residue.
        let mut frame = b"hello ax25".to_vec();
        let fcs = crc16_x25(&frame);
        frame.extend_from_slice(&fcs.to_le_bytes());
        let mut c = Crc16X25::new();
        c.update(&frame);
        assert_eq!(c.0, 0xF0B8);
    }
}
