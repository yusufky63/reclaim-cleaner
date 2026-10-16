//! "What is using this?": the processes that hold a file or folder open.
//!
//! A single file is checked with the Restart Manager, the documented API installers use to find
//! what keeps a file busy: it sees open handles and loaded program files, works without
//! administrator rights, and names services and other users' processes.
//!
//! The Restart Manager refuses folders and takes about a tenth of a second per file, so a folder
//! is checked differently, with a time budget:
//! - every process whose program file lives in the folder;
//! - the folder and its subfolders, which also finds programs that have one open as their
//!   working folder (a terminal opened there);
//! - a bounded set of files inside, program files first, then the most recently changed.
//!
//! Each path is asked for the processes using it (`FileProcessIdsUsingFileInformation`), which
//! opens the path for its attributes only. The processes found are then described by the
//! Restart Manager (window, service, Explorer, system).
//!
//! The only action offered is a graceful close: the same request as clicking the window's close
//! button. Nothing here kills a process or closes another process's handles.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rayon::prelude::*;
use serde::Serialize;
use windows::Wdk::Storage::FileSystem::{
    FileProcessIdsUsingFileInformation, NtQueryInformationFile,
};
use windows::Win32::Foundation::{
    CloseHandle, E_ACCESSDENIED, ERROR_MORE_DATA, ERROR_SUCCESS, FILETIME, HWND, LPARAM,
    WAIT_OBJECT_0, WPARAM,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::IO::IO_STATUS_BLOCK;
use windows::Win32::System::RestartManager::{
    CCH_RM_SESSION_KEY, RM_APP_TYPE, RM_PROCESS_INFO, RM_UNIQUE_PROCESS, RmConsole, RmCritical,
    RmEndSession, RmExplorer, RmGetList, RmMainWindow, RmOtherWindow, RmRegisterResources,
    RmService, RmStartSession,
};
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SYNCHRONIZE, QueryFullProcessImageNameW, WaitForSingleObject,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GetWindow, GetWindowThreadProcessId, IsWindowVisible, PostMessageW,
    WM_CLOSE,
};
use windows::core::{BOOL, PCWSTR, PWSTR};

use crate::fsmeta::{is_link, mtime};
use crate::paths::{is_within, key};
use crate::winstr::{from_wide, wide};
use crate::{Error, Result};

/// Files inside a folder that are checked at most.
pub const MAX_FILES: usize = 1500;
/// Subfolders checked at most.
pub const MAX_DIRS: usize = 400;
/// Entries looked at while choosing them, so a huge folder stays quick.
const MAX_VISITED: usize = 40_000;
/// Time after which no new path is checked; the report then says the check was partial.
const BUDGET: Duration = Duration::from_secs(8);
/// Each query mostly waits on the kernel, so more threads than cores still pay off.
const THREADS: usize = 16;
/// Program files are what a running app keeps open, so they are checked first.
const FIRST_EXTS: &[&str] = &["exe", "dll", "sys", "node", "pyd", "ocx", "jar", "db"];

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum LockerKind {
    /// An app with a window.
    App,
    /// A process without a window of its own.
    Background,
    Service,
    Explorer,
    Console,
    /// Windows itself (critical system processes).
    System,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Locker {
    pub pid: u32,
    /// Process start time (FILETIME ticks). A close request must repeat it, so a reused process
    /// id never closes a different app.
    pub started: u64,
    pub name: String,
    pub exe: Option<String>,
    pub service: Option<String>,
    pub kind: LockerKind,
    /// It has files or folders open.
    pub uses_files: bool,
    /// Its program file lives inside the folder.
    pub runs_from: bool,
    /// It has a window that can be asked to close.
    pub can_close: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LockReport {
    pub path: String,
    pub is_dir: bool,
    /// Files checked.
    pub checked: u64,
    /// Folders checked (the folder itself included).
    pub folders: u64,
    /// The folder has more files or subfolders than were checked.
    pub truncated: bool,
    /// Paths that could not be checked, usually because Windows itself holds them (the page
    /// file, registry hives).
    pub unchecked: u64,
    pub lockers: Vec<Locker>,
}

fn filetime(t: FILETIME) -> u64 {
    (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)
}

fn to_filetime(v: u64) -> FILETIME {
    FILETIME {
        dwLowDateTime: v as u32,
        dwHighDateTime: (v >> 32) as u32,
    }
}

/// Ids of the processes that have `path` (a file or folder) open, including programs that
/// have it loaded and, for folders, use it as their working folder. `None` when the path
/// cannot be opened even for its attributes.
pub fn pids_using(path: &Path) -> Option<Vec<u32>> {
    let w = wide(path);
    let handle = unsafe {
        CreateFileW(
            PCWSTR(w.as_ptr()),
            FILE_READ_ATTRIBUTES.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    }
    .ok()?;
    // FILE_PROCESS_IDS_USING_FILE_INFORMATION: a count, then pointer-sized process ids.
    let mut buf = vec![0usize; 64];
    let mut out = None;
    for _ in 0..6 {
        let mut status_block = IO_STATUS_BLOCK::default();
        let status = unsafe {
            NtQueryInformationFile(
                handle,
                &mut status_block,
                buf.as_mut_ptr() as *mut core::ffi::c_void,
                (buf.len() * size_of::<usize>()) as u32,
                FileProcessIdsUsingFileInformation,
            )
        };
        // STATUS_INFO_LENGTH_MISMATCH: the list did not fit.
        if status.0 as u32 == 0xC000_0004 {
            let grown = buf.len() * 4;
            buf.resize(grown, 0);
            continue;
        }
        if status.is_ok() {
            let n = (buf[0] & 0xFFFF_FFFF).min(buf.len() - 1);
            out = Some(buf[1..=n].iter().map(|&p| p as u32).collect());
        }
        break;
    }
    let _ = unsafe { CloseHandle(handle) };
    out
}

/// One Restart Manager session over `files` and/or `processes`. Errors carry the Win32 code.
fn rm_list(
    files: &[PathBuf],
    processes: &[RM_UNIQUE_PROCESS],
) -> std::result::Result<Vec<RM_PROCESS_INFO>, u32> {
    let mut session = 0u32;
    let mut session_key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
    let r = unsafe { RmStartSession(&mut session, None, PWSTR(session_key.as_mut_ptr())) };
    if r != ERROR_SUCCESS {
        return Err(r.0);
    }
    let res = (|| {
        let names: Vec<Vec<u16>> = files.iter().map(wide).collect();
        let ptrs: Vec<PCWSTR> = names.iter().map(|w| PCWSTR(w.as_ptr())).collect();
        let r = unsafe {
            RmRegisterResources(
                session,
                (!ptrs.is_empty()).then_some(ptrs.as_slice()),
                (!processes.is_empty()).then_some(processes),
                None,
            )
        };
        if r != ERROR_SUCCESS {
            return Err(r.0);
        }
        let mut infos: Vec<RM_PROCESS_INFO> = Vec::new();
        // The list can grow between calls; a few retries are plenty.
        for _ in 0..6 {
            let (mut needed, mut count, mut reasons) = (0u32, infos.len() as u32, 0u32);
            let ptr = (!infos.is_empty()).then_some(infos.as_mut_ptr());
            let r = unsafe { RmGetList(session, &mut needed, &mut count, ptr, &mut reasons) };
            if r == ERROR_MORE_DATA {
                infos.resize(needed as usize + 4, RM_PROCESS_INFO::default());
                continue;
            }
            if r != ERROR_SUCCESS {
                return Err(r.0);
            }
            infos.truncate(count as usize);
            return Ok(infos);
        }
        Err(ERROR_MORE_DATA.0)
    })();
    let _ = unsafe { RmEndSession(session) };
    res
}

/// Up to [`MAX_FILES`] files and [`MAX_DIRS`] subfolders inside `dir`, breadth first so the
/// top of the folder is always covered. Files: program files first, then the most recently
/// changed, which are the likeliest to be open. Links are not followed.
pub fn folder_items(dir: &Path) -> (Vec<PathBuf>, Vec<PathBuf>, bool) {
    let mut queue = VecDeque::from([dir.to_path_buf()]);
    let mut files: Vec<(bool, i64, PathBuf)> = Vec::new();
    let mut dirs = Vec::new();
    let mut visited = 0usize;
    let mut cut = false;
    'walk: while let Some(d) = queue.pop_front() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for entry in rd.flatten() {
            visited += 1;
            if visited > MAX_VISITED {
                cut = true;
                break 'walk;
            }
            let Ok(ft) = entry.file_type() else { continue };
            if is_link(&ft) {
                continue;
            }
            let path = entry.path();
            if ft.is_dir() {
                dirs.push(path.clone());
                queue.push_back(path);
                continue;
            }
            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            let changed = entry.metadata().map(|m| mtime(&m)).unwrap_or(0);
            files.push((FIRST_EXTS.contains(&ext.as_str()), changed, path));
        }
    }
    files.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    let truncated = cut || files.len() > MAX_FILES || dirs.len() > MAX_DIRS;
    files.truncate(MAX_FILES);
    dirs.truncate(MAX_DIRS);
    (files.into_iter().map(|f| f.2).collect(), dirs, truncated)
}

struct ProcFacts {
    exe: Option<PathBuf>,
    started: Option<u64>,
}

fn proc_facts(pid: u32) -> ProcFacts {
    let mut facts = ProcFacts {
        exe: None,
        started: None,
    };
    let Ok(h) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else {
        return facts;
    };
    let mut buf = vec![0u16; 32_768];
    let mut len = buf.len() as u32;
    if unsafe {
        QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)
    }
    .is_ok()
    {
        facts.exe = Some(PathBuf::from(String::from_utf16_lossy(
            &buf[..len as usize],
        )));
    }
    let (mut created, mut exited, mut kernel, mut user) = Default::default();
    if unsafe { GetProcessTimes(h, &mut created, &mut exited, &mut kernel, &mut user) }.is_ok() {
        facts.started = Some(filetime(created));
    }
    let _ = unsafe { CloseHandle(h) };
    facts
}

/// Visible top-level windows that no other window owns, by process id: the windows a user
/// would close by hand.
fn main_windows() -> HashMap<u32, Vec<isize>> {
    unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let map = unsafe { &mut *(lparam.0 as *mut HashMap<u32, Vec<isize>>) };
        let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
        let owned = unsafe { GetWindow(hwnd, GW_OWNER) }.is_ok_and(|o| !o.is_invalid());
        if visible && !owned {
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
            if pid != 0 {
                map.entry(pid).or_default().push(hwnd.0 as isize);
            }
        }
        BOOL(1)
    }
    let mut map: HashMap<u32, Vec<isize>> = HashMap::new();
    let _ = unsafe { EnumWindows(Some(each), LPARAM(&mut map as *mut _ as isize)) };
    map
}

fn kind_of(app_type: RM_APP_TYPE, has_window: bool) -> LockerKind {
    match app_type {
        t if t == RmMainWindow || t == RmOtherWindow => LockerKind::App,
        t if t == RmService => LockerKind::Service,
        t if t == RmExplorer => LockerKind::Explorer,
        t if t == RmConsole => LockerKind::Console,
        t if t == RmCritical => LockerKind::System,
        _ if has_window => LockerKind::App,
        _ => LockerKind::Background,
    }
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Builds the report entries for `pids` (id, has files open, runs from the folder), described
/// by `infos` where the Restart Manager knows them.
fn describe(
    pids: &[(u32, bool, bool)],
    infos: &[RM_PROCESS_INFO],
    windows: &HashMap<u32, Vec<isize>>,
) -> Vec<Locker> {
    let own = std::process::id();
    let mut out: Vec<Locker> = pids
        .iter()
        .map(|&(pid, uses_files, runs_from)| {
            let info = infos.iter().find(|i| i.Process.dwProcessId == pid);
            let facts = proc_facts(pid);
            let has_window = windows.contains_key(&pid);
            let kind = info.map_or_else(
                || kind_of(RM_APP_TYPE(0), has_window),
                |i| kind_of(i.ApplicationType, has_window),
            );
            let name = info
                .map(|i| from_wide(&i.strAppName))
                .filter(|n| !n.is_empty())
                .or_else(|| facts.exe.as_deref().map(file_name))
                .unwrap_or_default();
            Locker {
                pid,
                started: info
                    .map(|i| filetime(i.Process.ProcessStartTime))
                    .or(facts.started)
                    .unwrap_or(0),
                name,
                exe: facts.exe.map(|p| p.to_string_lossy().into_owned()),
                service: info
                    .map(|i| from_wide(&i.strServiceShortName))
                    .filter(|s| !s.is_empty()),
                kind,
                uses_files,
                runs_from,
                can_close: kind == LockerKind::App && has_window && pid != own,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        (a.kind as u8, a.name.to_lowercase()).cmp(&(b.kind as u8, b.name.to_lowercase()))
    });
    out
}

fn check_file(path: &Path) -> LockReport {
    let (infos, unchecked) = match rm_list(&[path.to_path_buf()], &[]) {
        Ok(list) => (list, 0),
        Err(_) => (Vec::new(), 1),
    };
    let mut pids: Vec<(u32, bool, bool)> = Vec::new();
    for i in &infos {
        if !pids.iter().any(|p| p.0 == i.Process.dwProcessId) {
            pids.push((i.Process.dwProcessId, true, false));
        }
    }
    LockReport {
        path: path.to_string_lossy().into_owned(),
        is_dir: false,
        checked: 1,
        folders: 0,
        truncated: false,
        unchecked,
        lockers: describe(&pids, &infos, &main_windows()),
    }
}

fn check_folder(dir: &Path) -> LockReport {
    let deadline = Instant::now() + BUDGET;
    let (files, dirs, mut truncated) = folder_items(dir);
    // Folders first: they are cheap and catch working folders; then program files.
    let targets: Vec<(&Path, bool)> = std::iter::once((dir, true))
        .chain(dirs.iter().map(|d| (d.as_path(), true)))
        .chain(files.iter().map(|f| (f.as_path(), false)))
        .collect();
    let query = || {
        targets
            .par_iter()
            .map(|(p, is_dir)| (Instant::now() < deadline).then(|| (*is_dir, pids_using(p))))
            .collect::<Vec<_>>()
    };
    let results = match rayon::ThreadPoolBuilder::new().num_threads(THREADS).build() {
        Ok(pool) => pool.install(query),
        Err(_) => query(),
    };
    let (mut checked, mut folders, mut unchecked) = (0u64, 0u64, 0u64);
    let mut using: Vec<u32> = Vec::new();
    for r in results {
        match r {
            None => truncated = true,
            Some((is_dir, found)) => {
                if is_dir {
                    folders += 1;
                } else {
                    checked += 1;
                }
                match found {
                    Some(list) => using.extend(list),
                    None => unchecked += 1,
                }
            }
        }
    }
    using.sort_unstable();
    using.dedup();

    let dir_key = key(dir);
    let runs_from: Vec<u32> = crate::system::process_ids()
        .into_iter()
        .filter(|&pid| {
            proc_facts(pid)
                .exe
                .is_some_and(|exe| is_within(&key(&exe), &dir_key))
        })
        .collect();
    let mut pids: Vec<(u32, bool, bool)> = using
        .iter()
        .map(|&pid| (pid, true, runs_from.contains(&pid)))
        .collect();
    for &pid in &runs_from {
        if !using.contains(&pid) {
            pids.push((pid, false, true));
        }
    }
    // The Restart Manager describes the processes it is given: window, service, system.
    let processes: Vec<RM_UNIQUE_PROCESS> = pids
        .iter()
        .filter_map(|p| {
            proc_facts(p.0).started.map(|t| RM_UNIQUE_PROCESS {
                dwProcessId: p.0,
                ProcessStartTime: to_filetime(t),
            })
        })
        .collect();
    let infos = if processes.is_empty() {
        Vec::new()
    } else {
        rm_list(&[], &processes).unwrap_or_default()
    };
    LockReport {
        path: dir.to_string_lossy().into_owned(),
        is_dir: true,
        checked,
        folders,
        truncated,
        unchecked,
        lockers: describe(&pids, &infos, &main_windows()),
    }
}

/// Every process using `path` (a file or a folder).
pub fn find_lockers(path: &Path) -> Result<LockReport> {
    let md = path.symlink_metadata().map_err(|e| Error::io(path, e))?;
    if is_link(&md.file_type()) {
        return Err(Error::Other(String::from("links are not followed")));
    }
    Ok(if md.is_dir() {
        check_folder(path)
    } else {
        check_file(path)
    })
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CloseOutcome {
    /// The app exited.
    Closed,
    /// The app got the request but is still running, usually asking to save work.
    StillRunning,
    /// The app runs as administrator, so a standard-user request cannot reach it.
    Denied,
    /// The process is gone or its id now belongs to another process.
    Gone,
    /// Not an app with a window, or ReClaim itself.
    NotAllowed,
}

/// Asks the app to close its windows, exactly like clicking their close buttons, and waits up
/// to `wait_ms` for it to exit. Unsaved work prompts still appear; nothing is forced.
pub fn close_gracefully(pid: u32, started: u64, wait_ms: u32) -> CloseOutcome {
    if pid == std::process::id() || pid == 0 {
        return CloseOutcome::NotAllowed;
    }
    if proc_facts(pid).started != Some(started) {
        return CloseOutcome::Gone;
    }
    let Some(handles) = main_windows().remove(&pid) else {
        return CloseOutcome::NotAllowed;
    };
    let (mut posted, mut denied) = (0, false);
    for h in handles {
        match unsafe {
            PostMessageW(
                Some(HWND(h as *mut core::ffi::c_void)),
                WM_CLOSE,
                WPARAM(0),
                LPARAM(0),
            )
        } {
            Ok(()) => posted += 1,
            Err(e) if e.code() == E_ACCESSDENIED => denied = true,
            Err(_) => {}
        }
    }
    if posted == 0 {
        return if denied {
            CloseOutcome::Denied
        } else {
            CloseOutcome::NotAllowed
        };
    }
    let Ok(h) = (unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) }) else {
        return CloseOutcome::Closed;
    };
    let waited = unsafe { WaitForSingleObject(h, wait_ms) };
    let _ = unsafe { CloseHandle(h) };
    if waited == WAIT_OBJECT_0 {
        CloseOutcome::Closed
    } else {
        CloseOutcome::StillRunning
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_this_process_holding_a_file_and_a_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join(r"sub\held.txt");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, b"x").unwrap();
        fs::write(tmp.path().join("free.txt"), b"y").unwrap();
        let me = std::process::id();

        let handle = fs::File::open(&file).unwrap();
        assert!(pids_using(&file).unwrap().contains(&me));
        let r = find_lockers(&file).unwrap();
        assert!(!r.is_dir && r.checked == 1);
        let mine = r
            .lockers
            .iter()
            .find(|l| l.pid == me)
            .expect("this process is listed");
        assert!(
            mine.uses_files && !mine.can_close,
            "ReClaim never offers to close itself"
        );
        assert!(mine.started > 0);

        let r = find_lockers(tmp.path()).unwrap();
        assert!(r.is_dir && r.checked == 2 && r.folders == 2 && !r.truncated);
        assert!(r.lockers.iter().any(|l| l.pid == me && l.uses_files));

        drop(handle);
        let r = find_lockers(&file).unwrap();
        assert!(r.lockers.iter().all(|l| l.pid != me));
        assert_eq!(close_gracefully(me, 0, 0), CloseOutcome::NotAllowed);
    }

    #[test]
    fn picks_program_files_and_recent_files_first_and_caps_the_count() {
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..(MAX_FILES + 5) {
            fs::write(tmp.path().join(format!("{i}.txt")), b"").unwrap();
        }
        fs::create_dir_all(tmp.path().join("bin")).unwrap();
        fs::write(tmp.path().join(r"bin\app.exe"), b"").unwrap();
        let (files, dirs, truncated) = folder_items(tmp.path());
        assert!(truncated);
        assert_eq!(files.len(), MAX_FILES);
        assert_eq!(dirs, vec![tmp.path().join("bin")]);
        assert!(files[0].ends_with(r"bin\app.exe"));
    }

    #[test]
    fn missing_paths_are_errors() {
        assert!(find_lockers(Path::new(r"C:\definitely\not\here.bin")).is_err());
        assert!(pids_using(Path::new(r"C:\definitely\not\here.bin")).is_none());
    }
}
