//! Executes reviewed cleanups.
//!
//! Every entry point re-checks its targets against the [`Guard`] right before deleting, never
//! follows links, and records what it did in a [`CleanReport`]. Locked and in-use files are
//! skipped and reported; processes are never killed.

use std::fs;
use std::io::ErrorKind;
#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;
#[cfg(not(target_os = "linux"))]
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::fsmeta::{is_cloud_only, is_link, mtime};
use crate::job::Job;
use crate::junk::{cutoff_for, matching_files};
use crate::paths::{Guard, Origin, expand_pattern, is_within, key};
use crate::rules::{Mode, TargetRule};
use crate::walk::{DirAction, WalkContext, measure_path};
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum DeleteMethod {
    #[default]
    Permanent,
    Recycle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Failure {
    pub path: String,
    /// `in_use`, `access_denied`, `protected`, `not_found`, `link`, `app_running`,
    /// `needs_admin`, `excluded_inside`, `use_command`, `io`.
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CleanReport {
    /// Bytes deleted permanently.
    pub freed: u64,
    /// Bytes moved to the Recycle Bin (still on disk until the bin is emptied).
    pub recycled: u64,
    pub files: u64,
    pub dirs: u64,
    pub failed: u64,
    pub failures: Vec<Failure>,
    pub cancelled: bool,
    /// Rule ids or paths that were cleaned.
    pub items: Vec<String>,
}

impl CleanReport {
    pub fn absorb(&mut self, other: CleanReport) {
        self.freed += other.freed;
        self.recycled += other.recycled;
        self.files += other.files;
        self.dirs += other.dirs;
        self.failed += other.failed;
        self.cancelled |= other.cancelled;
        for f in other.failures {
            if self.failures.len() < MAX_FAILURES {
                self.failures.push(f);
            }
        }
        self.items.extend(other.items);
    }
}

const MAX_FAILURES: usize = 200;

struct Acc<'a> {
    job: &'a Job,
    freed: AtomicU64,
    recycled: AtomicU64,
    files: AtomicU64,
    dirs: AtomicU64,
    failed: AtomicU64,
    failures: Mutex<Vec<Failure>>,
}

impl<'a> Acc<'a> {
    fn new(job: &'a Job) -> Self {
        Acc {
            job,
            freed: AtomicU64::new(0),
            recycled: AtomicU64::new(0),
            files: AtomicU64::new(0),
            dirs: AtomicU64::new(0),
            failed: AtomicU64::new(0),
            failures: Mutex::new(Vec::new()),
        }
    }

    fn fail(&self, path: &Path, reason: &str) {
        self.failed.fetch_add(1, Ordering::Relaxed);
        self.job.add_error();
        let mut f = self.failures.lock();
        if f.len() < MAX_FAILURES {
            f.push(Failure {
                path: path.to_string_lossy().into_owned(),
                reason: reason.to_string(),
            });
        }
    }

    fn fail_io(&self, path: &Path, e: &std::io::Error) {
        self.fail(path, io_reason(e));
    }

    fn deleted_file(&self, bytes: u64) {
        self.freed.fetch_add(bytes, Ordering::Relaxed);
        self.files.fetch_add(1, Ordering::Relaxed);
        self.job.add_files(1, bytes);
    }

    fn into_report(self, items: Vec<String>) -> CleanReport {
        CleanReport {
            freed: self.freed.into_inner(),
            recycled: self.recycled.into_inner(),
            files: self.files.into_inner(),
            dirs: self.dirs.into_inner(),
            failed: self.failed.into_inner(),
            failures: self.failures.into_inner(),
            cancelled: self.job.is_cancelled(),
            items,
        }
    }
}

fn io_reason(e: &std::io::Error) -> &'static str {
    match (e.kind(), e.raw_os_error()) {
        // ERROR_SHARING_VIOLATION and ERROR_LOCK_VIOLATION.
        #[cfg(windows)]
        (_, Some(32)) | (_, Some(33)) => "in_use",
        // A mount point or a folder busy some other way.
        #[cfg(target_os = "linux")]
        (ErrorKind::ResourceBusy, _) => "in_use",
        // A link where a folder was: `linux_delete::open_dir` answers this for it.
        #[cfg(target_os = "linux")]
        (_, Some(code)) if code == rustix::io::Errno::LOOP.raw_os_error() => "link",
        (ErrorKind::PermissionDenied, _) => "access_denied",
        (ErrorKind::NotFound, _) => "not_found",
        _ => "io",
    }
}

/// Why a Recycle Bin move failed. The shell runs without UI, so for a locked or protected item
/// it usually just aborts and `trash` gets no HRESULT back; the file system is asked instead.
fn recycle_reason(path: &Path, e: &trash::Error) -> &'static str {
    trash_reason(e).unwrap_or_else(|| probe_reason(path))
}

/// The reason the error itself carries, or `None` when `trash` only knows the item was aborted
/// or could not resolve its parent folder.
fn trash_reason(e: &trash::Error) -> Option<&'static str> {
    const COPYENGINE_E_REQUIRES_ELEVATION: u32 = 0x8027_0002;
    const COPYENGINE_E_ACCESS_DENIED_SRC: u32 = 0x8027_0021;
    const COPYENGINE_E_PATH_NOT_FOUND_SRC: u32 = 0x8027_0023;
    const COPYENGINE_E_SHARING_VIOLATION_SRC: u32 = 0x8027_0027;
    const COPYENGINE_E_ACCESSDENIED_READONLY: u32 = 0x8027_003F;
    match e {
        trash::Error::Os { code, .. } => Some(match *code as u32 {
            // HRESULT_FROM_WIN32: the low word is a Win32 error code.
            hr if hr & 0xFFFF_0000 == 0x8007_0000 => {
                io_reason(&std::io::Error::from_raw_os_error((hr & 0xFFFF) as i32))
            }
            COPYENGINE_E_REQUIRES_ELEVATION => "needs_admin",
            COPYENGINE_E_ACCESS_DENIED_SRC | COPYENGINE_E_ACCESSDENIED_READONLY => "access_denied",
            COPYENGINE_E_SHARING_VIOLATION_SRC => "in_use",
            COPYENGINE_E_PATH_NOT_FOUND_SRC => "not_found",
            _ => "io",
        }),
        trash::Error::CouldNotAccess { .. } => Some("access_denied"),
        trash::Error::Unknown { .. } | trash::Error::CanonicalizePath { .. } => None,
        _ => Some("io"),
    }
}

#[cfg(windows)]
const DELETE: u32 = 0x0001_0000;
#[cfg(windows)]
const FILE_SHARE_ALL: u32 = 0x7;
#[cfg(windows)]
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
#[cfg(windows)]
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
/// Items opened while looking for what blocks a folder, so a huge folder cannot stall a report.
const PROBE_LIMIT: usize = 10_000;

/// Opens `path`, and for a folder everything inside it, with delete access without deleting
/// anything, and returns the reason of the first item that refuses. A folder cannot be moved
/// while any file in it is locked, so the culprit can sit deep inside. `io` when nothing refuses.
fn probe_reason(path: &Path) -> &'static str {
    let mut budget = PROBE_LIMIT;
    probe(path, &mut budget).unwrap_or("io")
}

fn probe(path: &Path, budget: &mut usize) -> Option<&'static str> {
    let md = match path.symlink_metadata() {
        Ok(md) => md,
        Err(e) => return Some(io_reason(&e)),
    };
    if is_cloud_only(&md) {
        return None;
    }
    // Linux has no delete-access open; whether an entry can go depends on its folder.
    #[cfg(windows)]
    if let Err(e) = fs::OpenOptions::new()
        .access_mode(DELETE)
        .share_mode(FILE_SHARE_ALL)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
    {
        return Some(io_reason(&e));
    }
    if !md.is_dir() || is_link(&md.file_type()) {
        return None;
    }
    let rd = match fs::read_dir(path) {
        Ok(rd) => rd,
        Err(e) => return Some(io_reason(&e)),
    };
    for entry in rd.flatten() {
        if *budget == 0 {
            return None;
        }
        *budget -= 1;
        if let Some(reason) = probe(&entry.path(), budget) {
            return Some(reason);
        }
    }
    None
}

/// Removes a file, clearing the read-only flag if that is what blocks it (common in caches
/// that copy files from read-only sources).
#[cfg(not(target_os = "linux"))]
fn remove_file(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() == ErrorKind::PermissionDenied => {
            let md = fs::symlink_metadata(path)?;
            let mut perms = md.permissions();
            if perms.readonly() {
                #[allow(clippy::permissions_set_readonly_false)]
                perms.set_readonly(false);
                fs::set_permissions(path, perms)?;
                fs::remove_file(path)
            } else {
                Err(e)
            }
        }
        other => other,
    }
}

/// Deletes everything inside `dir` permanently. Files newer than `cutoff` and anything inside
/// `keep` (exclusion keys) stay. Links are removed as links, never followed. Returns true when
/// `dir` ended up empty.
#[cfg(not(target_os = "linux"))]
fn purge_dir(dir: &Path, cutoff: Option<i64>, keep: &[String], skip: &[String], acc: &Acc) -> bool {
    if acc.job.is_cancelled() {
        return false;
    }
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            acc.fail_io(dir, &e);
            return false;
        }
    };
    let mut empty = true;
    let mut subdirs = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        if !skip.is_empty()
            && crate::walk::name_excluded(&entry.file_name().to_string_lossy(), skip)
        {
            empty = false;
            continue;
        }
        if !keep.is_empty() {
            let k = key(&path);
            if keep.iter().any(|x| is_within(&k, x) || is_within(x, &k)) {
                if keep.iter().any(|x| is_within(&k, x)) {
                    empty = false;
                    continue;
                }
                // An exclusion lies deeper inside this folder: descend instead of removing it.
                if entry
                    .file_type()
                    .map(|t| t.is_dir() && !is_link(&t))
                    .unwrap_or(false)
                {
                    subdirs.push(path);
                    continue;
                }
            }
        }
        let Ok(ft) = entry.file_type() else {
            empty = false;
            continue;
        };
        if is_link(&ft) {
            // RemoveDirectory on a junction or directory symlink deletes the link, not its target.
            let res = fs::remove_dir(&path).or_else(|_| fs::remove_file(&path));
            if let Err(e) = res {
                acc.fail_io(&path, &e);
                empty = false;
            }
            continue;
        }
        if ft.is_dir() {
            subdirs.push(path);
            continue;
        }
        let Ok(md) = entry.metadata() else {
            empty = false;
            continue;
        };
        if is_cloud_only(&md) || cutoff.is_some_and(|c| mtime(&md) > c) {
            empty = false;
            continue;
        }
        match remove_file(&path) {
            Ok(()) => acc.deleted_file(md.len()),
            Err(e) => {
                acc.fail_io(&path, &e);
                empty = false;
            }
        }
    }
    let results: Vec<bool> = subdirs
        .par_iter()
        .map(|sub| {
            if purge_dir(sub, cutoff, keep, skip, acc) {
                match fs::remove_dir(sub) {
                    Ok(()) => {
                        acc.dirs.fetch_add(1, Ordering::Relaxed);
                        true
                    }
                    Err(e) => {
                        acc.fail_io(sub, &e);
                        false
                    }
                }
            } else {
                false
            }
        })
        .collect();
    empty && results.into_iter().all(|r| r)
}

/// Permanent deletes on Linux. Every step goes through a folder descriptor opened from `/` without
/// following a link, so swapping a folder for a link while the delete runs ends in an error
/// rather than somewhere else. `std::fs::remove_dir_all` is not used: it descends into mounts.
#[cfg(target_os = "linux")]
mod linux_delete {
    use std::ffi::{OsStr, OsString};
    use std::io;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Component, Path};
    use std::sync::atomic::Ordering;

    use rustix::fd::{AsFd, OwnedFd};
    use rustix::fs::{
        AtFlags, CWD, Dir, FileType, Mode, OFlags, Stat, StatxAttributes, StatxFlags, fstat,
        openat, statat, statx, unlinkat,
    };
    use rustix::io::Errno;

    use super::Acc;
    use crate::fsmeta::plausible_time;
    use crate::mounts::MountTable;
    use crate::paths::{is_protected_name, is_within, key};
    use crate::walk::name_excluded;

    const READ_DIR: OFlags = OFlags::RDONLY
        .union(OFlags::DIRECTORY)
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC);
    /// `O_PATH` only needs search rights, and folders on the way may not be readable (`/home`).
    const TRAVERSE: OFlags = OFlags::PATH
        .union(OFlags::DIRECTORY)
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC);

    /// Opens a real path one name at a time from `/`. `O_NOFOLLOW` only guards the last name of a
    /// path, so a whole path handed to `open` could still pass through a link.
    fn open_real(path: &Path) -> io::Result<OwnedFd> {
        let mut dir = openat(CWD, "/", TRAVERSE, Mode::empty())?;
        for part in path.components() {
            match part {
                Component::RootDir => {}
                Component::Normal(name) => dir = open_dir(&dir, name, TRAVERSE)?,
                _ => return Err(io::ErrorKind::InvalidInput.into()),
            }
        }
        Ok(dir)
    }

    /// Opens the folder `name` without following a link. With `O_DIRECTORY` Linux answers `ENOTDIR`
    /// for a link as well as for a file, so a link is told apart here to be reported as one.
    fn open_dir(parent: &OwnedFd, name: &OsStr, flags: OFlags) -> rustix::io::Result<OwnedFd> {
        openat(parent, name, flags, Mode::empty()).map_err(|e| {
            let is_link = e == Errno::NOTDIR
                && statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
                    .is_ok_and(|st| file_type(&st) == FileType::Symlink);
            if is_link { Errno::LOOP } else { e }
        })
    }

    /// Space that removing this name gives back. Hard links share theirs until the last name goes.
    fn freed_by(st: &Stat) -> u64 {
        if st.st_nlink > 1 {
            return 0;
        }
        st.st_blocks as u64 * 512
    }

    /// True when `dir` is the root of a mount. A bind mount of the same file system keeps its
    /// `st_dev`, and this also sees a mount made after the table was read. Kernels before 5.8 do
    /// not report it, which leaves the table and the device check.
    fn is_mount_root(dir: impl AsFd) -> bool {
        statx(dir, "", AtFlags::EMPTY_PATH, StatxFlags::empty()).is_ok_and(|s| {
            s.stx_attributes_mask.contains(StatxAttributes::MOUNT_ROOT)
                && s.stx_attributes.contains(StatxAttributes::MOUNT_ROOT)
        })
    }

    fn read_names(dir: impl AsFd) -> io::Result<Vec<OsString>> {
        let mut names = Vec::new();
        for entry in Dir::read_from(dir)? {
            let entry = entry?;
            let name = entry.file_name().to_bytes();
            if name != b"." && name != b".." {
                names.push(OsStr::from_bytes(name).to_os_string());
            }
        }
        Ok(names)
    }

    fn file_type(st: &Stat) -> FileType {
        FileType::from_raw_mode(st.st_mode)
    }

    struct Purge<'a, 'j> {
        acc: &'a Acc<'j>,
        cutoff: Option<i64>,
        keep: &'a [String],
        skip: &'a [String],
        mounts: &'a MountTable,
        /// The device of the folder the delete started in. It is never left.
        dev: u64,
    }

    impl Purge<'_, '_> {
        /// Opens the folder `name` for deleting in, or reports why it may not be entered.
        fn enter(&self, parent: &OwnedFd, name: &OsStr, path: &Path) -> Option<OwnedFd> {
            if self.mounts.is_mount_point(path) {
                self.acc.fail(path, "protected");
                return None;
            }
            let dir = match open_dir(parent, name, READ_DIR) {
                Ok(dir) => dir,
                Err(e) => {
                    self.acc.fail_io(path, &e.into());
                    return None;
                }
            };
            let same_device = fstat(&dir).is_ok_and(|st| st.st_dev == self.dev);
            if !same_device || is_mount_root(&dir) {
                self.acc.fail(path, "protected");
                return None;
            }
            Some(dir)
        }

        /// Removes what may go inside `dir` and returns true when it ended up empty.
        fn contents(&mut self, dir: &OwnedFd, path: &Path) -> bool {
            if self.acc.job.is_cancelled() {
                return false;
            }
            let names = match read_names(dir) {
                Ok(names) => names,
                Err(e) => {
                    self.acc.fail_io(path, &e);
                    return false;
                }
            };
            let mut empty = true;
            for name in names {
                empty &= self.entry(dir, &name, &path.join(&name));
            }
            empty
        }

        /// Handles one folder entry and returns true when it is gone.
        fn entry(&mut self, dir: &OwnedFd, name: &OsStr, path: &Path) -> bool {
            let text = name.to_string_lossy();
            // Removing a folder removes what is below it, so these names are kept here too.
            if is_protected_name(&text) || name_excluded(&text, self.skip) {
                return false;
            }
            if !self.keep.is_empty() {
                let k = key(path);
                if self.keep.iter().any(|x| is_within(&k, x)) {
                    return false;
                }
            }
            let st = match statat(dir, name, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(st) => st,
                Err(e) => {
                    self.acc.fail_io(path, &e.into());
                    return false;
                }
            };
            match file_type(&st) {
                // Removed as a link; the target is not touched.
                FileType::Symlink => self.unlink(dir, name, path, AtFlags::empty()),
                FileType::Directory => {
                    let Some(child) = self.enter(dir, name, path) else {
                        return false;
                    };
                    if !self.contents(&child, path) {
                        return false;
                    }
                    drop(child);
                    let gone = self.unlink(dir, name, path, AtFlags::REMOVEDIR);
                    if gone {
                        self.acc.dirs.fetch_add(1, Ordering::Relaxed);
                    }
                    gone
                }
                _ => {
                    let mtime = plausible_time(st.st_mtime as i64);
                    if self.cutoff.is_some_and(|c| mtime > c) {
                        return false;
                    }
                    let gone = self.unlink(dir, name, path, AtFlags::empty());
                    if gone {
                        self.acc.deleted_file(freed_by(&st));
                    }
                    gone
                }
            }
        }

        fn unlink(&self, dir: &OwnedFd, name: &OsStr, path: &Path, flags: AtFlags) -> bool {
            match unlinkat(dir, name, flags) {
                Ok(()) => true,
                Err(e) => {
                    self.acc.fail_io(path, &e.into());
                    false
                }
            }
        }
    }

    /// The trusted parent of `real` and the name inside it. `real` must have its parent resolved.
    fn open_parent(real: &Path) -> io::Result<(OwnedFd, u64, &OsStr)> {
        let (Some(parent), Some(name)) = (real.parent(), real.file_name()) else {
            return Err(io::ErrorKind::InvalidInput.into());
        };
        let fd = open_real(parent)?;
        let dev = fstat(&fd)?.st_dev;
        Ok((fd, dev, name))
    }

    fn mount_table(path: &Path, acc: &Acc) -> Option<MountTable> {
        match MountTable::read_current() {
            Ok(table) => Some(table),
            Err(_) => {
                acc.fail(path, "protected");
                None
            }
        }
    }

    /// Deletes what may go inside `real`, keeping the folder itself.
    pub(super) fn permanent_contents(
        real: &Path,
        cutoff: Option<i64>,
        keep: &[String],
        skip: &[String],
        acc: &Acc,
    ) {
        if let Some(mounts) = mount_table(real, acc) {
            purge_below(real, cutoff, keep, skip, acc, &mounts);
        }
    }

    pub(super) fn purge_below(
        real: &Path,
        cutoff: Option<i64>,
        keep: &[String],
        skip: &[String],
        acc: &Acc,
        mounts: &MountTable,
    ) {
        let (parent, dev, name) = match open_parent(real) {
            Ok(opened) => opened,
            Err(e) => return acc.fail_io(real, &e),
        };
        let mut purge = Purge {
            acc,
            cutoff,
            keep,
            skip,
            mounts,
            dev,
        };
        if let Some(dir) = purge.enter(&parent, name, real) {
            purge.contents(&dir, real);
        }
    }

    /// Deletes the folder `real` with everything inside, unless something had to stay.
    pub(super) fn permanent_dir(real: &Path, keep: &[String], acc: &Acc) {
        if let Some(mounts) = mount_table(real, acc) {
            remove_dir(real, keep, acc, &mounts);
        }
    }

    pub(super) fn remove_dir(real: &Path, keep: &[String], acc: &Acc, mounts: &MountTable) {
        let (parent, dev, name) = match open_parent(real) {
            Ok(opened) => opened,
            Err(e) => return acc.fail_io(real, &e),
        };
        let mut purge = Purge {
            acc,
            cutoff: None,
            keep,
            skip: &[],
            mounts,
            dev,
        };
        let Some(dir) = purge.enter(&parent, name, real) else {
            return;
        };
        if purge.contents(&dir, real) {
            drop(dir);
            if purge.unlink(&parent, name, real, AtFlags::REMOVEDIR) {
                acc.dirs.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Deletes the single file `real`. A link is refused here too, in case one took its place.
    pub(super) fn permanent_file(real: &Path, acc: &Acc) {
        let (parent, _, name) = match open_parent(real) {
            Ok(opened) => opened,
            Err(e) => return acc.fail_io(real, &e),
        };
        let st = match statat(&parent, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(st) => st,
            Err(e) => return acc.fail_io(real, &e.into()),
        };
        match file_type(&st) {
            FileType::Symlink => acc.fail(real, "link"),
            FileType::Directory => acc.fail(real, "io"),
            _ => match unlinkat(&parent, name, AtFlags::empty()) {
                Ok(()) => acc.deleted_file(freed_by(&st)),
                Err(e) => acc.fail_io(real, &e.into()),
            },
        }
    }
}

#[cfg(target_os = "linux")]
use linux_delete::{permanent_contents, permanent_dir, permanent_file};

#[cfg(not(target_os = "linux"))]
fn permanent_contents(
    dir: &Path,
    cutoff: Option<i64>,
    keep: &[String],
    skip: &[String],
    acc: &Acc,
) {
    purge_dir(dir, cutoff, keep, skip, acc);
}

#[cfg(not(target_os = "linux"))]
fn permanent_dir(path: &Path, keep: &[String], acc: &Acc) {
    if purge_dir(path, None, keep, &[], acc) {
        match fs::remove_dir(path) {
            Ok(()) => {
                acc.dirs.fetch_add(1, Ordering::Relaxed);
            }
            Err(e) => acc.fail_io(path, &e),
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn permanent_file(path: &Path, acc: &Acc) {
    let bytes = fs::symlink_metadata(path).map(|m| m.len()).unwrap_or(0);
    match remove_file(path) {
        Ok(()) => acc.deleted_file(bytes),
        Err(e) => acc.fail_io(path, &e),
    }
}

/// The spelling of `path` that a permanent delete walks and that exclusions are compared with.
/// On Linux that is the path with its parent resolved, which is the one the guard approved.
#[cfg(target_os = "linux")]
fn delete_path(path: &Path) -> PathBuf {
    crate::paths::resolve_parent(path).unwrap_or_else(|| path.to_path_buf())
}

#[cfg(not(target_os = "linux"))]
fn delete_path(path: &Path) -> PathBuf {
    path.to_path_buf()
}

/// Moves paths to the Recycle Bin. Runs on its own thread because the shell API needs a
/// single-threaded COM apartment that the caller's thread may not have.
fn recycle(paths: Vec<PathBuf>, acc: &Acc) {
    if paths.is_empty() {
        return;
    }
    for chunk in paths.chunks(256) {
        if acc.job.is_cancelled() {
            return;
        }
        let sizes: Vec<u64> = chunk
            .iter()
            .map(|p| measure_path(p, None, &Job::new()).bytes)
            .collect();
        let owned: Vec<PathBuf> = chunk.to_vec();
        let res = std::thread::spawn(move || trash::delete_all(&owned)).join();
        match res {
            Ok(Ok(())) => {
                for (p, size) in chunk.iter().zip(sizes) {
                    if p.symlink_metadata().is_err() {
                        acc.recycled.fetch_add(size, Ordering::Relaxed);
                        acc.files.fetch_add(1, Ordering::Relaxed);
                        acc.job.add_files(1, size);
                    } else {
                        acc.fail(p, probe_reason(p));
                    }
                }
            }
            _ => {
                // The batch failed as a whole; retry one by one so one locked file does not
                // block the rest.
                for (p, size) in chunk.iter().zip(sizes) {
                    let one = p.clone();
                    match std::thread::spawn(move || trash::delete(&one)).join() {
                        Ok(Ok(())) => {
                            acc.recycled.fetch_add(size, Ordering::Relaxed);
                            acc.files.fetch_add(1, Ordering::Relaxed);
                        }
                        Ok(Err(e)) => acc.fail(p, recycle_reason(p, &e)),
                        Err(_) => acc.fail(p, "io"),
                    }
                }
            }
        }
    }
}

/// Files below `dir` eligible for deletion (respecting cutoff and exclusions), for recycling.
fn eligible_files(
    dir: &Path,
    cutoff: Option<i64>,
    keep: &[String],
    skip: &[String],
    ctx: &WalkContext,
    out: &mut Vec<PathBuf>,
) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let path = entry.path();
        if !keep.is_empty() && keep.iter().any(|x| is_within(&key(&path), x)) {
            continue;
        }
        if !skip.is_empty()
            && crate::walk::name_excluded(&entry.file_name().to_string_lossy(), skip)
        {
            continue;
        }
        let Ok(ft) = entry.file_type() else { continue };
        if is_link(&ft) {
            continue;
        }
        if ft.is_dir() {
            if ctx.action(&path) == DirAction::Enter {
                eligible_files(&path, cutoff, keep, skip, ctx, out);
            }
        } else if let Ok(md) = entry.metadata()
            && !is_cloud_only(&md)
            && cutoff.is_none_or(|c| mtime(&md) <= c)
        {
            out.push(path);
        }
    }
}

fn wipe_contents(
    dir: &Path,
    cutoff: Option<i64>,
    skip: &[String],
    method: DeleteMethod,
    guard: &Guard,
    acc: &Acc,
) {
    if let Err(e) = guard.check_wipe(dir) {
        acc.fail(dir, e.code());
        return;
    }
    match dir.symlink_metadata() {
        Ok(md) if is_link(&md.file_type()) => return acc.fail(dir, "link"),
        Ok(md) if !md.is_dir() => return acc.fail(dir, "io"),
        Err(e) => return acc.fail_io(dir, &e),
        _ => {}
    }
    let real = delete_path(dir);
    let keep = guard.exclusions_inside(&real);
    match method {
        DeleteMethod::Permanent => permanent_contents(&real, cutoff, &keep, skip, acc),
        DeleteMethod::Recycle => {
            let mut targets = Vec::new();
            if cutoff.is_none() && keep.is_empty() {
                if let Ok(rd) = fs::read_dir(dir) {
                    targets.extend(
                        rd.flatten()
                            .filter(|e| e.file_type().map(|t| !is_link(&t)).unwrap_or(false))
                            .map(|e| e.path()),
                    );
                }
            } else {
                eligible_files(
                    &real,
                    cutoff,
                    &keep,
                    skip,
                    &WalkContext::new(&real),
                    &mut targets,
                );
            }
            recycle(targets, acc);
        }
    }
}

/// Removes `path` itself (a file or a folder with everything inside).
fn remove_one(path: &Path, method: DeleteMethod, origin: Origin, guard: &Guard, acc: &Acc) {
    if let Err(e) = guard.check(path, origin) {
        return acc.fail(path, e.code());
    }
    let md = match path.symlink_metadata() {
        Ok(md) => md,
        Err(e) => return acc.fail_io(path, &e),
    };
    if is_link(&md.file_type()) {
        return acc.fail(path, "link");
    }
    let real = delete_path(path);
    let keep = guard.exclusions_inside(&real);
    match method {
        DeleteMethod::Recycle => {
            if !keep.is_empty() {
                return acc.fail(path, "excluded_inside");
            }
            recycle(vec![path.to_path_buf()], acc);
        }
        DeleteMethod::Permanent if md.is_dir() => permanent_dir(&real, &keep, acc),
        DeleteMethod::Permanent => permanent_file(&real, acc),
    }
}

/// Deletes paths the user picked (large files, duplicates, empty folders, project artifacts,
/// explorer items).
pub fn clean_paths(
    paths: &[PathBuf],
    method: DeleteMethod,
    guard: &Guard,
    job: &Job,
) -> CleanReport {
    let picked: Vec<(PathBuf, Origin)> = paths.iter().map(|p| (p.clone(), Origin::User)).collect();
    clean_picked(&picked, method, guard, job)
}

/// Like [`clean_paths`], with the guard origin chosen per path. The caller decides which paths
/// count as [`Origin::Leftover`]; only paths the engine itself reported as leftovers should.
pub fn clean_picked(
    picked: &[(PathBuf, Origin)],
    method: DeleteMethod,
    guard: &Guard,
    job: &Job,
) -> CleanReport {
    job.set_stage("clean");
    job.set_total(picked.len() as u64);
    let acc = Acc::new(job);
    for (p, origin) in picked {
        if job.is_cancelled() {
            break;
        }
        job.set_current(p);
        remove_one(p, method, *origin, guard, &acc);
        job.add_done(1);
    }
    acc.into_report(
        picked
            .iter()
            .map(|(p, _)| p.to_string_lossy().into_owned())
            .collect(),
    )
}

/// Cleans rule targets. Rules whose apps are running are skipped, and admin rules are skipped
/// unless `elevated` (the caller routes those through the elevated helper).
pub fn clean_targets(
    rules: &[&TargetRule],
    method: DeleteMethod,
    guard: &Guard,
    job: &Job,
    elevated: bool,
) -> CleanReport {
    job.set_stage("clean");
    job.set_total(rules.len() as u64);
    let running = crate::system::running_processes();
    let acc = Acc::new(job);
    let mut items = Vec::new();
    for rule in rules {
        if job.is_cancelled() {
            break;
        }
        job.add_done(1);
        let label = PathBuf::from(&rule.id);
        if rule.admin && !elevated {
            acc.fail(&label, "needs_admin");
            continue;
        }
        if rule
            .close_apps
            .iter()
            .any(|a| running.contains(&a.to_lowercase()))
        {
            acc.fail(&label, "app_running");
            continue;
        }
        items.push(rule.id.clone());
        let cutoff = cutoff_for(rule);
        match rule.mode {
            Mode::Report => {
                if rule
                    .command
                    .as_deref()
                    .is_some_and(|c| c.starts_with("api:SHEmptyRecycleBin"))
                {
                    let before = crate::system::recycle_bin_info();
                    match crate::system::empty_recycle_bin() {
                        Ok(()) => {
                            acc.freed.fetch_add(before.bytes, Ordering::Relaxed);
                            acc.files.fetch_add(before.items, Ordering::Relaxed);
                        }
                        Err(e) => acc.fail(&label, e.code()),
                    }
                } else {
                    acc.fail(&label, "use_command");
                }
            }
            Mode::Contents => {
                for pattern in &rule.paths {
                    for p in expand_pattern(pattern).unwrap_or_default() {
                        job.set_current(&p);
                        wipe_contents(&p, cutoff, &rule.exclude, method, guard, &acc);
                    }
                }
            }
            Mode::Folder => {
                for pattern in &rule.paths {
                    for p in expand_pattern(pattern).unwrap_or_default() {
                        job.set_current(&p);
                        remove_one(&p, method, Origin::Rule, guard, &acc);
                    }
                }
            }
            Mode::Files => {
                for pattern in &rule.paths {
                    for dir in expand_pattern(pattern).unwrap_or_default() {
                        let files: Vec<PathBuf> = matching_files(&dir, &rule.include)
                            .into_iter()
                            .filter(|(_, _, t)| cutoff.is_none_or(|c| *t <= c))
                            .map(|(p, _, _)| p)
                            .collect();
                        for f in files {
                            remove_one(&f, method, Origin::Rule, guard, &acc);
                        }
                    }
                }
            }
        }
    }
    acc.into_report(items)
}

/// Looks up built-in rules by id. Unknown ids are an error so a typo never silently cleans
/// nothing.
pub fn resolve_rules<'a>(ids: &[String], pool: &'a [TargetRule]) -> Result<Vec<&'a TargetRule>> {
    ids.iter()
        .map(|id| {
            pool.iter()
                .find(|r| &r.id == id)
                .ok_or_else(|| Error::UnknownRule(id.clone()))
        })
        .collect()
}

// The fixtures use Windows paths, junctions and share modes.
#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::rules::{Group, Risk};

    fn rule(path: &Path, mode: Mode) -> TargetRule {
        TargetRule {
            id: "test-rule".into(),
            group: Group::Apps,
            name: "t".into(),
            name_tr: "t".into(),
            description: "d".into(),
            description_tr: "d".into(),
            paths: vec![path.to_string_lossy().into_owned()],
            mode,
            include: vec!["*.log".into()],
            exclude: vec![],
            min_age_hours: None,
            risk: Risk::Safe,
            admin: false,
            close_apps: vec![],
            command: None,
            verified: true,
            source: None,
            user: false,
        }
    }

    #[test]
    fn contents_mode_keeps_root_and_skips_junction_targets() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("Cache");
        let outside = tmp.path().join("precious");
        fs::create_dir_all(cache.join(r"a\b")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(cache.join("1.bin"), [0u8; 10]).unwrap();
        fs::write(cache.join(r"a\b\2.bin"), [0u8; 20]).unwrap();
        fs::write(outside.join("keep.txt"), b"keep").unwrap();
        let ok = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(cache.join("link"))
            .arg(&outside)
            .output()
            .unwrap();
        assert!(ok.status.success());

        let job = Job::new();
        let guard = Guard::new(&[], None);
        let r = rule(&cache, Mode::Contents);
        let report = clean_targets(&[&r], DeleteMethod::Permanent, &guard, &job, false);
        assert_eq!(report.freed, 30);
        assert!(cache.exists(), "root folder is kept");
        assert_eq!(
            fs::read_dir(&cache).unwrap().count(),
            0,
            "contents removed, junction removed as a link"
        );
        assert!(
            outside.join("keep.txt").exists(),
            "junction target untouched"
        );
    }

    #[test]
    fn exclusions_survive_a_wipe() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("Cache");
        fs::create_dir_all(cache.join("keepme")).unwrap();
        fs::create_dir_all(cache.join("drop")).unwrap();
        fs::write(cache.join(r"keepme\k.bin"), [1u8; 5]).unwrap();
        fs::write(cache.join(r"drop\d.bin"), [1u8; 5]).unwrap();
        let guard = Guard::new(&[cache.join("keepme").to_string_lossy().into_owned()], None);
        let job = Job::new();
        let r = rule(&cache, Mode::Contents);
        let report = clean_targets(&[&r], DeleteMethod::Permanent, &guard, &job, false);
        assert_eq!(report.freed, 5);
        assert!(cache.join(r"keepme\k.bin").exists());
        assert!(!cache.join("drop").exists());
    }

    #[test]
    fn excluded_names_survive_a_wipe() {
        let tmp = tempfile::tempdir().unwrap();
        let temp = tmp.path().join("Temp");
        fs::create_dir_all(temp.join("gradio")).unwrap();
        fs::create_dir_all(temp.join("junk")).unwrap();
        fs::write(temp.join("quit.blend"), [1u8; 3]).unwrap();
        fs::write(temp.join(r"gradio\out.png"), [1u8; 4]).unwrap();
        fs::write(temp.join("junk").join("a.tmp"), [1u8; 5]).unwrap();
        let mut r = rule(&temp, Mode::Contents);
        r.exclude = vec!["*.blend".into(), "gradio".into()];
        let report = clean_targets(
            &[&r],
            DeleteMethod::Permanent,
            &Guard::new(&[], None),
            &Job::new(),
            false,
        );
        assert_eq!(report.freed, 5);
        assert!(temp.join("quit.blend").exists());
        assert!(temp.join(r"gradio\out.png").exists());
        assert!(!temp.join("junk").exists());
        let m = crate::walk::measure_dir_except(&temp, None, &r.exclude, &Job::new());
        assert_eq!(m.bytes, 0);
    }

    #[test]
    fn files_mode_is_not_recursive() {
        let tmp = tempfile::tempdir().unwrap();
        let logs = tmp.path().join("Logs");
        fs::create_dir_all(logs.join("nested")).unwrap();
        fs::write(logs.join("a.log"), [0u8; 4]).unwrap();
        fs::write(logs.join("b.txt"), [0u8; 4]).unwrap();
        fs::write(logs.join(r"nested\c.log"), [0u8; 4]).unwrap();
        let guard = Guard::new(&[], None);
        let job = Job::new();
        let r = rule(&logs, Mode::Files);
        let report = clean_targets(&[&r], DeleteMethod::Permanent, &guard, &job, false);
        assert_eq!(report.freed, 4);
        assert!(logs.join("b.txt").exists());
        assert!(logs.join(r"nested\c.log").exists());
    }

    #[test]
    fn user_paths_respect_guard() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("big.bin");
        fs::write(&f, [0u8; 64]).unwrap();
        let guard = Guard::new(&[], None);
        let job = Job::new();
        let windir = PathBuf::from(std::env::var("WINDIR").unwrap());
        let report = clean_paths(
            &[f.clone(), windir.join("notepad.exe")],
            DeleteMethod::Permanent,
            &guard,
            &job,
        );
        assert_eq!(report.freed, 64);
        assert!(!f.exists());
        assert_eq!(report.failed, 1);
        assert_eq!(report.failures[0].reason, "protected");
    }

    #[test]
    fn leftover_origin_reaches_app_folders_only() {
        let tmp = tempfile::tempdir().unwrap();
        let pf = tmp.path().join("PF");
        let app = pf.join("Acme");
        fs::create_dir_all(app.join("bin")).unwrap();
        fs::write(app.join(r"bin\acme.exe"), [0u8; 32]).unwrap();
        let guard = Guard::new(&[], None).with_app_root(&pf);

        let report = clean_paths(
            std::slice::from_ref(&app),
            DeleteMethod::Permanent,
            &guard,
            &Job::new(),
        );
        assert_eq!(report.failures[0].reason, "protected");
        assert!(
            app.exists(),
            "a hand-picked delete may not touch Program Files"
        );

        let picked = [
            (pf.clone(), Origin::Leftover),
            (app.join(r"bin\deeper"), Origin::Leftover),
            (app.clone(), Origin::Leftover),
        ];
        let report = clean_picked(&picked, DeleteMethod::Permanent, &guard, &Job::new());
        assert_eq!(report.freed, 32);
        assert!(!app.exists());
        assert!(pf.exists(), "the root itself is refused");
        let reasons: Vec<&str> = report.failures.iter().map(|f| f.reason.as_str()).collect();
        assert_eq!(reasons, ["protected", "protected"]);
    }

    #[test]
    fn admin_and_running_rules_are_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let mut r = rule(tmp.path(), Mode::Contents);
        r.admin = true;
        let guard = Guard::new(&[], None);
        let report = clean_targets(&[&r], DeleteMethod::Permanent, &guard, &Job::new(), false);
        assert_eq!(report.failures[0].reason, "needs_admin");
        let mut r2 = rule(tmp.path(), Mode::Contents);
        r2.close_apps = vec!["cargo.exe".into()];
        let report = clean_targets(&[&r2], DeleteMethod::Permanent, &guard, &Job::new(), false);
        // cargo.exe runs this test, so the rule must be skipped.
        assert_eq!(report.failures[0].reason, "app_running");
    }

    fn os(hr: u32) -> trash::Error {
        trash::Error::Os {
            code: hr as i32,
            description: String::new(),
        }
    }

    #[test]
    fn trash_errors_map_to_reasons() {
        assert_eq!(trash_reason(&os(0x8007_0005)), Some("access_denied"));
        assert_eq!(trash_reason(&os(0x8007_0020)), Some("in_use"));
        assert_eq!(trash_reason(&os(0x8007_0021)), Some("in_use"));
        assert_eq!(trash_reason(&os(0x8007_0002)), Some("not_found"));
        assert_eq!(trash_reason(&os(0x8027_0021)), Some("access_denied"));
        assert_eq!(trash_reason(&os(0x8027_0027)), Some("in_use"));
        assert_eq!(trash_reason(&os(0x8027_0002)), Some("needs_admin"));
        assert_eq!(trash_reason(&os(0x8000_4005)), Some("io"));
        let denied = trash::Error::CouldNotAccess { target: "x".into() };
        assert_eq!(trash_reason(&denied), Some("access_denied"));
        assert_eq!(trash_reason(&trash::Error::TargetedRoot), Some("io"));
        // What the shell reports under FOF_NO_UI for locked and admin-only items alike.
        let aborted = trash::Error::Unknown {
            description: "Some operations were aborted".into(),
        };
        assert_eq!(trash_reason(&aborted), None);
    }

    #[test]
    fn probe_finds_what_blocks_a_recycle() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("app");
        fs::create_dir_all(dir.join("logs")).unwrap();
        let log = dir.join(r"logs\now.log");
        fs::write(&log, b"x").unwrap();
        fs::write(dir.join("a.txt"), b"x").unwrap();
        assert_eq!(probe_reason(&dir), "io");
        assert_eq!(probe_reason(&tmp.path().join("gone")), "not_found");
        // Held the way a running program holds its log: without FILE_SHARE_DELETE.
        let held = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&log)
            .unwrap();
        assert_eq!(probe_reason(&log), "in_use");
        assert_eq!(probe_reason(&dir), "in_use");
        drop(held);
        assert!(log.exists());
    }
}

#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use super::*;
    use crate::mounts::{MountTable, parse_mountinfo};
    use crate::rules::{Group, Risk};
    use crate::walk::WalkContext;
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::process::Command;

    fn rule(path: &Path, mode: Mode) -> TargetRule {
        TargetRule {
            id: "test-rule".into(),
            group: Group::Apps,
            name: "t".into(),
            name_tr: "t".into(),
            description: "d".into(),
            description_tr: "d".into(),
            paths: vec![path.to_string_lossy().into_owned()],
            mode,
            include: vec!["*.log".into()],
            exclude: vec![],
            min_age_hours: None,
            risk: Risk::Safe,
            admin: false,
            close_apps: vec![],
            command: None,
            verified: true,
            source: None,
            user: false,
        }
    }

    /// A home folder in a temp folder, with a guard for it and a place outside it.
    struct Fixture {
        _tmp: tempfile::TempDir,
        home: PathBuf,
        outside: PathBuf,
        guard: Guard,
    }

    fn fixture(exclusions: &[&str]) -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(tmp.path()).unwrap();
        let home = root.join("home");
        let outside = root.join("outside");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep.txt"), [7u8; 16]).unwrap();
        let exclusions: Vec<String> = exclusions
            .iter()
            .map(|e| home.join(e).to_string_lossy().into_owned())
            .collect();
        let guard = Guard::for_home(&home, &exclusions, None);
        Fixture {
            _tmp: tmp,
            home,
            outside,
            guard,
        }
    }

    fn file(path: &Path, bytes: usize) -> u64 {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![1u8; bytes]).unwrap();
        crate::fsmeta::disk_size(&fs::metadata(path).unwrap())
    }

    fn wipe(rule: &TargetRule, guard: &Guard) -> CleanReport {
        clean_targets(&[rule], DeleteMethod::Permanent, guard, &Job::new(), false)
    }

    fn pick(paths: &[PathBuf], guard: &Guard) -> CleanReport {
        clean_paths(paths, DeleteMethod::Permanent, guard, &Job::new())
    }

    fn reasons(report: &CleanReport) -> Vec<&str> {
        report.failures.iter().map(|f| f.reason.as_str()).collect()
    }

    #[test]
    fn contents_mode_keeps_root_and_removes_symlinks_as_links() {
        let f = fixture(&[]);
        let cache = f.home.join(".cache/app");
        let one = file(&cache.join("1.bin"), 4096);
        let two = file(&cache.join("a/b/2.bin"), 4096);
        symlink(&f.outside, cache.join("dir-link")).unwrap();
        symlink(f.outside.join("keep.txt"), cache.join("a/file-link")).unwrap();
        symlink(f.home.join("gone"), cache.join("a/dangling")).unwrap();
        let report = wipe(&rule(&cache, Mode::Contents), &f.guard);
        assert_eq!(report.freed, one + two);
        assert_eq!(report.failed, 0, "{:?}", report.failures);
        assert_eq!(report.files, 2);
        assert_eq!(report.dirs, 2);
        assert!(cache.is_dir(), "root folder is kept");
        assert_eq!(fs::read_dir(&cache).unwrap().count(), 0);
        assert_eq!(fs::read(f.outside.join("keep.txt")).unwrap(), [7u8; 16]);
    }

    #[test]
    fn a_link_as_target_is_never_followed() {
        let f = fixture(&[]);
        let link = f.home.join("link");
        symlink(&f.outside, &link).unwrap();
        // A folder deleted by hand: the link is refused, as on Windows.
        let report = pick(std::slice::from_ref(&link), &f.guard);
        assert_eq!(reasons(&report), ["link"]);
        assert!(link.symlink_metadata().is_ok());
        // A rule pointing at a link expands to nothing.
        let report = wipe(&rule(&link, Mode::Contents), &f.guard);
        assert_eq!(report.freed, 0);
        // A path through a link reaches the outside and is refused.
        let report = pick(&[link.join("keep.txt")], &f.guard);
        assert_eq!(reasons(&report), ["protected"]);
        assert!(f.outside.join("keep.txt").exists());
    }

    #[test]
    fn a_path_through_a_link_inside_home_is_deleted_at_its_real_place() {
        let f = fixture(&[]);
        let real = f.home.join("real");
        file(&real.join("x/a.bin"), 4096);
        symlink(&real, f.home.join("alias")).unwrap();
        let report = pick(&[f.home.join("alias/x")], &f.guard);
        assert_eq!(report.failed, 0, "{:?}", report.failures);
        assert!(!real.join("x").exists());
        assert!(f.home.join("alias").symlink_metadata().is_ok());
    }

    #[test]
    fn a_folder_turned_into_a_link_stops_the_trusted_walk() {
        let f = fixture(&[]);
        let parent = f.home.join("parent");
        file(&parent.join("a.bin"), 4096);
        let job = Job::new();
        let acc = Acc::new(&job);
        // What a swap after the guard's check looks like: the checked path now holds a link.
        fs::rename(&parent, f.home.join("moved")).unwrap();
        symlink(&f.outside, &parent).unwrap();
        linux_delete::permanent_dir(&parent, &[], &acc);
        linux_delete::permanent_contents(&parent, None, &[], &[], &acc);
        linux_delete::permanent_file(&parent.join("keep.txt"), &acc);
        let report = acc.into_report(vec![]);
        assert_eq!(reasons(&report), ["link", "link", "link"]);
        assert!(f.outside.join("keep.txt").exists());
    }

    #[test]
    fn exclusions_survive_a_wipe_and_a_folder_delete() {
        let f = fixture(&[".cache/app/keepme", "work/app/deep/keep.bin"]);
        let cache = f.home.join(".cache/app");
        let kept = file(&cache.join("keepme/k.bin"), 4096);
        let dropped = file(&cache.join("drop/d.bin"), 4096);
        let report = wipe(&rule(&cache, Mode::Contents), &f.guard);
        assert_eq!(report.freed, dropped);
        assert!(kept > 0 && cache.join("keepme/k.bin").exists());
        assert!(!cache.join("drop").exists());

        let app = f.home.join("work/app");
        file(&app.join("deep/keep.bin"), 4096);
        let gone = file(&app.join("deep/gone.bin"), 4096);
        let also = file(&app.join("other/x.bin"), 4096);
        let report = pick(std::slice::from_ref(&app), &f.guard);
        assert_eq!(report.freed, gone + also);
        assert!(app.join("deep/keep.bin").exists());
        assert!(!app.join("deep/gone.bin").exists());
        assert!(!app.join("other").exists());
        // Excluded folders refuse a direct delete too.
        let report = pick(&[app.join("deep/keep.bin")], &f.guard);
        assert_eq!(reasons(&report), ["protected"]);
    }

    #[test]
    fn excluded_names_and_cutoff_hold() {
        let f = fixture(&[]);
        let temp = f.home.join("tmp-app");
        file(&temp.join("quit.blend"), 4096);
        file(&temp.join("gradio/out.png"), 4096);
        let junk = file(&temp.join("junk/a.tmp"), 4096);
        let mut r = rule(&temp, Mode::Contents);
        r.exclude = vec!["*.blend".into(), "gradio".into()];
        let report = wipe(&r, &f.guard);
        assert_eq!(report.freed, junk);
        assert!(temp.join("quit.blend").exists());
        assert!(temp.join("gradio/out.png").exists());
        assert!(!temp.join("junk").exists());

        let young = f.home.join("young");
        file(&young.join("new.bin"), 4096);
        let mut r = rule(&young, Mode::Contents);
        r.min_age_hours = Some(1);
        let report = wipe(&r, &f.guard);
        assert_eq!(report.freed, 0);
        assert!(young.join("new.bin").exists(), "too new to go");
    }

    #[test]
    fn protected_names_stay_inside_a_deleted_folder() {
        let f = fixture(&[]);
        let project = f.home.join("project");
        file(&project.join(".git/config"), 100);
        file(&project.join("sub/lost+found/x"), 100);
        file(&project.join("sub/.Trash-1000/files/y"), 100);
        file(&project.join("src/main.rs"), 100);
        let report = pick(std::slice::from_ref(&project), &f.guard);
        assert_eq!(report.failed, 0);
        assert!(project.join(".git/config").exists());
        assert!(project.join("sub/lost+found/x").exists());
        assert!(project.join("sub/.Trash-1000/files/y").exists());
        assert!(!project.join("src").exists());
    }

    #[test]
    fn files_mode_is_not_recursive_and_compares_case() {
        let f = fixture(&[]);
        let logs = f.home.join("Logs");
        let a = file(&logs.join("a.log"), 4096);
        file(&logs.join("b.txt"), 4096);
        file(&logs.join("C.LOG"), 4096);
        file(&logs.join("nested/c.log"), 4096);
        let report = wipe(&rule(&logs, Mode::Files), &f.guard);
        assert_eq!(report.freed, a);
        assert!(logs.join("b.txt").exists());
        assert!(logs.join("C.LOG").exists(), "globs compare case on Linux");
        assert!(logs.join("nested/c.log").exists());
    }

    #[test]
    fn hand_picked_paths_respect_the_guard() {
        let f = fixture(&[]);
        let big = f.home.join("big.bin");
        let size = file(&big, 8192);
        let report = pick(
            &[
                big.clone(),
                PathBuf::from("/etc/passwd"),
                f.outside.join("keep.txt"),
                f.home.clone(),
            ],
            &f.guard,
        );
        assert_eq!(report.freed, size);
        assert!(!big.exists());
        assert_eq!(reasons(&report), ["protected", "protected", "protected"]);
        assert!(f.outside.join("keep.txt").exists());
        assert!(f.home.exists());
    }

    #[test]
    fn a_guard_for_root_deletes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let file_path = fs::canonicalize(tmp.path()).unwrap().join("x.bin");
        fs::write(&file_path, b"x").unwrap();
        // This is what `Guard::new` builds under root; `Guard::default` refuses the same way.
        let report = pick(std::slice::from_ref(&file_path), &Guard::default());
        assert_eq!(reasons(&report), ["protected"]);
        assert!(file_path.exists());
    }

    #[test]
    fn read_only_files_go_without_clearing_anything() {
        let f = fixture(&[]);
        let dir = f.home.join("ro");
        let path = dir.join("locked.bin");
        file(&path, 4096);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        // Another name of the file shows its mode was left alone after the delete.
        let other = f.home.join("other-name.bin");
        fs::hard_link(&path, &other).unwrap();
        let report = pick(std::slice::from_ref(&dir), &f.guard);
        assert_eq!(report.failed, 0, "{:?}", report.failures);
        assert!(!dir.exists());
        let mode = fs::metadata(&other).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o444);
    }

    #[test]
    fn names_that_are_not_utf8_are_handled_as_bytes() {
        let f = fixture(&[]);
        let cache = f.home.join("odd-cache");
        let name = OsStr::from_bytes(b"caf\xe9");
        let size = file(&cache.join(name).join("x.bin"), 4096);
        let raw = cache.join(OsStr::from_bytes(b"\xff\xfe.bin"));
        let more = file(&raw, 4096);
        let report = wipe(&rule(&cache, Mode::Contents), &f.guard);
        assert_eq!(report.freed, size + more);
        assert_eq!(report.failed, 0);
        assert_eq!(fs::read_dir(&cache).unwrap().count(), 0);
    }

    #[test]
    fn hard_links_free_their_space_once() {
        let f = fixture(&[]);
        let cache = f.home.join("linked");
        let size = file(&cache.join("a.bin"), 8192);
        fs::hard_link(cache.join("a.bin"), cache.join("b.bin")).unwrap();
        let report = wipe(&rule(&cache, Mode::Contents), &f.guard);
        assert_eq!(report.freed, size);
        assert_eq!(report.files, 2);
    }

    #[test]
    fn a_hard_link_that_stays_elsewhere_frees_nothing() {
        let f = fixture(&[]);
        let cache = f.home.join("half-linked");
        file(&cache.join("a.bin"), 8192);
        let other = f.home.join("kept.bin");
        fs::hard_link(cache.join("a.bin"), &other).unwrap();
        let report = wipe(&rule(&cache, Mode::Contents), &f.guard);
        assert_eq!(report.files, 1);
        assert_eq!(report.freed, 0, "the space is still held by the other name");
        assert_eq!(fs::read(&other).unwrap().len(), 8192);
    }

    #[test]
    fn folders_the_mount_table_names_are_not_entered() {
        let f = fixture(&[]);
        let cache = f.home.join("mounted");
        let mount = cache.join("sub/usb");
        file(&mount.join("photo.jpg"), 4096);
        let plain = file(&cache.join("plain.bin"), 4096);
        let table = MountTable::from_mounts(parse_mountinfo(
            format!("2 1 8:17 / {} rw - ext4 /dev/sdb1 rw", mount.display()).as_bytes(),
        ));

        // The guard already refuses a parent of a mount point, but the walker holds on its own.
        let job = Job::new();
        let acc = Acc::new(&job);
        linux_delete::purge_below(&cache, None, &[], &[], &acc, &table);
        let report = acc.into_report(vec![]);
        assert_eq!(report.freed, plain);
        assert_eq!(reasons(&report), ["protected"]);
        assert!(mount.join("photo.jpg").exists());
        assert!(mount.is_dir());

        let job = Job::new();
        let acc = Acc::new(&job);
        linux_delete::remove_dir(&cache, &[], &acc, &table);
        let report = acc.into_report(vec![]);
        assert_eq!(reasons(&report), ["protected"]);
        assert!(mount.join("photo.jpg").exists());
        assert!(cache.exists(), "a folder with a mount inside stays");
    }

    #[test]
    fn a_recycle_never_lists_files_from_another_mount() {
        let f = fixture(&[]);
        let dir = f.home.join("rec");
        file(&dir.join("a.bin"), 10);
        file(&dir.join("nas/b.bin"), 10);
        let nas = dir.join("nas");
        let table = MountTable::from_mounts(parse_mountinfo(
            format!("2 1 0:9 / {} rw - nfs4 nas:/x rw", nas.display()).as_bytes(),
        ));
        let ctx = WalkContext::with_mounts(&dir, table);
        let mut out = Vec::new();
        eligible_files(&dir, Some(i64::MAX), &[], &[], &ctx, &mut out);
        assert_eq!(out, vec![dir.join("a.bin")]);
    }

    /// Mounts a real file system over `path` for as long as it lives. `None` without the rights
    /// to mount, which is the normal case: `docs/architecture.md` says how the checks run with them.
    struct RealMount(PathBuf);

    impl RealMount {
        fn new(args: &[&OsStr], path: &Path) -> Option<RealMount> {
            let ok = Command::new("mount")
                .args(args)
                .arg(path)
                .output()
                .is_ok_and(|o| o.status.success());
            if !ok {
                eprintln!("skipped: cannot mount here");
            }
            ok.then(|| RealMount(path.to_path_buf()))
        }
    }

    impl Drop for RealMount {
        fn drop(&mut self) {
            let _ = Command::new("umount").arg("-l").arg(&self.0).output();
        }
    }

    #[test]
    fn a_real_mount_is_not_crossed_even_with_an_outdated_table() {
        let f = fixture(&[]);
        let cache = f.home.join("real-mount");
        let point = cache.join("m");
        fs::create_dir_all(&point).unwrap();
        let args: Vec<&OsStr> = ["-t", "tmpfs", "none"].iter().map(OsStr::new).collect();
        let Some(_mount) = RealMount::new(&args, &point) else {
            return;
        };
        file(&point.join("on-tmpfs.bin"), 4096);
        let plain = file(&cache.join("plain.bin"), 4096);

        let guard = Guard::for_home(&f.home, &[], None);
        assert!(guard.check(&cache, Origin::User).is_err(), "a mount inside");
        assert!(
            guard.check(&point, Origin::User).is_err(),
            "the mount itself"
        );

        // An empty table is what a mount made after the table was read looks like.
        let job = Job::new();
        let acc = Acc::new(&job);
        linux_delete::purge_below(&cache, None, &[], &[], &acc, &MountTable::default());
        let report = acc.into_report(vec![]);
        assert_eq!(report.freed, plain);
        assert_eq!(reasons(&report), ["protected"]);
        assert!(point.join("on-tmpfs.bin").exists());
    }

    #[test]
    fn a_real_bind_mount_is_not_crossed_although_it_keeps_its_device() {
        let f = fixture(&[]);
        let cache = f.home.join("real-bind");
        let point = cache.join("m");
        fs::create_dir_all(&point).unwrap();
        let source = f.outside.clone();
        let args: Vec<&OsStr> = vec![OsStr::new("--bind"), source.as_os_str()];
        let Some(_mount) = RealMount::new(&args, &point) else {
            return;
        };
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            fs::metadata(&point).unwrap().dev(),
            fs::metadata(&cache).unwrap().dev(),
            "the device check alone would not see this mount"
        );
        let table = MountTable::read_current().unwrap();
        assert!(table.at(&point).is_some_and(|m| m.root != Path::new("/")));

        for mounts in [MountTable::default(), table] {
            let job = Job::new();
            let acc = Acc::new(&job);
            linux_delete::purge_below(&cache, None, &[], &[], &acc, &mounts);
            let report = acc.into_report(vec![]);
            assert_eq!(reasons(&report), ["protected"]);
            assert!(f.outside.join("keep.txt").exists());
            assert!(point.join("keep.txt").exists());
        }
    }

    #[test]
    fn running_apps_block_a_rule() {
        let f = fixture(&[]);
        let mut r = rule(&f.home, Mode::Contents);
        let exe = std::env::current_exe().unwrap();
        r.close_apps = vec![exe.file_name().unwrap().to_string_lossy().into_owned()];
        let report = wipe(&r, &f.guard);
        assert_eq!(reasons(&report), ["app_running"]);
    }
}
