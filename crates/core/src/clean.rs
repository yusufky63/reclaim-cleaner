//! Executes reviewed cleanups.
//!
//! Every entry point re-checks its targets against the [`Guard`] right before deleting, never
//! follows links, and records what it did in a [`CleanReport`]. Locked and in-use files are
//! skipped and reported; processes are never killed.

use std::fs;
use std::io::ErrorKind;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::fsmeta::{is_cloud_only, is_link, mtime};
use crate::job::Job;
use crate::junk::{cutoff_for, matching_files};
use crate::paths::{Guard, Origin, expand_pattern, is_within, key};
use crate::rules::{Mode, TargetRule};
use crate::walk::measure_path;
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
        (_, Some(32)) | (_, Some(33)) => "in_use",
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

const DELETE: u32 = 0x0001_0000;
const FILE_SHARE_ALL: u32 = 0x7;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
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
            eligible_files(&path, cutoff, keep, skip, out);
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
    let keep = guard.exclusions_inside(dir);
    match method {
        DeleteMethod::Permanent => {
            purge_dir(dir, cutoff, &keep, skip, acc);
        }
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
                eligible_files(dir, cutoff, &keep, skip, &mut targets);
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
    let keep = guard.exclusions_inside(path);
    match method {
        DeleteMethod::Recycle => {
            if !keep.is_empty() {
                return acc.fail(path, "excluded_inside");
            }
            recycle(vec![path.to_path_buf()], acc);
        }
        DeleteMethod::Permanent if md.is_dir() => {
            if purge_dir(path, None, &keep, &[], acc) {
                match fs::remove_dir(path) {
                    Ok(()) => {
                        acc.dirs.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(e) => acc.fail_io(path, &e),
                }
            }
        }
        DeleteMethod::Permanent => match remove_file(path) {
            Ok(()) => acc.deleted_file(md.len()),
            Err(e) => acc.fail_io(path, &e),
        },
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
    job.set_stage("clean");
    job.set_total(paths.len() as u64);
    let acc = Acc::new(job);
    for p in paths {
        if job.is_cancelled() {
            break;
        }
        job.set_current(p);
        remove_one(p, method, Origin::User, guard, &acc);
        job.add_done(1);
    }
    acc.into_report(
        paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
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

#[cfg(test)]
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
