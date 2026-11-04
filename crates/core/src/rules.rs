//! Cleanup rules: fixed locations (`rules/targets.json`, `rules/linux/targets.json` on Linux) and
//! project build artifacts (`rules/artifacts.json`, shared). All are embedded at build time.
//! Additional user rules can be dropped into the data folder; they are validated more strictly
//! and can never require administrator rights or run commands.

use std::path::Path;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

#[cfg(not(target_os = "linux"))]
use crate::paths::ALLOWED_VARS;
use crate::paths::STEAM_VARS;
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

#[cfg(not(target_os = "linux"))]
const TARGETS_JSON: &str = include_str!("../../../rules/targets.json");
#[cfg(target_os = "linux")]
const TARGETS_JSON: &str = include_str!("../../../rules/linux/targets.json");
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

/// How a rule file is written on one platform.
#[derive(Clone, Copy)]
struct RuleSyntax {
    vars: &'static [&'static str],
    sep: char,
    /// Whether a path may be a variable alone (`%TEMP%`). On Linux every variable is a shared
    /// root such as `~/.cache`, so a rule has to name a child.
    bare_vars: bool,
    /// Whether `closeApps` holds `.exe` names rather than process names.
    exe_names: bool,
}

#[cfg(not(target_os = "linux"))]
const SYNTAX: RuleSyntax = RuleSyntax {
    vars: ALLOWED_VARS,
    sep: '\\',
    bare_vars: true,
    exe_names: true,
};

// Spelled out here rather than taken from `paths`, so that the Linux catalog can be checked from
// any host: its tests below also run in the Windows CI.
#[cfg(any(target_os = "linux", test))]
const LINUX_SYNTAX: RuleSyntax = RuleSyntax {
    vars: &["HOME", "XDG_CACHE", "XDG_CONFIG", "XDG_DATA", "XDG_STATE"],
    sep: '/',
    bare_vars: false,
    exe_names: false,
};

#[cfg(target_os = "linux")]
const SYNTAX: RuleSyntax = LINUX_SYNTAX;

fn invalid(id: &str, reason: impl Into<String>) -> Error {
    Error::InvalidRule {
        id: id.to_string(),
        reason: reason.into(),
    }
}

/// Structural checks shared by built-in and user rules.
pub fn validate_target(rule: &TargetRule) -> Result<()> {
    validate_with(rule, SYNTAX)
}

fn validate_with(rule: &TargetRule, syntax: RuleSyntax) -> Result<()> {
    let id = &rule.id;
    let sep = syntax.sep;
    let other_sep = if sep == '/' { '\\' } else { '/' };
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
            |v: &&str| upper == format!("%{v}%") || upper.starts_with(&format!("%{v}%{sep}"));
        let steam = STEAM_VARS.iter().any(starts_with);
        if steam && rule.admin {
            return Err(invalid(
                id,
                format!("administrator rules cannot use Steam folders: {p}"),
            ));
        }
        if !steam && !syntax.vars.iter().any(starts_with) {
            return Err(invalid(
                id,
                format!("path must start with an allowed variable: {p}"),
            ));
        }
        if p.contains(other_sep)
            || p.split(sep).any(|s| s == ".." || s == "." || s.is_empty())
            || p.contains("**")
        {
            return Err(invalid(
                id,
                format!("relative, empty or recursive segments are not allowed: {p}"),
            ));
        }
        // A wildcard may not stand for the whole first folder under a root variable, so a rule
        // can never mean "every folder in AppData".
        let segs: Vec<&str> = p.split(sep).collect();
        if (segs.len() >= 2 && segs[1].trim_matches('*').is_empty())
            || (segs.len() < 2 && !syntax.bare_vars)
        {
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
    let is_exe = |a: &String| a.to_ascii_lowercase().ends_with(".exe");
    if syntax.exe_names {
        if !rule.close_apps.iter().all(is_exe) {
            return Err(invalid(id, "closeApps entries must be .exe names"));
        }
    } else if rule
        .close_apps
        .iter()
        .any(|a| a.is_empty() || a.contains(['/', '\\']) || is_exe(a))
    {
        return Err(invalid(id, "closeApps entries must be process names"));
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
    #[cfg(any(windows, target_os = "linux"))]
    use crate::paths::{Guard, Origin, expand_vars};
    use std::collections::HashSet;
    #[cfg(any(windows, target_os = "linux"))]
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

    /// The Linux twin of `builtin_rule_paths_pass_the_guard`. Variables go through the real
    /// `paths` module, so this also catches `LINUX_SYNTAX` drifting away from it.
    // Ignored until the Linux guard and the XDG expansion from the linux-engine session are on
    // main: `expand_vars` refuses %HOME% and the XDG variables without them.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "needs the Linux guard and XDG expansion from the linux-engine branch"]
    fn linux_rule_paths_pass_the_guard() {
        let guard = Guard::new(&[], None);
        for r in builtin_targets() {
            for p in &r.paths {
                let sample = p.replace('*', "sample");
                // Steam folders are found at run time. The first Linux version only deletes
                // inside the home folder, so both samples live there.
                let concrete: Vec<String> = match sample.split_once('/') {
                    Some((var, rest))
                        if STEAM_VARS
                            .iter()
                            .any(|v| var.eq_ignore_ascii_case(&format!("%{v}%"))) =>
                    {
                        vec![
                            expand_vars(&format!("%XDG_DATA%/Steam/{rest}")).unwrap(),
                            expand_vars(&format!("%HOME%/Games/SteamLibrary/{rest}")).unwrap(),
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

    fn linux_catalog() -> Vec<TargetRule> {
        serde_json::from_str(include_str!("../../../rules/linux/targets.json"))
            .expect("rules/linux/targets.json is valid")
    }

    fn linux_rule(paths: &[&str], close_apps: &[&str]) -> TargetRule {
        let mut rule = linux_catalog().remove(0);
        rule.paths = paths.iter().map(|p| p.to_string()).collect();
        rule.close_apps = close_apps.iter().map(|a| a.to_string()).collect();
        rule
    }

    #[test]
    fn linux_catalog_parses_and_validates() {
        let targets = linux_catalog();
        assert!(targets.len() > 100);
        let mut ids = HashSet::new();
        for r in &targets {
            validate_with(r, LINUX_SYNTAX).unwrap_or_else(|e| panic!("{e}"));
            assert!(ids.insert(r.id.clone()), "duplicate id {}", r.id);
            assert!(!r.name_tr.is_empty() && !r.description_tr.is_empty());
        }
        // The code looks these up by id, whatever the platform.
        for id in [
            "steam-shadercache",
            "recycle-bin",
            "npm-cache",
            "pnpm-cache",
        ] {
            assert!(ids.contains(id), "missing {id}");
        }
        let steam = targets.iter().find(|r| r.id == "steam-shadercache");
        assert!(steam.unwrap().paths[0].starts_with("%STEAMLIBRARY%/"));
    }

    #[test]
    fn linux_syntax_refuses_windows_shapes_and_shared_roots() {
        for path in [
            r"%LOCALAPPDATA%\App\Cache",
            r"%XDG_CACHE%\app",
            "%WINDIR%/Temp",
            "%XDG_CACHE%",
            "%HOME%",
            "%STEAM%",
            "%XDG_CACHE%/*",
            "%XDG_CACHE%/../app",
            "%XDG_CACHE%/app/**",
            "/var/cache/apt",
        ] {
            let rule = linux_rule(&[path], &[]);
            assert!(validate_with(&rule, LINUX_SYNTAX).is_err(), "{path}");
        }
        let good = linux_rule(&["%XDG_CACHE%/app"], &["app"]);
        assert!(validate_with(&good, LINUX_SYNTAX).is_ok());
        for app in ["code.exe", "/usr/bin/code", ""] {
            let rule = linux_rule(&["%XDG_CACHE%/app"], &[app]);
            assert!(validate_with(&rule, LINUX_SYNTAX).is_err(), "{app}");
        }
    }

    /// The first Linux version never runs as root: system areas come with a command to copy.
    #[test]
    fn linux_catalog_never_needs_root() {
        for r in linux_catalog() {
            assert!(!r.admin, "{} asks for root", r.id);
            let Some(cmd) = r.command.as_deref() else {
                continue;
            };
            // The first word is how the scan tells whether this distro has the tool at all.
            let first = cmd.split_whitespace().next().unwrap();
            assert!(!["sudo", "pkexec", "su", "rm"].contains(&first), "{cmd}");
            assert!(!cmd.starts_with("api:"), "{cmd}");
            // A placeholder or a pipe would do something else when pasted into a shell.
            assert!(!cmd.contains(['<', '>', '|', ';', '&']), "{cmd}");
            assert!(
                !cmd.contains("--volumes") && !cmd.contains("--delete-data"),
                "{cmd} removes user data"
            );
        }
    }

    #[test]
    fn linux_catalog_stays_out_of_shared_roots_and_precious_data() {
        // Folders that hold other apps' data as a whole; only named children may be targets.
        const SHARED: &[&str] = &[
            "%HOME%/.cache",
            "%HOME%/.config",
            "%HOME%/.local",
            "%HOME%/.local/share",
            "%HOME%/.local/state",
            "%HOME%/.local/bin",
            "%HOME%/.var",
            "%HOME%/.var/app",
            "%HOME%/snap",
            "%XDG_DATA%/Trash",
            "%XDG_CACHE%/huggingface",
        ];
        // Never inside a rule, not even a child. `~/.cache/doc` is Flatpak's document portal.
        const PRECIOUS: &[&str] = &[
            "%HOME%/.ssh",
            "%HOME%/.gnupg",
            "%HOME%/.password-store",
            "%HOME%/.pki",
            "%HOME%/.bitcoin",
            "%HOME%/.mozilla",
            "%HOME%/.thunderbird",
            "%HOME%/.gnome2/keyrings",
            "%XDG_DATA%/keyrings",
            "%XDG_DATA%/kwalletd",
            "%XDG_DATA%/evolution",
            "%XDG_CONFIG%/mozilla",
            "%XDG_CACHE%/doc",
        ];
        const BROWSER_PROFILES: &[&str] = &[
            "%XDG_CONFIG%/google-chrome",
            "%XDG_CONFIG%/chromium",
            "%XDG_CONFIG%/BraveSoftware",
            "%XDG_CONFIG%/microsoft-edge",
            "%XDG_CONFIG%/vivaldi",
        ];
        for r in linux_catalog() {
            for p in &r.paths {
                let inside = |root: &str| p == root || p.starts_with(&format!("{root}/"));
                assert!(!SHARED.iter().any(|s| p == s), "{} targets {p}", r.id);
                assert!(!PRECIOUS.iter().any(|d| inside(d)), "{} targets {p}", r.id);
                if BROWSER_PROFILES.iter().any(|b| p.starts_with(*b)) {
                    assert!(
                        p.ends_with("/*/Service Worker/CacheStorage"),
                        "{} reaches into a browser profile: {p}",
                        r.id
                    );
                }
                if p.contains("/tdata") {
                    assert!(
                        p.ends_with("/tdata/user_data*/cache")
                            || p.ends_with("/tdata/user_data*/media_cache"),
                        "{} reaches into Telegram's session: {p}",
                        r.id
                    );
                }
                assert!(
                    !p.contains("attachments.noindex") && !p.ends_with("Signal/sql"),
                    "{} reaches into Signal's messages: {p}",
                    r.id
                );
                // Inside a sandbox only the cache may go, and the guard refuses the rest. Numbered
                // Snap revisions would also count the same app twice.
                if let Some(rest) = p.strip_prefix("%HOME%/snap/") {
                    let segs: Vec<&str> = rest.split('/').collect();
                    assert!(
                        segs[1..].starts_with(&["common", ".cache"]),
                        "{}: {p}",
                        r.id
                    );
                }
                if let Some(rest) = p.strip_prefix("%HOME%/.var/app/") {
                    let first = rest.split('/').nth(1);
                    assert!(
                        !matches!(first, Some("config" | "data" | ".local")),
                        "{}: {p}",
                        r.id
                    );
                }
                // Prefixes hold save games that may exist nowhere else.
                if ["/compatdata", "/.wine", "/drive_c", "/Prefixes"]
                    .iter()
                    .any(|m| p.contains(m))
                {
                    assert!(
                        r.mode == Mode::Report && r.risk == Risk::Caution,
                        "{}",
                        r.id
                    );
                }
                if p.starts_with("%XDG_DATA%/flatpak") || p.contains("/containers/storage") {
                    assert!(r.mode == Mode::Report, "{}", r.id);
                }
            }
        }
    }

    /// The verified badge has to mean a primary source named the path.
    #[test]
    fn linux_catalog_marks_derived_paths_unverified() {
        for r in linux_catalog() {
            if !r.verified {
                continue;
            }
            assert!(
                r.source
                    .as_deref()
                    .is_some_and(|s| s.starts_with("https://")),
                "{} has no source",
                r.id
            );
            for p in &r.paths {
                assert!(
                    !p.contains("/.var/app/") && !p.contains("/snap/"),
                    "{}: sandbox paths are derived from conventions: {p}",
                    r.id
                );
            }
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn user_rules_cannot_escalate() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("mine.json"),
            r#"[
              {"id":"my-cache","group":"apps","name":"My cache","nameTr":"Önbelleğim","description":"d","descriptionTr":"d",
               "paths":["%XDG_CACHE%/my-app"],"mode":"contents","risk":"safe","admin":false},
              {"id":"evil","group":"apps","name":"x","nameTr":"x","description":"d","descriptionTr":"d",
               "paths":["%XDG_CONFIG%/app"],"mode":"contents","risk":"safe","admin":true},
              {"id":"broad","group":"apps","name":"x","nameTr":"x","description":"d","descriptionTr":"d",
               "paths":["%XDG_CACHE%"],"mode":"contents","risk":"safe","admin":false},
              {"id":"my-steam","group":"games","name":"x","nameTr":"x","description":"d","descriptionTr":"d",
               "paths":["%STEAMLIBRARY%/steamapps/temp"],"mode":"contents","risk":"safe","admin":false}
            ]"#,
        )
        .unwrap();
        let (rules, problems) = load_user_targets(tmp.path());
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].id, "my-cache");
        assert!(rules[0].user);
        assert_eq!(problems.len(), 2);
    }

    #[cfg(not(target_os = "linux"))]
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
        rule.paths = vec![format!("%STEAMLIBRARYX%{}steamapps", SYNTAX.sep)];
        assert!(validate_target(&rule).is_err());
    }
}
