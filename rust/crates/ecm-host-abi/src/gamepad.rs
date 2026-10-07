//! Driver for the Wireless Xbox Controller (peripheral type `xbox_controller`).
//!
//! A connected controller is attached to the computer as `controller_1` ..
//! `controller_4` (its player number). Programs see Xbox buttons and axes
//! only; which keyboard keys produce them is configured on the controller item.
//!
//! ```ignore
//! use ecm_host_abi::gamepad::{self, Button};
//! let pad = gamepad::first().expect("no controller connected");
//! let s = pad.poll()?;
//! if s.is_down(Button::A) { jump(); }
//! let (x, y) = (s.lx, s.ly);           // -127..127, up/right positive
//! ```
//!
//! [`poll`](Gamepad::poll) reads the current state without waiting for the
//! server; call it once per frame.

use crate::peripheral::{self, Error, Value};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// Peripheral type of a controller.
pub const TYPE: &str = "xbox_controller";

/// A digital button; its value is its bit in [`State::buttons`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum Button {
    A = 1 << 0,
    B = 1 << 1,
    X = 1 << 2,
    Y = 1 << 3,
    LB = 1 << 4,
    RB = 1 << 5,
    Back = 1 << 6,
    Start = 1 << 7,
    Guide = 1 << 8,
    LeftStick = 1 << 9,
    RightStick = 1 << 10,
    DpadUp = 1 << 11,
    DpadDown = 1 << 12,
    DpadLeft = 1 << 13,
    DpadRight = 1 << 14,
}

/// One snapshot of a controller.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct State {
    /// Held buttons, a mask of [`Button`] values.
    pub buttons: u16,
    /// Left stick, -127 (left / down) .. 127 (right / up).
    pub lx: i8,
    pub ly: i8,
    /// Right stick.
    pub rx: i8,
    pub ry: i8,
    /// Triggers, 0 .. 255.
    pub lt: u8,
    pub rt: u8,
}

impl State {
    pub fn is_down(&self, b: Button) -> bool {
        self.buttons & (b as u16) != 0
    }

    /// Buttons pressed in `self` but not in `prev`.
    pub fn pressed_since(&self, prev: &State) -> u16 {
        self.buttons & !prev.buttons
    }

    /// Parse the result of the peripheral's `get_raw` method.
    pub fn from_raw(v: &Value) -> Option<State> {
        let Value::List(items) = v else { return None };
        let mut n = [0i64; 7];
        if items.len() != n.len() {
            return None;
        }
        for (slot, item) in n.iter_mut().zip(items) {
            *slot = match item {
                Value::Int(i) => *i,
                Value::Float(f) => *f as i64,
                _ => return None,
            };
        }
        let axis = |v: i64| v.clamp(-127, 127) as i8;
        let trig = |v: i64| v.clamp(0, 255) as u8;
        Some(State {
            buttons: n[0] as u16,
            lx: axis(n[1]),
            ly: axis(n[2]),
            rx: axis(n[3]),
            ry: axis(n[4]),
            lt: trig(n[5]),
            rt: trig(n[6]),
        })
    }
}

/// A connected controller.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Gamepad {
    name: String,
}

impl Gamepad {
    /// The controller attached as `name` (e.g. `"controller_2"`); not checked.
    pub fn new(name: &str) -> Gamepad {
        Gamepad { name: String::from(name) }
    }

    /// Attachment name, e.g. `"controller_1"`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Player number (1-4), from the attachment name.
    pub fn player(&self) -> u8 {
        self.name.rsplit('_').next().and_then(|n| n.parse().ok()).unwrap_or(0)
    }

    /// Current state. Fails once the controller has disconnected.
    pub fn poll(&self) -> Result<State, Error> {
        let v = peripheral::call(&self.name, "get_raw", &[])?;
        State::from_raw(&v).ok_or(Error::Malformed("get_raw result"))
    }
}

/// Every connected controller, by player number.
pub fn connected() -> Vec<Gamepad> {
    let mut pads: Vec<Gamepad> = peripheral::list()
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, ty)| ty == TYPE)
        .map(|(name, _)| Gamepad { name })
        .collect();
    pads.sort_by_key(|p| p.player());
    pads
}

/// The connected controller with the lowest player number.
pub fn first() -> Option<Gamepad> {
    connected().into_iter().next()
}

/// Player `n`'s controller, if connected.
pub fn player(n: u8) -> Option<Gamepad> {
    let name = format!("controller_{}", n);
    connected().into_iter().find(|p| p.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn raw_state_parses_and_clamps() {
        let v = Value::List(vec![
            Value::Int((Button::A as i64) | (Button::DpadUp as i64)),
            Value::Int(-127),
            Value::Int(500),
            Value::Int(0),
            Value::Int(5),
            Value::Int(255),
            Value::Int(-3),
        ]);
        let s = State::from_raw(&v).unwrap();
        assert!(s.is_down(Button::A));
        assert!(s.is_down(Button::DpadUp));
        assert!(!s.is_down(Button::B));
        assert_eq!((s.lx, s.ly, s.ry, s.lt, s.rt), (-127, 127, 5, 255, 0));
    }

    #[test]
    fn malformed_raw_state_is_rejected() {
        assert_eq!(State::from_raw(&Value::List(vec![Value::Int(1)])), None);
        assert_eq!(State::from_raw(&Value::Nil), None);
    }

    #[test]
    fn player_number_comes_from_the_name() {
        assert_eq!(Gamepad::new("controller_3").player(), 3);
        assert_eq!(Gamepad::new("left").player(), 0);
    }
}
