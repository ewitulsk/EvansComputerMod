//! The physical console: the VTE that renders into the shared framebuffer.
//! All kernel and child text output goes through here.

use crate::framebuffer;
use crate::vte::Vte;

pub struct Console {
    vte: Vte,
}

impl Console {
    pub fn new() -> Self {
        framebuffer::init(framebuffer::DEFAULT_WIDTH, framebuffer::DEFAULT_HEIGHT);
        Self { vte: Vte::new_physical(framebuffer::width(), framebuffer::height()) }
    }

    /// Raw bytes (child output), ANSI-interpreted.
    pub fn write(&mut self, data: &[u8]) {
        self.vte.write(data);
    }

    pub fn print(&mut self, s: &str) {
        self.vte.write_str(s);
    }

    pub fn println(&mut self, s: &str) {
        self.vte.write_str(s);
        self.vte.write_str("\n");
    }

    pub fn clear(&mut self) {
        self.vte.write_str("\x1b[2J\x1b[H");
    }

    pub fn width(&self) -> u16 {
        self.vte.width()
    }

    pub fn height(&self) -> u16 {
        self.vte.height()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vte_survives_hostile_child_output() {
        let _g = crate::hal::test_lock();
        let mut c = Console::new();
        let mut x: u64 = 0x1234_5678_9abc_def0;
        let mut buf = Vec::with_capacity(4096);
        for _ in 0..200 {
            buf.clear();
            for _ in 0..4096 {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                // Bias towards escape-sequence bytes.
                let b = match x % 8 {
                    0 => 0x1b,
                    1 => b'[',
                    2 => b'0' + (x >> 8) as u8 % 10,
                    3 => b';',
                    _ => (x >> 16) as u8,
                };
                buf.push(b);
            }
            c.write(&buf);
        }
        c.write(b"\x1b[65535;65535H\x1b[65535A\x1b[65535S\x1b[65535T\x1b[?1049h\x1b[?1049l");
    }
}
