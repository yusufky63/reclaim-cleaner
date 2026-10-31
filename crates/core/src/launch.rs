//! Ways to start a quick clean from outside the app: a task in the taskbar and Start jump list,
//! and a shortcut on the desktop (the setup adds one to the Start menu too).
//!
//! Every one of them starts the app with `--quick-clean`, which only opens the quick clean
//! review on the Overview. The user still sees what will be removed and confirms once. Any
//! program can pass the flag, so it never deletes anything by itself.
//!
//! Only Windows has jump lists and `.lnk` shortcuts; elsewhere the jump list is a no-op and the
//! shortcut reports as unsupported.

use std::path::{Path, PathBuf};

#[cfg(windows)]
use windows::Win32::Foundation::PROPERTYKEY;
#[cfg(windows)]
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
#[cfg(windows)]
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoTaskMemFree, CoUninitialize, IPersistFile, STGM_READ,
};
#[cfg(windows)]
use windows::Win32::UI::Shell::Common::{IObjectArray, IObjectCollection};
#[cfg(windows)]
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
#[cfg(windows)]
use windows::Win32::UI::Shell::{
    DestinationList, EnumerableObjectCollection, FOLDERID_Desktop, ICustomDestinationList,
    IShellLinkW, KF_FLAG_DEFAULT, SHGetKnownFolderPath, ShellLink,
};
#[cfg(windows)]
use windows::core::{HSTRING, Interface};

use crate::{Error, Result};

/// Command-line flag that opens the quick clean review.
pub const QUICK_CLEAN_FLAG: &str = "--quick-clean";
/// File name of the desktop and Start menu shortcuts. It stays in English whatever the UI
/// language is, so the uninstaller can find it.
pub const SHORTCUT_NAME: &str = "ReClaim Quick Clean.lnk";
/// Longest title or description written into a shortcut. The shell's own limit for
/// descriptions is 1024 UTF-16 units, and jump list titles are cut far earlier on screen.
const MAX_TEXT: usize = 260;

/// `System.Title`, which a jump list shows as the task name.
#[cfg(windows)]
const PKEY_TITLE: PROPERTYKEY = PROPERTYKEY {
    fmtid: windows::core::GUID::from_u128(0xf29f85e0_4ff9_1068_ab91_08002b27b3d9),
    pid: 2,
};

/// Whether a command line asks for the quick clean review. The first item is the program.
pub fn quick_clean_requested(args: &[String]) -> bool {
    args.iter()
        .skip(1)
        .any(|a| a.eq_ignore_ascii_case(QUICK_CLEAN_FLAG))
}

/// Makes UI text safe to store in a shortcut: one line, no control characters, bounded length.
pub fn shortcut_text(s: &str) -> String {
    let one_line: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let trimmed = one_line.split_whitespace().collect::<Vec<_>>().join(" ");
    trimmed.chars().take(MAX_TEXT).collect()
}

/// Runs COM work on its own single-threaded apartment. The shell objects used here are
/// apartment-threaded, and the caller may be a thread that already chose another model.
#[cfg(windows)]
fn on_sta_thread<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    std::thread::spawn(move || {
        let inited = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
        let out = work();
        if inited {
            unsafe { CoUninitialize() };
        }
        out
    })
    .join()
    .map_err(|_| Error::Other(String::from("the shortcut thread stopped unexpectedly")))?
}

/// A shell link that starts `exe --quick-clean`.
#[cfg(windows)]
fn quick_clean_link(exe: &Path, title: &str, description: &str) -> Result<IShellLinkW> {
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(&HSTRING::from(exe.as_os_str()))?;
        link.SetArguments(&HSTRING::from(QUICK_CLEAN_FLAG))?;
        link.SetIconLocation(&HSTRING::from(exe.as_os_str()), 0)?;
        if let Some(dir) = exe.parent() {
            link.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()))?;
        }
        link.SetDescription(&HSTRING::from(shortcut_text(description)))?;
        let store: IPropertyStore = link.cast()?;
        store.SetValue(
            &PKEY_TITLE,
            &PROPVARIANT::from(shortcut_text(title).as_str()),
        )?;
        store.Commit()?;
        Ok(link)
    }
}

/// Replaces this app's jump list tasks with one "Quick clean" task. Without an explicit
/// AppUserModelID the list belongs to the ID Windows gives the running window, so it shows on
/// that taskbar button and on a pinned copy of it.
#[cfg(windows)]
pub fn set_jump_list(exe: &Path, title: &str, description: &str) -> Result<()> {
    if shortcut_text(title).is_empty() {
        return Err(Error::Other(String::from(
            "the jump list task needs a title",
        )));
    }
    let (exe, title, description) = (
        exe.to_path_buf(),
        title.to_string(),
        description.to_string(),
    );
    on_sta_thread(move || unsafe {
        let list: ICustomDestinationList =
            CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER)?;
        let mut slots = 0u32;
        let _removed: IObjectArray = list.BeginList(&mut slots)?;
        let tasks: IObjectCollection =
            CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER)?;
        tasks.AddObject(&quick_clean_link(&exe, &title, &description)?)?;
        list.AddUserTasks(&tasks.cast::<IObjectArray>()?)?;
        list.CommitList()?;
        Ok(())
    })
}

/// Program and arguments of a `.lnk` file, or `None` when it cannot be read as one.
#[cfg(windows)]
fn read_link(path: &Path) -> Option<(PathBuf, String)> {
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        link.cast::<IPersistFile>()
            .ok()?
            .Load(&HSTRING::from(path.as_os_str()), STGM_READ)
            .ok()?;
        let mut target = [0u16; 32_768];
        link.GetPath(&mut target, std::ptr::null_mut(), 0).ok()?;
        let mut args = [0u16; 4096];
        link.GetArguments(&mut args).ok()?;
        Some((
            PathBuf::from(crate::winstr::from_wide(&target)),
            crate::winstr::from_wide(&args),
        ))
    }
}

/// Writes the quick clean shortcut at `path`. An existing file is replaced only when it is a
/// plain `.lnk` that already opens a quick clean (an older copy of this shortcut); anything
/// else with that name belongs to the user and is left alone.
#[cfg(windows)]
pub fn write_shortcut(path: &Path, exe: &Path, title: &str, description: &str) -> Result<()> {
    let (path, exe, title, description) = (
        path.to_path_buf(),
        exe.to_path_buf(),
        title.to_string(),
        description.to_string(),
    );
    on_sta_thread(move || {
        match std::fs::symlink_metadata(&path) {
            Ok(meta) => {
                let ours = meta.is_file()
                    && !crate::fsmeta::is_link(&meta.file_type())
                    && read_link(&path).is_some_and(|(_, args)| args.trim() == QUICK_CLEAN_FLAG);
                if !ours {
                    return Err(Error::Other(format!(
                        "{} already exists and is not a ReClaim shortcut",
                        path.display()
                    )));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::io(&path, e)),
        }
        let link = quick_clean_link(&exe, &title, &description)?;
        unsafe {
            link.cast::<IPersistFile>()?
                .Save(&HSTRING::from(path.as_os_str()), true)?;
        }
        Ok(())
    })
}

#[cfg(windows)]
fn desktop_dir() -> Result<PathBuf> {
    unsafe {
        let raw = SHGetKnownFolderPath(&FOLDERID_Desktop, KF_FLAG_DEFAULT, None)?;
        let dir = raw.to_string();
        CoTaskMemFree(Some(raw.0 as *const _));
        dir.map(PathBuf::from)
            .map_err(|e| Error::Other(e.to_string()))
    }
}

/// Puts the quick clean shortcut on this user's desktop and returns its path.
#[cfg(windows)]
pub fn create_desktop_shortcut(exe: &Path, title: &str, description: &str) -> Result<PathBuf> {
    let path = desktop_dir()?.join(SHORTCUT_NAME);
    write_shortcut(&path, exe, title, description)?;
    Ok(path)
}

#[cfg(not(windows))]
pub fn set_jump_list(_exe: &Path, _title: &str, _description: &str) -> Result<()> {
    Ok(())
}

#[cfg(not(windows))]
pub fn create_desktop_shortcut(_exe: &Path, _title: &str, _description: &str) -> Result<PathBuf> {
    Err(Error::Unsupported("desktop shortcuts"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn reads_the_quick_clean_flag() {
        assert!(quick_clean_requested(&args(&[
            "reclaim-cleaner.exe",
            "--quick-clean"
        ])));
        assert!(quick_clean_requested(&args(&[
            "x",
            "--analyze",
            "D:\\",
            "--Quick-Clean"
        ])));
        assert!(!quick_clean_requested(&args(&["x"])));
        assert!(!quick_clean_requested(&args(&[
            "x",
            "--quick-clean=yes",
            "quick-clean"
        ])));
        // The program path itself never counts, whatever it is called.
        assert!(!quick_clean_requested(&args(&["--quick-clean"])));
    }

    #[test]
    fn keeps_shortcut_text_on_one_bounded_line() {
        assert_eq!(shortcut_text("  Quick\nclean\t now "), "Quick clean now");
        assert_eq!(shortcut_text("\u{0}\u{7}"), "");
        assert_eq!(shortcut_text(&"é".repeat(400)).chars().count(), MAX_TEXT);
    }

    #[cfg(windows)]
    #[test]
    fn writes_and_refreshes_its_own_shortcut_only() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = std::env::current_exe().unwrap();
        let link = tmp.path().join(SHORTCUT_NAME);

        write_shortcut(&link, &exe, "Quick clean", "Review and clean safe items").unwrap();
        let (target, flags) = on_sta_thread({
            let link = link.clone();
            move || Ok(read_link(&link))
        })
        .unwrap()
        .unwrap();
        assert_eq!(crate::paths::key(&target), crate::paths::key(&exe));
        assert_eq!(flags, QUICK_CLEAN_FLAG);
        // A second write refreshes the same shortcut.
        write_shortcut(&link, &exe, "Quick clean", "Again").unwrap();

        let foreign = tmp.path().join("other.lnk");
        std::fs::write(&foreign, b"not a shortcut").unwrap();
        assert!(write_shortcut(&foreign, &exe, "Quick clean", "x").is_err());
        assert_eq!(std::fs::read(&foreign).unwrap(), b"not a shortcut");
    }

    #[cfg(windows)]
    #[test]
    fn titles_the_jump_list_task() {
        let exe = std::env::current_exe().unwrap();
        let title = on_sta_thread(move || {
            let link = quick_clean_link(&exe, "Quick clean", "Review")?;
            let store: IPropertyStore = link.cast()?;
            let value = unsafe { store.GetValue(&PKEY_TITLE)? };
            Ok(windows::core::BSTR::try_from(&value)?.to_string())
        })
        .unwrap();
        assert_eq!(title, "Quick clean");
    }
}
