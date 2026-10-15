use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use reclaim_core::artifacts::Project;
use reclaim_core::explorer::Tree;
use reclaim_core::junk::TargetResult;
use reclaim_core::paths::Guard;
use reclaim_core::rules::{Mode, Risk, TargetRule, builtin_targets, load_user_targets};
use reclaim_core::settings::{Settings, data_dir};
use reclaim_core::{Error, Job};

pub struct AppState {
    pub data_dir: PathBuf,
    pub settings: RwLock<Settings>,
    /// Set when the settings file exists but cannot be read. Destructive commands refuse to
    /// run until it is fixed or reset, because the user's exclusions would be missing.
    pub settings_error: RwLock<Option<String>>,
    pub user_rules: Vec<TargetRule>,
    pub rule_problems: Vec<String>,
    pub tree: RwLock<Option<Tree>>,
    pub junk: Mutex<Vec<TargetResult>>,
    pub projects: Mutex<Vec<Project>>,
    pub jobs: Mutex<HashMap<String, Arc<Job>>>,
}

impl AppState {
    pub fn load() -> Self {
        let dir = data_dir();
        let (settings, settings_error) = match Settings::load(&dir) {
            Ok(s) => (s, None),
            Err(e) => (Settings::default(), Some(e.to_string())),
        };
        let (user_rules, rule_problems) = load_user_targets(&dir.join("rules"));
        AppState {
            data_dir: dir,
            settings: RwLock::new(settings),
            settings_error: RwLock::new(settings_error),
            user_rules,
            rule_problems,
            tree: RwLock::new(None),
            junk: Mutex::new(Vec::new()),
            projects: Mutex::new(Vec::new()),
            jobs: Mutex::new(HashMap::new()),
        }
    }

    /// Built-in rules followed by valid user rules.
    pub fn all_rules(&self) -> Vec<TargetRule> {
        builtin_targets()
            .iter()
            .cloned()
            .chain(self.user_rules.iter().cloned())
            .collect()
    }

    pub fn guard(&self) -> Result<Guard, Error> {
        if let Some(e) = self.settings_error.read().clone() {
            return Err(Error::Other(format!(
                "settings could not be read, cleaning is paused: {e}"
            )));
        }
        Ok(Guard::new(
            &self.settings.read().exclusions,
            Some(&self.data_dir),
        ))
    }

    /// Re-paints the explorer tree with reclaimable folders from the latest scans.
    pub fn remark_tree(&self) {
        let mut tree = self.tree.write();
        let Some(tree) = tree.as_mut() else { return };
        let rules = self.all_rules();
        let mut entries: Vec<(PathBuf, Risk, u64)> = Vec::new();
        for res in self.junk.lock().iter() {
            let Some(rule) = rules.iter().find(|r| r.id == res.id) else {
                continue;
            };
            if matches!(rule.mode, Mode::Files | Mode::Report) {
                continue;
            }
            for f in &res.found {
                entries.push((PathBuf::from(&f.path), rule.risk, f.bytes));
            }
        }
        for p in self.projects.lock().iter() {
            for a in &p.artifacts {
                entries.push((PathBuf::from(&a.path), a.risk, a.bytes));
            }
        }
        tree.mark(&entries);
    }
}
