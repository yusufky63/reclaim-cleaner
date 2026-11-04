//! ReClaim Cleaner engine.
//!
//! Everything that touches the file system, the registry or other processes lives here, so it
//! can be tested without a WebView and reused by the elevated helper. The Tauri shell only
//! translates between this crate and the UI.

pub mod apps;
pub mod artifacts;
pub mod categories;
pub mod clean;
pub mod duplicates;
pub mod empty;
pub mod error;
pub mod explorer;
pub mod fsmeta;
pub mod hidden;
pub mod history;
pub mod icons;
pub mod installers;
pub mod job;
pub mod junk;
pub mod launch;
pub mod leftovers;
pub mod locks;
#[cfg(target_os = "linux")]
pub mod mounts;
pub mod organize;
pub mod paths;
pub mod rules;
pub mod settings;
pub mod shell;
pub mod startup;
pub mod steam;
pub mod system;
#[cfg(target_os = "linux")]
mod system_linux;
pub mod walk;
#[cfg(windows)]
mod winstr;
#[cfg(target_os = "linux")]
pub mod xdg;

pub use error::{Error, Result};
pub use job::{Job, ProgressSnapshot};
