//! Cleanup rules: fixed locations (`rules/targets.json`) and project build artifacts
//! (`rules/artifacts.json`). Both are embedded at build time. Additional user rules can be
//! dropped into the data folder; they are validated more strictly and can never require
//! administrator rights or run commands.

use std::path::Path;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::paths::{ALLOWED_VARS, STEAM_VARS};
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Risk {
    Safe,
    Review,
    Caution,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Group {
    System,
    Browser,
    Apps,
    Dev,
    Games,
    Ai,
    Media,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Delete everything inside each path, keep the folder.
    Contents,
    /// Delete each path itself.
    Folder,
    /// Delete files directly inside each path that match `include` (not recursive).
    Files,
    /// Measure only. Cleaning happens through `command`, never by deleting files.
    Report,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TargetRule {
    pub id: String,
    pub group: Group,
    pub name: String,
    pub name_tr: String,
    pub description: String,
    pub description_tr: String,
    pub paths: Vec<String>,
    pub mode: Mode,
    #[serde(default)]
    pub include: Vec<String>,
    /// File or folder name globs that are kept wherever they appear inside the paths, such as
    /// autosave files that apps write into %TEMP%.
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub min_age_hours: Option<u32>,
    pub risk: Risk,
    pub admin: bool,
    #[serde(default)]
    pub close_apps: Vec<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub verified: bool,
    #[serde(default)]
    pub source: Option<String>,
    /// Set for rules loaded from the user's rules folder.
    #[serde(default, skip_deserializing)]
    pub user: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactRule {
    pub id: String,
    /// Folder name, `*` allowed (`cmake-build-*`).
    pub dir_name: String,
    /// Any one of these must exist next to the folder (in its parent). Globs and relative
    /// paths are allowed. Empty means the folder name alone is proof.
    #[serde(default)]
    pub markers: Vec<String>,
    /// All of these must exist inside the folder.
    #[serde(default)]
    pub contains: Vec<String>,
    pub risk: Risk,
    pub description: String,
    pub description_tr: String,
}

const TARGETS_JSON: &str = include_str!("../../../rules/targets.json");
const ARTIFACTS_JSON: &str = include_str!("../../../rules/artifacts.json");

/// Built-in cleanup targets. Parsing is checked by tests, so a broken rule file fails CI.
pub fn builtin_targets() -> &'static [TargetRule] {
    static CELL: OnceLock<Vec<TargetRule>> = OnceLock::new();
    CELL.get_or_init(|| serde_json::from_str(TARGETS_JSON).expect("rules/targets.json is valid"))
}

pub fn builtin_artifacts() -> &'static [ArtifactRule] {
    static CELL: OnceLock<Vec<ArtifactRule>> = OnceLock::new();
    CELL.get_or_init(|| {
        serde_json::from_str(ARTIFACTS_JSON).expect("rules/artifacts.json is valid")
    })
}

pub fn find_builtin(id: &str) -> Option<&'static TargetRule> {
    builtin_targets().iter().find(|r| r.id == id)
}

fn invalid(id: &str, reason: impl Into<String>) -> Error {
    Error::InvalidRule {
        id: id.to_string(),
        reason: reason.into(),
    }
}

/// Structural checks shared by built-in and user rules.
pub fn validate_target(rule: &TargetRule) -> Result<()> {
    let id = &rule.id;
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(invalid(id, "id must be kebab-case"));
    }
    if rule.paths.is_empty() && rule.command.is_none() {
        return Err(invalid(id, "needs paths or a command"));
    }
    for p in &rule.paths {
        let upper = p.to_ascii_uppercase();
        let starts_with =
            |v: &&str| upper == format!("%{v}%") || upper.starts_with(&format!("%{v}%\\"));
        let steam = STEAM_VARS.iter().any(starts_with);
        if steam && rule.admin {
            return Err(invalid(
                id,
                format!("administrator rules cannot use Steam folders: {p}"),
            ));
        }
        if !steam && !ALLOWED_VARS.iter().any(starts_with) {
            return Err(invalid(
                id,
                format!("path must start with an allowed variable: {p}"),
            ));
        }
        if p.contains('/')
            || p.split('\\').any(|s| s == ".." || s == "." || s.is_empty())
            || p.contains("**")
        {
            return Err(invalid(
                id,
                format!("relative, empty or recursive segments are not allowed: {p}"),
            ));
        }
        // A wildcard may not stand for the whole first folder under a root variable, so a rule
        // can never mean "every folder in AppData".
        let segs: Vec<&str> = p.split('\\').collect();
        if segs.len() >= 2 && segs[1].trim_matches('*').is_empty() {
            return Err(invalid(
                id,
                format!("first segment must be a fixed name: {p}"),
            ));
        }
    }
    if rule.mode == Mode::Files {
        if rule.include.is_empty() {
            return Err(invalid(id, "files mode needs include patterns"));
        }
        if rule
            .include
            .iter()
            .any(|g| g.contains('\\') || g.contains('/') || g.trim_matches('*').is_empty())
        {
            return Err(invalid(
                id,
                "include patterns must be specific file name globs",
            ));
        }
    }
    if rule
        .exclude
        .iter()
        .any(|g| g.contains('\\') || g.contains('/') || g.trim_matches('*').is_empty())
    {
        return Err(invalid(id, "exclude patterns must be specific names"));
    }
    if rule
        .close_apps
        .iter()
        .any(|a| !a.to_ascii_lowercase().ends_with(".exe"))
    {
        return Err(invalid(id, "closeApps entries must be .exe names"));
    }
    Ok(())
}

/// Loads `*.json` rule files from the user's rules folder. Invalid files are reported, not
/// fatal; user rules cannot ask for admin rights or commands and cannot reuse a built-in id.
pub fn load_user_targets(dir: &Path) -> (Vec<TargetRule>, Vec<String>) {
    let mut rules = Vec::new();
    let mut problems = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return (rules, problems);
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("json"))
            != Some(true)
        {
            continue;
        }
        let parsed: std::result::Result<Vec<TargetRule>, String> = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()));
        match parsed {
            Ok(list) => {
                for mut r in list {
                    let check = validate_target(&r).and_then(|_| {
                        if r.admin {
                            Err(invalid(
                                &r.id,
                                "user rules cannot require administrator rights",
                            ))
                        } else if r.command.is_some() {
                            Err(invalid(&r.id, "user rules cannot run commands"))
                        } else if find_builtin(&r.id).is_some() {
                            Err(invalid(&r.id, "id is already used by a built-in rule"))
                        } else {
                            Ok(())
                        }
                    });
                    match check {
                        Ok(()) => {
                            r.user = true;
                            r.verified = false;
                            rules.push(r);
                        }
                        Err(e) => problems.push(format!("{}: {e}", path.display())),
                    }
                }
            }
            Err(e) => problems.push(format!("{}: {e}", path.display())),
        }
    }
    (rules, problems)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    use crate::paths::{Guard, Origin, expand_vars};
    use std::collections::HashSet;
    #[cfg(windows)]
    use std::path::PathBuf;

    #[test]
    fn builtin_rules_parse_and_validate() {
        let targets = builtin_targets();
        assert!(targets.len() > 100);
        let mut ids = HashSet::new();
        for r in targets {
            validate_target(r).unwrap_or_else(|e| panic!("{e}"));
            assert!(ids.insert(r.id.clone()), "duplicate id {}", r.id);
            assert!(!r.name_tr.is_empty() && !r.description_tr.is_empty());
        }
        let artifacts = builtin_artifacts();
        assert!(artifacts.len() > 30);
        let mut aids = HashSet::new();
        for a in artifacts {
            assert!(aids.insert(a.id.clone()), "duplicate artifact id {}", a.id);
            assert!(!a.dir_name.contains('\\'));
        }
    }

    /// Every rule path, with wildcards replaced by a sample name, must pass the same guard the
    /// cleaner uses. This keeps a rule edit from ever pointing at a protected folder.
    #[cfg(windows)]
    #[test]
    fn builtin_rule_paths_pass_the_guard() {
        let guard = Guard::new(&[], None);
        for r in builtin_targets() {
            for p in &r.paths {
                let sample = p.replace('*', "sample");
                // Steam folders are found at run time; check the default install folder and a
                // library on another drive in their place.
                let concrete: Vec<String> = match sample.split_once('\\') {
                    Some((var, rest))
                        if STEAM_VARS
                            .iter()
                            .any(|v| var.eq_ignore_ascii_case(&format!("%{v}%"))) =>
                    {
                        vec![
                            expand_vars(&format!(
                                r"%SYSTEMDRIVE%\Program Files (x86)\Steam\{rest}"
                            ))
                            .unwrap(),
                            format!(r"D:\SteamLibrary\{rest}"),
                        ]
                    }
                    _ => vec![expand_vars(&sample).unwrap()],
                };
                for c in concrete {
                    let path = PathBuf::from(c);
                    let res = match r.mode {
                        Mode::Contents => guard.check_wipe(&path),
                        Mode::Folder => guard.check(&path, Origin::Rule),
                        Mode::Files => guard.check(&path.join("sample.file"), Origin::Rule),
                        Mode::Report => Ok(()),
                    };
                    assert!(
                        res.is_ok(),
                        "rule {} path {} fails the guard: {:?}",
                        r.id,
                        p,
                        res
                    );
                }
            }
        }
    }

    #[test]
    fn user_rules_cannot_escalate() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("mine.json"),
            r#"[
              {"id":"my-cache","group":"apps","name":"My cache","nameTr":"Önbelleğim","description":"d","descriptionTr":"d",
               "paths":["%LOCALAPPDATA%\\MyApp\\Cache"],"mode":"contents","risk":"safe","admin":false},
              {"id":"evil","group":"apps","name":"x","nameTr":"x","description":"d","descriptionTr":"d",
               "paths":["%WINDIR%\\System32"],"mode":"contents","risk":"safe","admin":true},
              {"id":"broad","group":"apps","name":"x","nameTr":"x","description":"d","descriptionTr":"d",
               "paths":["%LOCALAPPDATA%\\*"],"mode":"contents","risk":"safe","admin":false},
              {"id":"my-steam","group":"games","name":"x","nameTr":"x","description":"d","descriptionTr":"d",
               "paths":["%STEAMLIBRARY%\\steamapps\\temp"],"mode":"contents","risk":"safe","admin":false}
            ]"#,
        )
        .unwrap();
        let (rules, problems) = load_user_targets(tmp.path());
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].id, "my-cache");
        assert!(rules[0].user);
        assert_eq!(problems.len(), 2);
    }

    #[test]
    fn steam_folders_are_for_standard_user_rules_only() {
        let mut rule = builtin_targets()
            .iter()
            .find(|r| r.id == "steam-shadercache")
            .unwrap()
            .clone();
        assert!(rule.paths[0].starts_with("%STEAMLIBRARY%"));
        assert!(validate_target(&rule).is_ok());
        rule.admin = true;
        assert!(validate_target(&rule).is_err());
        rule.admin = false;
        rule.paths = vec!["%STEAMLIBRARYX%\\steamapps".into()];
        assert!(validate_target(&rule).is_err());
    }
}
