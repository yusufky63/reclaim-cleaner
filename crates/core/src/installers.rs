//! Old installers, disk images and archives left in Downloads and on the Desktop. Always a
//! "review" item: the app only points at them.

use std::fs;
use std::path::PathBuf;

use serde::Serialize;

use crate::fsmeta::{is_cloud_only, is_link, mtime, now_unix};

const EXTENSIONS: &[&str] = &[
    "exe",
    "msi",
    "msix",
    "msixbundle",
    "appx",
    "appxbundle",
    "iso",
    "img",
    "zip",
    "7z",
    "rar",
    "dmg",
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallerFile {
    pub path: String,
    pub name: String,
    pub bytes: u64,
    pub mtime: i64,
    pub ext: String,
}

/// Files with installer or archive extensions directly inside `dirs` (one level of subfolders
/// is included) that are older than `min_age_days`.
pub fn find_installers(dirs: &[PathBuf], min_age_days: u32) -> Vec<InstallerFile> {
    let cutoff = now_unix() - i64::from(min_age_days) * 86_400;
    let mut out = Vec::new();
    let mut stack: Vec<(PathBuf, u8)> = dirs.iter().map(|d| (d.clone(), 0)).collect();
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if is_link(&ft) {
                continue;
            }
            if ft.is_dir() {
                if depth == 0 {
                    stack.push((e.path(), 1));
                }
                continue;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            let ext = name
                .rsplit_once('.')
                .map(|(_, x)| x.to_lowercase())
                .unwrap_or_default();
            if !EXTENSIONS.contains(&ext.as_str()) {
                continue;
            }
            let Ok(md) = e.metadata() else { continue };
            if is_cloud_only(&md) || mtime(&md) > cutoff {
                continue;
            }
            out.push(InstallerFile {
                path: e.path().to_string_lossy().into_owned(),
                name,
                bytes: md.len(),
                mtime: mtime(&md),
                ext,
            });
        }
    }
    out.sort_by_key(|x| std::cmp::Reverse(x.bytes));
    out
}
