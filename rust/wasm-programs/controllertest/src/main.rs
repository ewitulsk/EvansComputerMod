//! `controllertest` — shows a Wireless Xbox Controller live on the terminal.
//!
//! Draws the controller in a double-buffered RGB565 display and lights up
//! every button, stick and trigger as it is used, once per vertical blank.
//! Hold Back + Start (or press Ctrl+T) to quit.
//!
//! Usage: controllertest [player]     player 1-4, default: the first connected

use ecm_host_abi::gamepad::{self, Button, State};
use ecm_host_abi::gfx_child::{self, FLAG_DOUBLE_BUFFER, FORMAT_RGB565, PRESENT_WAIT_VBLANK};
use ecm_host_abi::video::Target;

const W: i32 = 320;
const H: i32 = 200;

const fn rgb(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3)
}

const BG: u16 = rgb(13, 17, 23);
const BODY: u16 = rgb(59, 63, 69);
const EDGE: u16 = rgb(24, 25, 28);
const OFF: u16 = rgb(38, 40, 43);
const ON: u16 = rgb(230, 237, 243);
const GREEN: u16 = rgb(63, 185, 80);
const RED: u16 = rgb(229, 83, 75);
const BLUE: u16 = rgb(68, 147, 248);
const YELLOW: u16 = rgb(240, 180, 41);

struct Canvas {
    px: Vec<u16>,
}

impl Canvas {
    fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: u16) {
        for yy in y.max(0)..(y + h).min(H) {
            for xx in x.max(0)..(x + w).min(W) {
                self.px[(yy * W + xx) as usize] = c;
            }
        }
    }

    fn disc(&mut self, cx: i32, cy: i32, r: i32, c: u16) {
        for yy in -r..=r {
            for xx in -r..=r {
                if xx * xx + yy * yy <= r * r {
                    self.rect(cx + xx, cy + yy, 1, 1, c);
                }
            }
        }
    }

    fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.px.len() * 2);
        for p in &self.px {
            out.extend_from_slice(&p.to_le_bytes());
        }
        out
    }
}

fn button(c: &mut Canvas, s: &State, b: Button, x: i32, y: i32, lit: u16) {
    c.disc(x, y, 7, EDGE);
    c.disc(x, y, 6, if s.is_down(b) { lit } else { OFF });
}

fn draw(c: &mut Canvas, s: &State) {
    c.rect(0, 0, W, H, BG);
    // Body and grips.
    c.rect(50, 60, 220, 70, EDGE);
    c.rect(52, 62, 216, 66, BODY);
    c.rect(52, 120, 70, 50, BODY);
    c.rect(198, 120, 70, 50, BODY);
    // Triggers (height = how far pressed) and bumpers.
    c.rect(60, 20, 40, 22, OFF);
    c.rect(60, 42 - (s.lt as i32 * 22 / 255), 40, s.lt as i32 * 22 / 255, ON);
    c.rect(220, 20, 40, 22, OFF);
    c.rect(220, 42 - (s.rt as i32 * 22 / 255), 40, s.rt as i32 * 22 / 255, ON);
    c.rect(60, 46, 50, 10, if s.is_down(Button::LB) { ON } else { OFF });
    c.rect(210, 46, 50, 10, if s.is_down(Button::RB) { ON } else { OFF });
    // Sticks: well, then the cap moved by the axes.
    for (cx, cy, x, y, click) in [
        (95, 85, s.lx, s.ly, Button::LeftStick),
        (195, 120, s.rx, s.ry, Button::RightStick),
    ] {
        c.disc(cx, cy, 15, EDGE);
        let dx = x as i32 * 8 / 127;
        let dy = -(y as i32) * 8 / 127;
        c.disc(cx + dx, cy + dy, 8, if s.is_down(click) { ON } else { rgb(107, 112, 120) });
    }
    // D-pad.
    let d = |b| if s.is_down(b) { ON } else { OFF };
    c.rect(119, 105, 12, 12, OFF);
    c.rect(119, 93, 12, 12, d(Button::DpadUp));
    c.rect(119, 117, 12, 12, d(Button::DpadDown));
    c.rect(107, 105, 12, 12, d(Button::DpadLeft));
    c.rect(131, 105, 12, 12, d(Button::DpadRight));
    // Face buttons.
    button(c, s, Button::Y, 225, 72, YELLOW);
    button(c, s, Button::X, 210, 87, BLUE);
    button(c, s, Button::B, 240, 87, RED);
    button(c, s, Button::A, 225, 102, GREEN);
    // Back, Guide, Start.
    c.rect(134, 82, 12, 6, d(Button::Back));
    c.disc(160, 75, 7, if s.is_down(Button::Guide) { GREEN } else { OFF });
    c.rect(174, 82, 12, 6, d(Button::Start));
}

fn main() {
    let want: Option<u8> = std::env::args().nth(1).and_then(|a| a.parse().ok());
    let pad = match want {
        Some(n) => gamepad::player(n),
        None => gamepad::first(),
    };
    let Some(pad) = pad else {
        eprintln!("controllertest: no controller connected.");
        eprintln!("  Pair a Wireless Xbox Controller with this computer (right-click the Terminal),");
        eprintln!("  then right-click with it, or use the Controller toggle at the top of this window.");
        std::process::exit(1);
    };
    println!("controllertest: {} (player {}). Hold Back + Start to quit.", pad.name(), pad.player());

    if let Err(e) = gfx_child::init2(Target::Terminal, W, H, FORMAT_RGB565, FLAG_DOUBLE_BUFFER) {
        eprintln!("controllertest: can't open the display: {:?}", e);
        std::process::exit(1);
    }
    let _ = gfx_child::set_refresh(Target::Terminal, 30);

    let mut canvas = Canvas { px: vec![BG; (W * H) as usize] };
    let mut last = None;
    loop {
        let state = match pad.poll() {
            Ok(s) => s,
            Err(_) => {
                let _ = gfx_child::set_mode(Target::Terminal, 0);
                println!("controllertest: controller disconnected.");
                return;
            }
        };
        if state.is_down(Button::Back) && state.is_down(Button::Start) {
            break;
        }
        if last != Some(state) {
            draw(&mut canvas, &state);
            if gfx_child::blit_rect(Target::Terminal, 0, 0, W, H, &canvas.bytes(), FORMAT_RGB565).is_err() {
                return;
            }
            last = Some(state);
        }
        if gfx_child::present(Target::Terminal, PRESENT_WAIT_VBLANK).is_err() {
            return;
        }
    }
    let _ = gfx_child::set_mode(Target::Terminal, 0);
    println!("controllertest: bye.");
}
