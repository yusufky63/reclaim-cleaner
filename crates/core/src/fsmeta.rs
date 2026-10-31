//! Small helpers over file metadata.

use std::fs::{FileType, Metadata};
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(windows)]
const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
#[cfg(windows)]
const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
#[cfg(windows)]
const FILE_ATTRIBUTE_OFFLINE: u32 = 0x1000;
#[cfg(windows)]
const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x4_0000;
#[cfg(windows)]
const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;

/// True for symlinks and junctions (name-surrogate reparse points). These are never followed:
/// a junction inside a cache folder must not let a scan or a delete escape into other data.
/// Other reparse points, such as OneDrive folders, are regular folders for our purposes.
pub fn is_link(ft: &FileType) -> bool {
    ft.is_symlink()
}

/// Online-only cloud files (OneDrive and other sync providers). Reading them triggers a
/// download, and they take no local space, so they are neither hashed nor counted.
#[cfg(windows)]
pub fn is_cloud_only(md: &Metadata) -> bool {
    md.file_attributes()
        & (FILE_ATTRIBUTE_OFFLINE
            | FILE_ATTRIBUTE_RECALL_ON_OPEN
            | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS)
        != 0
}

/// Linux has no placeholder attribute. Cloud drives there are FUSE mounts, which need a mount
/// check rather than a file check.
#[cfg(not(windows))]
pub fn is_cloud_only(_md: &Metadata) -> bool {
    false
}

#[cfg(windows)]
pub fn is_hidden_system(md: &Metadata) -> bool {
    let a = md.file_attributes();
    a & FILE_ATTRIBUTE_HIDDEN != 0 && a & FILE_ATTRIBUTE_SYSTEM != 0
}

#[cfg(not(windows))]
pub fn is_hidden_system(_md: &Metadata) -> bool {
    false
}

/// Last write time as Unix seconds; 0 when unknown. Timestamps more than a day in the future
/// (broken archives and copy tools produce dates like 2106) count as unknown, so one bad file
/// cannot make a whole folder look recently used.
pub fn mtime(md: &Metadata) -> i64 {
    let t = md.modified().ok().map(to_unix).unwrap_or(0);
    if t > latest_plausible() { 0 } else { t }
}

fn latest_plausible() -> i64 {
    static CELL: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *CELL.get_or_init(|| now_unix() + 86_400)
}

pub fn to_unix(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    }
}

pub fn now_unix() -> i64 {
    to_unix(SystemTime::now())
}
