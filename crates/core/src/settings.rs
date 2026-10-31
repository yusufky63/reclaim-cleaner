//! User settings and the app's data folder. In portable mode (a `portable` file next to the
//! exe) everything lives in `.\data` beside the exe instead of `%APPDATA%\ReClaim`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Result;
use crate::clean::DeleteMethod;
use crate::error::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// `system`, `en` or `tr`.
    pub language: String,
    /// `system`, `light` or `dark`.
    pub theme: String,
    /// For caches, temp files and build artifacts, which regenerate.
    pub cache_delete: DeleteMethod,
    /// For the user's own files: large files, duplicates, empty folders.
    pub files_delete: DeleteMethod,
    pub show_caution: bool,
    pub exclusions: Vec<String>,
    pub project_roots: Vec<String>,
    pub idle_days: u32,
    pub large_min_mb: u64,
    pub duplicate_min_kb: u64,
    pub installer_age_days: u32,
    pub check_updates: bool,
    pub onboarded: bool,
    /// The Explorer entry is on by default for installed copies. This records that the default
    /// was applied once, so an entry the user turned off stays off.
    pub explorer_menu_default_applied: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            language: String::from("system"),
            theme: String::from("system"),
            cache_delete: DeleteMethod::Permanent,
            files_delete: DeleteMethod::Recycle,
            show_caution: false,
            exclusions: Vec::new(),
            project_roots: Vec::new(),
            idle_days: 30,
            large_min_mb: 100,
            duplicate_min_kb: 1024,
            installer_age_days: 30,
            check_updates: false,
            onboarded: false,
            explorer_menu_default_applied: false,
        }
    }
}

/// Where settings, history and user rules are stored.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = portable_dir() {
        return dir;
    }
    crate::paths::env_path("APPDATA")
        .unwrap_or_else(std::env::temp_dir)
        .join("ReClaim")
}

/// `<exe dir>\data` when a `portable` marker file sits next to the exe.
pub fn portable_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    dir.join("portable").is_file().then(|| dir.join("data"))
}

pub fn is_portable() -> bool {
    portable_dir().is_some()
}

fn settings_path(dir: &Path) -> PathBuf {
    dir.join("settings.json")
}

impl Settings {
    /// Loads settings. A missing file gives defaults; an unreadable one is an error, because
    /// silently dropping the user's exclusions could delete what they protected.
    pub fn load(dir: &Path) -> Result<Settings> {
        let path = settings_path(dir);
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| {
                Error::Other(format!(
                    "settings file is invalid ({}): {e}",
                    path.display()
                ))
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(e) => Err(Error::io(path, e)),
        }
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        let path = settings_path(dir);
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?).map_err(|e| Error::io(&tmp, e))?;
        std::fs::rename(&tmp, &path).map_err(|e| Error::io(&path, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_fails_closed() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(Settings::load(tmp.path()).unwrap().idle_days, 30);
        let mut s = Settings::default();
        s.exclusions.push(r"D:\Keep".into());
        s.save(tmp.path()).unwrap();
        assert_eq!(
            Settings::load(tmp.path()).unwrap().exclusions,
            vec![r"D:\Keep".to_string()]
        );
        std::fs::write(tmp.path().join("settings.json"), b"{ not json").unwrap();
        assert!(Settings::load(tmp.path()).is_err());
    }
}
