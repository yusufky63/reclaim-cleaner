//! Installed programs from the registry Uninstall keys (the same source as Settings > Apps)
//! plus Store apps through PowerShell.

use std::collections::HashSet;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};
use winreg::RegKey;
use winreg::enums::{
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
};

use crate::system::{LaunchOutcome, run_and_wait};
use crate::{Error, Result};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
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
}

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
            icon: s("DisplayIcon"),
            uninstall,
            quiet_uninstall: s("QuietUninstallString"),
            scope: scope.to_string(),
            source: String::from("desktop"),
        });
    }
}

/// Desktop programs, deduplicated by name and version (some installers register twice).
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

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct AppxRow {
    name: String,
    package_full_name: String,
    version: Option<String>,
    publisher_display_name: Option<String>,
    display_name: Option<String>,
    install_location: Option<String>,
}

/// Store apps installed for the current user, excluding frameworks and system apps.
pub fn store_apps() -> Result<Vec<InstalledApp>> {
    let script = r#"$ErrorActionPreference='SilentlyContinue'; [Console]::OutputEncoding=[Text.Encoding]::UTF8;
Get-AppxPackage -PackageTypeFilter Main | Where-Object { $_.SignatureKind -eq 'Store' -and -not $_.IsFramework -and -not $_.NonRemovable } | ForEach-Object {
  $m = $null; try { $m = Get-AppxPackageManifest -Package $_.PackageFullName } catch {}
  $dn = $null; $pub = $null
  if ($m) { $dn = $m.Package.Properties.DisplayName; $pub = $m.Package.Properties.PublisherDisplayName }
  [pscustomobject]@{ Name=$_.Name; PackageFullName=$_.PackageFullName; Version=[string]$_.Version; PublisherDisplayName=$pub; DisplayName=$dn; InstallLocation=$_.InstallLocation }
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
        .into_iter()
        .map(|r| {
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
                bytes: None,
                location: r.install_location,
                icon: None,
                uninstall: None,
                quiet_uninstall: None,
                scope: String::from("user"),
                source: String::from("store"),
            }
        })
        .collect();
    apps.sort_by_key(|a| a.name.to_lowercase());
    Ok(apps)
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

    #[test]
    fn lists_desktop_apps() {
        let apps = desktop_apps();
        assert!(apps.iter().all(|a| !a.name.is_empty()));
    }
}
