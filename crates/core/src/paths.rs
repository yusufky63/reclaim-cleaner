//! Path expansion and the protected-path guard.
//!
//! Every delete in the engine passes through [`Guard`]. On Windows the guard is deliberately
//! lexical: it never resolves links (links are refused elsewhere) and compares lowercase keys,
//! which is how Windows compares paths on NTFS by default.
//!
//! On Linux paths are case-sensitive and need not be UTF-8, so keys keep every byte. The guard
//! also resolves a path's parent, because any folder on the way can be a link to somewhere else.
//! The last name is left alone: a link is checked itself, not its target.

use std::fs;
use std::path::{Path, PathBuf};

use crate::fsmeta::is_link;
#[cfg(not(target_os = "linux"))]
use crate::system::known_folders;
use crate::{Error, Result};

/// The separator in path keys and rule patterns.
#[cfg(windows)]
pub const SEP: char = '\\';
#[cfg(not(windows))]
pub const SEP: char = '/';

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

/// Variables a Linux rule may use. They name folders rather than environment variables: each one
/// is `$HOME` or an XDG base directory, with the spec's default when the variable is unset, empty
/// or relative. These are the only names `expand_pattern` accepts on Linux.
pub const LINUX_VARS: &[&str] = &["HOME", "XDG_CACHE", "XDG_CONFIG", "XDG_DATA", "XDG_STATE"];

/// Rule variables that stand for folders found at run time rather than environment variables:
/// `%STEAM%` is the Steam install folder and `%STEAMLIBRARY%` every Steam library folder on any
/// drive (the install folder included). Both come from Steam's registry key (its folders on
/// Linux) and `libraryfolders.vdf`, so they expand to nothing when Steam is not installed. Rules
/// that need administrator rights may not use them: the elevated helper must not take folders
/// from per-user data.
pub const STEAM_VARS: &[&str] = &["STEAM", "STEAMLIBRARY"];

/// Splits `%STEAM%\rest` or `%STEAMLIBRARY%\rest` into its folders and `\rest`, with `/` on
/// Linux.
fn steam_roots(pattern: &str) -> Option<(Vec<PathBuf>, &str)> {
    let upper = pattern.to_ascii_uppercase();
    for var in STEAM_VARS {
        let token = format!("%{var}%");
        if upper.starts_with(&token) {
            let rest = &pattern[token.len()..];
            if !(rest.is_empty() || rest.starts_with(SEP)) {
                return None;
            }
            let installs = crate::steam::installs();
            let roots = if *var == "STEAM" {
                installs
            } else {
                installs
                    .iter()
                    .flat_map(|root| crate::steam::libraries_in(root))
                    .collect()
            };
            return Some((roots, rest));
        }
    }
    None
}

pub fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Splits `%NAME%/rest` into the folder `NAME` stands for and `/rest`. `None` when the pattern
/// does not start with a variable.
#[cfg(target_os = "linux")]
fn linux_root<'a>(
    pattern: &'a str,
    xdg: Option<&crate::xdg::Xdg>,
) -> Result<Option<(PathBuf, &'a str)>> {
    let Some(after) = pattern.strip_prefix('%') else {
        return Ok(None);
    };
    let (name, rest) = after
        .split_once('%')
        .ok_or_else(|| Error::Pattern(format!("unclosed variable in {pattern}")))?;
    if !(rest.is_empty() || rest.starts_with(SEP)) {
        return Err(Error::Pattern(format!(
            "variable not followed by /: {pattern}"
        )));
    }
    if !LINUX_VARS.iter().any(|v| v.eq_ignore_ascii_case(name)) {
        return Err(Error::Pattern(format!("variable %{name}% is not allowed")));
    }
    let xdg = xdg.ok_or_else(|| Error::Pattern(format!("variable %{name}% is not set")))?;
    let root = match name.to_ascii_uppercase().as_str() {
        "HOME" => &xdg.home,
        "XDG_CACHE" => &xdg.cache,
        "XDG_CONFIG" => &xdg.config,
        "XDG_DATA" => &xdg.data,
        _ => &xdg.state,
    };
    Ok(Some((root.clone(), rest)))
}

/// Replaces a leading `%NAME%` with its folder. Only [`LINUX_VARS`] are accepted. A folder that
/// is not UTF-8 is an error here, because the result is a string; [`expand_pattern`] keeps such
/// folders working by never going through a string.
#[cfg(target_os = "linux")]
pub fn expand_vars(pattern: &str) -> Result<String> {
    expand_vars_with(pattern, crate::xdg::Xdg::current().as_ref())
}

#[cfg(target_os = "linux")]
fn expand_vars_with(pattern: &str, xdg: Option<&crate::xdg::Xdg>) -> Result<String> {
    let Some((root, rest)) = linux_root(pattern, xdg)? else {
        return Ok(pattern.to_string());
    };
    let root = root
        .to_str()
        .ok_or_else(|| Error::Pattern(format!("folder is not UTF-8 in {pattern}")))?;
    Ok(format!("{root}{rest}"))
}

/// Replaces `%NAME%` with the variable's value. Only [`ALLOWED_VARS`] are accepted.
#[cfg(not(target_os = "linux"))]
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

/// How names compare: without case on Windows, exactly elsewhere.
#[cfg(windows)]
fn fold_case(s: &str) -> std::borrow::Cow<'_, str> {
    s.to_lowercase().into()
}

#[cfg(not(windows))]
fn fold_case(s: &str) -> std::borrow::Cow<'_, str> {
    s.into()
}

/// Glob match for a single path segment (`*` and `?`). Case-insensitive on Windows and exact
/// elsewhere, the way each system compares names.
pub fn wildcard_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = fold_case(pattern).chars().collect();
    let n: Vec<char> = fold_case(name).chars().collect();
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
#[cfg(windows)]
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
#[cfg(windows)]
pub fn expand_pattern(pattern: &str) -> Result<Vec<PathBuf>> {
    if let Some((roots, rest)) = steam_roots(pattern) {
        let mut out = Vec::new();
        for root in roots {
            let full = format!("{}{rest}", root.to_string_lossy().trim_end_matches('\\'));
            out.extend(expand_absolute(&full)?);
        }
        out.sort();
        out.dedup();
        return Ok(out);
    }
    expand_absolute(&expand_vars(pattern)?)
}

/// Expands a rule pattern into the paths that exist right now. A variable's folder is used as it
/// is, so `%` or `*` in the name of a home folder means nothing and any bytes survive. Anything
/// without a leading variable is an absolute path whose `%` is an ordinary character. Wildcard
/// segments only match real folders (never links), and the final paths themselves must not be
/// links.
#[cfg(not(windows))]
pub fn expand_pattern(pattern: &str) -> Result<Vec<PathBuf>> {
    if let Some((roots, rest)) = steam_roots(pattern) {
        let mut out = Vec::new();
        for root in roots {
            out.extend(expand_below(root, rest)?);
        }
        out.sort();
        out.dedup();
        return Ok(out);
    }
    #[cfg(target_os = "linux")]
    if let Some((root, rest)) = linux_root(pattern, crate::xdg::Xdg::current().as_ref())? {
        return expand_below(root, rest);
    }
    if !pattern.starts_with(SEP) {
        return Err(Error::Pattern(format!("not an absolute path: {pattern}")));
    }
    expand_below(PathBuf::from("/"), pattern)
}

/// [`expand_pattern`] for a path whose variables are already replaced.
#[cfg(windows)]
fn expand_absolute(expanded: &str) -> Result<Vec<PathBuf>> {
    let (drive, segs) = split_absolute(expanded)?;
    Ok(expand_segments(PathBuf::from(format!("{drive}\\")), &segs))
}

/// The paths below `root` that `rest` (`/a/*/b`) names, refusing `.` and `..` segments.
#[cfg(not(windows))]
fn expand_below(root: PathBuf, rest: &str) -> Result<Vec<PathBuf>> {
    let segs: Vec<String> = rest
        .split(SEP)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    if segs.iter().any(|s| s == "." || s == "..") {
        return Err(Error::Pattern(format!(
            "relative segments are not allowed: {rest}"
        )));
    }
    Ok(expand_segments(root, &segs))
}

fn expand_segments(root: PathBuf, segs: &[String]) -> Vec<PathBuf> {
    let mut current = vec![root];
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
    current
}

/// Comparison key: lowercase, backslashes, no verbatim prefix, no trailing separator except on
/// a drive root (`c:\`).
#[cfg(windows)]
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

/// First of the 256 characters that stand for a single byte in a key. It sits at the very end of
/// Unicode, in a private use plane.
#[cfg(not(windows))]
const BYTE_BASE: u32 = 0x10_FF00;

/// Every byte of a Unix name, kept in a string. Valid UTF-8 stays as it is. A byte that is not
/// valid UTF-8 becomes a character of its own, and so do the bytes of a name that really uses
/// those characters, which keeps two different names from ever sharing a key.
#[cfg(not(windows))]
fn encode_bytes(bytes: &[u8]) -> String {
    let byte_char = |b: u8| char::from_u32(BYTE_BASE + u32::from(b)).unwrap_or('\u{FFFD}');
    let mut out = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        for c in chunk.valid().chars() {
            if u32::from(c) >= BYTE_BASE {
                out.extend(c.encode_utf8(&mut [0; 4]).bytes().map(byte_char));
            } else {
                out.push(c);
            }
        }
        out.extend(chunk.invalid().iter().copied().map(byte_char));
    }
    out
}

/// Comparison key: every byte of the path, compared exactly, with `/` separators. Repeated and
/// trailing slashes go, so `/a//b/` cannot slip past an entry for `/a/b`; `.` and `..` stay, so
/// the guard can see and refuse them.
#[cfg(not(windows))]
pub fn key(p: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let encoded = encode_bytes(p.as_os_str().as_bytes());
    let mut k = String::with_capacity(encoded.len());
    for c in encoded.chars() {
        if !(c == SEP && k.ends_with(SEP)) {
            k.push(c);
        }
    }
    if k.len() > 1 && k.ends_with(SEP) {
        k.pop();
    }
    k
}

/// The key of a single name, in the form [`key`] gives it inside a path.
#[cfg(windows)]
pub fn name_key(name: &str) -> String {
    name.to_lowercase()
}

/// The key of a single name, in the form [`key`] gives it inside a path.
#[cfg(not(windows))]
pub fn name_key(name: &str) -> String {
    encode_bytes(name.as_bytes())
}

/// True when `child` equals `parent` or lies below it. Both must be keys.
pub fn is_within(child: &str, parent: &str) -> bool {
    if child == parent {
        return true;
    }
    if parent.ends_with(SEP) {
        return child.starts_with(parent);
    }
    child.len() > parent.len()
        && child.starts_with(parent)
        && child.as_bytes()[parent.len()] == SEP as u8
}

#[cfg(windows)]
pub fn is_drive_root(k: &str) -> bool {
    k.len() <= 3 && k.as_bytes().get(1) == Some(&b':')
}

/// A Unix system has one root.
#[cfg(not(windows))]
pub fn is_drive_root(k: &str) -> bool {
    k == "/"
}

/// `path` with its parent's real location and its own name: the link itself, not its target. A
/// parent that does not exist yet is resolved as far as it exists. `None` for a path without a
/// name, such as `/`.
#[cfg(target_os = "linux")]
pub(crate) fn resolve_parent(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?;
    let parent = path.parent()?;
    let mut missing = Vec::new();
    let mut cur = parent;
    let mut real = loop {
        match fs::canonicalize(cur) {
            Ok(real) => break real,
            Err(_) => {
                missing.push(cur.file_name()?);
                cur = cur.parent()?;
            }
        }
    };
    real.extend(missing.into_iter().rev());
    real.push(name);
    Some(real)
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
    #[cfg(not(target_os = "linux"))]
    user_subtree: Vec<String>,
    /// Program Files and ProgramData: app folders right below them may be removed as leftovers.
    #[cfg(not(target_os = "linux"))]
    app_roots: Vec<String>,
    exclusions: Vec<String>,
    /// Keys, passwords, mail and wallets. Nothing at or below these folders may be removed.
    #[cfg(target_os = "linux")]
    precious: Vec<String>,
    /// Browser profile roots, each with the folders between it and a profile: precious, except
    /// for each profile's Service Worker cache.
    #[cfg(target_os = "linux")]
    browser_roots: Vec<(String, usize)>,
    /// The home folder, under each of its spellings. Nothing outside it may be removed.
    #[cfg(target_os = "linux")]
    home: Vec<String>,
    /// Set when no delete can be trusted at all. A guard that was never built refuses too,
    /// because its `home` is empty.
    #[cfg(target_os = "linux")]
    refuse_all: Option<&'static str>,
}

/// Folder names that are never deleted wherever they appear.
#[cfg(not(target_os = "linux"))]
const PROTECTED_NAMES: &[&str] = &[
    ".git",
    "system volume information",
    "$recycle.bin",
    "$windows.~bt",
    "$windows.~ws",
    "windows.old",
];

/// Folder names that are never deleted wherever they appear. `.Trash-$uid` is matched by
/// [`is_protected_name`] as well: only an "empty trash" action may touch a trash folder.
#[cfg(target_os = "linux")]
const PROTECTED_NAMES: &[&str] = &[".git", "lost+found", ".Trash"];

#[cfg(target_os = "linux")]
pub(crate) fn is_protected_name(seg: &str) -> bool {
    PROTECTED_NAMES.contains(&seg) || seg.starts_with(".Trash-")
}

/// Nothing inside these may be deleted: kernels and bootloaders, files the package manager owns,
/// kernel interfaces, package databases and read-only system trees.
#[cfg(target_os = "linux")]
const LINUX_SUBTREE: &[&str] = &[
    "/boot",
    "/efi",
    "/boot/efi",
    "/usr",
    "/bin",
    "/sbin",
    "/lib",
    "/lib32",
    "/lib64",
    "/libx32",
    "/etc",
    "/proc",
    "/sys",
    "/dev",
    "/run",
    "/var/lib",
    "/snap",
    "/nix/store",
    "/sysroot",
    "/ostree",
];

/// May not be removed or emptied as a whole; rules may target named children.
#[cfg(target_os = "linux")]
const LINUX_NO_WIPE: &[&str] = &[
    "/",
    "/home",
    "/root",
    "/opt",
    "/srv",
    "/usr/local",
    "/var",
    "/var/cache",
    "/var/log",
    "/var/tmp",
    "/tmp",
    "/var/spool",
    "/var/mail",
    "/var/crash",
    "/media",
    "/mnt",
];

/// Below the home folder, under the same rule as [`LINUX_NO_WIPE`].
#[cfg(target_os = "linux")]
const HOME_NO_WIPE: &[&str] = &[
    ".config",
    ".local",
    ".local/share",
    ".local/state",
    ".local/bin",
    ".cache",
    ".var",
    ".var/app",
    "snap",
    ".local/share/flatpak",
];

/// Common names of the user's folders. `user-dirs.dirs` is what counts, but it can be missing on
/// a minimal desktop, and then these are still the user's own folders.
#[cfg(target_os = "linux")]
const HOME_USER_FOLDERS: &[&str] = &[
    "Desktop",
    "Downloads",
    "Templates",
    "Public",
    "Documents",
    "Music",
    "Pictures",
    "Videos",
];

/// Below the home folder: keys and passwords, mail and wallets. Never inside a rule.
#[cfg(target_os = "linux")]
const HOME_PRECIOUS: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".password-store",
    ".local/share/keyrings",
    ".gnome2/keyrings",
    ".local/share/kwalletd",
    ".pki",
    ".mozilla",
    ".config/mozilla",
    ".thunderbird",
    ".local/share/evolution",
    ".bitcoin",
    ".electrum",
    ".ethereum",
];

/// Browser profiles below the config folder. These browsers keep their HTTP cache in `~/.cache`,
/// so the profile can be protected whole.
#[cfg(target_os = "linux")]
const BROWSER_PROFILES: &[&str] = &[
    "google-chrome",
    "google-chrome-beta",
    "google-chrome-unstable",
    "chromium",
    "microsoft-edge",
    "vivaldi",
];

/// Brave adds a folder for its channel: `BraveSoftware/Brave-Browser/<profile>`.
#[cfg(target_os = "linux")]
const BROWSER_PROFILES_IN_CHANNELS: &[&str] = &["BraveSoftware"];

/// What the Linux guard is built from, so tests can build one without touching the environment.
#[cfg(target_os = "linux")]
struct LinuxFacts {
    xdg: Option<crate::xdg::Xdg>,
    mounts: Option<crate::mounts::MountTable>,
    as_root: bool,
    password_store: Option<PathBuf>,
}

/// Every spelling under which `path` can reach the guard: as written, with its parent resolved,
/// and fully resolved when it is a link itself.
#[cfg(target_os = "linux")]
fn path_forms(path: &Path) -> Vec<String> {
    let mut forms = vec![key(path)];
    forms.extend(resolve_parent(path).map(|p| key(&p)));
    forms.extend(fs::canonicalize(path).ok().map(|p| key(&p)));
    forms.sort();
    forms.dedup();
    forms
}

#[cfg(target_os = "linux")]
fn forms_of(paths: Vec<PathBuf>) -> Vec<String> {
    let mut keys: Vec<String> = paths.iter().flat_map(|p| path_forms(p)).collect();
    keys.sort();
    keys.dedup();
    keys
}

/// Inside a sandboxed app's folder, only its cache may go. Flatpak keeps an app's XDG folders in
/// `~/.var/app/<id>/{cache,config,data}`. A strict Snap has `~/snap/<name>/<revision>` as its
/// home, so there only `.cache` is a cache. `home` must be a key.
#[cfg(target_os = "linux")]
fn is_sandbox_data(k: &str, home: &str) -> bool {
    let Some(rest) = k.strip_prefix(home).and_then(|r| r.strip_prefix(SEP)) else {
        return false;
    };
    let segs: Vec<&str> = rest.split(SEP).collect();
    match segs.as_slice() {
        [".var", "app", _] | ["snap", _] | ["snap", _, _] => true,
        [".var", "app", _, "config" | "data", ..] => true,
        [".var", "app", _, ".local"] | [".var", "app", _, ".local", "state", ..] => true,
        ["snap", _, _, first, ..] => *first != ".cache",
        _ => false,
    }
}

#[cfg(target_os = "linux")]
impl Guard {
    /// Builds the guard for the current user. `exclusions` are user-chosen paths; `app_data` is
    /// this app's own data folder.
    pub fn new(exclusions: &[String], app_data: Option<&Path>) -> Guard {
        let facts = LinuxFacts {
            xdg: crate::xdg::Xdg::current(),
            mounts: crate::mounts::MountTable::read_current().ok(),
            as_root: crate::system::is_elevated(),
            password_store: env_path("PASSWORD_STORE_DIR"),
        };
        Guard::from_facts(&facts, exclusions, app_data)
    }

    /// A guard for a user whose home folder is `home`, with this machine's real mounts. Tests run
    /// under any user, root included, so this one never refuses for that reason.
    #[cfg(test)]
    pub(crate) fn for_home(home: &Path, exclusions: &[String], app_data: Option<&Path>) -> Guard {
        Guard::from_facts(&LinuxFacts::for_home(home), exclusions, app_data)
    }

    fn from_facts(facts: &LinuxFacts, exclusions: &[String], app_data: Option<&Path>) -> Guard {
        let mut g = Guard {
            exclusions: forms_of(
                exclusions
                    .iter()
                    .filter(|e| !e.trim().is_empty())
                    .map(|e| PathBuf::from(e.trim()))
                    .collect(),
            ),
            ..Guard::default()
        };
        // Refusing is the only safe answer without these: as root every path looks writable, a
        // missing home leaves nothing to stay inside, and without the mount table a delete could
        // not tell another disk from a folder.
        g.refuse_all = if facts.as_root {
            Some("running as root")
        } else if facts.xdg.is_none() {
            Some("no usable home folder")
        } else if facts.mounts.is_none() {
            Some("mount table not readable")
        } else {
            None
        };
        let (Some(xdg), Some(mounts)) = (&facts.xdg, &facts.mounts) else {
            return g;
        };
        let home = &xdg.home;
        let in_home =
            |list: &[&str]| -> Vec<PathBuf> { list.iter().map(|s| home.join(s)).collect() };

        let mut subtree: Vec<PathBuf> = LINUX_SUBTREE.iter().map(PathBuf::from).collect();
        // Flatpak's document portal is a FUSE mount and is never entered.
        subtree.push(home.join(".cache/doc"));
        subtree.push(xdg.cache.join("doc"));
        // Only an "empty trash" action may remove what is in the trash.
        subtree.push(home.join(".local/share/Trash"));
        subtree.push(xdg.data.join("Trash"));
        subtree.extend(app_data.map(Path::to_path_buf));

        let mut no_wipe: Vec<PathBuf> = LINUX_NO_WIPE.iter().map(PathBuf::from).collect();
        no_wipe.push(home.clone());
        no_wipe.extend(in_home(HOME_NO_WIPE));
        no_wipe.extend(in_home(HOME_USER_FOLDERS));
        no_wipe.extend([
            xdg.cache.clone(),
            xdg.config.clone(),
            xdg.data.clone(),
            xdg.state.clone(),
        ]);
        no_wipe.extend(xdg.runtime.clone());
        no_wipe.extend(xdg.user_dirs().into_iter().map(|(_, p)| p));
        no_wipe.extend(mounts.mounts().iter().map(|m| m.mount_point.clone()));

        let mut precious = in_home(HOME_PRECIOUS);
        precious.push(xdg.config.join("mozilla"));
        precious.push(xdg.data.join("keyrings"));
        precious.push(xdg.data.join("kwalletd"));
        precious.push(xdg.data.join("evolution"));
        precious.extend(facts.password_store.clone());

        let mut browser_roots = Vec::new();
        for config in [home.join(".config"), xdg.config.clone()] {
            let roots = |names: &[&str], depth: usize| -> Vec<(String, usize)> {
                let paths = names.iter().map(|b| config.join(b)).collect();
                forms_of(paths).into_iter().map(|k| (k, depth)).collect()
            };
            browser_roots.extend(roots(BROWSER_PROFILES, 0));
            browser_roots.extend(roots(BROWSER_PROFILES_IN_CHANNELS, 1));
        }

        g.subtree = forms_of(subtree);
        g.no_wipe = forms_of(no_wipe);
        g.no_delete = g.no_wipe.clone();
        g.precious = forms_of(precious);
        browser_roots.sort();
        browser_roots.dedup();
        g.browser_roots = browser_roots;
        g.home = path_forms(home);
        g
    }

    /// May `path` itself be removed?
    pub fn check(&self, path: &Path, origin: Origin) -> Result<()> {
        self.check_linux(path, origin, &self.no_delete)
    }

    /// May the contents of `dir` be removed as a whole (rule `contents` mode)?
    pub fn check_wipe(&self, dir: &Path) -> Result<()> {
        self.check_linux(dir, Origin::Rule, &self.no_wipe)
    }

    /// True for what lies inside system folders, keys, mail, browser profiles or a sandboxed
    /// app's data. Duplicates and Large files stay out of these: nothing there may be deleted,
    /// and the hashing would read private files for no result. Unlike `check`, a folder that only
    /// holds such a place (`~/.config`) is not private.
    pub fn is_private(&self, path: &Path) -> bool {
        let real = resolve_parent(path).map(|p| key(&p));
        [Some(key(path)), real].iter().flatten().any(|k| {
            self.subtree
                .iter()
                .chain(&self.precious)
                .any(|p| is_within(k, p))
                || self
                    .browser_roots
                    .iter()
                    .any(|(root, _)| is_within(k, root))
                || self.home.iter().any(|h| is_sandbox_data(k, h))
        })
    }

    fn check_linux(&self, path: &Path, origin: Origin, whole: &[String]) -> Result<()> {
        let deny = |why: &str| Err(Error::Protected(format!("{} ({why})", path.display())));
        if let Some(why) = self.refuse_all {
            return deny(why);
        }
        let written = key(path);
        if !path.is_absolute() || written.split(SEP).any(|s| s == ".." || s == ".") {
            return deny("not absolute");
        }
        let Some(real) = resolve_parent(path) else {
            return deny("root folder");
        };
        let real = key(&real);
        for k in [&written, &real] {
            if is_drive_root(k) {
                return deny("root folder");
            }
            if k.split(SEP).any(is_protected_name) {
                return deny("protected folder");
            }
            // Removing a folder removes everything protected below it, so ancestors count too.
            if whole.iter().any(|p| is_within(p, k)) {
                return deny("system or user folder");
            }
            if self
                .subtree
                .iter()
                .any(|p| is_within(k, p) || is_within(p, k))
            {
                return deny("system folder");
            }
            if self
                .precious
                .iter()
                .any(|p| is_within(k, p) || is_within(p, k))
                || self.touches_browser_profile(k, origin)
                || self.home.iter().any(|h| is_sandbox_data(k, h))
            {
                return deny("private user data");
            }
            if self.is_excluded(k) {
                return deny("excluded in settings");
            }
        }
        // Only the real location counts: it is what a delete would reach.
        if !self.home.iter().any(|h| is_within(&real, h)) {
            return deny("outside the home folder");
        }
        Ok(())
    }

    /// A browser profile is off limits as a whole, except that a rule may name one profile's
    /// `Service Worker/CacheStorage`, which is a cache that stays inside the profile.
    fn touches_browser_profile(&self, k: &str, origin: Origin) -> bool {
        self.browser_roots.iter().any(|(root, depth)| {
            if is_within(root, k) {
                return true;
            }
            let Some(rest) = k
                .strip_prefix(root.as_str())
                .and_then(|r| r.strip_prefix(SEP))
            else {
                return false;
            };
            let segs: Vec<&str> = rest.split(SEP).collect();
            let cache = segs.get(depth + 1) == Some(&"Service Worker")
                && segs.get(depth + 2) == Some(&"CacheStorage");
            !(origin == Origin::Rule && cache)
        })
    }
}

#[cfg(all(test, target_os = "linux"))]
impl LinuxFacts {
    fn for_home(home: &Path) -> LinuxFacts {
        LinuxFacts {
            xdg: crate::xdg::Xdg::from_vars(|name| {
                (name == "HOME").then(|| home.as_os_str().to_os_string())
            }),
            mounts: crate::mounts::MountTable::read_current().ok(),
            as_root: false,
            password_store: None,
        }
    }
}

#[cfg(not(target_os = "linux"))]
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
        refuse_without_platform_list()?;
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
    #[cfg(all(test, windows))]
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
        refuse_without_platform_list()?;
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
}

impl Guard {
    /// Windows has no such list: its protected folders are checked when deleting.
    #[cfg(not(target_os = "linux"))]
    pub fn is_private(&self, _path: &Path) -> bool {
        false
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

/// The protected lists above describe Windows. On systems without lists of their own they would
/// protect nothing, so every delete is refused there.
#[cfg(not(target_os = "linux"))]
fn refuse_without_platform_list() -> Result<()> {
    if cfg!(windows) {
        Ok(())
    } else {
        Err(Error::Unsupported("deleting files"))
    }
}

#[cfg(not(target_os = "linux"))]
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
        assert!(wildcard_match("webcache*", "webcache_4430"));
        assert!(wildcard_match("cmake-build-*", "cmake-build-debug"));
        assert!(wildcard_match("a?c", "abc"));
        assert!(!wildcard_match("*.dmp", "memory.dmp.txt"));
        assert!(!wildcard_match("cache", "cache2"));
    }

    #[cfg(windows)]
    #[test]
    fn wildcards_ignore_case_on_windows() {
        assert!(wildcard_match("THUMBCACHE_*.DB", "thumbcache_idx.db"));
        assert!(wildcard_match("FOUND.*", "found.000"));
    }

    #[cfg(not(windows))]
    #[test]
    fn wildcards_keep_case_on_unix() {
        assert!(!wildcard_match("THUMBCACHE_*.DB", "thumbcache_idx.db"));
        assert!(!wildcard_match("Cache", "cache"));
        assert!(wildcard_match("Cache*", "Cache_Data"));
    }

    #[cfg(windows)]
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

    #[cfg(windows)]
    #[test]
    fn expand_vars_rejects_unknown() {
        assert!(expand_vars("%PATH%\\x").is_err());
        assert!(expand_vars("%TEMP").is_err());
        let t = expand_vars("%TEMP%\\x").unwrap();
        assert!(t.ends_with("\\x"));
    }

    #[cfg(windows)]
    #[test]
    fn split_refuses_relative() {
        assert!(split_absolute(r"C:\a\..\b").is_err());
        assert!(split_absolute(r"relative\path").is_err());
        assert!(split_absolute(r"C:\a\b").is_ok());
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    #[test]
    fn guard_refuses_every_delete_without_a_platform_list() {
        let g = Guard::new(&[], None);
        let tmp = std::env::temp_dir().join("reclaim-guard-test");
        assert!(matches!(
            g.check(&tmp, Origin::Rule),
            Err(Error::Unsupported(_))
        ));
        assert!(matches!(g.check_wipe(&tmp), Err(Error::Unsupported(_))));
    }

    #[cfg(target_os = "linux")]
    mod linux {
        use super::*;
        use crate::mounts::{MountTable, parse_mountinfo};
        use crate::xdg::Xdg;
        use std::ffi::{OsStr, OsString};
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        use std::os::unix::fs::symlink;

        const HOME: &str = "/home/test-user";

        fn p(s: &str) -> PathBuf {
            PathBuf::from(s)
        }

        /// A guard for a home folder that need not exist, with the given mount table.
        fn guard_with(mountinfo: &str, exclusions: &[String]) -> Guard {
            let mut facts = LinuxFacts::for_home(Path::new(HOME));
            facts.mounts = Some(MountTable::from_mounts(parse_mountinfo(
                mountinfo.as_bytes(),
            )));
            facts.password_store = Some(p("/home/test-user/secrets/pass"));
            Guard::from_facts(
                &facts,
                exclusions,
                Some(Path::new("/home/test-user/.local/share/ReClaim")),
            )
        }

        fn guard() -> Guard {
            guard_with("1 0 8:1 / / rw - ext4 /dev/sda1 rw", &[])
        }

        fn refused(g: &Guard, path: &str) {
            for origin in [Origin::Rule, Origin::User, Origin::Leftover] {
                assert!(
                    matches!(g.check(Path::new(path), origin), Err(Error::Protected(_))),
                    "{path} must be refused for {origin:?}"
                );
            }
            assert!(
                matches!(g.check_wipe(Path::new(path)), Err(Error::Protected(_))),
                "{path} must not be wiped"
            );
        }

        fn allowed(g: &Guard, path: &str) {
            assert!(
                g.check(Path::new(path), Origin::User).is_ok(),
                "{path} must be allowed: {:?}",
                g.check(Path::new(path), Origin::User)
            );
        }

        #[test]
        fn keys_keep_every_byte() {
            assert_eq!(key(Path::new("/Home/User/")), "/Home/User");
            assert_eq!(key(Path::new("/")), "/");
            assert_eq!(key(Path::new("//a///b//")), "/a/b");
            assert_eq!(key(Path::new("/a/./b/../c")), "/a/./b/../c");
            assert_eq!(
                key(Path::new(r"/a\b")),
                r"/a\b",
                "a backslash is a plain byte"
            );
            assert_ne!(key(Path::new("/a/B")), key(Path::new("/a/b")));
            assert_eq!(name_key("Résumé"), "Résumé");
        }

        #[test]
        fn keys_never_merge_different_bytes() {
            let raw = |b: &[u8]| PathBuf::from(OsString::from_vec(b.to_vec()));
            assert_ne!(key(&raw(b"/a/\xff")), key(&raw(b"/a/\xfe")));
            assert_ne!(key(&raw(b"/a/\xff")), key(Path::new("/a/\u{fffd}")));
            // A real name made of the characters that stand for raw bytes stays apart as well.
            let lookalike = char::from_u32(BYTE_BASE + 0xff).unwrap().to_string();
            assert_ne!(
                key(&raw(b"/a/\xff")),
                key(&Path::new("/a").join(&lookalike))
            );
            // Containment still works across a byte that is not UTF-8.
            let parent = key(&raw(b"/a/\xff"));
            assert!(is_within(&key(&raw(b"/a/\xff/b")), &parent));
            assert!(!is_within(&key(&raw(b"/a/\xff\xfe")), &parent));
        }

        #[test]
        fn containment_needs_a_whole_segment() {
            assert!(is_within("/home/u/b", "/home/u"));
            assert!(!is_within("/home/ub", "/home/u"));
            assert!(!is_within("/home/U/b", "/home/u"));
            assert!(is_within("/x", "/"));
            assert!(is_drive_root("/"));
            assert!(!is_drive_root("/x"));
        }

        fn xdg() -> Xdg {
            Xdg::from_vars(|n| match n {
                "HOME" => Some(HOME.into()),
                "XDG_CACHE_HOME" => Some("/ssd/cache".into()),
                "XDG_CONFIG_HOME" => Some("relative".into()),
                _ => None,
            })
            .unwrap()
        }

        #[test]
        fn expands_the_rule_variables() {
            let x = Some(xdg());
            let e = |pattern| expand_vars_with(pattern, x.as_ref());
            assert_eq!(e("%HOME%/.npm").unwrap(), "/home/test-user/.npm");
            assert_eq!(e("%XDG_CACHE%/pip").unwrap(), "/ssd/cache/pip");
            assert_eq!(
                e("%XDG_CONFIG%/Code").unwrap(),
                "/home/test-user/.config/Code"
            );
            assert_eq!(e("%XDG_DATA%").unwrap(), "/home/test-user/.local/share");
            assert_eq!(
                e("%XDG_STATE%/x").unwrap(),
                "/home/test-user/.local/state/x"
            );
            assert_eq!(e("%xdg_cache%/pip").unwrap(), "/ssd/cache/pip");
        }

        #[test]
        fn refuses_other_variables() {
            let x = Some(xdg());
            let e = |pattern| expand_vars_with(pattern, x.as_ref());
            for bad in [
                "%PATH%/x",
                "%LOCALAPPDATA%/x",
                "%USERPROFILE%",
                "%TEMP",
                "%HOMEX%/x",
                "%HOME%x",
            ] {
                assert!(e(bad).is_err(), "{bad}");
            }
            assert!(
                expand_vars_with("%HOME%/x", None).is_err(),
                "no usable home"
            );
            assert_eq!(LINUX_VARS.len(), 5);
        }

        #[test]
        fn expands_patterns_exactly_and_refuses_relative_ones() {
            let tmp = tempfile::tempdir().unwrap();
            let base = tmp.path().to_str().unwrap();
            fs::create_dir_all(tmp.path().join("Cache/a")).unwrap();
            fs::create_dir_all(tmp.path().join("cache/b")).unwrap();
            fs::create_dir_all(tmp.path().join("100%/c")).unwrap();
            let outside = tmp.path().join("outside");
            fs::create_dir_all(&outside).unwrap();
            symlink(&outside, tmp.path().join("Cache/link")).unwrap();

            let found = expand_pattern(&format!("{base}/Cache/*")).unwrap();
            assert_eq!(
                found,
                vec![tmp.path().join("Cache/a")],
                "case kept, link left out"
            );
            assert_eq!(
                expand_pattern(&format!("{base}/cache/*")).unwrap(),
                vec![tmp.path().join("cache/b")]
            );
            assert_eq!(
                expand_pattern(&format!("{base}/100%/c")).unwrap(),
                vec![tmp.path().join("100%/c")]
            );
            assert!(
                expand_pattern(&format!("{base}/Cache/link"))
                    .unwrap()
                    .is_empty()
            );
            assert!(expand_pattern(&format!("{base}/Cache/../cache")).is_err());
            assert!(expand_pattern(&format!("{base}/./Cache")).is_err());
            assert!(expand_pattern("relative/path").is_err());
            // A byte that is not UTF-8 in a matched name survives expansion.
            let odd = tmp.path().join("odd");
            let name = OsStr::from_bytes(b"caf\xe9");
            fs::create_dir_all(odd.join(name)).unwrap();
            assert_eq!(
                expand_pattern(&format!("{base}/odd/caf*")).unwrap(),
                vec![odd.join(name)]
            );
        }

        #[test]
        fn nothing_inside_system_folders_may_go() {
            let g = guard();
            for path in [
                "/boot",
                "/boot/vmlinuz",
                "/efi",
                "/boot/efi/EFI",
                "/usr",
                "/usr/bin/ls",
                "/bin/ls",
                "/sbin/init",
                "/lib/x.so",
                "/lib32/x",
                "/lib64/x",
                "/libx32/x",
                "/etc",
                "/etc/passwd",
                "/proc/1",
                "/sys/kernel",
                "/dev/null",
                "/run/lock",
                "/var/lib/dpkg/status",
                "/snap/core",
                "/nix/store/abc",
                "/sysroot/x",
                "/ostree/x",
            ] {
                refused(&g, path);
            }
        }

        #[test]
        fn roots_and_shared_folders_may_not_be_removed() {
            let g = guard();
            for path in [
                "/",
                "/home",
                "/root",
                "/opt",
                "/srv",
                "/usr/local",
                "/var",
                "/var/cache",
                "/var/log",
                "/var/tmp",
                "/tmp",
                "/var/spool",
                "/var/mail",
                "/var/crash",
                "/media",
                "/mnt",
                "/run/media",
                HOME,
            ] {
                refused(&g, path);
            }
        }

        #[test]
        fn xdg_folders_may_not_be_removed_but_named_children_may() {
            let g = guard();
            for rel in [
                ".config",
                ".local",
                ".local/share",
                ".local/state",
                ".local/bin",
                ".cache",
                ".var",
                ".var/app",
                "snap",
                ".local/share/flatpak",
            ] {
                refused(&g, &format!("{HOME}/{rel}"));
            }
            assert!(g.check_wipe(&p("/home/test-user/.cache/pip")).is_ok());
            assert!(
                g.check_wipe(&p("/home/test-user/.config/Code/Cache"))
                    .is_ok()
            );
            allowed(&g, "/home/test-user/.cache/pip/x");
            refused(&g, "/home/test-user/.cache/doc");
            refused(&g, "/home/test-user/.cache/doc/by-app/x");
        }

        #[test]
        fn honours_a_moved_cache_and_config_folder() {
            let mut facts = LinuxFacts::for_home(Path::new(HOME));
            facts.xdg = Xdg::from_vars(|n| match n {
                "HOME" => Some(HOME.into()),
                "XDG_CACHE_HOME" => Some("/home/test-user/ssd/cache".into()),
                "XDG_CONFIG_HOME" => Some("/home/test-user/cfg".into()),
                "XDG_DATA_HOME" => Some("/home/test-user/dat".into()),
                "XDG_RUNTIME_DIR" => Some("/home/test-user/rt".into()),
                _ => None,
            });
            let g = Guard::from_facts(&facts, &[], None);
            for path in [
                "/home/test-user/ssd/cache",
                "/home/test-user/ssd/cache/doc/x",
                "/home/test-user/cfg",
                "/home/test-user/cfg/google-chrome/Default/Cookies",
                "/home/test-user/cfg/mozilla",
                "/home/test-user/dat",
                "/home/test-user/dat/Trash/files/x",
                "/home/test-user/dat/keyrings/login.keyring",
                "/home/test-user/rt",
            ] {
                refused(&g, path);
            }
            allowed(&g, "/home/test-user/ssd/cache/pip");
        }

        #[test]
        fn every_mount_point_is_protected_with_its_parents() {
            let g = guard_with(
                "1 0 8:1 / / rw - ext4 /dev/sda1 rw\n\
                 2 1 8:17 / /home/test-user/My\\040Disk rw - ext4 /dev/sdb1 rw\n\
                 3 1 8:1 /x /home/test-user/work/bound rw - ext4 /dev/sda1 rw\n\
                 4 1 0:9 / /home/test-user/.cache/app/remote rw - fuse.sshfs h:/ rw\n",
                &[],
            );
            refused(&g, "/home/test-user/My Disk");
            refused(&g, "/home/test-user/work/bound");
            refused(&g, "/home/test-user/work");
            refused(&g, "/home/test-user/.cache/app");
            allowed(&g, "/home/test-user/work/other");
        }

        #[test]
        fn user_folders_come_from_the_file_and_stay_protected() {
            let tmp = tempfile::tempdir().unwrap();
            let home = tmp.path().join("home");
            fs::create_dir_all(home.join(".config")).unwrap();
            fs::write(
                home.join(".config/user-dirs.dirs"),
                "XDG_DOWNLOAD_DIR=\"$HOME/İndirilenler\"\nXDG_MUSIC_DIR=\"$HOME/\"\n",
            )
            .unwrap();
            let g = Guard::for_home(&home, &[], None);
            let downloads = home.join("İndirilenler");
            assert!(g.check(&downloads, Origin::User).is_err());
            assert!(g.check_wipe(&downloads).is_err());
            assert!(g.check(&downloads.join("big.iso"), Origin::User).is_ok());
            // English names stay protected even when the file is silent about them.
            assert!(g.check(&home.join("Documents"), Origin::User).is_err());
            assert!(
                g.check(&home.join("Documents/big.iso"), Origin::User)
                    .is_ok()
            );
            // The `$HOME/` entry named no folder, so the home folder's children are not affected.
            assert!(g.check(&home.join("Other"), Origin::User).is_ok());
        }

        #[test]
        fn precious_folders_are_off_limits_at_any_depth() {
            let g = guard();
            for rel in [
                ".ssh",
                ".ssh/id_ed25519",
                ".gnupg/private-keys-v1.d",
                ".password-store/a.gpg",
                ".local/share/keyrings/login.keyring",
                ".gnome2/keyrings/x",
                ".local/share/kwalletd/x",
                ".pki/nssdb",
                ".mozilla/firefox/a.default",
                ".config/mozilla/firefox",
                ".thunderbird/x",
                ".local/share/evolution/mail",
                ".bitcoin/wallet.dat",
                "secrets/pass/a.gpg",
                ".config/google-chrome",
                ".config/google-chrome/Default/Cookies",
                ".config/chromium/x",
                ".config/BraveSoftware/Brave-Browser/Default/x",
                ".config/microsoft-edge/x",
                ".config/vivaldi/x",
            ] {
                refused(&g, &format!("{HOME}/{rel}"));
            }
        }

        #[test]
        fn only_a_rule_may_reach_a_browsers_service_worker_cache() {
            let g = guard();
            let cache =
                p("/home/test-user/.config/google-chrome/Default/Service Worker/CacheStorage");
            assert!(g.check(&cache, Origin::Rule).is_ok());
            assert!(g.check_wipe(&cache).is_ok());
            assert!(g.check(&cache.join("abc"), Origin::Rule).is_ok());
            assert!(g.check(&cache, Origin::User).is_err());
            let parent = cache.parent().unwrap();
            assert!(g.check(parent, Origin::Rule).is_err());
            assert!(g.check(&parent.join("Database"), Origin::Rule).is_err());
            assert!(g.check_wipe(parent).is_err());
        }

        #[test]
        fn brave_keeps_its_profiles_one_folder_deeper() {
            let g = guard();
            let brave = p("/home/test-user/.config/BraveSoftware");
            let cache = brave.join("Brave-Browser/Default/Service Worker/CacheStorage");
            assert!(g.check_wipe(&cache).is_ok());
            assert!(g.check(&cache.join("abc"), Origin::Rule).is_ok());
            assert!(g.check(&cache, Origin::User).is_err());
            for refused in [
                brave.clone(),
                brave.join("Brave-Browser"),
                brave.join("Brave-Browser/Default"),
                brave.join("Brave-Browser/Default/Cookies"),
                brave.join("Brave-Browser/Default/Service Worker"),
                // No profile folder between the channel and the cache.
                brave.join("Brave-Browser/Service Worker/CacheStorage"),
                brave.join("Other/x"),
            ] {
                assert!(
                    g.check(&refused, Origin::Rule).is_err(),
                    "{}",
                    refused.display()
                );
            }
        }

        #[test]
        fn sandboxed_apps_keep_everything_but_their_caches() {
            let g = guard();
            for rel in [
                ".var/app/org.gimp.GIMP",
                ".var/app/org.gimp.GIMP/config",
                ".var/app/org.gimp.GIMP/config/GIMP/x",
                ".var/app/org.gimp.GIMP/data/x",
                ".var/app/org.gimp.GIMP/.local",
                ".var/app/org.gimp.GIMP/.local/state/x",
                "snap/firefox",
                "snap/firefox/common",
                "snap/firefox/current",
                "snap/firefox/123/.mozilla/x",
                "snap/firefox/common/.local/x",
            ] {
                refused(&g, &format!("{HOME}/{rel}"));
            }
            for rel in [
                ".var/app/org.gimp.GIMP/cache",
                ".var/app/org.gimp.GIMP/cache/x",
                "snap/firefox/common/.cache",
                "snap/firefox/current/.cache/x",
            ] {
                assert!(g.check_wipe(&p(&format!("{HOME}/{rel}"))).is_ok(), "{rel}");
            }
        }

        #[test]
        fn protected_names_hold_anywhere() {
            let g = guard();
            for rel in [
                "code/app/.git",
                "code/app/.git/config",
                "disk/lost+found",
                "disk/lost+found/x",
                "disk/.Trash",
                "disk/.Trash/1000/files/x",
                "disk/.Trash-1000",
                "disk/.Trash-1000/files/x",
                ".local/share/Trash",
                ".local/share/Trash/files/x",
                ".local/share/Trash/info/x.trashinfo",
                ".local/share/ReClaim",
                ".local/share/ReClaim/settings.json",
            ] {
                refused(&g, &format!("{HOME}/{rel}"));
            }
            // Names compare exactly on Linux.
            allowed(&g, "/home/test-user/code/app/.GIT");
            allowed(&g, "/home/test-user/code/app/.gitignore");
            allowed(&g, "/home/test-user/code/app/lost+found2");
        }

        #[test]
        fn nothing_outside_the_home_folder_may_go() {
            let g = guard();
            for path in [
                "/data/file.bin",
                "/tmp/x",
                "/var/tmp/x",
                "/var/cache/apt/archives/a.deb",
                "/var/log/syslog",
                "/mnt/usb/photos",
                "/opt/app",
                "/home/other-user/file",
                "/home/test-user2/file",
                "/root/file",
            ] {
                refused(&g, path);
            }
        }

        #[test]
        fn refuses_paths_that_are_not_plain_and_absolute() {
            let g = guard();
            for path in [
                "relative/file",
                "/home/test-user/a/../../../etc/passwd",
                "/home/test-user/./a",
                "/home/test-user/a/..",
            ] {
                refused(&g, path);
            }
            // Extra slashes do not get a protected path past the lists.
            refused(&g, "//etc//passwd");
            refused(&g, "/home//test-user//.ssh//id_rsa");
            refused(&g, "/home/test-user/.cache//");
        }

        #[test]
        fn paths_compare_with_their_case() {
            let g = guard();
            allowed(&g, "/home/test-user/.SSH/x");
            allowed(&g, "/home/test-user/documents");
            refused(&g, "/HOME/test-user/file");
        }

        #[test]
        fn resolves_links_in_the_parent_and_checks_links_themselves() {
            let tmp = tempfile::tempdir().unwrap();
            let home = tmp.path().join("home");
            let outside = tmp.path().join("outside");
            fs::create_dir_all(home.join(".ssh")).unwrap();
            fs::create_dir_all(&outside).unwrap();
            fs::write(home.join(".ssh/id_rsa"), b"k").unwrap();
            fs::write(outside.join("f"), b"x").unwrap();
            symlink(&outside, home.join("to-outside")).unwrap();
            symlink(home.join(".ssh"), home.join("to-ssh")).unwrap();
            symlink("/etc", home.join("to-etc")).unwrap();
            let g = Guard::for_home(&home, &[], None);

            // The real location is what counts, whatever the path says.
            assert!(g.check(&home.join("to-outside/f"), Origin::User).is_err());
            assert!(g.check(&home.join("to-ssh/id_rsa"), Origin::User).is_err());
            assert!(g.check(&home.join("to-etc/passwd"), Origin::User).is_err());
            // The link itself is an ordinary entry of the home folder.
            assert!(g.check(&home.join("to-outside"), Origin::User).is_ok());
            assert!(g.check(&home.join("to-ssh"), Origin::User).is_ok());
            // A link from elsewhere into the home folder reaches a real path inside it.
            let from_outside = tmp.path().join("into-home");
            symlink(&home, &from_outside).unwrap();
            assert!(g.check(&from_outside.join("file"), Origin::User).is_ok());
            assert!(
                g.check(&from_outside.join(".ssh/id_rsa"), Origin::User)
                    .is_err()
            );
            // Parents that do not exist yet resolve as far as they exist.
            assert!(g.check(&home.join("to-etc/none/x"), Origin::User).is_err());
            assert!(g.check(&home.join("none/x"), Origin::User).is_ok());
        }

        #[test]
        fn a_home_behind_a_link_works_under_both_spellings() {
            let tmp = tempfile::tempdir().unwrap();
            let real = tmp.path().join("var-home/u");
            fs::create_dir_all(real.join(".ssh")).unwrap();
            fs::create_dir_all(real.join("work")).unwrap();
            let link = tmp.path().join("u");
            symlink(&real, &link).unwrap();
            let g = Guard::for_home(&link, &[], None);
            for base in [&link, &real] {
                assert!(g.check(base, Origin::User).is_err());
                assert!(g.check(&base.join(".ssh/x"), Origin::User).is_err());
                assert!(g.check(&base.join("work/x"), Origin::User).is_ok());
            }
        }

        #[test]
        fn exclusions_hold_for_each_spelling() {
            let tmp = tempfile::tempdir().unwrap();
            let real = tmp.path().join("real/u");
            fs::create_dir_all(real.join("keep/inner")).unwrap();
            let home = tmp.path().join("u");
            symlink(&real, &home).unwrap();
            let excluded = home.join("keep").to_string_lossy().into_owned();
            let g = Guard::for_home(&home, std::slice::from_ref(&excluded), None);
            for base in [&home, &real] {
                assert!(g.check(&base.join("keep"), Origin::User).is_err());
                assert!(g.check(&base.join("keep/inner/x"), Origin::User).is_err());
                assert!(g.check(&base.join("other"), Origin::User).is_ok());
                assert!(g.check_wipe(&base.join("keep/inner")).is_err());
            }
            // The walker gets the exclusion in the spelling of the folder it was given.
            assert!(
                g.exclusions_inside(&real)
                    .contains(&key(&real.join("keep")))
            );
            assert!(
                g.exclusions_inside(&home)
                    .contains(&key(&home.join("keep")))
            );
            assert!(g.exclusions_inside(&real.join("other")).is_empty());
        }

        #[test]
        fn fails_closed_without_the_facts_it_needs() {
            let refuses = |facts: &LinuxFacts| {
                let g = Guard::from_facts(facts, &[], None);
                let target = Path::new("/home/test-user/.cache/pip/x");
                assert!(matches!(
                    g.check(target, Origin::User),
                    Err(Error::Protected(_))
                ));
                assert!(matches!(g.check_wipe(target), Err(Error::Protected(_))));
            };
            let home = Path::new(HOME);
            refuses(&LinuxFacts {
                as_root: true,
                ..LinuxFacts::for_home(home)
            });
            refuses(&LinuxFacts {
                xdg: None,
                ..LinuxFacts::for_home(home)
            });
            refuses(&LinuxFacts {
                mounts: None,
                ..LinuxFacts::for_home(home)
            });
            // A guard that was never built protects by refusing too.
            assert!(
                Guard::default()
                    .check(Path::new("/home/test-user/x"), Origin::User)
                    .is_err()
            );
        }

        /// Running as root is refused outright, whatever the path. Only this test depends on
        /// who runs it.
        #[test]
        fn the_real_guard_refuses_root() {
            if crate::system::is_elevated() {
                let g = Guard::new(&[], None);
                let home = std::env::temp_dir().join("reclaim-root-test");
                assert!(g.check(&home, Origin::User).is_err());
                assert!(g.check_wipe(&home).is_err());
            }
        }
    }

    #[cfg(windows)]
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

    #[cfg(windows)]
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
