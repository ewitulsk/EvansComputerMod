//! The 16 console colours, numbered as ComputerCraft numbers them: each colour
//! is one bit (white = 1 ... black = 32768), so sets of colours (bundled
//! cables) are sums of them. Blit strings name a colour by one hex digit
//! (white "0" ... black "f").

/// Colour names, by bit index (colour = 1 << index).
pub const NAMES: [&str; 16] = [
    "white", "orange", "magenta", "lightBlue", "yellow", "lime", "pink", "gray",
    "lightGray", "cyan", "purple", "blue", "brown", "green", "red", "black",
];

/// The default palette (0xRRGGBB), by bit index.
pub const DEFAULT_PALETTE: [u32; 16] = [
    0xF0F0F0, 0xF2B233, 0xE57FD8, 0x99B2F2, 0xDEDE6C, 0x7FCC19, 0xF2B2CC, 0x4C4C4C,
    0x999999, 0x4C99B2, 0xB266E5, 0x3366CC, 0x7F664C, 0x57A64E, 0xCC4C4C, 0x111111,
];

pub const WHITE: u32 = 1;
pub const BLACK: u32 = 1 << 15;

/// The console's own 16 colours (SGR 30-37 / 90-97) nearest to each
/// colour, by bit index. The console has a fixed palette, so orange and brown
/// share one; programs still read back the colour they set.
const CONSOLE: [u8; 16] = [15, 3, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0];

/// Bit index of a colour: its highest set bit, as a colour's value can be a
/// set of colours. Err for 0, negatives, fractions and values past black.
pub fn index(colour: f64) -> Result<usize, String> {
    if !(colour >= 1.0 && colour < 65536.0) || colour.fract() != 0.0 {
        return Err(format!("Colour out of range ({colour})"));
    }
    Ok(31 - (colour as u32).leading_zeros() as usize)
}

/// The colour (a single bit) for a bit index.
pub fn from_index(i: usize) -> u32 {
    1 << (i & 15)
}

/// The blit digit of a colour ("0"-"9", "a"-"f").
pub fn to_blit(colour: f64) -> Result<char, String> {
    Ok(char::from_digit(index(colour)? as u32, 16).unwrap())
}

/// The colour of a blit digit, or None if it isn't one.
pub fn from_blit(c: char) -> Option<u32> {
    c.to_digit(16).map(|d| from_index(d as usize))
}

/// The console colour (0-15) to draw bit index `i` with.
pub fn console_colour(i: usize) -> u8 {
    CONSOLE[i & 15]
}

/// SGR parameter setting the foreground (or background) to bit index `i`.
pub fn sgr(i: usize, background: bool) -> u8 {
    let c = console_colour(i);
    let base = match (background, c < 8) {
        (false, true) => 30,
        (false, false) => 82,
        (true, true) => 40,
        (true, false) => 92,
    };
    base + c
}

/// Set operations on colour sets (bundled cable colours).
pub fn combine(colours: &[f64]) -> u32 {
    colours.iter().fold(0, |acc, &c| acc | c as u32)
}

pub fn subtract(set: f64, colours: &[f64]) -> u32 {
    colours.iter().fold(set as u32, |acc, &c| acc & !(c as u32))
}

pub fn test(set: f64, colour: f64) -> bool {
    let c = colour as u32;
    (set as u32) & c == c
}

/// 0xRRGGBB from red, green and blue in 0..1.
pub fn pack_rgb(r: f64, g: f64, b: f64) -> u32 {
    let ch = |v: f64| ((v.clamp(0.0, 1.0) * 255.0) as u32) & 0xFF;
    (ch(r) << 16) | (ch(g) << 8) | ch(b)
}

/// Red, green and blue in 0..1 from 0xRRGGBB.
pub fn unpack_rgb(rgb: u32) -> (f64, f64, f64) {
    (
        ((rgb >> 16) & 0xFF) as f64 / 255.0,
        ((rgb >> 8) & 0xFF) as f64 / 255.0,
        (rgb & 0xFF) as f64 / 255.0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_are_bits_and_blit_digits() {
        assert_eq!(index(1.0), Ok(0));
        assert_eq!(index(32768.0), Ok(15));
        assert_eq!(index(3.0), Ok(1), "a set uses its highest colour");
        assert!(index(0.0).is_err() && index(65536.0).is_err() && index(1.5).is_err());
        assert_eq!(to_blit(16384.0), Ok('e'));
        assert_eq!(from_blit('f'), Some(32768));
        assert_eq!(from_blit('z'), None);
        assert_eq!(sgr(0, false), 97, "white is bright white");
        assert_eq!(sgr(15, true), 40, "black background");
        assert_eq!(sgr(14, false), 31, "red");
    }

    #[test]
    fn sets_and_rgb() {
        assert_eq!(combine(&[1.0, 2.0, 2.0]), 3);
        assert_eq!(subtract(7.0, &[2.0]), 5);
        assert!(test(5.0, 4.0) && !test(5.0, 2.0));
        assert_eq!(pack_rgb(1.0, 0.0, 0.5), 0xFF007F);
        let (r, g, b) = unpack_rgb(0xFF0000);
        assert_eq!((r, g, b), (1.0, 0.0, 0.0));
    }
}
