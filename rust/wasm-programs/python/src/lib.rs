//! Library half of the `python` program: native modules that don't touch the
//! host ABI, so they can be tested on the host.

pub mod auth_module;
pub mod radio_module;
pub mod ui_module;

#[cfg(test)]
mod audio_tests;
pub mod web_module;

#[cfg(test)]
mod cards_tests;
#[cfg(test)]
mod cards_apps_tests;
