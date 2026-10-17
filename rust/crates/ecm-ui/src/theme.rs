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
//! | `hover` | `0x464d5aff` | ordinary buttons under the pointer |
//! | `accent_hover` | `0x5c9dfdff` | primary buttons under the pointer |
//! | `row_hover` | `0x2c323bff` | list rows and menu items under the pointer |
//! | `selection` | `0x3d8bfd70` | selected text |
//! | `wallpaper` | `0x283040ff` | the desktop behind the windows (shells) |
//! | `taskbar` | `0x171b21f0` | taskbars and status bars (shells) |
//! | `titlebar` | `0x2f5f9eff` | the focused window's title bar (shells) |
//! | `titlebar_inactive` | `0x30353eff` | other windows' title bars (shells) |
//! | `titlebar_text` | `0xffffffff` | title bar text (shells) |
//! | `grid_line` | `0x00000070` | pixel grid gridlines |
//! | `masked` | `0x8c95a3a0` | locked pixel-grid texels (hatched over the colour) |
//!
//! [`Theme::named`] knows the built-in themes ([`Theme::NAMES`]): the shell
//! sends the one `/etc/ui.toml` names, so every app restyles with it.

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
    hover: 0x464d_5aff,
    accent_hover: 0x5c9d_fdff,
    row_hover: 0x2c32_3bff,
    selection: 0x3d8b_fd70,
    wallpaper: 0x2830_40ff,
    taskbar: 0x171b_21f0,
    titlebar: 0x2f5f_9eff,
    titlebar_inactive: 0x3035_3eff,
    titlebar_text: 0xffff_ffff,
    grid_line: 0x0000_0070,
    masked: 0x8c95_a3a0,
}

impl Theme {
    /// The built-in themes, in the order Settings lists them.
    pub const NAMES: &'static [&'static str] = &["ecm-default", "ecm-light", "ecm-ocean", "ecm-contrast"];

    /// A human name for a built-in theme.
    pub fn label(name: &str) -> &'static str {
        match name {
            "ecm-default" => "Dark (default)",
            "ecm-light" => "Light",
            "ecm-ocean" => "Ocean",
            "ecm-contrast" => "High contrast",
            _ => "Custom",
        }
    }

    /// A built-in theme by name (`None` for names it doesn't know).
    pub fn named(name: &str) -> Option<Theme> {
        let mut t = Theme::default();
        t.name = name.to_string();
        match name {
            "ecm-default" => {}
            "ecm-light" => {
                t.bg = 0xf3f4_f6ff;
                t.surface = 0xffff_ffff;
                t.surface_alt = 0xdbe4_f0ff;
                t.text = 0x1d23_2bff;
                t.text_muted = 0x6b72_7eff;
                t.accent = 0x2563_ebff;
                t.accent_text = 0xffff_ffff;
                t.border = 0xc4ca_d3ff;
                t.focus = 0x2563_ebff;
                t.danger = 0xdc26_26ff;
                t.scrim = 0x0000_0066;
                t.track = 0xc4ca_d3ff;
                t.button = 0xe3e7_edff;
                t.hover = 0xd3d9_e2ff;
                t.accent_hover = 0x3b78_f0ff;
                t.row_hover = 0xe8ed_f4ff;
                t.selection = 0x2563_eb50;
                t.wallpaper = 0x9fb6_d0ff;
                t.taskbar = 0xe9ec_f0f0;
                t.titlebar = 0x3b78_d8ff;
                t.titlebar_inactive = 0xc9cf_d8ff;
                t.titlebar_text = 0xffff_ffff;
            }
            "ecm-ocean" => {
                t.bg = 0x0f25_33ff;
                t.surface = 0x163345ff;
                t.surface_alt = 0x1f4559ff;
                t.text = 0xdff3_f7ff;
                t.text_muted = 0x86a9_b5ff;
                t.accent = 0x22b8_a6ff;
                t.accent_text = 0x0412_18ff;
                t.border = 0x2a52_66ff;
                t.focus = 0x5fe0_d0ff;
                t.track = 0x2a52_66ff;
                t.button = 0x1d3f_52ff;
                t.hover = 0x2a52_66ff;
                t.accent_hover = 0x3fd0_beff;
                t.row_hover = 0x1a3a_4cff;
                t.selection = 0x22b8_a670;
                t.wallpaper = 0x0b3a_4eff;
                t.taskbar = 0x0a1b_25f0;
                t.titlebar = 0x1b7f_80ff;
                t.titlebar_inactive = 0x1d3a_4aff;
            }
            "ecm-contrast" => {
                t.bg = 0x0000_00ff;
                t.surface = 0x0000_00ff;
                t.surface_alt = 0x3a3a_00ff;
                t.text = 0xffff_ffff;
                t.text_muted = 0xd0d0_d0ff;
                t.accent = 0xffd8_00ff;
                t.accent_text = 0x0000_00ff;
                t.border = 0xffff_ffff;
                t.focus = 0x00e5_ffff;
                t.track = 0x8080_80ff;
                t.button = 0x0000_00ff;
                t.hover = 0x3030_30ff;
                t.accent_hover = 0xffe8_60ff;
                t.row_hover = 0x2020_20ff;
                t.selection = 0x00e5_ff80;
                t.wallpaper = 0x0000_00ff;
                t.taskbar = 0x0000_00ff;
                t.titlebar = 0xffd8_00ff;
                t.titlebar_inactive = 0x4040_40ff;
                t.titlebar_text = 0x0000_00ff;
                t.border_w = 2;
            }
            _ => return None,
        }
        Some(t)
    }
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

    #[test]
    fn named_themes_differ_and_round_trip() {
        for n in Theme::NAMES {
            let t = Theme::named(n).unwrap();
            assert_eq!(&t.name, n);
            assert_eq!(Theme::from_wire(&t.to_wire()), t);
        }
        assert_ne!(Theme::named("ecm-light").unwrap().bg, Theme::default().bg);
        assert!(Theme::named("nope").is_none());
    }
}
