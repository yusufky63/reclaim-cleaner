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
pub mod history;
pub mod installers;
pub mod job;
pub mod junk;
pub mod leftovers;
pub mod paths;
pub mod rules;
pub mod settings;
pub mod startup;
pub mod system;
pub mod walk;
mod winstr;

pub use error::{Error, Result};
pub use job::{Job, ProgressSnapshot};
