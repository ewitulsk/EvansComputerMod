//! Theme as data (§8 "The shell sets the theme"): colors, corner radii, font
//! sizes and spacing. The shell sends a theme with `SetTheme`; every app gets
//! it in `Welcome` and `ThemeChanged` as a sorted list of `(token, u32)`
//! entries. Tokens an app doesn't know are ignored; tokens a theme leaves
//! out keep their defaults, so themes stay compatible as tokens are added.
//!
//! | Token | Default | Meaning |
//! | --- | --- | --- |
//! | `bg` | `0x1e2228ff` | window background |
//! | `surface` | `0x2a2f37ff` | fields, cards, dialogs |
//! | `surface_alt` | `0x353b45ff` | selected rows, pressed controls |
//! | `text` | `0xe6e9eeff` | body text |
//! | `text_muted` | `0x8c95a3ff` | placeholders, disabled text |
//! | `accent` | `0x3d8bfdff` | primary buttons, checked boxes, sliders |
//! | `accent_text` | `0xffffffff` | text on accent |
//! | `border` | `0x454c58ff` | control borders, dividers |
//! | `focus` | `0x6faeffff` | keyboard focus ring |
//! | `danger` | `0xe5534bff` | destructive buttons |
//! | `scrim` | `0x00000099` | behind dialogs |
//! | `track` | `0x454c58ff` | slider and toggle tracks |
//! | `button` | `0x3a404bff` | ordinary buttons |
//! | `radius` | 4 | control corner radius (lu) |
//! | `font_body` | 12 | body text size (a supported `ui` size) |
//! | `font_small` | 8 | captions |
//! | `font_title` | 16 | dialog titles |
//! | `spacing` | 8 | default gap |
//! | `padding` | 8 | default container / dialog padding |
//! | `control_pad_x` | 8 | horizontal padding inside controls |
//! | `control_pad_y` | 4 | vertical padding inside controls |
//! | `border_w` | 1 | control border width |
//! | `scrollbar` | 4 | scrollbar width |
//! | `scroll_step` | 16 | lu scrolled per wheel step |

use ecm_ui_proto as proto;

macro_rules! theme {
    ($($field:ident : $default:expr),* $(,)?) => {
        /// Every token the toolkit draws with.
        #[derive(Clone, Debug, PartialEq, Eq)]
        pub struct Theme {
            pub name: String,
            $(pub $field: u32,)*
        }

        impl Default for Theme {
            fn default() -> Self {
                Theme { name: String::from("ecm-default"), $($field: $default,)* }
            }
        }

        impl Theme {
            /// Defaults overlaid with the tokens a wire theme sets.
            pub fn from_wire(t: &proto::Theme) -> Theme {
                let mut out = Theme::default();
                if !t.name.is_empty() {
                    out.name = t.name.clone();
                }
                $(if let Some(v) = t.get(stringify!($field)) { out.$field = v; })*
                out
            }

            /// Every token, as the wire format sends it (sorted by key).
            pub fn to_wire(&self) -> proto::Theme {
                let mut t = proto::Theme { name: self.name.clone(), entries: Vec::new() };
                $(t.set(stringify!($field), self.$field);)*
                t
            }

            /// The names of all tokens.
            pub const TOKENS: &'static [&'static str] = &[$(stringify!($field)),*];
        }
    };
}

theme! {
    bg: 0x1e22_28ff,
    surface: 0x2a2f_37ff,
    surface_alt: 0x353b_45ff,
    text: 0xe6e9_eeff,
    text_muted: 0x8c95_a3ff,
    accent: 0x3d8b_fdff,
    accent_text: 0xffff_ffff,
    border: 0x454c_58ff,
    focus: 0x6fae_ffff,
    danger: 0xe553_4bff,
    scrim: 0x0000_0099,
    track: 0x454c_58ff,
    button: 0x3a40_4bff,
    radius: 4,
    font_body: 12,
    font_small: 8,
    font_title: 16,
    spacing: 8,
    padding: 8,
    control_pad_x: 8,
    control_pad_y: 4,
    border_w: 1,
    scrollbar: 4,
    scroll_step: 16,
}

impl Theme {
    pub(crate) fn body(&self) -> u8 {
        valid_size(self.font_body, 12)
    }
    pub(crate) fn title(&self) -> u8 {
        valid_size(self.font_title, 16)
    }
    pub(crate) fn small(&self) -> u8 {
        valid_size(self.font_small, 8)
    }
    pub(crate) fn px(&self) -> i32 {
        self.control_pad_x as i32
    }
    pub(crate) fn py(&self) -> i32 {
        self.control_pad_y as i32
    }
}

/// A theme size that isn't a supported `ui` size falls back to `default`.
fn valid_size(v: u32, default: u8) -> u8 {
    match u8::try_from(v) {
        Ok(s) if proto::fonts::face(proto::Font::Ui, s).is_some() => s,
        _ => default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_round_trip_and_partial_themes() {
        let t = Theme::default();
        assert_eq!(Theme::from_wire(&t.to_wire()), t);
        let mut w = proto::Theme { name: "night".into(), entries: vec![] };
        w.set("accent", 0xff0000ff);
        w.set("unknown_token", 7);
        let n = Theme::from_wire(&w);
        assert_eq!(n.accent, 0xff0000ff);
        assert_eq!(n.bg, Theme::default().bg);
        assert_eq!(n.name, "night");
    }
}
