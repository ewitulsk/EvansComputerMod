//! Colour theme and metrics.

use crate::geom::Color;

#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub background: Color,
    pub panel: Color,
    pub border: Color,
    pub text: Color,
    pub text_dim: Color,
    pub accent: Color,
    pub accent_text: Color,
    pub button: Color,
    pub button_hover: Color,
    pub button_active: Color,
    pub danger: Color,
    pub bar_bg: Color,
    pub bar_fill: Color,
    pub selection: Color,
    /// Item slot background / hovered slot background.
    pub slot: Color,
    pub slot_hover: Color,
    /// Text field background.
    pub field_bg: Color,
    pub scrollbar_track: Color,
    pub scrollbar_thumb: Color,
    /// Modal backdrop (translucent; blended over everything drawn before).
    pub backdrop: Color,
    pub tooltip_bg: Color,
    /// Integer text scale used by every widget.
    pub text_scale: i32,
    /// Scrollbar width in pixels.
    pub scrollbar_width: i32,
}

impl Default for Theme {
    fn default() -> Theme {
        Theme {
            background: Color::hex(0x181A20),
            panel: Color::hex(0x23262E),
            border: Color::hex(0x3A3F4B),
            text: Color::hex(0xE6E8EC),
            text_dim: Color::hex(0x8A909C),
            accent: Color::hex(0x4F8CFF),
            accent_text: Color::hex(0xFFFFFF),
            button: Color::hex(0x323743),
            button_hover: Color::hex(0x3E4555),
            button_active: Color::hex(0x2757B8),
            danger: Color::hex(0xE05555),
            bar_bg: Color::hex(0x15171C),
            bar_fill: Color::hex(0x4FB477),
            selection: Color::hex(0xFFC940),
            slot: Color::hex(0x1E2128),
            slot_hover: Color::hex(0x333A48),
            field_bg: Color::hex(0x121419),
            scrollbar_track: Color::hex(0x1E2128),
            scrollbar_thumb: Color::hex(0x4A5163),
            backdrop: Color::rgba(0, 0, 0, 150),
            tooltip_bg: Color::hex(0x0E0F13),
            text_scale: 1,
            scrollbar_width: 6,
        }
    }
}

impl Theme {
    /// Default theme with text scale / scrollbar width picked for a canvas size
    /// (scale 1 below 480px wide, 2 up to 960, 3 beyond; wider bars for touch).
    pub fn for_size(width: i32, height: i32) -> Theme {
        let m = width.min(height * 4 / 3);
        let scale = if m >= 960 { 3 } else if m >= 480 { 2 } else { 1 };
        Theme { text_scale: scale, scrollbar_width: 4 + 3 * scale, ..Theme::default() }
    }

    /// Height of a comfortable single-line control (button, field, tab).
    pub fn control_height(&self) -> i32 {
        crate::canvas::GLYPH_HEIGHT * self.text_scale.max(1) + 6
    }
}
