//! Music (`dev.ecm.music`): songs streamed from every music backend
//! (`musicd`) the computer can reach, found by themselves (docs/MUSIC.md).
//!
//! * [`engine`]: the playback engine: `Range` reads, decoding, buffering,
//!   seeking, prefetch, the speaker as the clock. Host-tested against musicd.
//! * [`model`]: the backends, the pages' data, the queue and the settings.
//! * [`view`]: the window at the three size classes.
//! * [`app`]: [`app::MusicApp`] runs it all: discovery, accounts, API calls,
//!   the engine, the background mode and the Now Playing card.
//! * [`net`]: the traits for the network and the speaker; [`httpc`]: HTTP.
//! * [`viz`]: the Now Playing visualizer (a motion canvas committed at the audio's pts).
//! * [`source`]: the package folder (`music --source <dir>`), for `pkg build`.

pub mod app;
pub mod engine;
pub mod httpc;
pub mod model;
pub mod net;
pub mod source;
pub mod view;
pub mod viz;

/// Host tests link ecm-host-abi's file wrappers (pulled in with the code they share
/// an object with); off a computer there are no files behind them.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod host_abi_stubs {
    #[no_mangle]
    extern "C" fn file_read(_: i32, _: i32, _: i32, _: i32) -> i32 {
        -1
    }
    #[no_mangle]
    extern "C" fn file_write(_: i32, _: i32, _: i32, _: i32) -> i32 {
        -1
    }
    #[no_mangle]
    extern "C" fn file_exists(_: i32, _: i32) -> i32 {
        0
    }
    #[no_mangle]
    extern "C" fn file_size(_: i32, _: i32) -> i32 {
        -1
    }
    #[no_mangle]
    extern "C" fn file_mkdir(_: i32, _: i32) -> i32 {
        -1
    }
    #[no_mangle]
    extern "C" fn file_delete(_: i32, _: i32) -> i32 {
        -1
    }
    #[no_mangle]
    extern "C" fn file_list(_: i32, _: i32) -> i32 {
        -1
    }
    #[no_mangle]
    extern "C" fn file_list_dir(_: i32, _: i32, _: i32, _: i32) -> i32 {
        -1
    }
}

/// The app id.
pub const APP_ID: &str = "dev.ecm.music";
/// The icon (32x32, made in Blockbench: assets/icon.bbmodel).
pub const ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");
