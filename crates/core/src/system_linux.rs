//! System facts on Linux: disks from the mount table plus `statvfs`, processes from `/proc`, user
//! folders from `user-dirs.dirs` and the trash through `trash::os_limited`.

use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};

use crate::mounts::{Mount, MountClass, MountTable};
use crate::paths::{is_within, key};
use crate::system::{Drive, LibraryFolders, RecycleBinInfo};
use crate::walk::measure_path;
use crate::xdg::Xdg;
use crate::{Error, Job, Result};

/// A mount shown as a drive: a disk the user could fill, and only once. A Btrfs subvolume or a
/// bind mount is the same disk again, so the shortest mount point speaks for its source.
fn drive_mounts(table: &MountTable) -> Vec<&Mount> {
    let mut seen = HashSet::new();
    let mut mounts: Vec<&Mount> = table
        .mounts()
        .iter()
        .filter(|m| m.mount_point == Path::new("/") || is_user_disk(m))
        .collect();
    mounts.sort_by_key(|m| m.mount_point.as_os_str().len());
    mounts.retain(|m| seen.insert(m.source.clone()));
    mounts
}

fn is_user_disk(m: &Mount) -> bool {
    let point = &m.mount_point;
    // A bind mount shows part of a disk that is listed anyway; Docker binds single files too.
    let whole = m.root == Path::new("/") || m.fs_type == "btrfs";
    let under = |p: &str| point.starts_with(p);
    let hidden = (under("/run") && !under("/run/media"))
        || under("/proc")
        || under("/sys")
        || under("/dev")
        || under("/snap")
        || under("/var/lib")
        || under("/boot/efi")
        || under("/efi");
    m.class() == MountClass::Local && whole && !hidden
}

fn is_removable(m: &Mount) -> bool {
    if ["/media", "/run/media"]
        .iter()
        .any(|p| m.mount_point.starts_with(p))
    {
        return true;
    }
    // `removable` sits on the whole disk, so a partition reads it from its parent.
    let (major, minor) = m.device;
    let dev = PathBuf::from(format!("/sys/dev/block/{major}:{minor}"));
    [dev.join("removable"), dev.join("../removable")]
        .iter()
        .filter_map(|f| fs::read_to_string(f).ok())
        .any(|v| v.trim() == "1")
}

/// Undoes the `\x20` escapes udev uses in `/dev/disk/by-label` names.
fn decode_label(name: &OsStr) -> String {
    let name = name.to_string_lossy();
    let mut out = Vec::with_capacity(name.len());
    let bytes = name.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i..i + 4)
            .filter(|b| b.starts_with(br"\x"))
            .and_then(|b| std::str::from_utf8(&b[2..]).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        if let Some(value) = hex {
            out.push(value);
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Labels by the device they point at. Most disks have none, and an empty label is fine.
fn disk_labels() -> HashMap<PathBuf, String> {
    let Ok(rd) = fs::read_dir("/dev/disk/by-label") else {
        return HashMap::new();
    };
    rd.flatten()
        .filter_map(|e| {
            let device = fs::canonicalize(e.path()).ok()?;
            Some((device, decode_label(&e.file_name())))
        })
        .collect()
}

pub fn drives() -> Vec<Drive> {
    let Ok(table) = MountTable::read_current() else {
        return Vec::new();
    };
    let labels = disk_labels();
    drive_mounts(&table)
        .into_iter()
        .filter_map(|m| {
            let vfs = rustix::fs::statvfs(&m.mount_point).ok()?;
            let total = vfs.f_blocks * vfs.f_frsize;
            if total == 0 {
                return None;
            }
            let device = fs::canonicalize(Path::new(&m.source)).ok();
            Some(Drive {
                root: m.mount_point.to_string_lossy().into_owned(),
                label: device
                    .and_then(|d| labels.get(&d).cloned())
                    .unwrap_or_default(),
                file_system: m.fs_type.clone(),
                total,
                free: vfs.f_bavail * vfs.f_frsize,
                removable: is_removable(m),
                system: m.mount_point == Path::new("/"),
            })
        })
        .collect()
}

/// Free bytes on the file system holding `path`, as an unprivileged user can use them.
pub fn free_space(path: &Path) -> Option<u64> {
    let vfs = rustix::fs::statvfs(path).ok()?;
    Some(vfs.f_bavail * vfs.f_frsize)
}

pub fn is_elevated() -> bool {
    rustix::process::geteuid().is_root() || rustix::process::getuid().is_root()
}

/// Ids of every process this user can see.
pub fn process_ids() -> Vec<u32> {
    let Ok(rd) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    rd.flatten()
        .filter_map(|e| e.file_name().to_str()?.parse().ok())
        .collect()
}

/// The last part of `/usr/bin/app` or `/usr/bin/app (deleted)`, lowercase. Rule `closeApps` and
/// Windows' process names compare the same way.
fn program_name(path: &[u8]) -> Option<String> {
    let path = path.strip_suffix(b" (deleted)").unwrap_or(path);
    let name = path.rsplit(|&b| b == b'/').next()?;
    (!name.is_empty()).then(|| String::from_utf8_lossy(name).to_lowercase())
}

/// Every name a process goes by. `comm` is cut to 15 bytes and renamed by some programs, `exe` is
/// only readable for the user's own processes, and an interpreter or wrapper shows up in argv[0].
fn names_of(pid: u32) -> Vec<String> {
    use std::os::unix::ffi::OsStrExt;
    let dir = PathBuf::from(format!("/proc/{pid}"));
    let mut names = Vec::new();
    if let Ok(comm) = fs::read(dir.join("comm")) {
        names.extend(program_name(comm.trim_ascii()));
    }
    if let Ok(exe) = fs::read_link(dir.join("exe")) {
        names.extend(program_name(exe.as_os_str().as_bytes()));
    }
    if let Ok(cmdline) = fs::read(dir.join("cmdline"))
        && let Some(first) = cmdline.split(|&b| b == 0).next()
    {
        names.extend(program_name(first));
        let runtimes: Vec<String> = names.clone();
        names.extend(owners_behind_runtime(&runtimes, &cmdline));
    }
    names
}

/// JetBrains products as `closeApps` names them. `idea` also matches every product's own launcher
/// flags, which at worst blocks one more rule while an IDE runs.
const JETBRAINS_NAMES: &[&str] = &[
    "idea",
    "pycharm",
    "webstorm",
    "clion",
    "goland",
    "rider",
    "phpstorm",
    "rubymine",
    "datagrip",
    "rustrover",
    "studio",
];

/// The apps a shared runtime is running. A script-started IDE or a Gradle daemon shows up only as
/// `java`, and Arch's packaged Electron apps only as `electron33`, with the app in the arguments.
fn owners_behind_runtime(runtimes: &[String], cmdline: &[u8]) -> Vec<String> {
    let args: Vec<String> = cmdline
        .split(|&b| b == 0)
        .skip(1)
        .map(|a| String::from_utf8_lossy(a).to_lowercase())
        .collect();
    let mut owners = Vec::new();
    if runtimes.iter().any(|r| r == "java") {
        let all = args.join(" ");
        if all.contains("org.gradle") {
            owners.push("gradle".to_string());
        }
        if all.contains("com.intellij") || all.contains("idea.paths.selector") {
            owners.extend(
                JETBRAINS_NAMES
                    .iter()
                    .filter(|n| all.contains(*n))
                    .map(|n| n.to_string()),
            );
        }
    }
    if runtimes.iter().any(|r| r.starts_with("electron"))
        && let Some(app) = args.iter().find(|a| !a.starts_with('-'))
    {
        // `/usr/lib/code/out/cli.js` names the app somewhere along the path, and a name that means
        // nothing is compared with `closeApps` and ignored.
        owners.extend(app.split('/').filter(|s| !s.is_empty()).map(String::from));
    }
    owners
}

/// Browsers and mail clients keep a lock in their profile while they run, whatever the program is
/// called (an AppImage, a wrapper), as a link whose target ends in the process id.
const CHROMIUM_LOCKS: &[(&str, &str)] = &[
    ("google-chrome", "chrome"),
    ("google-chrome-beta", "chrome"),
    ("google-chrome-unstable", "chrome"),
    ("chromium", "chromium"),
    ("BraveSoftware/Brave-Browser", "brave"),
    ("microsoft-edge", "msedge"),
    ("vivaldi", "vivaldi-bin"),
    ("opera", "opera"),
];

const FIREFOX_LOCKS: &[(&str, &str)] = &[
    (".mozilla/firefox", "firefox"),
    (".thunderbird", "thunderbird"),
    (".librewolf", "librewolf"),
    (".zen", "zen"),
    (".waterfox", "waterfox"),
];

/// The process id at the end of a lock link: `host-1234` (Chromium), `127.0.1.1:+1234` (Firefox).
fn lock_pid(target: &[u8]) -> Option<u32> {
    let start = target.iter().rposition(|b| !b.is_ascii_digit())? + 1;
    std::str::from_utf8(&target[start..]).ok()?.parse().ok()
}

fn lock_is_live(link: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    fs::read_link(link)
        .ok()
        .and_then(|t| lock_pid(t.as_os_str().as_bytes()))
        .is_some_and(|pid| Path::new(&format!("/proc/{pid}")).exists())
}

/// Program names whose profile lock is held by a live process. Only reads the lock links.
fn names_from_locks(home: &Path, config: &Path) -> Vec<String> {
    let mut names = Vec::new();
    for (dir, name) in CHROMIUM_LOCKS {
        if lock_is_live(&config.join(dir).join("SingletonLock")) {
            names.push(name.to_string());
        }
    }
    for (dir, name) in FIREFOX_LOCKS {
        let Ok(profiles) = fs::read_dir(home.join(dir)) else {
            continue;
        };
        if profiles
            .flatten()
            .any(|p| lock_is_live(&p.path().join("lock")))
        {
            names.push(name.to_string());
        }
    }
    names
}

/// Lowercase program names of every process this user can see. Empty means the list could not be
/// read, because this app itself is always on it.
pub fn running_processes() -> HashSet<String> {
    let mut names: HashSet<String> = process_ids().into_iter().flat_map(names_of).collect();
    if !names.is_empty()
        && let Some(xdg) = Xdg::current()
    {
        names.extend(names_from_locks(&xdg.home, &xdg.config));
    }
    names
}

/// Device and inode of every regular file this user's processes hold open, or `None` when the
/// process list could not be read. Removing such a file works on Linux, but the program keeps
/// using it and the space stays taken until it lets go.
pub fn open_files() -> Option<HashSet<(u64, u64)>> {
    use std::os::unix::fs::MetadataExt;
    let pids = process_ids();
    if pids.is_empty() {
        return None;
    }
    let mut open = HashSet::new();
    for pid in pids {
        // Other users' processes refuse this, and their files are not this user's to delete.
        let Ok(fds) = fs::read_dir(format!("/proc/{pid}/fd")) else {
            continue;
        };
        for fd in fds.flatten() {
            if let Ok(md) = fs::metadata(fd.path())
                && md.is_file()
            {
                open.insert((md.dev(), md.ino()));
            }
        }
    }
    Some(open)
}

pub fn known_folders() -> Vec<PathBuf> {
    let Some(xdg) = Xdg::current() else {
        return Vec::new();
    };
    let mut folders: Vec<PathBuf> = xdg.user_dirs().into_iter().map(|(_, p)| p).collect();
    folders.sort();
    folders.dedup();
    folders
}

pub fn library_folders() -> LibraryFolders {
    let xdg = Xdg::current();
    let s = |name: &str| {
        xdg.as_ref()?
            .user_dir(name)
            .map(|p| p.to_string_lossy().into_owned())
    };
    LibraryFolders {
        desktop: s("DESKTOP"),
        documents: s("DOCUMENTS"),
        downloads: s("DOWNLOAD"),
        pictures: s("PICTURES"),
        music: s("MUSIC"),
        videos: s("VIDEOS"),
    }
}

/// The trash `trash::os_limited` lists, or an error. The crate panics on an info file with a
/// line without `=`, and another program's stray file must not take the app down with it.
fn trash_items() -> Result<Vec<trash::TrashItem>> {
    std::panic::catch_unwind(trash::os_limited::list)
        .map_err(|_| Error::Other("a trash info file could not be read".into()))?
        .map_err(|e| Error::Other(format!("reading the trash failed: {e}")))
}

/// Where a trash entry's file is: `<trash>/info/<name>.trashinfo` goes with `<trash>/files/<name>`.
fn trashed_file(info_file: &OsStr) -> Option<PathBuf> {
    let info = Path::new(info_file);
    let folder = info.parent()?.parent()?;
    Some(folder.join("files").join(info.file_stem()?))
}

fn trash_info(items: &[trash::TrashItem]) -> RecycleBinInfo {
    let mut info = RecycleBinInfo {
        bytes: 0,
        items: items.len() as u64,
    };
    for item in items {
        if let Some(file) = trashed_file(&item.id) {
            info.bytes += measure_path(&file, None, &Job::new()).bytes;
        }
    }
    info
}

/// Size of the user's trash on every mounted file system.
pub fn recycle_bin_info() -> RecycleBinInfo {
    trash_info(&trash_items().unwrap_or_default())
}

/// Size of the user's trash on the file system mounted at `root`.
pub fn recycle_bin_info_on(root: &Path) -> RecycleBinInfo {
    let Ok(table) = MountTable::read_current() else {
        return RecycleBinInfo::default();
    };
    let items: Vec<trash::TrashItem> = trash_items()
        .unwrap_or_default()
        .into_iter()
        .filter(|item| {
            trashed_file(&item.id).is_some_and(|f| {
                table
                    .holding(&f)
                    .is_some_and(|m| m.mount_point.as_path() == root)
            })
        })
        .collect();
    trash_info(&items)
}

/// Whether `trash` is this user's own trash folder under the spec: `$XDG_DATA_HOME/Trash`,
/// `$topdir/.Trash/$uid` or `$topdir/.Trash-$uid`. `trash` must be the folder that holds `info`
/// and `files`.
fn is_own_trash(trash: &Path, home_trash: &Path, uid: u32) -> bool {
    if trash == home_trash {
        return true;
    }
    let Some(name) = trash.file_name() else {
        return false;
    };
    let own = OsString::from(format!(".Trash-{uid}"));
    if name == own {
        return true;
    }
    name == OsStr::new(&uid.to_string())
        && trash
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|parent| parent == ".Trash")
}

/// Removes `items` from their trash folders, each file with its `.trashinfo`, and returns how many
/// could not be removed. An item of another user's trash, or one with a mount below it, stays:
/// `purge_all` uses `remove_dir_all`, which would descend into that mount.
fn purge_items(
    items: Vec<trash::TrashItem>,
    home_trash: &Path,
    uid: u32,
    table: &MountTable,
) -> usize {
    let points: Vec<String> = table.mounts().iter().map(|m| key(&m.mount_point)).collect();
    let mut failed = 0;
    for item in items {
        let Some(file) = trashed_file(&item.id) else {
            failed += 1;
            continue;
        };
        let own = file
            .parent()
            .and_then(Path::parent)
            .is_some_and(|t| is_own_trash(t, home_trash, uid));
        let k = key(&file);
        if !own || points.iter().any(|p| is_within(p, &k)) {
            failed += 1;
            continue;
        }
        if trash::os_limited::purge_all([item]).is_err() {
            failed += 1;
        }
    }
    failed
}

/// Empties the user's trash on every mounted file system, as the spec describes: only `files/*`
/// together with the matching `info/*.trashinfo`. The app asks first. Refused as root, whose
/// trash holds nothing the app put there.
pub fn empty_recycle_bin() -> Result<()> {
    if is_elevated() {
        return Err(Error::Protected("the trash (running as root)".into()));
    }
    let xdg = Xdg::current().ok_or(Error::Unsupported(
        "emptying the trash without a home folder",
    ))?;
    let table = MountTable::read_current().map_err(|e| Error::io("/proc/self/mountinfo", e))?;
    let uid = rustix::process::getuid().as_raw();
    let failed = purge_items(trash_items()?, &xdg.data.join("Trash"), uid, &table);
    if failed > 0 {
        return Err(Error::Other(format!(
            "{failed} trash items could not be removed"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mounts::parse_mountinfo;

    fn table(text: &str) -> MountTable {
        MountTable::from_mounts(parse_mountinfo(text.as_bytes()))
    }

    #[test]
    fn lists_each_user_disk_once() {
        let t = table(
            "1 0 8:2 /@ / rw - btrfs /dev/sda2 rw,subvol=/@\n\
             2 1 8:2 /@home /home rw - btrfs /dev/sda2 rw,subvol=/@home\n\
             3 1 8:1 / /boot/efi rw - vfat /dev/sda1 rw\n\
             4 1 8:17 / /data rw - ext4 /dev/sdb1 rw\n\
             5 1 8:17 /sub /data2 rw - ext4 /dev/sdb1 rw\n\
             6 1 8:33 / /mnt/win rw - fuseblk /dev/sdc1 rw\n\
             7 1 0:5 / /run rw - tmpfs tmpfs rw\n\
             8 7 8:49 / /run/media/u/USB rw - exfat /dev/sdd1 rw\n\
             9 1 0:6 / /proc rw - proc proc rw\n\
             10 1 7:0 / /snap/core/1 ro - squashfs /dev/loop0 ro\n\
             11 1 0:7 / /mnt/nas rw - nfs4 nas:/x rw\n\
             12 1 0:8 / /mnt/ssh rw - fuse.sshfs h:/ rw\n\
             13 1 8:65 / /var/lib/docker/x rw - ext4 /dev/sde1 rw\n\
             14 1 0:9 / /tmp rw - tmpfs tmpfs rw\n",
        );
        let points: Vec<&Path> = drive_mounts(&t)
            .iter()
            .map(|m| m.mount_point.as_path())
            .collect();
        assert_eq!(
            points,
            vec![
                Path::new("/"),
                Path::new("/data"),
                Path::new("/mnt/win"),
                Path::new("/run/media/u/USB"),
            ]
        );
    }

    #[test]
    fn a_container_root_is_still_listed() {
        let t = table(
            "1 0 0:50 / / rw - overlay overlay rw\n\
             2 1 8:1 /x/hosts /etc/hosts rw - ext4 /dev/sda1 rw\n",
        );
        let points: Vec<&Path> = drive_mounts(&t)
            .iter()
            .map(|m| m.mount_point.as_path())
            .collect();
        assert_eq!(points, vec![Path::new("/")]);
    }

    #[test]
    fn reads_this_machine() {
        let d = drives();
        assert!(d.iter().any(|d| d.system && d.root == "/"), "root listed");
        assert!(d.iter().all(|d| d.total >= d.free));
        let cwd = std::env::current_dir().unwrap();
        assert!(free_space(&cwd).is_some());
        assert!(free_space(Path::new("/no/such/folder")).is_none());
        assert_eq!(is_elevated(), rustix::process::geteuid().is_root());
    }

    #[test]
    fn decodes_disk_labels() {
        assert_eq!(decode_label(OsStr::new(r"My\x20Disk")), "My Disk");
        assert_eq!(decode_label(OsStr::new("plain")), "plain");
        assert_eq!(decode_label(OsStr::new(r"a\x2")), r"a\x2");
    }

    #[test]
    fn finds_processes_by_their_program_name() {
        assert_eq!(program_name(b"/usr/bin/App"), Some("app".to_string()));
        assert_eq!(
            program_name(b"/opt/x/chrome (deleted)"),
            Some("chrome".to_string())
        );
        assert_eq!(program_name(b"firefox"), Some("firefox".to_string()));
        assert_eq!(program_name(b"/usr/bin/"), None);
        assert_eq!(program_name(b""), None);
        let pids = process_ids();
        assert!(pids.contains(&std::process::id()));
        let exe = std::env::current_exe().unwrap();
        let me = exe.file_name().unwrap().to_string_lossy().to_lowercase();
        assert!(running_processes().contains(&me));
    }

    #[test]
    fn sees_the_apps_behind_a_runtime() {
        let names = |runtime: &str, args: &[u8]| {
            let mut cmdline = b"argv0\0".to_vec();
            cmdline.extend_from_slice(args);
            owners_behind_runtime(&[runtime.to_string()], &cmdline)
        };
        assert_eq!(
            names(
                "java",
                b"-Xmx1g\0org.gradle.launcher.daemon.bootstrap.GradleDaemon\08.5\0"
            ),
            ["gradle"]
        );
        let ide = names(
            "java",
            b"-Didea.paths.selector=PyCharm2026.1\0com.intellij.idea.Main\0",
        );
        assert!(ide.contains(&"pycharm".to_string()), "{ide:?}");
        assert!(names("java", b"-jar\0/opt/server.jar\0").is_empty());
        let code = names(
            "electron33",
            b"--enable-features=x\0/usr/lib/code/out/cli.js\0",
        );
        assert!(code.contains(&"code".to_string()), "{code:?}");
        assert!(names("code", b"/usr/lib/code/out/cli.js\0").is_empty());
    }

    #[test]
    fn reads_profile_locks() {
        use std::os::unix::fs::symlink;

        assert_eq!(lock_pid(b"host-1234"), Some(1234));
        assert_eq!(lock_pid(b"127.0.1.1:+1234"), Some(1234));
        assert_eq!(lock_pid(b"host-"), None);

        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let config = home.join(".config");
        let live = format!("host-{}", std::process::id());
        // Above the kernel's limit for process ids, so never alive.
        let dead = "host-99999999";
        fs::create_dir_all(config.join("chromium")).unwrap();
        fs::create_dir_all(config.join("vivaldi")).unwrap();
        fs::create_dir_all(home.join(".thunderbird/abc")).unwrap();
        symlink(&live, config.join("chromium/SingletonLock")).unwrap();
        symlink(dead, config.join("vivaldi/SingletonLock")).unwrap();
        symlink(&live, home.join(".thunderbird/abc/lock")).unwrap();
        assert_eq!(names_from_locks(home, &config), ["chromium", "thunderbird"]);
    }

    #[test]
    fn sees_files_this_process_holds_open() {
        use std::os::unix::fs::MetadataExt;

        let tmp = tempfile::tempdir().unwrap();
        let held = tmp.path().join("held.bin");
        let free = tmp.path().join("free.bin");
        fs::write(&held, b"x").unwrap();
        fs::write(&free, b"x").unwrap();
        let handle = fs::File::open(&held).unwrap();
        let open = open_files().unwrap();
        let id = |p: &Path| {
            let md = fs::metadata(p).unwrap();
            (md.dev(), md.ino())
        };
        assert!(open.contains(&id(&held)));
        assert!(!open.contains(&id(&free)));
        drop(handle);
    }

    #[test]
    fn knows_which_trash_is_the_users_own() {
        let home = Path::new("/home/u/.local/share/Trash");
        let own = |p: &str| is_own_trash(Path::new(p), home, 1000);
        assert!(own("/home/u/.local/share/Trash"));
        assert!(own("/mnt/usb/.Trash-1000"));
        assert!(own("/mnt/usb/.Trash/1000"));
        assert!(!own("/mnt/usb/.Trash-1001"));
        assert!(!own("/mnt/usb/.Trash/1001"));
        assert!(!own("/mnt/usb/.Trash"));
        assert!(!own("/mnt/usb/other/1000"));
        assert!(!own("/home/u/Documents"));
    }

    #[test]
    fn pairs_info_files_with_their_trashed_file() {
        assert_eq!(
            trashed_file(OsStr::new("/t/.Trash-1000/info/a b.txt.trashinfo")),
            Some(PathBuf::from("/t/.Trash-1000/files/a b.txt"))
        );
    }

    const CHILD_ENV: &str = "RECLAIM_TRASH_TEST_HOME";

    /// `trash` reads `$HOME`, and changing the environment of a test binary running many threads
    /// is unsound, so this test starts itself again with its own home folder.
    #[test]
    fn empties_only_this_trash_through_the_crate() {
        let Some(home) = std::env::var_os(CHILD_ENV) else {
            let tmp = tempfile::tempdir().unwrap();
            let out = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "--nocapture",
                    "system_linux::tests::empties_only_this_trash_through_the_crate",
                ])
                .env(CHILD_ENV, tmp.path())
                .env("HOME", tmp.path())
                .env_remove("XDG_DATA_HOME")
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stdout)
            );
            return;
        };
        let home = PathBuf::from(home);
        let trash_dir = home.join(".local/share/Trash");
        let doomed = home.join("doomed");
        fs::create_dir_all(doomed.join("inner")).unwrap();
        fs::write(doomed.join("inner/a.bin"), vec![1u8; 8192]).unwrap();
        fs::write(home.join("note.txt"), vec![1u8; 8192]).unwrap();
        fs::write(home.join("kept.txt"), b"x").unwrap();
        trash::delete(&doomed).unwrap();
        trash::delete(home.join("note.txt")).unwrap();
        assert!(!doomed.exists());
        // Only what lies below this test's home: a real run must not touch the machine's trash
        // on other disks.
        let mine = || -> Vec<trash::TrashItem> {
            trash_items()
                .unwrap()
                .into_iter()
                .filter(|i| Path::new(&i.id).starts_with(&home))
                .collect()
        };
        let info = trash_info(&mine());
        assert_eq!(info.items, 2);
        assert!(info.bytes >= 16384, "{}", info.bytes);

        // A mount below an item keeps that item in the trash.
        let uid = rustix::process::getuid().as_raw();
        let held = table(&format!(
            "2 1 0:9 / {} rw - tmpfs tmpfs rw",
            trash_dir.join("files/doomed/inner").display()
        ));
        assert_eq!(purge_items(mine(), &trash_dir, uid, &held), 1);
        assert_eq!(mine().len(), 1);
        assert!(trash_dir.join("files/doomed/inner/a.bin").exists());

        // Without the mount, the rest goes, each file with its info file.
        assert_eq!(
            purge_items(mine(), &trash_dir, uid, &MountTable::default()),
            0
        );
        assert!(mine().is_empty());
        assert!(!trash_dir.join("files/doomed").exists());
        assert!(!trash_dir.join("info/doomed.trashinfo").exists());
        assert!(home.join("kept.txt").exists());

        // A stray info file must not take the app down: the crate panics on it.
        fs::write(
            trash_dir.join("info/bad.trashinfo"),
            "[Trash Info]\nno equals sign\n",
        )
        .unwrap();
        assert!(trash_items().is_err());
        assert_eq!(recycle_bin_info().items, 0);
    }
}
