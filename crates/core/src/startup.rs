//! Startup apps from the Run keys and Startup folders. Enabling and disabling writes the
//! `StartupApproved` values, exactly what Task Manager does, so it is reversible and never
//! deletes the entry.

use std::path::PathBuf;

use serde::Serialize;
use winreg::RegKey;
use winreg::enums::{
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE, KEY_WOW64_32KEY,
    KEY_WOW64_64KEY, REG_BINARY,
};
use winreg::{RegValue, enums::RegType};

use crate::paths::env_path;
use crate::{Error, Result};

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";

#[derive(Debug, Clone, Copy, Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum StartupSource {
    RunUser,
    RunMachine,
    RunMachine32,
    FolderUser,
    FolderCommon,
}

impl StartupSource {
    fn approved_key(self) -> (&'static str, bool) {
        match self {
            StartupSource::RunUser => ("Run", false),
            StartupSource::RunMachine => ("Run", true),
            StartupSource::RunMachine32 => ("Run32", true),
            StartupSource::FolderUser => ("StartupFolder", false),
            StartupSource::FolderCommon => ("StartupFolder", true),
        }
    }

    pub fn needs_admin(self) -> bool {
        self.approved_key().1
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupItem {
    pub name: String,
    pub command: String,
    pub source: StartupSource,
    pub enabled: bool,
    pub needs_admin: bool,
}

fn approved_enabled(source: StartupSource, name: &str) -> bool {
    let (sub, machine) = source.approved_key();
    let hive = RegKey::predef(if machine {
        HKEY_LOCAL_MACHINE
    } else {
        HKEY_CURRENT_USER
    });
    let Ok(k) = hive.open_subkey_with_flags(format!(r"{APPROVED}\{sub}"), KEY_READ) else {
        return true;
    };
    match k.get_raw_value(name) {
        // First byte: even means enabled (0x02, 0x06), odd means disabled (0x01, 0x03).
        Ok(v) => v.bytes.first().is_none_or(|b| b % 2 == 0),
        Err(_) => true,
    }
}

fn read_run(hive: &RegKey, flags: u32, source: StartupSource, out: &mut Vec<StartupItem>) {
    let Ok(k) = hive.open_subkey_with_flags(RUN, KEY_READ | flags) else {
        return;
    };
    for (name, value) in k.enum_values().flatten() {
        if name.is_empty() {
            continue;
        }
        let command = match value.vtype {
            RegType::REG_SZ | RegType::REG_EXPAND_SZ => {
                let mut v = value.to_string();
                v.retain(|c| c != '\0');
                v
            }
            _ => continue,
        };
        out.push(StartupItem {
            enabled: approved_enabled(source, &name),
            needs_admin: source.needs_admin(),
            name,
            command,
            source,
        });
    }
}

fn read_folder(dir: Option<PathBuf>, source: StartupSource, out: &mut Vec<StartupItem>) {
    let Some(dir) = dir else { return };
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return;
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.eq_ignore_ascii_case("desktop.ini")
            || !e.file_type().map(|t| t.is_file()).unwrap_or(false)
        {
            continue;
        }
        out.push(StartupItem {
            enabled: approved_enabled(source, &name),
            needs_admin: source.needs_admin(),
            command: e.path().to_string_lossy().into_owned(),
            name,
            source,
        });
    }
}

pub fn list() -> Vec<StartupItem> {
    let mut out = Vec::new();
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    read_run(&hkcu, 0, StartupSource::RunUser, &mut out);
    read_run(&hklm, KEY_WOW64_64KEY, StartupSource::RunMachine, &mut out);
    read_run(
        &hklm,
        KEY_WOW64_32KEY,
        StartupSource::RunMachine32,
        &mut out,
    );
    read_folder(
        env_path("APPDATA").map(|p| p.join(r"Microsoft\Windows\Start Menu\Programs\Startup")),
        StartupSource::FolderUser,
        &mut out,
    );
    read_folder(
        env_path("PROGRAMDATA").map(|p| p.join(r"Microsoft\Windows\Start Menu\Programs\StartUp")),
        StartupSource::FolderCommon,
        &mut out,
    );
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

/// Enables or disables an item through `StartupApproved`.
pub fn set_enabled(source: StartupSource, name: &str, enabled: bool) -> Result<()> {
    let (sub, machine) = source.approved_key();
    if machine && !crate::system::is_elevated() {
        return Err(Error::NeedsAdmin);
    }
    let hive = RegKey::predef(if machine {
        HKEY_LOCAL_MACHINE
    } else {
        HKEY_CURRENT_USER
    });
    let (k, _) = hive
        .create_subkey_with_flags(format!(r"{APPROVED}\{sub}"), KEY_SET_VALUE)
        .map_err(|e| Error::Other(e.to_string()))?;
    let mut bytes = vec![0u8; 12];
    if enabled {
        bytes[0] = 0x02;
    } else {
        bytes[0] = 0x03;
        // Bytes 4..12 hold the FILETIME of the change; Task Manager shows it as "disabled on".
        let ft = (chrono::Utc::now().timestamp() + 11_644_473_600) as u64 * 10_000_000;
        bytes[4..12].copy_from_slice(&ft.to_le_bytes());
    }
    k.set_raw_value(
        name,
        &RegValue {
            bytes: bytes.into(),
            vtype: REG_BINARY,
        },
    )
    .map_err(|e| Error::Other(e.to_string()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn lists_without_panicking() {
        let items = super::list();
        assert!(items.iter().all(|i| !i.name.is_empty()));
    }
}
