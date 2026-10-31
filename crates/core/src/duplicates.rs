//! Duplicate finder: group by size, then by a quick hash of the first and last 16 KB, then by
//! a full BLAKE3 hash. Each stage only reads files that are still candidates, the pipeline
//! Czkawka popularised.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
#[cfg(windows)]
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::Serialize;
#[cfg(windows)]
use windows::Win32::Foundation::HANDLE;
#[cfg(windows)]
use windows::Win32::Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle};
use xxhash_rust::xxh3::Xxh3;

use crate::fsmeta::{is_cloud_only, is_link, mtime};
use crate::job::Job;
use crate::paths::{Guard, Origin};

const EDGE: u64 = 16 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DupFile {
    pub path: String,
    pub mtime: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DupGroup {
    pub hash: String,
    pub bytes: u64,
    pub files: Vec<DupFile>,
    /// Space freed by keeping one copy.
    pub wasted: u64,
}

struct Candidate {
    path: PathBuf,
    size: u64,
    mtime: i64,
}

fn collect(
    dir: &Path,
    min: u64,
    guard: &Guard,
    job: &Job,
    out: &parking_lot::Mutex<Vec<Candidate>>,
) {
    if job.is_cancelled() {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else {
        job.add_error();
        return;
    };
    job.add_dir();
    let mut local = Vec::new();
    let mut subdirs = Vec::new();
    for e in rd.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        if is_link(&ft) {
            continue;
        }
        let path = e.path();
        if ft.is_dir() {
            let name = e.file_name().to_string_lossy().to_lowercase();
            if name == ".git"
                || name == "node_modules"
                || name == "$recycle.bin"
                || name == "system volume information"
            {
                continue;
            }
            subdirs.push(path);
            continue;
        }
        let Ok(md) = e.metadata() else { continue };
        if is_cloud_only(&md) || md.len() < min {
            continue;
        }
        local.push(Candidate {
            path,
            size: md.len(),
            mtime: mtime(&md),
        });
    }
    job.add_files(local.len() as u64, local.iter().map(|c| c.size).sum());
    if !local.is_empty() {
        out.lock().extend(local);
    }
    let _ = guard;
    subdirs
        .par_iter()
        .for_each(|d| collect(d, min, guard, job, out));
}

fn quick_hash(path: &Path, size: u64) -> Option<u64> {
    let mut f = File::open(path).ok()?;
    let mut h = Xxh3::new();
    let mut buf = vec![0u8; EDGE as usize];
    let n = f.read(&mut buf).ok()?;
    h.update(&buf[..n]);
    if size > EDGE * 2 {
        f.seek(SeekFrom::End(-(EDGE as i64))).ok()?;
        let n = f.read(&mut buf).ok()?;
        h.update(&buf[..n]);
    }
    h.update(&size.to_le_bytes());
    Some(h.digest())
}

fn full_hash(path: &Path, job: &Job) -> Option<String> {
    let mut f = File::open(path).ok()?;
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if job.is_cancelled() {
            return None;
        }
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        job.bytes
            .fetch_add(n as u64, std::sync::atomic::Ordering::Relaxed);
    }
    Some(h.finalize().to_hex().to_string())
}

/// Volume serial plus file index: two paths with the same id are hard links to one file, and
/// deleting one frees nothing.
#[cfg(windows)]
fn file_id(path: &Path) -> Option<(u64, u64)> {
    let f = File::open(path).ok()?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(f.as_raw_handle()), &mut info) }.ok()?;
    Some((
        u64::from(info.dwVolumeSerialNumber),
        (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
    ))
}

/// Device plus inode, the Unix form of the same identity.
#[cfg(unix)]
fn file_id(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let md = fs::metadata(path).ok()?;
    Some((md.dev(), md.ino()))
}

/// Finds duplicate files of at least `min_bytes` below `roots`.
pub fn find_duplicates(
    roots: &[PathBuf],
    min_bytes: u64,
    guard: &Guard,
    job: &Job,
) -> Vec<DupGroup> {
    job.set_stage("collect");
    let all = parking_lot::Mutex::new(Vec::new());
    roots
        .par_iter()
        .for_each(|r| collect(r, min_bytes.max(1), guard, job, &all));
    let all = all.into_inner();

    // Overlapping roots would list the same file twice.
    let mut seen = std::collections::HashSet::new();
    let all: Vec<Candidate> = all
        .into_iter()
        .filter(|c| seen.insert(c.path.to_string_lossy().to_lowercase()))
        .collect();

    let mut by_size: HashMap<u64, Vec<Candidate>> = HashMap::new();
    for c in all {
        by_size.entry(c.size).or_default().push(c);
    }
    let size_groups: Vec<Vec<Candidate>> = by_size.into_values().filter(|g| g.len() > 1).collect();
    if job.is_cancelled() {
        return Vec::new();
    }

    job.set_stage("quick");
    job.set_total(size_groups.iter().map(|g| g.len() as u64).sum());
    let quick_groups: Vec<Vec<Candidate>> = size_groups
        .into_par_iter()
        .flat_map(|group| {
            let mut by_quick: HashMap<u64, Vec<Candidate>> = HashMap::new();
            for c in group {
                if job.is_cancelled() {
                    break;
                }
                job.add_done(1);
                if let Some(q) = quick_hash(&c.path, c.size) {
                    by_quick.entry(q).or_default().push(c);
                }
            }
            by_quick
                .into_values()
                .filter(|g| g.len() > 1)
                .collect::<Vec<_>>()
        })
        .collect();
    if job.is_cancelled() {
        return Vec::new();
    }

    job.set_stage("full");
    job.bytes.store(0, std::sync::atomic::Ordering::Relaxed);
    job.set_total(quick_groups.iter().map(|g| g.len() as u64).sum());
    let mut groups: Vec<DupGroup> = quick_groups
        .into_par_iter()
        .flat_map(|group| {
            let mut by_full: HashMap<String, Vec<Candidate>> = HashMap::new();
            for c in group {
                job.add_done(1);
                if let Some(h) = full_hash(&c.path, job) {
                    by_full.entry(h).or_default().push(c);
                }
            }
            by_full
                .into_iter()
                .filter(|(_, g)| g.len() > 1)
                .filter_map(|(hash, g)| {
                    let mut ids = std::collections::HashSet::new();
                    let files: Vec<&Candidate> = g
                        .iter()
                        .filter(|c| file_id(&c.path).is_none_or(|id| ids.insert(id)))
                        .collect();
                    if files.len() < 2 {
                        return None;
                    }
                    let bytes = files[0].size;
                    Some(DupGroup {
                        hash,
                        bytes,
                        wasted: bytes * (files.len() as u64 - 1),
                        files: files
                            .iter()
                            .map(|c| DupFile {
                                path: c.path.to_string_lossy().into_owned(),
                                mtime: c.mtime,
                            })
                            .collect(),
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect();
    groups.retain(|g| {
        g.files
            .iter()
            .any(|f| guard.check(Path::new(&f.path), Origin::User).is_ok())
    });
    groups.sort_by_key(|g| std::cmp::Reverse(g.wasted));
    groups
}

// Rule expansion and the guard only know Windows paths so far.
#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn groups_identical_files_only() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a");
        let b = tmp.path().join(r"b\c");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        let data: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        fs::write(a.join("one.bin"), &data).unwrap();
        fs::write(b.join("two.bin"), &data).unwrap();
        let mut other = data.clone();
        other[50_000] ^= 1; // same size, same edges, different middle
        fs::write(a.join("near.bin"), &other).unwrap();
        fs::hard_link(a.join("one.bin"), a.join("hard.bin")).unwrap();
        let guard = Guard::new(&[], None);
        let groups = find_duplicates(&[tmp.path().to_path_buf()], 1024, &guard, &Job::new());
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0].files.len(),
            2,
            "hard link counted once, near-duplicate excluded"
        );
        assert_eq!(groups[0].wasted, 100_000);
    }
}
