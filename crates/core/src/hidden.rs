//! Hidden space: why a drive's used space is larger than the folders a scan can add up.
//!
//! The gap is split into parts Windows keeps outside ordinary folders (shadow copies, reserved
//! storage, NTFS metadata), large system files, virtual disks that keep their peak size, and
//! folders the scan was not allowed to read. Everything here only reads; the UI links to
//! Windows' own screens for changes. Other systems report that the page is not available yet.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
#[cfg(windows)]
use windows::Win32::Foundation::{ERROR_SUCCESS, GetLastError};
#[cfg(windows)]
use windows::Win32::Storage::FileSystem::{
    DISK_SPACE_INFORMATION, GetCompressedFileSizeW, GetDiskSpaceInformationW, GetFileAttributesExW,
    GetFileExInfoStandard, GetVolumeInformationW, GetVolumeNameForVolumeMountPointW,
    WIN32_FILE_ATTRIBUTE_DATA,
};
#[cfg(windows)]
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
    CoSetProxyBlanket, CoUninitialize, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL,
    RPC_C_IMP_LEVEL_IMPERSONATE,
};
#[cfg(windows)]
use windows::Win32::System::Variant::{VARIANT, VariantClear, VariantToString};
#[cfg(windows)]
use windows::Win32::System::Wmi::{
    IWbemLocator, IWbemServices, WBEM_FLAG_FORWARD_ONLY, WBEM_FLAG_RETURN_IMMEDIATELY,
    WBEM_INFINITE, WbemLocator,
};
#[cfg(windows)]
use windows::core::{BSTR, HSTRING, PCWSTR};
#[cfg(windows)]
use winreg::RegKey;
#[cfg(windows)]
use winreg::enums::HKEY_CURRENT_USER;

use crate::explorer::Tree;
#[cfg(windows)]
use crate::paths::{env_path, is_within, key};
#[cfg(windows)]
use crate::system::{current_user_sid, recycle_bin_info_on, system_files_on};
#[cfg(windows)]
use crate::winstr::{from_wide, wide};
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PartKind {
    /// System Restore points and other shadow copies, in System Volume Information.
    ShadowCopies,
    /// Space Windows holds back for updates.
    ReservedStorage,
    PageFile,
    HiberFile,
    SwapFile,
    /// The NTFS master file table.
    Mft,
    RecycleBin,
    /// WSL and Docker disks (`ext4.vhdx`, `docker_data.vhdx`).
    VirtualDisk,
    /// Folders the scan could not read; their size is unknown.
    Unreadable,
    /// Online-only cloud files: listed in folders but not stored on this drive.
    CloudOnly,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HiddenPart {
    pub kind: PartKind,
    /// Space this part takes on the drive; `None` when it cannot be measured (yet).
    pub bytes: Option<u64>,
    /// Already counted by the scan, so it does not add to the gap.
    pub in_scan: bool,
    pub path: Option<String>,
    /// A distro name for virtual disks.
    pub label: Option<String>,
    /// The size limit of shadow storage.
    pub limit: Option<u64>,
    /// Shadow copies, unreadable folders or online-only files.
    pub count: Option<u64>,
    /// Measuring needs administrator rights.
    pub needs_admin: bool,
}

#[cfg(windows)]
impl HiddenPart {
    fn new(kind: PartKind, bytes: Option<u64>) -> Self {
        HiddenPart {
            kind,
            bytes,
            in_scan: false,
            path: None,
            label: None,
            limit: None,
            count: None,
            needs_admin: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HiddenSpace {
    /// `C:\`
    pub root: String,
    pub total: u64,
    pub free: u64,
    pub used: u64,
    /// Bytes the scan added up; `None` without a scan of the whole drive.
    pub scanned: Option<u64>,
    pub parts: Vec<HiddenPart>,
    /// Used minus scanned minus the parts outside the scan. Negative when the scan counts more
    /// than the drive stores (hard links and compressed files are counted at full size).
    pub unexplained: Option<i64>,
    /// Some of the folders that could not be read, shallowest first.
    pub denied_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ShadowStorage {
    /// `volume{guid}` (lowercase) of the volume that holds the storage.
    pub volume: String,
    pub used: u64,
    pub allocated: u64,
    pub max: u64,
    /// Shadow copies (restore points, backups) of this volume.
    pub copies: u32,
}

/// Drive totals from `GetDiskSpaceInformationW`, which also reports the storage reserve.
#[derive(Debug, Clone, Copy, Default)]
pub struct DiskSpace {
    pub total: u64,
    pub free: u64,
    pub reserved: u64,
}

#[cfg(windows)]
pub fn disk_space(root: &Path) -> Option<DiskSpace> {
    let w = wide(root);
    let mut info = DISK_SPACE_INFORMATION::default();
    unsafe { GetDiskSpaceInformationW(PCWSTR(w.as_ptr()), &mut info) }.ok()?;
    let unit = u64::from(info.SectorsPerAllocationUnit) * u64::from(info.BytesPerSector);
    Some(DiskSpace {
        total: info.ActualTotalAllocationUnits * unit,
        free: info.ActualAvailableAllocationUnits * unit,
        reserved: info.VolumeStorageReserveAllocationUnits * unit,
    })
}

/// File size from the directory entry, which works for NTFS metadata files such as `$MFT`
/// that cannot be opened.
#[cfg(windows)]
fn size_by_attributes(path: &Path) -> Option<u64> {
    let w = wide(path);
    let mut data = WIN32_FILE_ATTRIBUTE_DATA::default();
    unsafe {
        GetFileAttributesExW(
            PCWSTR(w.as_ptr()),
            GetFileExInfoStandard,
            &mut data as *mut _ as *mut core::ffi::c_void,
        )
    }
    .ok()?;
    Some((u64::from(data.nFileSizeHigh) << 32) | u64::from(data.nFileSizeLow))
}

/// Space a file really takes: less than its length for sparse and compressed files, which is
/// how WSL keeps its disks.
#[cfg(windows)]
fn allocated_size(path: &Path) -> Option<u64> {
    let w = wide(path);
    let mut high = 0u32;
    let low = unsafe { GetCompressedFileSizeW(PCWSTR(w.as_ptr()), Some(&mut high)) };
    if low == u32::MAX && unsafe { GetLastError() } != ERROR_SUCCESS {
        return None;
    }
    Some((u64::from(high) << 32) | u64::from(low))
}

#[cfg(windows)]
fn file_system(root: &Path) -> String {
    let w = wide(root);
    let mut fs = vec![0u16; 64];
    let _ =
        unsafe { GetVolumeInformationW(PCWSTR(w.as_ptr()), None, None, None, None, Some(&mut fs)) };
    from_wide(&fs)
}

/// `volume{guid}` inside a volume path or a WMI reference, lowercase.
pub fn volume_guid(s: &str) -> Option<String> {
    let lower = s.to_ascii_lowercase();
    let start = lower.find("volume{")?;
    let end = lower[start..].find('}')? + start;
    Some(lower[start..=end].to_string())
}

#[cfg(windows)]
fn volume_of(root: &Path) -> Option<String> {
    let w = wide(root);
    let mut buf = [0u16; 64];
    unsafe { GetVolumeNameForVolumeMountPointW(PCWSTR(w.as_ptr()), &mut buf) }.ok()?;
    volume_guid(&from_wide(&buf))
}

/// WSL distro disks (from `HKCU\...\Lxss`) and Docker Desktop's data disk.
#[cfg(windows)]
pub fn virtual_disks() -> Vec<(PathBuf, String)> {
    let mut out: Vec<(PathBuf, String)> = Vec::new();
    if let Ok(lxss) = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Lxss")
    {
        for id in lxss.enum_keys().flatten() {
            let Ok(k) = lxss.open_subkey(&id) else {
                continue;
            };
            let Ok(base) = k.get_value::<String, _>("BasePath") else {
                continue;
            };
            let base = base.strip_prefix(r"\\?\").unwrap_or(&base).to_string();
            let file = k
                .get_value::<String, _>("VhdFileName")
                .unwrap_or_else(|_| String::from("ext4.vhdx"));
            let name = k
                .get_value::<String, _>("DistributionName")
                .unwrap_or_else(|_| String::from("WSL"));
            out.push((PathBuf::from(base).join(file), name));
        }
    }
    if let Some(local) = env_path("LOCALAPPDATA") {
        for rel in [
            r"Docker\wsl\disk\docker_data.vhdx",
            r"Docker\wsl\data\ext4.vhdx",
        ] {
            out.push((local.join(rel), String::from("Docker Desktop")));
        }
    }
    let mut seen = Vec::new();
    out.retain(|(p, _)| {
        let k = key(p);
        let fresh = !seen.contains(&k) && p.is_absolute() && p.is_file();
        seen.push(k);
        fresh
    });
    out
}

/// Measures shadow storage through WMI. Needs administrator rights: a standard user gets an
/// access error, and the app then offers to measure through the elevated helper.
#[cfg(windows)]
pub fn shadow_storage() -> Result<Vec<ShadowStorage>> {
    std::thread::spawn(wmi_shadow_storage)
        .join()
        .map_err(|_| Error::Other(String::from("the WMI query failed")))?
}

#[cfg(windows)]
fn wmi_query(services: &IWbemServices, wql: &str, props: &[&str]) -> Result<Vec<Vec<String>>> {
    let rows = unsafe {
        services.ExecQuery(
            &BSTR::from("WQL"),
            &BSTR::from(wql),
            WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
            None,
        )?
    };
    let mut out = Vec::new();
    loop {
        let mut objs = [None];
        let mut returned = 0u32;
        unsafe { rows.Next(WBEM_INFINITE, &mut objs, &mut returned) }.ok()?;
        let Some(obj) = objs[0].take().filter(|_| returned > 0) else {
            break;
        };
        let mut row = Vec::with_capacity(props.len());
        for p in props {
            let name = HSTRING::from(*p);
            let mut value = VARIANT::default();
            let mut buf = [0u16; 512];
            let text = unsafe {
                obj.Get(PCWSTR(name.as_ptr()), 0, &mut value, None, None)
                    .and_then(|_| VariantToString(&value, &mut buf))
            }
            .map(|_| from_wide(&buf))
            .unwrap_or_default();
            let _ = unsafe { VariantClear(&mut value) };
            row.push(text);
        }
        out.push(row);
    }
    Ok(out)
}

#[cfg(windows)]
fn wmi_shadow_storage() -> Result<Vec<ShadowStorage>> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok()?;
    let res = (|| -> Result<Vec<ShadowStorage>> {
        let locator: IWbemLocator =
            unsafe { CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER)? };
        let services = unsafe {
            locator.ConnectServer(
                &BSTR::from(r"ROOT\CIMV2"),
                &BSTR::new(),
                &BSTR::new(),
                &BSTR::new(),
                0,
                &BSTR::new(),
                None,
            )?
        };
        // RPC_C_AUTHN_WINNT and RPC_C_AUTHZ_NONE.
        unsafe {
            CoSetProxyBlanket(
                &services,
                10,
                0,
                None,
                RPC_C_AUTHN_LEVEL_CALL,
                RPC_C_IMP_LEVEL_IMPERSONATE,
                None,
                EOAC_NONE,
            )?
        };
        let storage = wmi_query(
            &services,
            "SELECT DiffVolume, UsedSpace, AllocatedSpace, MaxSpace FROM Win32_ShadowStorage",
            &["DiffVolume", "UsedSpace", "AllocatedSpace", "MaxSpace"],
        )?;
        let copies = wmi_query(
            &services,
            "SELECT VolumeName FROM Win32_ShadowCopy",
            &["VolumeName"],
        )?;
        let num = |s: &str| s.trim().parse::<u64>().unwrap_or(0);
        Ok(storage
            .iter()
            .filter_map(|r| {
                let volume = volume_guid(&r[0])?;
                Some(ShadowStorage {
                    copies: copies
                        .iter()
                        .filter(|c| volume_guid(&c[0]).as_deref() == Some(volume.as_str()))
                        .count() as u32,
                    used: num(&r[1]),
                    allocated: num(&r[2]),
                    max: num(&r[3]),
                    volume,
                })
            })
            .collect())
    })();
    unsafe { CoUninitialize() };
    res
}

/// Whether the scan counted `path`: the file itself is a node, or its folder was read.
#[cfg(windows)]
fn counted(tree: &Tree, path: &Path) -> bool {
    if tree.find(path).is_some() {
        return true;
    }
    path.parent()
        .and_then(|p| tree.find(p))
        .is_some_and(|id| !tree.nodes[id as usize].denied)
}

/// The tree when it is a full scan of the drive at `root`.
#[cfg(windows)]
fn drive_scan<'a>(tree: Option<&'a Tree>, root: &Path) -> Option<&'a Tree> {
    tree.filter(|t| key(&t.root_path) == key(root) && t.summary.selection.is_empty())
}

/// Explains the used space of the drive at `root` (`C:\`). `tree` is the latest space map;
/// it is used only when it covers the whole drive. `shadow` is a shadow storage measurement,
/// if one was made.
#[cfg(windows)]
pub fn hidden_space(
    root: &Path,
    tree: Option<&Tree>,
    shadow: Option<&[ShadowStorage]>,
) -> Result<HiddenSpace> {
    let space =
        disk_space(root).ok_or_else(|| Error::Other(format!("no drive at {}", root.display())))?;
    let used = space.total.saturating_sub(space.free);
    let tree = drive_scan(tree, root);
    let mark = |mut part: HiddenPart, path: &Path| {
        part.in_scan = tree.is_some_and(|t| counted(t, path));
        part.path = Some(path.to_string_lossy().into_owned());
        part
    };
    let mut parts = Vec::new();

    let volume = volume_of(root);
    let mut shadow_part = HiddenPart::new(PartKind::ShadowCopies, None);
    match shadow {
        Some(list) => {
            let mine: Vec<&ShadowStorage> = list
                .iter()
                .filter(|s| volume.as_deref() == Some(s.volume.as_str()))
                .collect();
            shadow_part.bytes = Some(mine.iter().map(|s| s.used).sum());
            shadow_part.limit = mine.iter().map(|s| s.max).max();
            shadow_part.count = Some(mine.iter().map(|s| u64::from(s.copies)).sum());
        }
        None => shadow_part.needs_admin = true,
    }
    parts.push(shadow_part);

    // Zero means reserved storage is off on this PC; the UI says so instead of hiding it.
    parts.push(HiddenPart::new(
        PartKind::ReservedStorage,
        Some(space.reserved),
    ));
    for f in system_files_on(root) {
        let kind = match f.name.as_str() {
            "pagefile.sys" => PartKind::PageFile,
            "hiberfil.sys" => PartKind::HiberFile,
            _ => PartKind::SwapFile,
        };
        parts.push(mark(
            HiddenPart::new(kind, Some(f.bytes)),
            Path::new(&f.path),
        ));
    }
    if file_system(root).eq_ignore_ascii_case("NTFS")
        && let Some(bytes) = size_by_attributes(&root.join("$MFT"))
    {
        let mut part = HiddenPart::new(PartKind::Mft, Some(bytes));
        part.path = Some(root.join("$MFT").to_string_lossy().into_owned());
        parts.push(part);
    }
    let bin = recycle_bin_info_on(root);
    if bin.bytes > 0 {
        // Windows reports the current user's bin, which lives in a folder named after their
        // SID; a scan often cannot open it.
        let own = root
            .join("$Recycle.Bin")
            .join(current_user_sid().unwrap_or_default());
        let mut part = mark(
            HiddenPart::new(PartKind::RecycleBin, Some(bin.bytes)),
            &own.join("x"),
        );
        part.path = Some(own.to_string_lossy().into_owned());
        part.count = Some(bin.items);
        parts.push(part);
    }
    let root_key = key(root);
    for (path, label) in virtual_disks() {
        if !is_within(&key(&path), &root_key) {
            continue;
        }
        let bytes = allocated_size(&path).or_else(|| path.metadata().ok().map(|m| m.len()));
        let mut part = mark(HiddenPart::new(PartKind::VirtualDisk, bytes), &path);
        part.label = Some(label);
        parts.push(part);
    }

    let mut denied_paths = Vec::new();
    if let Some(t) = tree {
        if t.summary.denied > 0 {
            let mut part = HiddenPart::new(PartKind::Unreadable, None);
            part.count = Some(t.summary.denied);
            parts.push(part);
            denied_paths = t.denied_paths(12);
        }
        if t.summary.cloud_files > 0 {
            let mut part = HiddenPart::new(PartKind::CloudOnly, Some(t.summary.cloud_bytes));
            part.count = Some(t.summary.cloud_files);
            parts.push(part);
        }
    }

    let scanned = tree.map(|t| t.summary.bytes);
    let unexplained = scanned.map(|s| {
        let outside: u64 = parts
            .iter()
            .filter(|p| !p.in_scan && p.kind != PartKind::CloudOnly)
            .filter_map(|p| p.bytes)
            .sum();
        used as i64 - s as i64 - outside as i64
    });
    Ok(HiddenSpace {
        root: root.to_string_lossy().into_owned(),
        total: space.total,
        free: space.free,
        used,
        scanned,
        parts,
        unexplained,
        denied_paths,
    })
}

#[cfg(not(windows))]
pub fn disk_space(_root: &Path) -> Option<DiskSpace> {
    None
}

#[cfg(not(windows))]
pub fn virtual_disks() -> Vec<(PathBuf, String)> {
    Vec::new()
}

#[cfg(not(windows))]
pub fn shadow_storage() -> Result<Vec<ShadowStorage>> {
    Err(Error::Unsupported("shadow storage"))
}

#[cfg(not(windows))]
pub fn hidden_space(
    _root: &Path,
    _tree: Option<&Tree>,
    _shadow: Option<&[ShadowStorage]>,
) -> Result<HiddenSpace> {
    Err(Error::Unsupported("explaining hidden space"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_volume_guids() {
        assert_eq!(
            volume_guid(r#"Win32_Volume.DeviceID="\\\\?\\Volume{3A9F-11}\\""#).as_deref(),
            Some("volume{3a9f-11}")
        );
        assert_eq!(
            volume_guid(r"\\?\Volume{abc}\").as_deref(),
            Some("volume{abc}")
        );
        assert!(volume_guid("C:").is_none());
    }

    #[cfg(windows)]
    #[test]
    fn explains_the_system_drive_without_admin() {
        let sys = PathBuf::from(format!(
            "{}\\",
            std::env::var("SYSTEMDRIVE").unwrap_or_else(|_| "C:".into())
        ));
        let h = hidden_space(&sys, None, None).unwrap();
        assert!(h.total > 0 && h.used > 0 && h.used <= h.total);
        assert!(h.scanned.is_none() && h.unexplained.is_none());
        let shadow = h
            .parts
            .iter()
            .find(|p| p.kind == PartKind::ShadowCopies)
            .unwrap();
        assert!(shadow.needs_admin && shadow.bytes.is_none());
        let mft = h.parts.iter().find(|p| p.kind == PartKind::Mft).unwrap();
        assert!(
            mft.bytes.unwrap() > 0,
            "the MFT size is readable without admin"
        );
    }

    #[cfg(windows)]
    #[test]
    fn reconciles_a_scan_with_parts_outside_it() {
        use crate::job::Job;
        // A scan of a temp folder stands in for a drive scan: only the arithmetic is checked.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.bin"), [0u8; 100]).unwrap();
        let tree = Tree::scan(tmp.path(), &Job::new());
        assert!(drive_scan(Some(&tree), tmp.path()).is_some());
        assert!(drive_scan(Some(&tree), Path::new(r"Q:\")).is_none());
        assert!(counted(&tree, &tmp.path().join("a.bin")));
        assert!(counted(&tree, &tmp.path().join("small-file-folded.bin")));
        assert!(!counted(&tree, &tmp.path().join(r"missing\x.bin")));
    }
}
