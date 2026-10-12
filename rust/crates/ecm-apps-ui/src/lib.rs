//! `ecm-apps-ui`: the app platform's user-facing parts (docs/gui/APPS.md,
//! "The App Store, the file picker and the chooser").
//!
//! * [`store`]: the App Store (catalog, search, detail page, install /
//!   update / remove with progress, Updates and Installed tabs), adaptive
//!   at compact / medium / wide.
//! * [`picker`]: the system file picker (`PickFile`), drawn by the system,
//!   not by the app that asked.
//! * [`chooser`]: the "open with" chooser.
//! * [`settings`]: the Settings → Apps section (installed apps, permission
//!   toggles, one-file grants, open-with defaults), for the Settings app.
//! * [`uictl`]: how the picker and chooser answer the compositor.
//! * [`appctl`]: the `app_ctl` requests and replies; [`perms`]: permissions in
//!   plain words; [`png`]: icons and screenshots; [`samples`]: the sample
//!   repository.
//!
//! Everything here runs on the host against mock connections (tests and the
//! golden sessions in `examples/record_app_sessions.rs`); the `store`,
//! `picker` and `chooser` programs are thin `main`s around it.

pub mod appctl;
pub mod assets;
pub mod chooser;
pub mod perms;
pub mod picker;
pub mod png;
pub mod samples;
pub mod settings;
pub mod store;
pub mod uictl;
pub mod widgets;

/// The App Store's app id (a system app: `/apps/dev.ecm.store/`).
pub const STORE_ID: &str = "dev.ecm.store";
