//! Windows facts: drives, elevation, running processes, the Recycle Bin, known folders and the
//! OS version. Thin wrappers over Win32 so the rest of the engine stays platform-neutral.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Serialize;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::Storage::FileSystem::{
    GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDriveStringsW, GetVolumeInformationW,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, INFINITE, OpenProcessToken, WaitForSingleObject,
};
use windows::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Music, FOLDERID_Pictures,
    FOLDERID_Videos, KNOWN_FOLDER_FLAG, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI, SHERB_NOSOUND, SHEmptyRecycleBinW,
    SHGetKnownFolderPath, SHQUERYRBINFO, SHQueryRecycleBinW, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, SW_SHOWNORMAL};
use windows::core::{GUID, PCWSTR, w};
use winreg::RegKey;
use winreg::enums::HKEY_LOCAL_MACHINE;

use crate::winstr::{from_wide, wide};
use crate::{Error, Result};

const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Drive {
    /// `C:\`
    pub root: String,
    pub label: String,
    pub file_system: String,
    pub total: u64,
    pub free: u64,
    pub removable: bool,
    pub system: bool,
}

pub fn drives() -> Vec<Drive> {
    let mut buf = vec![0u16; 512];
    let len = unsafe { GetLogicalDriveStringsW(Some(&mut buf)) } as usize;
    let sysdrive = std::env::var("SYSTEMDRIVE")
        .unwrap_or_else(|_| "C:".into())
        .to_uppercase();
    let mut out = Vec::new();
    for root in buf[..len.min(buf.len())]
        .split(|&c| c == 0)
        .filter(|s| !s.is_empty())
    {
        let root_str = String::from_utf16_lossy(root);
        let w_root = wide(&root_str);
        let kind = unsafe { GetDriveTypeW(PCWSTR(w_root.as_ptr())) };
        if kind != DRIVE_FIXED && kind != DRIVE_REMOVABLE {
            continue;
        }
        let (mut free, mut total, mut _total_free) = (0u64, 0u64, 0u64);
        if unsafe {
            GetDiskFreeSpaceExW(
                PCWSTR(w_root.as_ptr()),
                Some(&mut free),
                Some(&mut total),
                Some(&mut _total_free),
            )
        }
        .is_err()
        {
            continue;
        }
        let mut label = vec![0u16; 261];
        let mut fs = vec![0u16; 64];
        let _ = unsafe {
            GetVolumeInformationW(
                PCWSTR(w_root.as_ptr()),
                Some(&mut label),
                None,
                None,
                None,
                Some(&mut fs),
            )
        };
        out.push(Drive {
            system: root_str.to_uppercase().starts_with(&sysdrive),
            root: root_str,
            label: from_wide(&label),
            file_system: from_wide(&fs),
            total,
            free,
            removable: kind == DRIVE_REMOVABLE,
        });
    }
    out
}

/// Free bytes on the volume holding `path`.
pub fn free_space(path: &Path) -> Option<u64> {
    let w_path = wide(path);
    let mut free = 0u64;
    unsafe { GetDiskFreeSpaceExW(PCWSTR(w_path.as_ptr()), Some(&mut free), None, None) }.ok()?;
    Some(free)
}

pub fn is_elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut core::ffi::c_void),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        );
        let _ = CloseHandle(token);
        ok.is_ok() && elevation.TokenIsElevated != 0
    }
}

/// Lowercase executable names of every running process.
pub fn running_processes() -> HashSet<String> {
    let mut out = HashSet::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return out;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snap, &mut entry).is_ok() {
            loop {
                out.insert(from_wide(&entry.szExeFile).to_lowercase());
                if Process32NextW(snap, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    out
}

#[derive(Debug, Clone, Copy, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RecycleBinInfo {
    pub bytes: u64,
    pub items: u64,
}

/// Size of the Recycle Bin on all drives.
pub fn recycle_bin_info() -> RecycleBinInfo {
    let mut info = SHQUERYRBINFO {
        cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32,
        ..Default::default()
    };
    match unsafe { SHQueryRecycleBinW(PCWSTR::null(), &mut info) } {
        Ok(()) => RecycleBinInfo {
            bytes: info.i64Size.max(0) as u64,
            items: info.i64NumItems.max(0) as u64,
        },
        Err(_) => RecycleBinInfo::default(),
    }
}

/// Empties the Recycle Bin on all drives without the shell's own confirmation; the app asks
/// first.
pub fn empty_recycle_bin() -> Result<()> {
    let flags = SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND;
    match unsafe { SHEmptyRecycleBinW(None, PCWSTR::null(), flags) } {
        Ok(()) => Ok(()),
        // E_UNEXPECTED is returned when the bin is already empty.
        Err(e) if e.code().0 as u32 == 0x8000_FFFF => Ok(()),
        Err(e) => Err(e.into()),
    }
}

fn known_folder(id: &GUID) -> Option<PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(id, KNOWN_FOLDER_FLAG(0), None).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const core::ffi::c_void));
        s.map(PathBuf::from)
    }
}

/// The user's library folders, following redirection (for example to OneDrive).
pub fn known_folders() -> Vec<PathBuf> {
    [
        FOLDERID_Desktop,
        FOLDERID_Documents,
        FOLDERID_Downloads,
        FOLDERID_Pictures,
        FOLDERID_Music,
        FOLDERID_Videos,
    ]
    .iter()
    .filter_map(known_folder)
    .collect()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryFolders {
    pub desktop: Option<String>,
    pub documents: Option<String>,
    pub downloads: Option<String>,
    pub pictures: Option<String>,
    pub music: Option<String>,
    pub videos: Option<String>,
}

pub fn library_folders() -> LibraryFolders {
    let s = |id: &GUID| known_folder(id).map(|p| p.to_string_lossy().into_owned());
    LibraryFolders {
        desktop: s(&FOLDERID_Desktop),
        documents: s(&FOLDERID_Documents),
        downloads: s(&FOLDERID_Downloads),
        pictures: s(&FOLDERID_Pictures),
        music: s(&FOLDERID_Music),
        videos: s(&FOLDERID_Videos),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OsInfo {
    pub name: String,
    pub display_version: String,
    pub build: String,
}

pub fn os_info() -> OsInfo {
    let key = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");
    let get = |name: &str| {
        key.as_ref()
            .ok()
            .and_then(|k| k.get_value::<String, _>(name).ok())
            .unwrap_or_default()
    };
    let build = get("CurrentBuild");
    let mut name = get("ProductName");
    // ProductName still says "Windows 10" on Windows 11; the build number is the reliable signal.
    if build.parse::<u32>().unwrap_or(0) >= 22000 {
        name = name.replace("Windows 10", "Windows 11");
    }
    let ubr = key
        .as_ref()
        .ok()
        .and_then(|k| k.get_value::<u32, _>("UBR").ok());
    OsInfo {
        name,
        display_version: get("DisplayVersion"),
        build: match ubr {
            Some(u) => format!("{build}.{u}"),
            None => build,
        },
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemFile {
    pub name: String,
    pub path: String,
    pub bytes: u64,
}

/// `hiberfil.sys`, `pagefile.sys` and `swapfile.sys` on the system drive. Read from the
/// directory listing, because opening them fails while Windows holds them.
pub fn system_files() -> Vec<SystemFile> {
    let root = format!(
        "{}\\",
        std::env::var("SYSTEMDRIVE").unwrap_or_else(|_| "C:".into())
    );
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&root) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_lowercase();
            if matches!(
                name.as_str(),
                "hiberfil.sys" | "pagefile.sys" | "swapfile.sys"
            ) && let Ok(md) = e.metadata()
            {
                out.push(SystemFile {
                    name,
                    path: e.path().to_string_lossy().into_owned(),
                    bytes: md.len(),
                });
            }
        }
    }
    out
}

/// Opens a file, folder or URL with its default handler.
pub fn shell_open(target: &str) -> Result<()> {
    let file = wide(target);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        lpVerb: w!("open"),
        lpFile: PCWSTR(file.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }.map_err(Error::from)
}

/// How a launched process ended.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum LaunchOutcome {
    Exited(u32),
    /// The user declined the UAC prompt.
    Cancelled,
}

/// Runs `exe` with `params`, optionally elevated (`runas`), and waits for it to finish.
/// `visible` controls whether a console window is shown, which we want for long system tools
/// like DISM so the user sees their progress.
pub fn run_and_wait(
    exe: &str,
    params: &str,
    elevated: bool,
    visible: bool,
) -> Result<LaunchOutcome> {
    let file = wide(exe);
    let args = wide(params);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: if elevated { w!("runas") } else { w!("open") },
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(args.as_ptr()),
        nShow: if visible { SW_SHOWNORMAL.0 } else { SW_HIDE.0 },
        ..Default::default()
    };
    if let Err(e) = unsafe { ShellExecuteExW(&mut info) } {
        // ERROR_CANCELLED: the user said no to UAC.
        if e.code().0 as u32 == 0x8007_04C7 {
            return Ok(LaunchOutcome::Cancelled);
        }
        return Err(e.into());
    }
    if info.hProcess.is_invalid() {
        return Ok(LaunchOutcome::Exited(0));
    }
    unsafe {
        let wait = WaitForSingleObject(info.hProcess, INFINITE);
        let mut code = 0u32;
        let _ = GetExitCodeProcess(info.hProcess, &mut code);
        let _ = CloseHandle(info.hProcess);
        if wait != WAIT_OBJECT_0 {
            return Err(Error::Other("waiting for the process failed".into()));
        }
        Ok(LaunchOutcome::Exited(code))
    }
}

/// Starts a process without waiting (used to relaunch the app elevated).
pub fn launch(exe: &str, params: &str, elevated: bool) -> Result<LaunchOutcome> {
    let file = wide(exe);
    let args = wide(params);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        lpVerb: if elevated { w!("runas") } else { w!("open") },
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(args.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    match unsafe { ShellExecuteExW(&mut info) } {
        Ok(()) => Ok(LaunchOutcome::Exited(0)),
        Err(e) if e.code().0 as u32 == 0x8007_04C7 => Ok(LaunchOutcome::Cancelled),
        Err(e) => Err(e.into()),
    }
}

/// True if an executable with this name can be found on `PATH`.
pub fn which(exe: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exts: Vec<String> = if Path::new(exe).extension().is_some() {
        vec![String::new()]
    } else {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT".into())
            .split(';')
            .map(|s| s.to_string())
            .collect()
    };
    for dir in std::env::split_paths(&path) {
        for ext in &exts {
            let candidate = dir.join(format!("{exe}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_system_drive_and_processes() {
        let d = drives();
        assert!(d.iter().any(|d| d.system), "system drive listed");
        assert!(d.iter().all(|d| d.total >= d.free));
        let procs = running_processes();
        assert!(!procs.is_empty());
        assert!(!os_info().build.is_empty());
        assert!(!known_folders().is_empty());
        assert!(which("cmd").is_some());
    }
}
