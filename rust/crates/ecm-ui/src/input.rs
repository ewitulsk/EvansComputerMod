//! Input events.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

impl MouseButton {
    pub(crate) fn index(self) -> usize {
        match self {
            MouseButton::Left => 0,
            MouseButton::Right => 1,
            MouseButton::Middle => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Backspace,
    Delete,
    Enter,
    Escape,
    Tab,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
}

/// Keyboard modifier state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
}

/// One input event. Coordinates are canvas pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputEvent {
    MouseMove { x: i32, y: i32 },
    MouseDown { x: i32, y: i32, button: MouseButton },
    MouseUp { x: i32, y: i32, button: MouseButton },
    /// `delta`: +1 = wheel up (scroll towards the top), -1 = down.
    Scroll { x: i32, y: i32, delta: i32 },
    Key(Key),
    /// A typed printable character (control characters are ignored by fields).
    Char(char),
    /// Modifier state changed; applies to the events that follow it (and is
    /// remembered across frames). Alternatively set [`crate::Ui::modifiers`].
    Modifiers(Modifiers),
}

/// Output of the on-screen keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyOrChar {
    Key(Key),
    Char(char),
}

impl KeyOrChar {
    pub fn to_event(self) -> InputEvent {
        match self {
            KeyOrChar::Key(k) => InputEvent::Key(k),
            KeyOrChar::Char(c) => InputEvent::Char(c),
        }
    }
}
