//! Path expansion and the protected-path guard.
//!
//! Every delete in the engine passes through [`Guard`]. The guard is deliberately lexical: it
//! never resolves links (links are refused elsewhere) and compares lowercase keys, which is how
//! Windows compares paths on NTFS by default.

use std::fs;
use std::path::{Path, PathBuf};

use crate::fsmeta::is_link;
use crate::system::known_folders;
use crate::{Error, Result};

/// Environment variables a rule may use. Anything else is rejected so a rule file cannot point
/// at a location derived from an arbitrary variable.
pub const ALLOWED_VARS: &[&str] = &[
    "LOCALAPPDATA",
    "APPDATA",
    "USERPROFILE",
    "TEMP",
    "WINDIR",
    "PROGRAMDATA",
    "SYSTEMDRIVE",
];

pub fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Replaces `%NAME%` with the variable's value. Only [`ALLOWED_VARS`] are accepted.
pub fn expand_vars(pattern: &str) -> Result<String> {
    let mut out = String::with_capacity(pattern.len() + 32);
    let mut rest = pattern;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after
            .find('%')
            .ok_or_else(|| Error::Pattern(format!("unclosed variable in {pattern}")))?;
        let name = &after[..end];
        if !ALLOWED_VARS.iter().any(|v| v.eq_ignore_ascii_case(name)) {
            return Err(Error::Pattern(format!("variable %{name}% is not allowed")));
        }
        let value = std::env::var(name)
            .map_err(|_| Error::Pattern(format!("variable %{name}% is not set")))?;
        if value.is_empty() {
            return Err(Error::Pattern(format!("variable %{name}% is empty")));
        }
        out.push_str(value.trim_end_matches('\\'));
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

pub fn has_wildcard(s: &str) -> bool {
    s.contains('*') || s.contains('?')
}

/// Case-insensitive glob match for a single path segment (`*` and `?`).
pub fn wildcard_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let n: Vec<char> = name.to_lowercase().chars().collect();
    let (mut pi, mut ni) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Splits an expanded absolute path into its drive (`C:`) and segments, refusing relative
/// paths and `.`/`..` segments.
fn split_absolute(path: &str) -> Result<(String, Vec<String>)> {
    let normalized = path.replace('/', "\\");
    let mut parts = normalized.split('\\').filter(|s| !s.is_empty());
    let drive = parts
        .next()
        .ok_or_else(|| Error::Pattern(path.to_string()))?;
    let bytes = drive.as_bytes();
    if bytes.len() != 2 || bytes[1] != b':' || !bytes[0].is_ascii_alphabetic() {
        return Err(Error::Pattern(format!(
            "not an absolute drive path: {path}"
        )));
    }
    let segs: Vec<String> = parts.map(str::to_string).collect();
    if segs.iter().any(|s| s == "." || s == "..") {
        return Err(Error::Pattern(format!(
            "relative segments are not allowed: {path}"
        )));
    }
    Ok((drive.to_string(), segs))
}

/// Expands a rule pattern into the paths that exist right now. Wildcard segments only match
/// real folders (never links), and the final paths themselves must not be links.
pub fn expand_pattern(pattern: &str) -> Result<Vec<PathBuf>> {
    let expanded = expand_vars(pattern)?;
    let (drive, segs) = split_absolute(&expanded)?;
    let mut current = vec![PathBuf::from(format!("{drive}\\"))];
    for (i, seg) in segs.iter().enumerate() {
        let last = i + 1 == segs.len();
        let mut next = Vec::new();
        for base in &current {
            if has_wildcard(seg) {
                let Ok(rd) = fs::read_dir(base) else { continue };
                for entry in rd.flatten() {
                    let Ok(ft) = entry.file_type() else { continue };
                    if is_link(&ft) || (!last && !ft.is_dir()) {
                        continue;
                    }
                    let name = entry.file_name();
                    if wildcard_match(seg, &name.to_string_lossy()) {
                        next.push(base.join(name));
                    }
                }
            } else {
                next.push(base.join(seg));
            }
        }
        current = next;
        if current.is_empty() {
            break;
        }
    }
    current.retain(|p| {
        p.symlink_metadata()
            .map(|m| !is_link(&m.file_type()))
            .unwrap_or(false)
    });
    current.sort();
    current.dedup();
    Ok(current)
}

/// Comparison key: lowercase, backslashes, no verbatim prefix, no trailing separator except on
/// a drive root (`c:\`).
pub fn key(p: &Path) -> String {
    let s = p.to_string_lossy();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    let mut k = s.replace('/', "\\").to_lowercase();
    while k.ends_with('\\') && k.len() > 3 {
        k.pop();
    }
    if k.len() == 2 && k.ends_with(':') {
        k.push('\\');
    }
    k
}

/// True when `child` equals `parent` or lies below it. Both must be keys.
pub fn is_within(child: &str, parent: &str) -> bool {
    if child == parent {
        return true;
    }
    if parent.ends_with('\\') {
        return child.starts_with(parent);
    }
    child.len() > parent.len()
        && child.starts_with(parent)
        && child.as_bytes()[parent.len()] == b'\\'
}

pub fn is_drive_root(k: &str) -> bool {
    k.len() <= 3 && k.as_bytes().get(1) == Some(&b':')
}

/// Who asked for a delete. Rule deletes may reach into Windows-owned cache folders such as
/// `%WINDIR%\Temp`; deletes the user picks by hand may not touch Windows or program folders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Rule,
    User,
    /// A folder an uninstalled app left behind, as found by [`crate::leftovers`]. The same
    /// limits as `User`, except that an app's own folder one or two levels below Program Files
    /// or ProgramData (`<root>\App`, `<root>\Publisher\App`) may be removed.
    Leftover,
}

/// Folders below Program Files and ProgramData that Windows or many apps share. They are never
/// treated as an app's leftover, and nothing inside them is either.
pub const SHARED_APP_FOLDERS: &[&str] = &[
    "application data",
    "common files",
    "desktop",
    "documents",
    "dotnet",
    "favorites",
    "internet explorer",
    "microsoft",
    "microsoft.net",
    "modifiablewindowsapps",
    "msbuild",
    "package cache",
    "packages",
    "reference assemblies",
    "regid.1991-06.com.microsoft",
    "softwaredistribution",
    "ssh",
    "start menu",
    "templates",
    "uninstall information",
    "usoprivate",
    "usoshared",
    "windows",
    "windows defender",
    "windows defender advanced threat protection",
    "windows mail",
    "windows media player",
    "windows multimedia platform",
    "windows nt",
    "windows photo viewer",
    "windows portable devices",
    "windows security",
    "windows sidebar",
    "windowsapps",
    "windowspowershell",
];

#[derive(Debug, Clone, Default)]
pub struct Guard {
    /// Neither these folders nor their contents as a whole may be removed (profile, libraries,
    /// Windows, program folders). Files inside may still be picked one by one by the user.
    no_wipe: Vec<String>,
    /// These folders may never be removed themselves (a superset of `no_wipe`).
    no_delete: Vec<String>,
    /// Nothing at or below these folders may be removed.
    subtree: Vec<String>,
    /// Additionally off limits for hand-picked deletes.
    user_subtree: Vec<String>,
    /// Program Files and ProgramData: app folders right below them may be removed as leftovers.
    app_roots: Vec<String>,
    exclusions: Vec<String>,
}

/// Folder names that are never deleted wherever they appear.
const PROTECTED_NAMES: &[&str] = &[
    ".git",
    "system volume information",
    "$recycle.bin",
    "$windows.~bt",
    "$windows.~ws",
    "windows.old",
];

impl Guard {
    /// Builds the guard for the current user. `exclusions` are user-chosen paths; `app_data` is
    /// this app's own data folder.
    pub fn new(exclusions: &[String], app_data: Option<&Path>) -> Guard {
        let mut g = Guard::default();
        let profile = env_path("USERPROFILE");
        let windir = env_path("WINDIR").unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let sysdrive = env_path("SYSTEMDRIVE").unwrap_or_else(|| PathBuf::from("C:"));
        let sysroot = PathBuf::from(format!("{}\\", sysdrive.to_string_lossy()));

        let mut no_wipe: Vec<PathBuf> = vec![sysroot.join("Users"), windir.clone()];
        for var in [
            "PROGRAMFILES",
            "PROGRAMFILES(X86)",
            "ProgramW6432",
            "PROGRAMDATA",
            "PUBLIC",
            "OneDrive",
            "OneDriveConsumer",
            "OneDriveCommercial",
            "APPDATA",
            "LOCALAPPDATA",
        ] {
            if let Some(p) = env_path(var) {
                no_wipe.push(p);
            }
        }
        if let Some(profile) = &profile {
            no_wipe.push(profile.clone());
            for sub in [
                "AppData",
                r"AppData\LocalLow",
                "Desktop",
                "Documents",
                "Downloads",
                "Pictures",
                "Music",
                "Videos",
                "Favorites",
                "Contacts",
                "Links",
                "Saved Games",
                "Searches",
                "OneDrive",
                ".ssh",
                ".gnupg",
            ] {
                no_wipe.push(profile.join(sub));
            }
        }
        if let Some(local) = env_path("LOCALAPPDATA") {
            no_wipe.push(local.join("Packages"));
            no_wipe.push(local.join("Programs"));
            no_wipe.push(local.join("Microsoft"));
        }
        if let Some(roaming) = env_path("APPDATA") {
            no_wipe.push(roaming.join("Microsoft"));
        }
        no_wipe.extend(known_folders());

        let mut no_delete = no_wipe.clone();
        if let Some(t) = env_path("TEMP") {
            no_delete.push(t);
        }
        no_delete.push(windir.join("Temp"));

        let subtree: Vec<PathBuf> = vec![
            windir.join("System32"),
            windir.join("SysWOW64"),
            windir.join("WinSxS"),
            windir.join("Installer"),
            windir.join("servicing"),
            windir.join("Boot"),
            windir.join("Fonts"),
            windir.join("assembly"),
            windir.join("SystemApps"),
            windir.join("SystemResources"),
            windir.join("Microsoft.NET"),
            sysroot.join("Recovery"),
            sysroot.join("Boot"),
            sysroot.join("EFI"),
        ];
        let app_roots: Vec<PathBuf> = [
            "PROGRAMFILES",
            "PROGRAMFILES(X86)",
            "ProgramW6432",
            "PROGRAMDATA",
        ]
        .iter()
        .filter_map(|v| env_path(v))
        .collect();
        let mut user_subtree = vec![windir.clone()];
        user_subtree.extend(app_roots.iter().cloned());

        g.no_wipe = dedup_keys(no_wipe);
        g.no_delete = dedup_keys(no_delete);
        g.subtree = dedup_keys(subtree);
        if let Some(own) = app_data {
            g.subtree.push(key(own));
        }
        g.user_subtree = dedup_keys(user_subtree);
        g.app_roots = dedup_keys(app_roots);
        g.app_roots.retain(|r| !is_drive_root(r));
        g.exclusions = exclusions
            .iter()
            .filter(|e| !e.trim().is_empty())
            .map(|e| key(Path::new(e.trim())))
            .collect();
        g
    }

    /// May `path` itself be removed?
    pub fn check(&self, path: &Path, origin: Origin) -> Result<()> {
        let k = key(path);
        let deny = |why: &str| Err(Error::Protected(format!("{} ({why})", path.display())));
        if !path.is_absolute() || k.split('\\').any(|s| s == ".." || s == ".") {
            return deny("not absolute");
        }
        if is_drive_root(&k) {
            return deny("drive root");
        }
        if k.split('\\').any(|seg| PROTECTED_NAMES.contains(&seg)) {
            return deny("protected folder");
        }
        // Removing a folder removes everything protected below it, so ancestors count too.
        if self.no_delete.iter().any(|p| is_within(p, &k)) {
            return deny("system or user folder");
        }
        if self
            .subtree
            .iter()
            .any(|p| is_within(&k, p) || is_within(p, &k))
        {
            return deny("system folder");
        }
        if origin != Origin::Rule
            && self.user_subtree.iter().any(|p| is_within(&k, p))
            && !(origin == Origin::Leftover && self.is_app_folder(&k))
        {
            return deny("Windows or program folder");
        }
        if self.is_excluded(&k) {
            return deny("excluded in settings");
        }
        Ok(())
    }

    /// True for `<root>\App` and `<root>\Publisher\App` below Program Files or ProgramData when
    /// no part of it is a folder Windows or many apps share. `k` must be a key.
    fn is_app_folder(&self, k: &str) -> bool {
        self.app_roots.iter().any(|root| {
            let Some(rest) = k
                .strip_prefix(root.as_str())
                .and_then(|r| r.strip_prefix('\\'))
            else {
                return false;
            };
            let segs: Vec<&str> = rest.split('\\').collect();
            (1..=2).contains(&segs.len())
                && segs
                    .iter()
                    .all(|s| !s.is_empty() && !SHARED_APP_FOLDERS.contains(s))
        })
    }

    /// Treats `root` like Program Files, so tests can exercise [`Origin::Leftover`] in a temp
    /// folder.
    #[cfg(test)]
    pub(crate) fn with_app_root(mut self, root: &Path) -> Guard {
        let k = key(root);
        self.no_wipe.push(k.clone());
        self.no_delete.push(k.clone());
        self.user_subtree.push(k.clone());
        self.app_roots.push(k);
        self
    }

    /// May the contents of `dir` be removed as a whole (rule `contents` mode)?
    pub fn check_wipe(&self, dir: &Path) -> Result<()> {
        let k = key(dir);
        let deny = |why: &str| Err(Error::Protected(format!("{} ({why})", dir.display())));
        if !dir.is_absolute() || is_drive_root(&k) {
            return deny("drive root");
        }
        if k.split('\\').any(|seg| PROTECTED_NAMES.contains(&seg)) {
            return deny("protected folder");
        }
        if self.no_wipe.iter().any(|p| is_within(p, &k)) {
            return deny("system or user folder");
        }
        if self
            .subtree
            .iter()
            .any(|p| is_within(&k, p) || is_within(p, &k))
        {
            return deny("system folder");
        }
        if self.exclusions.iter().any(|e| is_within(&k, e)) {
            return deny("excluded in settings");
        }
        Ok(())
    }

    pub fn is_excluded(&self, k: &str) -> bool {
        self.exclusions.iter().any(|e| is_within(k, e))
    }

    /// Exclusions that sit inside `dir`; only these need a per-file check while wiping it.
    pub fn exclusions_inside(&self, dir: &Path) -> Vec<String> {
        let k = key(dir);
        self.exclusions
            .iter()
            .filter(|e| is_within(e, &k) && **e != k)
            .cloned()
            .collect()
    }
}

fn dedup_keys(paths: Vec<PathBuf>) -> Vec<String> {
    let mut keys: Vec<String> = paths.iter().map(|p| key(p)).collect();
    keys.sort();
    keys.dedup();
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_matching() {
        assert!(wildcard_match("*", "Default"));
        assert!(wildcard_match("thumbcache_*.db", "thumbcache_1024.db"));
        assert!(wildcard_match("THUMBCACHE_*.DB", "thumbcache_idx.db"));
        assert!(wildcard_match("webcache*", "webcache_4430"));
        assert!(wildcard_match("FOUND.*", "found.000"));
        assert!(wildcard_match("cmake-build-*", "cmake-build-debug"));
        assert!(wildcard_match("a?c", "abc"));
        assert!(!wildcard_match("*.dmp", "memory.dmp.txt"));
        assert!(!wildcard_match("cache", "cache2"));
    }

    #[test]
    fn keys_and_containment() {
        assert_eq!(key(Path::new(r"C:\Users\A\")), r"c:\users\a");
        assert_eq!(key(Path::new("C:")), r"c:\");
        assert_eq!(key(Path::new(r"\\?\C:\X")), r"c:\x");
        assert!(is_within(r"c:\users\a\b", r"c:\users\a"));
        assert!(!is_within(r"c:\users\ab", r"c:\users\a"));
        assert!(is_within(r"c:\x", r"c:\"));
        assert!(is_drive_root(r"d:\"));
    }

    #[test]
    fn expand_vars_rejects_unknown() {
        assert!(expand_vars("%PATH%\\x").is_err());
        assert!(expand_vars("%TEMP").is_err());
        let t = expand_vars("%TEMP%\\x").unwrap();
        assert!(t.ends_with("\\x"));
    }

    #[test]
    fn split_refuses_relative() {
        assert!(split_absolute(r"C:\a\..\b").is_err());
        assert!(split_absolute(r"relative\path").is_err());
        assert!(split_absolute(r"C:\a\b").is_ok());
    }

    #[test]
    fn guard_blocks_system_and_user_roots() {
        let g = Guard::new(&[r"D:\Keep".to_string()], None);
        let windir = env_path("WINDIR").unwrap();
        let profile = env_path("USERPROFILE").unwrap();
        assert!(g.check(Path::new(r"C:\"), Origin::Rule).is_err());
        assert!(g.check(&windir, Origin::Rule).is_err());
        assert!(
            g.check(&windir.join(r"System32\drivers"), Origin::Rule)
                .is_err()
        );
        assert!(g.check(&windir.join(r"Temp\x.tmp"), Origin::Rule).is_ok());
        assert!(g.check(&windir.join(r"Temp\x.tmp"), Origin::User).is_err());
        assert!(g.check(&profile, Origin::User).is_err());
        assert!(g.check(&profile.join("Documents"), Origin::User).is_err());
        assert!(
            g.check(&profile.join(r"Documents\big.iso"), Origin::User)
                .is_ok()
        );
        assert!(g.check(Path::new(r"C:\Users"), Origin::User).is_err());
        assert!(
            g.check(&profile.join(r"code\app\.git"), Origin::User)
                .is_err()
        );
        assert!(
            g.check(Path::new(r"D:\Keep\file.bin"), Origin::User)
                .is_err()
        );
        assert!(
            g.check(Path::new(r"D:\Other\file.bin"), Origin::User)
                .is_ok()
        );
        assert!(g.check_wipe(&profile.join("Downloads")).is_err());
        assert!(g.check_wipe(&env_path("TEMP").unwrap()).is_ok());
        assert!(g.check(&env_path("TEMP").unwrap(), Origin::Rule).is_err());
    }

    #[test]
    fn leftover_origin_opens_only_app_folders() {
        let pf = env_path("PROGRAMFILES").unwrap();
        let pd = env_path("PROGRAMDATA").unwrap();
        let g = Guard::new(&[pf.join("Kept App").to_string_lossy().into_owned()], None);
        let ok = |p: &Path| g.check(p, Origin::Leftover).is_ok();

        for app in [pf.join("Acme"), pf.join(r"Acme\Writer"), pd.join("Acme")] {
            assert!(ok(&app), "{}", app.display());
            assert!(g.check(&app, Origin::User).is_err(), "{}", app.display());
        }
        assert!(!ok(&pf), "the root itself");
        assert!(!ok(&pd));
        assert!(
            !ok(&pf.join(r"Acme\Writer\bin")),
            "deeper than Publisher\\App"
        );
        assert!(!ok(&pf.join("Common Files")));
        assert!(
            !ok(&pf.join(r"Common Files\Acme")),
            "inside a shared folder"
        );
        assert!(!ok(&pf.join("WindowsApps")));
        assert!(!ok(&pd.join("Microsoft")));
        assert!(!ok(&pd.join(r"Microsoft\Windows")));
        assert!(!ok(&pf.join(r"Acme\.git")));
        assert!(!ok(&pf.join("Kept App")), "user exclusions still apply");
        // Everything else behaves exactly like a hand-picked delete.
        let windir = env_path("WINDIR").unwrap();
        let profile = env_path("USERPROFILE").unwrap();
        assert!(!ok(&windir.join("Acme")));
        assert!(!ok(&windir.join(r"System32\Acme")));
        assert!(!ok(&profile.join("Documents")));
        assert!(!ok(Path::new(r"C:\")));
        assert!(ok(&env_path("APPDATA").unwrap().join("Acme")));
        assert!(!ok(&env_path("APPDATA").unwrap().join("Microsoft")));
        assert!(ok(Path::new(r"D:\Apps\Acme")));
    }
}
