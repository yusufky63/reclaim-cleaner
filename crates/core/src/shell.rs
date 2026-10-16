//! Explorer integration: the "Analyze with ReClaim" entry on folders, folder backgrounds and
//! drives, and the paths Explorer hands back to the app.
//!
//! The entries are classic per-user verbs under `HKCU\Software\Classes`. They need no
//! administrator rights and no shell extension DLL, and the app writes and removes them itself,
//! so portable copies work too. On Windows 11 they sit under "Show more options": the new
//! top-level menu only takes handlers from signed packages with an identity, which this project
//! does not ship yet.
//!
//! Paths from the command line are untrusted. They can come from Explorer, from another
//! program that starts the exe, or (forwarded by the running copy) from any process on the
//! desktop. They only ever choose what to scan; nothing is deleted from a command line.

use std::path::{Path, PathBuf};

use serde::Serialize;
use windows::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};

use crate::paths::key;
use crate::{Error, Result};

/// Registry key name of the verb under each class.
pub const VERB: &str = "ReClaim.Analyze";
/// Command-line flag that carries a path to analyze.
pub const ANALYZE_FLAG: &str = "--analyze";
/// Most paths taken from one burst of Explorer launches.
pub const MAX_PATHS: usize = 100;

/// Class keys and whether `%V` is quoted. Drive roots get it unquoted: `"C:\"` reaches the app
/// as `C:"`, because `\"` is an escaped quote, and drive roots never contain spaces.
const CLASSES: &[(&str, bool)] = &[
    (r"Directory\shell", true),
    (r"Directory\Background\shell", true),
    (r"Drive\shell", false),
];

fn label(lang: &str) -> &'static str {
    if lang.eq_ignore_ascii_case("tr") {
        "ReClaim ile analiz et"
    } else {
        "Analyze with ReClaim"
    }
}

fn command_line(exe: &Path, quoted: bool) -> String {
    if quoted {
        format!("\"{}\" {ANALYZE_FLAG} \"%V\"", exe.display())
    } else {
        format!("\"{}\" {ANALYZE_FLAG} %V", exe.display())
    }
}

/// Program path inside a verb command (`"C:\...\reclaim-cleaner.exe" --analyze "%V"`).
fn exe_in_command(cmd: &str) -> Option<String> {
    let rest = cmd.trim().strip_prefix('"')?;
    Some(rest[..rest.find('"')?].to_string())
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MenuStatus {
    /// At least one entry exists.
    pub enabled: bool,
    /// The program the entries start.
    pub exe: Option<String>,
    /// Every entry exists and starts this copy of the app.
    pub current: bool,
    /// The program the entries start still exists.
    pub exe_exists: bool,
}

/// Writes the entries below `classes` (normally `HKCU\Software\Classes`).
pub fn register_in(classes: &RegKey, exe: &Path, lang: &str) -> Result<()> {
    let werr = |e: std::io::Error| Error::io(r"HKCU\Software\Classes", e);
    for (class, quoted) in CLASSES {
        let (verb, _) = classes
            .create_subkey(format!(r"{class}\{VERB}"))
            .map_err(werr)?;
        verb.set_value("MUIVerb", &label(lang)).map_err(werr)?;
        verb.set_value("Icon", &format!("\"{}\",0", exe.display()))
            .map_err(werr)?;
        // One process per selected item; the running copy merges them into one request.
        verb.set_value("MultiSelectModel", &"Player")
            .map_err(werr)?;
        let (cmd, _) = verb.create_subkey("command").map_err(werr)?;
        cmd.set_value("", &command_line(exe, *quoted))
            .map_err(werr)?;
    }
    Ok(())
}

/// Removes the entries below `classes`. Missing entries are fine.
pub fn unregister_in(classes: &RegKey) -> Result<()> {
    for (class, _) in CLASSES {
        match classes.delete_subkey_all(format!(r"{class}\{VERB}")) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::io(r"HKCU\Software\Classes", e)),
        }
    }
    Ok(())
}

pub fn status_in(classes: &RegKey, exe: &Path) -> MenuStatus {
    let commands: Vec<Option<String>> = CLASSES
        .iter()
        .map(|(class, _)| {
            classes
                .open_subkey_with_flags(format!(r"{class}\{VERB}\command"), KEY_READ)
                .ok()
                .and_then(|k| k.get_value::<String, _>("").ok())
        })
        .collect();
    let found = commands.iter().flatten().find_map(|c| exe_in_command(c));
    let mine = key(exe);
    MenuStatus {
        enabled: commands.iter().any(Option::is_some),
        current: commands.iter().all(|c| {
            c.as_deref()
                .and_then(exe_in_command)
                .is_some_and(|e| key(Path::new(&e)) == mine)
        }),
        exe_exists: found.as_deref().is_some_and(|e| Path::new(e).is_file()),
        exe: found,
    }
}

fn user_classes() -> Result<RegKey> {
    RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(r"Software\Classes")
        .map(|(k, _)| k)
        .map_err(|e| Error::io(r"HKCU\Software\Classes", e))
}

/// Tells Explorer that verbs changed, so open windows pick it up.
fn notify_shell() {
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
}

pub fn register(exe: &Path, lang: &str) -> Result<()> {
    register_in(&user_classes()?, exe, lang)?;
    notify_shell();
    Ok(())
}

pub fn unregister() -> Result<()> {
    unregister_in(&user_classes()?)?;
    notify_shell();
    Ok(())
}

/// Points existing entries at `exe` when the program they start is gone (a portable copy that
/// moved, a reinstall into another folder). Entries of another copy that still exists are left
/// alone. Returns whether anything changed.
pub fn repair_in(classes: &RegKey, exe: &Path) -> Result<bool> {
    let s = status_in(classes, exe);
    if !s.enabled || s.current || s.exe_exists {
        return Ok(false);
    }
    let werr = |e: std::io::Error| Error::io(r"HKCU\Software\Classes", e);
    for (class, quoted) in CLASSES {
        let Ok(verb) = classes.open_subkey_with_flags(
            format!(r"{class}\{VERB}"),
            winreg::enums::KEY_READ | winreg::enums::KEY_WRITE,
        ) else {
            continue;
        };
        verb.set_value("Icon", &format!("\"{}\",0", exe.display()))
            .map_err(werr)?;
        let (cmd, _) = verb.create_subkey("command").map_err(werr)?;
        cmd.set_value("", &command_line(exe, *quoted))
            .map_err(werr)?;
    }
    Ok(true)
}

pub fn repair(exe: &Path) -> Result<bool> {
    let changed = repair_in(&user_classes()?, exe)?;
    if changed {
        notify_shell();
    }
    Ok(changed)
}

pub fn status(exe: &Path) -> MenuStatus {
    match RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(r"Software\Classes", KEY_READ) {
        Ok(classes) => status_in(&classes, exe),
        Err(_) => MenuStatus {
            enabled: false,
            exe: None,
            current: false,
            exe_exists: false,
        },
    }
}

/// Repairs and checks one path from Explorer. Accepts only existing folders on drive-letter
/// paths; network shares, device paths and shell namespaces (`::{CLSID}`) are refused.
pub fn normalize_shell_arg(raw: &str) -> Option<PathBuf> {
    let s = raw.trim().trim_matches('"').trim();
    if s.is_empty() || s.len() > 32_000 || s.contains('\0') {
        return None;
    }
    let mut s = s.replace('/', "\\");
    // `"C:\"` arrives as `C:` once the stray quote is gone.
    if s.len() == 2 && s.ends_with(':') {
        s.push('\\');
    }
    let b = s.as_bytes();
    if !(b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\') {
        return None;
    }
    if s.split('\\').any(|seg| seg == ".." || seg == ".") {
        return None;
    }
    let p = PathBuf::from(s);
    p.is_dir().then_some(p)
}

/// Paths given with `--analyze` in one command line. Everything else is ignored.
pub fn analyze_args(args: &[String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i].eq_ignore_ascii_case(ANALYZE_FLAG) {
            if let Some(p) = args.get(i + 1).and_then(|a| normalize_shell_arg(a)) {
                out.push(p);
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    out
}

/// What to scan for a burst of paths.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum AnalyzeRequest {
    /// One folder or drive.
    One { path: String },
    /// Several folders with the same parent, picked together in Explorer.
    Selection { parent: String, names: Vec<String> },
}

/// Merges the paths of one burst (Explorer starts one process per selected folder) into one
/// request: siblings become a selection inside their parent, anything else scans the first.
pub fn plan_analyze(paths: &[PathBuf]) -> Option<AnalyzeRequest> {
    let mut unique: Vec<&PathBuf> = Vec::new();
    for p in paths.iter().take(MAX_PATHS) {
        if !unique.iter().any(|u| key(u) == key(p)) {
            unique.push(p);
        }
    }
    let first = *unique.first()?;
    let one = || AnalyzeRequest::One {
        path: first.to_string_lossy().into_owned(),
    };
    if unique.len() == 1 {
        return Some(one());
    }
    // Drive roots have no parent to group them under.
    let Some(parent) = first.parent() else {
        return Some(one());
    };
    let siblings = unique
        .iter()
        .all(|p| p.parent().is_some_and(|q| key(q) == key(parent)) && p.file_name().is_some());
    if !siblings {
        return Some(one());
    }
    Some(AnalyzeRequest::Selection {
        parent: parent.to_string_lossy().into_owned(),
        names: unique
            .iter()
            .filter_map(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repairs_and_checks_explorer_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_string_lossy().into_owned();
        assert_eq!(normalize_shell_arg(&dir), Some(tmp.path().to_path_buf()));
        assert_eq!(
            normalize_shell_arg(&format!("\"{dir}\"")),
            Some(tmp.path().to_path_buf())
        );
        let sys = std::env::var("SYSTEMDRIVE").unwrap_or_else(|_| "C:".into());
        // `"C:\"` arrives as `C:"`.
        assert_eq!(
            normalize_shell_arg(&format!("{sys}\"")),
            Some(PathBuf::from(format!("{sys}\\")))
        );
        assert!(normalize_shell_arg(r"\\server\share").is_none());
        assert!(normalize_shell_arg(r"\\?\C:\Windows").is_none());
        assert!(normalize_shell_arg("::{20D04FE0-3AEA-1069-A2D8-08002B30309D}").is_none());
        assert!(normalize_shell_arg(&format!(r"{dir}\..\x")).is_none());
        assert!(normalize_shell_arg("relative").is_none());
        let file = tmp.path().join("f.txt");
        std::fs::write(&file, b"").unwrap();
        assert!(normalize_shell_arg(&file.to_string_lossy()).is_none());
    }

    #[test]
    fn reads_only_analyze_flags() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_string_lossy().into_owned();
        let args: Vec<String> = vec![
            "reclaim-cleaner.exe".into(),
            "--elevated-clean".into(),
            dir.clone(),
            "--analyze".into(),
            dir.clone(),
            "--analyze".into(),
        ];
        assert_eq!(analyze_args(&args), vec![tmp.path().to_path_buf()]);
    }

    #[test]
    fn merges_bursts_into_one_request() {
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(plan_analyze(&[]), None);
        assert_eq!(
            plan_analyze(&[p(r"D:\Games"), p(r"d:\games")]),
            Some(AnalyzeRequest::One {
                path: r"D:\Games".into()
            })
        );
        assert_eq!(
            plan_analyze(&[p(r"D:\Games\A"), p(r"D:\Games\B")]),
            Some(AnalyzeRequest::Selection {
                parent: r"D:\Games".into(),
                names: vec!["A".into(), "B".into()]
            })
        );
        assert_eq!(
            plan_analyze(&[p(r"C:\"), p(r"D:\")]),
            Some(AnalyzeRequest::One {
                path: r"C:\".into()
            })
        );
        assert_eq!(
            plan_analyze(&[p(r"D:\Games\A"), p(r"E:\Other")]),
            Some(AnalyzeRequest::One {
                path: r"D:\Games\A".into()
            })
        );
    }

    #[test]
    fn writes_reads_and_removes_verbs() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = format!(r"Software\ReClaimTest-{stamp}");
        let (classes, _) = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey(&base)
            .unwrap();
        let exe_buf = PathBuf::from(format!(r"C:\ReClaimTest-{stamp}\reclaim-cleaner.exe"));
        let exe = exe_buf.as_path();
        assert!(!status_in(&classes, exe).enabled);

        register_in(&classes, exe, "en").unwrap();
        let s = status_in(&classes, exe);
        assert!(s.enabled && s.current);
        assert_eq!(s.exe.as_deref(), exe.to_str());
        let drive: String = classes
            .open_subkey(format!(r"Drive\shell\{VERB}\command"))
            .unwrap()
            .get_value("")
            .unwrap();
        assert!(drive.ends_with("--analyze %V"), "drive roots stay unquoted");
        let dir: String = classes
            .open_subkey(format!(r"Directory\Background\shell\{VERB}\command"))
            .unwrap()
            .get_value("")
            .unwrap();
        assert!(dir.ends_with("--analyze \"%V\""));
        let moved = Path::new(r"D:\Portable\reclaim-cleaner.exe");
        assert!(!status_in(&classes, moved).current);
        // The old program does not exist on this machine, so the entries follow the new copy.
        assert!(repair_in(&classes, moved).unwrap());
        assert!(status_in(&classes, moved).current);
        assert!(!repair_in(&classes, moved).unwrap());

        unregister_in(&classes).unwrap();
        assert!(!status_in(&classes, exe).enabled);
        unregister_in(&classes).unwrap();
        RegKey::predef(HKEY_CURRENT_USER)
            .delete_subkey_all(&base)
            .unwrap();
    }
}
