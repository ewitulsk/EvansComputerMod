//! Built-in pictures: 32x32 pixel-art icons drawn by `assets/make_assets.py`.

pub const STORE: &[u8] = include_bytes!("../assets/store.png");
pub const PICKER: &[u8] = include_bytes!("../assets/picker.png");
pub const CHOOSER: &[u8] = include_bytes!("../assets/chooser.png");
/// An app without an icon of its own.
pub const APP: &[u8] = include_bytes!("../assets/app.png");

pub const ALL: &[(&str, &[u8])] = &[
    ("store", STORE),
    ("picker", PICKER),
    ("chooser", CHOOSER),
    ("app", APP),
    ("notes", crate::samples::NOTES_ICON),
    ("counter", crate::samples::COUNTER_ICON),
    ("netcheck", crate::samples::NETCHECK_ICON),
    ("notes screenshot", crate::samples::NOTES_SHOT),
];
