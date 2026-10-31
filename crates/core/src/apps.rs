//! Installed programs from the registry Uninstall keys (the same source as Settings > Apps)
//! plus Store apps through PowerShell. Other systems list nothing until they are ported.

#[cfg(windows)]
use std::collections::HashSet;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::Path;
#[cfg(windows)]
use std::process::Command;

#[cfg(windows)]
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
#[cfg(windows)]
use winreg::RegKey;
#[cfg(windows)]
use winreg::enums::{
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
};

use crate::system::LaunchOutcome;
#[cfg(windows)]
use crate::system::run_and_wait;
use crate::{Error, Result};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledApp {
    /// `hklm64:<key>`, `hklm32:<key>`, `hkcu:<key>` or `store:<PackageFullName>`.
    pub id: String,
    pub name: String,
    pub publisher: String,
    pub version: String,
    /// `YYYY-MM-DD` when known.
    pub installed: Option<String>,
    pub bytes: Option<u64>,
    pub location: Option<String>,
    pub icon: Option<String>,
    pub uninstall: Option<String>,
    pub quiet_uninstall: Option<String>,
    pub scope: String,
    pub source: String,
    /// The uninstaller is gone (or Windows Installer no longer knows the product): the entry
    /// is left over from an app that was already removed.
    #[serde(default)]
    pub broken: bool,
}

#[cfg(windows)]
fn read_hive(hive: &RegKey, flags: u32, tag: &str, scope: &str, out: &mut Vec<InstalledApp>) {
    let Ok(root) = hive.open_subkey_with_flags(UNINSTALL, KEY_READ | flags) else {
        return;
    };
    for sub in root.enum_keys().flatten() {
        let Ok(k) = root.open_subkey_with_flags(&sub, KEY_READ | flags) else {
            continue;
        };
        let s = |n: &str| {
            k.get_value::<String, _>(n)
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let d = |n: &str| k.get_value::<u32, _>(n).ok();
        let Some(name) = s("DisplayName") else {
            continue;
        };
        // Hidden components, updates and patches are not separate apps for the user.
        if d("SystemComponent") == Some(1)
            || s("ParentKeyName").is_some()
            || s("ParentDisplayName").is_some()
        {
            continue;
        }
        if let Some(rt) = s("ReleaseType")
            && ["update", "hotfix", "security update", "service pack"]
                .contains(&rt.to_lowercase().as_str())
        {
            continue;
        }
        let uninstall = s("UninstallString");
        if uninstall.is_none() && s("QuietUninstallString").is_none() {
            continue;
        }
        let icon = s("DisplayIcon")
            .and_then(|v| crate::icons::icon_file(&v))
            .or_else(|| main_exe(s("InstallLocation").as_deref(), &name));
        let broken = entry_is_broken(
            &sub,
            d("WindowsInstaller") == Some(1),
            uninstall
                .as_deref()
                .or(s("QuietUninstallString").as_deref()),
        );
        let installed = s("InstallDate").and_then(|v| {
            (v.len() == 8 && v.chars().all(|c| c.is_ascii_digit()))
                .then(|| format!("{}-{}-{}", &v[0..4], &v[4..6], &v[6..8]))
        });
        out.push(InstalledApp {
            id: format!("{tag}:{sub}"),
            name,
            publisher: s("Publisher").unwrap_or_default(),
            version: s("DisplayVersion").unwrap_or_default(),
            installed,
            bytes: d("EstimatedSize")
                .filter(|v| *v > 0)
                .map(|kb| u64::from(kb) * 1024),
            // Installers often leave a stale InstallLocation behind; only keep real folders.
            location: s("InstallLocation")
                .map(|l| l.trim_matches('"').to_string())
                .filter(|l| Path::new(l).is_dir()),
            icon,
            uninstall,
            quiet_uninstall: s("QuietUninstallString"),
            scope: scope.to_string(),
            source: String::from("desktop"),
            broken,
        });
    }
}

/// Desktop programs, deduplicated by name and version (some installers register twice).
#[cfg(windows)]
pub fn desktop_apps() -> Vec<InstalledApp> {
    let mut out = Vec::new();
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    read_hive(&hklm, KEY_WOW64_64KEY, "hklm64", "machine", &mut out);
    read_hive(&hklm, KEY_WOW64_32KEY, "hklm32", "machine", &mut out);
    read_hive(&hkcu, 0, "hkcu", "user", &mut out);
    let mut seen = HashSet::new();
    out.retain(|a| seen.insert(format!("{}|{}", a.name.to_lowercase(), a.version)));
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

#[cfg(windows)]
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct AppxRow {
    name: String,
    package_full_name: String,
    version: Option<String>,
    publisher_display_name: Option<String>,
    display_name: Option<String>,
    install_location: Option<String>,
    logo: Option<String>,
}

/// Store apps installed for the current user, excluding frameworks and system apps.
#[cfg(windows)]
pub fn store_apps() -> Result<Vec<InstalledApp>> {
    let script = r#"$ErrorActionPreference='SilentlyContinue'; [Console]::OutputEncoding=[Text.Encoding]::UTF8;
Get-AppxPackage -PackageTypeFilter Main | Where-Object { $_.SignatureKind -eq 'Store' -and -not $_.IsFramework -and -not $_.NonRemovable } | ForEach-Object {
  $m = $null; try { $m = Get-AppxPackageManifest -Package $_.PackageFullName } catch {}
  $dn = $null; $pub = $null; $logo = $null
  if ($m) {
    $dn = $m.Package.Properties.DisplayName; $pub = $m.Package.Properties.PublisherDisplayName
    $app = @($m.Package.Applications.Application)[0]
    if ($app -and $app.VisualElements) { $logo = $app.VisualElements.Square44x44Logo }
    if (-not $logo) { $logo = $m.Package.Properties.Logo }
  }
  [pscustomobject]@{ Name=$_.Name; PackageFullName=$_.PackageFullName; Version=[string]$_.Version; PublisherDisplayName=$pub; DisplayName=$dn; InstallLocation=$_.InstallLocation; Logo=[string]$logo }
} | ConvertTo-Json -Compress"#;
    let out = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| Error::Other(e.to_string()))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let text = text.trim();
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<AppxRow> = if text.starts_with('[') {
        serde_json::from_str(text)?
    } else {
        vec![serde_json::from_str(text)?]
    };
    let mut apps: Vec<InstalledApp> = rows
        .into_par_iter()
        .map(|r| {
            let icon = match (&r.install_location, &r.logo) {
                (Some(loc), Some(logo)) if !logo.is_empty() => store_logo(loc, logo),
                _ => None,
            };
            // Package folders are small and readable, so measure them for a real size.
            let bytes = r
                .install_location
                .as_deref()
                .map(|l| {
                    crate::walk::measure_dir(Path::new(l), None, &crate::job::Job::new()).bytes
                })
                .filter(|b| *b > 0);
            let name = r
                .display_name
                .filter(|d| !d.starts_with("ms-resource:") && !d.is_empty())
                .unwrap_or_else(|| pretty_package_name(&r.name));
            InstalledApp {
                id: format!("store:{}", r.package_full_name),
                name,
                publisher: r
                    .publisher_display_name
                    .filter(|p| !p.starts_with("ms-resource:"))
                    .unwrap_or_default(),
                version: r.version.unwrap_or_default(),
                installed: None,
                bytes,
                location: r.install_location,
                icon,
                uninstall: None,
                quiet_uninstall: None,
                scope: String::from("user"),
                source: String::from("store"),
                broken: false,
            }
        })
        .collect();
    apps.sort_by_key(|a| a.name.to_lowercase());
    Ok(apps)
}

/// Resolves a manifest logo such as `Assets\Square44x44Logo.png` to the file Windows would
/// show. Packages ship scaled variants (`.targetsize-48_altform-unplated.png`); the unplated
/// ones look right on any background, so they win.
#[cfg(windows)]
pub fn store_logo(location: &str, rel: &str) -> Option<String> {
    let rel = rel.replace('/', "\\");
    if rel.split('\\').any(|s| s == ".." || s.is_empty()) {
        return None;
    }
    let full = Path::new(location).join(&rel);
    let dir = full.parent()?;
    let stem = full.file_stem()?.to_string_lossy().to_lowercase();
    let ext = full.extension()?.to_string_lossy().to_lowercase();
    let score = |n: &str| -> i32 {
        if n.contains("contrast-") {
            return -1;
        }
        let unplated = n.contains("altform-unplated");
        let base = if n.contains("targetsize-48") {
            40
        } else if n.contains("targetsize-32") {
            36
        } else if n.contains("targetsize-64") || n.contains("targetsize-44") {
            34
        } else if n.contains("targetsize") {
            30
        } else if n.contains("scale-200") {
            28
        } else if n.contains("scale-100") {
            26
        } else if n.contains("scale") {
            24
        } else {
            20
        };
        base + if unplated { 20 } else { 0 }
    };
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_lowercase();
            let same = n == format!("{stem}.{ext}")
                || (n.starts_with(&format!("{stem}.")) && n.ends_with(&format!(".{ext}")));
            same.then(|| (score(&n), e.path()))
        })
        .filter(|(s, _)| *s > 0)
        .max_by_key(|(s, _)| *s)
        .map(|(_, p)| p.to_string_lossy().into_owned())
}

/// `Microsoft.WindowsCalculator` -> `Windows Calculator`.
pub fn pretty_package_name(name: &str) -> String {
    let last = name.rsplit('.').next().unwrap_or(name);
    let mut out = String::new();
    let chars: Vec<char> = last.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if i > 0
            && c.is_uppercase()
            && (chars[i - 1].is_lowercase()
                || chars.get(i + 1).is_some_and(|n| n.is_lowercase())
                    && chars[i - 1].is_uppercase())
        {
            out.push(' ');
        }
        out.push(*c);
    }
    out
}

/// Splits an uninstall command line into the program and its arguments.
pub fn split_command(cmd: &str) -> (String, String) {
    let cmd = cmd.trim();
    if let Some(rest) = cmd.strip_prefix('"')
        && let Some(end) = rest.find('"')
    {
        return (rest[..end].to_string(), rest[end + 1..].trim().to_string());
    }
    let lower = cmd.to_lowercase();
    if let Some(pos) = lower.find(".exe") {
        let end = pos + 4;
        return (cmd[..end].to_string(), cmd[end..].trim().to_string());
    }
    match cmd.split_once(' ') {
        Some((a, b)) => (a.to_string(), b.trim().to_string()),
        None => (cmd.to_string(), String::new()),
    }
}

/// Starts the vendor uninstaller and waits for it. The uninstaller shows its own UI and asks
/// for elevation itself when it needs it.
#[cfg(windows)]
pub fn uninstall(app: &InstalledApp) -> Result<LaunchOutcome> {
    if let Some(full) = app.id.strip_prefix("store:") {
        if !full
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-~".contains(c))
        {
            return Err(Error::Other("unexpected package name".into()));
        }
        let status = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &format!("Remove-AppxPackage -Package '{full}'"),
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map_err(|e| Error::Other(e.to_string()))?;
        return Ok(LaunchOutcome::Exited(status.code().unwrap_or(1) as u32));
    }
    let cmd = app
        .uninstall
        .as_deref()
        .or(app.quiet_uninstall.as_deref())
        .ok_or_else(|| Error::Other("no uninstaller registered".into()))?;
    let (exe, args) = split_command(cmd);
    let lower = exe.to_lowercase();
    // MsiExec /I (repair/modify) is turned into /X so the button really uninstalls.
    let args = if lower.ends_with("msiexec.exe") || lower == "msiexec" {
        args.replacen("/I", "/X", 1).replacen("/i", "/X", 1)
    } else {
        args
    };
    run_and_wait(&exe, &args, false, true)
}

/// Best guess at an app's main exe when the registry names no icon: the exe in the install
/// folder whose name shares the most with the app name, ignoring uninstallers and updaters.
#[cfg(windows)]
fn main_exe(location: Option<&str>, name: &str) -> Option<String> {
    let dir = Path::new(location?.trim_matches('"'));
    let rd = std::fs::read_dir(dir).ok()?;
    let words: Vec<String> = name
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2)
        .map(str::to_string)
        .collect();
    rd.flatten()
        .filter_map(|e| {
            let file = e.file_name().to_string_lossy().to_lowercase();
            let is_exe = file.ends_with(".exe");
            let helper = [
                "unins", "uninst", "update", "setup", "crash", "helper", "elevate",
            ]
            .iter()
            .any(|h| file.contains(h));
            (is_exe && !helper).then(|| {
                (
                    words.iter().filter(|w| file.contains(w.as_str())).count(),
                    e.path(),
                )
            })
        })
        .max_by_key(|(score, _)| *score)
        .map(|(_, p)| p.to_string_lossy().into_owned())
}

/// True when an uninstall entry points at nothing: a Windows Installer product the installer
/// service no longer knows, or an uninstaller exe that no longer exists.
#[cfg(windows)]
fn entry_is_broken(key: &str, windows_installer: bool, uninstall: Option<&str>) -> bool {
    let is_guid = key.len() == 38 && key.starts_with('{') && key.ends_with('}');
    let msi = uninstall.is_some_and(|u| u.to_ascii_lowercase().contains("msiexec"));
    if is_guid && (windows_installer || msi) {
        use windows::Win32::System::ApplicationInstallationAndServicing::{
            INSTALLSTATE_UNKNOWN, MsiQueryProductStateW,
        };
        let w = crate::winstr::wide(key);
        let state = unsafe { MsiQueryProductStateW(windows::core::PCWSTR(w.as_ptr())) };
        return state == INSTALLSTATE_UNKNOWN;
    }
    let Some(cmd) = uninstall else { return false };
    let (exe, _) = split_command(cmd);
    let exe = crate::icons::expand_env(exe.trim_matches('"'));
    // Only absolute paths can be checked; bare names (rundll32, msiexec) resolve via PATH.
    let p = Path::new(&exe);
    p.is_absolute() && !p.exists()
}

#[cfg(windows)]
fn key_parts(id: &str) -> Result<(RegKey, u32, &str, &'static str)> {
    let (tag, sub) = id
        .split_once(':')
        .ok_or_else(|| Error::Other("unexpected app id".into()))?;
    if sub.is_empty() || sub.contains('\\') || sub.contains('/') {
        return Err(Error::Other("unexpected app id".into()));
    }
    Ok(match tag {
        "hkcu" => (RegKey::predef(HKEY_CURRENT_USER), 0, sub, "HKCU"),
        "hklm64" => (
            RegKey::predef(HKEY_LOCAL_MACHINE),
            KEY_WOW64_64KEY,
            sub,
            "HKLM",
        ),
        "hklm32" => (
            RegKey::predef(HKEY_LOCAL_MACHINE),
            KEY_WOW64_32KEY,
            sub,
            "HKLM",
        ),
        _ => {
            return Err(Error::Other(
                "only desktop app entries can be removed".into(),
            ));
        }
    })
}

/// Removes the Uninstall entry of an app that is already gone, after exporting it to a
/// `.reg` file in `backup_dir` so it can be restored with a double click. Only entries that
/// are still detected as broken are removed; a working app's entry is never touched.
#[cfg(windows)]
pub fn remove_broken_entry(id: &str, backup_dir: &Path) -> Result<String> {
    let app = desktop_apps()
        .into_iter()
        .find(|a| a.id == id)
        .ok_or_else(|| Error::Other("entry not found".into()))?;
    if !app.broken {
        return Err(Error::Other(
            "this app still looks installed; use Uninstall instead".into(),
        ));
    }
    let (hive, flags, sub, hive_name) = key_parts(id)?;
    if hive_name == "HKLM" && !crate::system::is_elevated() {
        return Err(Error::NeedsAdmin);
    }
    std::fs::create_dir_all(backup_dir).map_err(|e| Error::io(backup_dir, e))?;
    let safe: String = sub
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_{}.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let file = backup_dir.join(format!("uninstall-entry-{safe}-{stamp}.reg"));
    let view = if flags == KEY_WOW64_32KEY {
        "/reg:32"
    } else {
        "/reg:64"
    };
    let status = Command::new("reg.exe")
        .args([
            "export",
            &format!(r"{hive_name}\{UNINSTALL}\{sub}"),
            &file.to_string_lossy(),
            "/y",
            view,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| Error::Other(e.to_string()))?;
    if !status.success() || !file.is_file() {
        return Err(Error::Other(
            "could not back up the registry entry, nothing was removed".into(),
        ));
    }
    let root = hive
        .open_subkey_with_flags(UNINSTALL, winreg::enums::KEY_ALL_ACCESS | flags)
        .map_err(|e| Error::Other(e.to_string()))?;
    root.delete_subkey_all(sub)
        .map_err(|e| Error::Other(e.to_string()))?;
    Ok(file.to_string_lossy().into_owned())
}

#[cfg(not(windows))]
pub fn desktop_apps() -> Vec<InstalledApp> {
    Vec::new()
}

#[cfg(not(windows))]
pub fn store_apps() -> Result<Vec<InstalledApp>> {
    Ok(Vec::new())
}

#[cfg(not(windows))]
pub fn uninstall(_app: &InstalledApp) -> Result<LaunchOutcome> {
    Err(Error::Unsupported("uninstalling apps"))
}

#[cfg(not(windows))]
pub fn remove_broken_entry(_id: &str, _backup_dir: &Path) -> Result<String> {
    Err(Error::Unsupported("removing uninstall entries"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_uninstall_commands() {
        assert_eq!(
            split_command(r#""C:\Program Files\App\uninst.exe" /S"#),
            (r"C:\Program Files\App\uninst.exe".into(), "/S".into())
        );
        assert_eq!(
            split_command(r"MsiExec.exe /X{1234}"),
            ("MsiExec.exe".into(), "/X{1234}".into())
        );
        assert_eq!(
            split_command(r"C:\Program Files\App\un.exe --remove"),
            (r"C:\Program Files\App\un.exe".into(), "--remove".into())
        );
    }

    #[test]
    fn prettifies_package_names() {
        assert_eq!(
            pretty_package_name("Microsoft.WindowsCalculator"),
            "Windows Calculator"
        );
        assert_eq!(
            pretty_package_name("SpotifyAB.SpotifyMusic"),
            "Spotify Music"
        );
    }

    #[cfg(windows)]
    #[test]
    fn picks_the_unplated_store_logo() {
        let tmp = tempfile::tempdir().unwrap();
        let assets = tmp.path().join("Assets");
        std::fs::create_dir_all(&assets).unwrap();
        for f in [
            "Square44x44Logo.scale-100.png",
            "Square44x44Logo.targetsize-48.png",
            "Square44x44Logo.targetsize-48_altform-unplated.png",
            "Square44x44Logo.targetsize-48_contrast-black.png",
            "Other.png",
        ] {
            std::fs::write(assets.join(f), b"png").unwrap();
        }
        let got = store_logo(&tmp.path().to_string_lossy(), r"Assets\Square44x44Logo.png").unwrap();
        assert!(
            got.ends_with("Square44x44Logo.targetsize-48_altform-unplated.png"),
            "{got}"
        );
        assert!(store_logo(&tmp.path().to_string_lossy(), r"..\x.png").is_none());
    }

    #[test]
    fn lists_desktop_apps() {
        let apps = desktop_apps();
        assert!(apps.iter().all(|a| !a.name.is_empty()));
    }
}
