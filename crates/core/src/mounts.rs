//! The Linux mount table, read from `/proc/self/mountinfo`, and what a scan may do with each
//! mount.
//!
//! `statfs` reports FUSE mounts only as `fuse` and a bind mount keeps its source's `st_dev`, so
//! neither the file system magic nor the device number can answer "is this another mount?".
//! Mountinfo can.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use crate::walk::{DirAction, MountKind};

const MOUNTINFO: &str = "/proc/self/mountinfo";

/// Kernel interfaces: no files live here, and reading some of them has side effects. An `autofs`
/// entry that is still the topmost one at its mount point has nothing mounted yet, and entering
/// it would trigger the mount.
const KERNEL: &[&str] = &[
    "proc",
    "sysfs",
    "devtmpfs",
    "devpts",
    "cgroup",
    "cgroup2",
    "debugfs",
    "tracefs",
    "securityfs",
    "pstore",
    "bpf",
    "binfmt_misc",
    "selinuxfs",
    "efivarfs",
    "configfs",
    "mqueue",
    "hugetlbfs",
    "fusectl",
    "nsfs",
    "rpc_pipefs",
    "autofs",
];

/// Reading these downloads data, the way opening an online-only file does on Windows.
const REMOTE: &[&str] = &[
    "nfs",
    "nfs4",
    "cifs",
    "smb3",
    "smbfs",
    "9p",
    "afs",
    "ceph",
    "glusterfs",
    "fuse",
];

/// Their space is already counted elsewhere: a snap's squashfs in its `.snap` file, Docker's
/// overlay `merged` folders in their layers.
const DOUBLE_COUNTED: &[&str] = &["squashfs", "overlay"];

const MEMORY: &[&str] = &["tmpfs", "ramfs"];

/// Never entered when met during a walk, whatever is mounted there: gdu and ncdu skip the same
/// folders, and `/run` is runtime state rather than files anyone wants to measure.
const SKIPPED_PATHS: &[&str] = &["/proc", "/sys", "/dev", "/run"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MountClass {
    Kernel,
    /// Network and FUSE file systems. `fuseblk` (ntfs-3g) is a local disk and not one of these.
    Remote,
    DoubleCounted,
    Memory,
    Local,
}

pub fn classify(fs_type: &str) -> MountClass {
    if KERNEL.contains(&fs_type) {
        MountClass::Kernel
    } else if REMOTE.contains(&fs_type) || fs_type.starts_with("fuse.") {
        MountClass::Remote
    } else if DOUBLE_COUNTED.contains(&fs_type) {
        MountClass::DoubleCounted
    } else if MEMORY.contains(&fs_type) {
        MountClass::Memory
    } else {
        MountClass::Local
    }
}

/// One line of mountinfo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub id: u32,
    pub parent: u32,
    /// `major:minor`, which equals the `st_dev` of the files on most file systems.
    pub device: (u32, u32),
    /// The folder of the file system that is mounted here; anything but `/` is a bind mount or
    /// a Btrfs subvolume.
    pub root: PathBuf,
    pub mount_point: PathBuf,
    pub fs_type: String,
    pub source: OsString,
}

impl Mount {
    pub fn class(&self) -> MountClass {
        classify(&self.fs_type)
    }
}

/// Undoes the kernel's octal escapes: `\040` space, `\011` tab, `\012` newline, `\134` backslash.
/// Anything that is not a backslash and three octal digits stays as it is.
pub fn decode_octal(field: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(field.len());
    let mut i = 0;
    while i < field.len() {
        if field[i] == b'\\'
            && let Some(digits) = field.get(i + 1..i + 4)
            && digits[0] <= b'3'
            && digits.iter().all(|d| (b'0'..=b'7').contains(d))
        {
            let value = digits.iter().fold(0u8, |acc, d| (acc << 3) | (d - b'0'));
            out.push(value);
            i += 4;
        } else {
            out.push(field[i]);
            i += 1;
        }
    }
    out
}

fn path_field(field: &[u8]) -> PathBuf {
    PathBuf::from(OsString::from_vec(decode_octal(field)))
}

fn number(field: &[u8]) -> Option<u32> {
    std::str::from_utf8(field).ok()?.parse().ok()
}

/// `36 35 98:0 /root /mount/point rw,noatime master:1 shared:2 - ext4 /dev/sda1 rw,errors=remount-ro`:
/// a variable number of optional fields ends at a lone `-`.
fn parse_line(line: &[u8]) -> Option<Mount> {
    let mut fields = line.split(|&b| b == b' ');
    let id = number(fields.next()?)?;
    let parent = number(fields.next()?)?;
    let device = fields.next()?;
    let colon = device.iter().position(|&b| b == b':')?;
    let device = (number(&device[..colon])?, number(&device[colon + 1..])?);
    let root = path_field(fields.next()?);
    let mount_point = path_field(fields.next()?);
    if !mount_point.is_absolute() {
        return None;
    }
    fields.find(|f| *f == b"-")?;
    let fs_type = String::from_utf8(decode_octal(fields.next()?)).ok()?;
    let source = OsString::from_vec(decode_octal(fields.next()?));
    Some(Mount {
        id,
        parent,
        device,
        root,
        mount_point,
        fs_type,
        source,
    })
}

/// Parses the whole file. A malformed line is skipped, never a panic: the caller still sees every
/// mount it could read.
pub fn parse_mountinfo(text: &[u8]) -> Vec<Mount> {
    text.split(|&b| b == b'\n').filter_map(parse_line).collect()
}

/// Mounts by mount point. Later entries sit on top of earlier ones at the same point, so the
/// last one is what a path resolves to.
#[derive(Debug, Clone, Default)]
pub struct MountTable {
    mounts: Vec<Mount>,
    topmost: HashMap<OsString, usize>,
}

impl MountTable {
    pub fn from_mounts(mounts: Vec<Mount>) -> MountTable {
        let topmost = mounts
            .iter()
            .enumerate()
            .map(|(i, m)| (m.mount_point.as_os_str().to_os_string(), i))
            .collect();
        MountTable { mounts, topmost }
    }

    pub fn read_current() -> std::io::Result<MountTable> {
        let text = std::fs::read(MOUNTINFO)?;
        Ok(MountTable::from_mounts(parse_mountinfo(&text)))
    }

    pub fn mounts(&self) -> &[Mount] {
        &self.mounts
    }

    /// The mount whose mount point is exactly `path`.
    pub fn at(&self, path: &Path) -> Option<&Mount> {
        self.topmost.get(path.as_os_str()).map(|&i| &self.mounts[i])
    }

    pub fn is_mount_point(&self, path: &Path) -> bool {
        self.topmost.contains_key(path.as_os_str())
    }

    /// The mount holding `path`: its closest ancestor that is a mount point.
    pub fn holding(&self, path: &Path) -> Option<&Mount> {
        path.ancestors().find_map(|p| self.at(p))
    }
}

impl MountTable {
    /// The scan policy for a folder met during a walk. Only the root of a walk may be on a mount
    /// of its own, so every mount point below it stops the walk: one mount by default.
    pub fn action_for(&self, path: &Path) -> DirAction {
        let Some(mount) = self.at(path) else {
            return if is_skipped_path(path) {
                DirAction::Skip
            } else {
                DirAction::Enter
            };
        };
        if is_skipped_path(path) {
            return DirAction::Skip;
        }
        match mount.class() {
            MountClass::Kernel | MountClass::DoubleCounted => DirAction::Skip,
            MountClass::Remote => DirAction::Stub(MountKind::Remote),
            MountClass::Memory => DirAction::Stub(MountKind::Memory),
            MountClass::Local => DirAction::Stub(MountKind::Local),
        }
    }

    /// May a walk start at `root`? The user chose it, so remote and double-counted mounts are
    /// fine (a container's `/` is an overlay). Kernel file systems are not, and neither is
    /// runtime state under `/run`; removable disks below `/run/media` are real mounts and stay
    /// allowed.
    pub fn root_allowed(&self, root: &Path) -> bool {
        let class = self.holding(root).map(Mount::class);
        if class == Some(MountClass::Kernel) {
            return false;
        }
        let below = |p: &str| root.starts_with(p);
        if below("/proc") || below("/sys") || below("/dev") {
            return false;
        }
        !(below("/run") && !matches!(class, Some(MountClass::Local | MountClass::Remote)))
    }
}

fn is_skipped_path(path: &Path) -> bool {
    SKIPPED_PATHS
        .iter()
        .any(|p| OsStr::new(p) == path.as_os_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = br"
22 28 0:21 / /sys rw,nosuid,nodev,noexec,relatime shared:7 - sysfs sysfs rw
23 28 0:22 / /proc rw,nosuid,nodev,noexec,relatime shared:13 - proc proc rw
24 28 0:5 / /dev rw,nosuid,relatime shared:2 - devtmpfs udev rw,size=8116816k
25 28 0:23 / /run rw,nosuid,nodev,noexec,relatime shared:5 - tmpfs tmpfs rw
28 1 8:2 /@ / rw,relatime shared:1 - btrfs /dev/sda2 rw,subvol=/@
29 28 8:2 /@home /home rw,relatime shared:30 - btrfs /dev/sda2 rw,subvol=/@home
30 28 8:1 / /boot/efi rw,relatime shared:31 - vfat /dev/sda1 rw
31 29 8:17 / /home/u/My\040Disk rw,relatime shared:32 master:7 propagate_from:2 - ext4 /dev/sdb1 rw
32 29 8:2 /@home/u/data /home/u/bound rw,relatime shared:30 - btrfs /dev/sda2 rw
33 25 0:45 / /run/user/1000/gvfs rw,nosuid,nodev,relatime shared:40 - fuse.gvfsd-fuse gvfsd-fuse rw
34 29 0:46 / /home/u/remote rw,relatime - fuse.sshfs me@host:/srv rw
35 29 0:47 / /home/u/nas rw,relatime - nfs4 nas:/export rw
36 28 7:3 / /snap/core/123 ro,nodev,relatime shared:50 - squashfs /dev/loop3 ro
37 29 0:48 / /home/u/merged rw,relatime - overlay overlay rw,lowerdir=/a
38 29 8:33 / /home/u/ntfs rw,relatime - fuseblk /dev/sdc1 rw
39 28 0:49 / /tmp rw,nosuid,nodev - tmpfs tmpfs rw
40 28 0:50 / /net rw,relatime - autofs /etc/auto.net rw
41 28 0:51 / /auto rw,relatime - autofs /etc/auto.misc rw
42 41 8:49 / /auto rw,relatime - ext4 /dev/sdd1 rw
this line is not mountinfo
43 28 0:52
";

    fn table() -> MountTable {
        MountTable::from_mounts(parse_mountinfo(SAMPLE))
    }

    #[test]
    fn decodes_octal_escapes() {
        assert_eq!(decode_octal(br"a\040b"), b"a b");
        assert_eq!(decode_octal(br"tab\011nl\012bs\134"), b"tab\tnl\nbs\\");
        assert_eq!(decode_octal(br"\303\251"), "\u{e9}".as_bytes());
        // Not escapes: too short, not octal, or above one byte.
        assert_eq!(decode_octal(br"a\04"), br"a\04");
        assert_eq!(decode_octal(br"a\089"), br"a\089");
        assert_eq!(decode_octal(br"a\777"), br"a\777");
        assert_eq!(decode_octal(b"plain"), b"plain");
    }

    #[test]
    fn parses_fields_and_skips_malformed_lines() {
        let t = table();
        assert_eq!(t.mounts().len(), 19);
        let disk = t.at(Path::new("/home/u/My Disk")).unwrap();
        assert_eq!(disk.id, 31);
        assert_eq!(disk.parent, 29);
        assert_eq!(disk.device, (8, 17));
        assert_eq!(disk.root, Path::new("/"));
        assert_eq!(disk.fs_type, "ext4");
        assert_eq!(disk.source, OsStr::new("/dev/sdb1"));
        assert!(
            t.at(Path::new(r"/home/u/My\040Disk")).is_none(),
            "only the decoded path is known"
        );
    }

    #[test]
    fn keeps_bytes_that_are_not_utf8() {
        let line = b"50 28 8:1 / /media/caf\xe9 rw - ext4 /dev/sdz1 rw";
        let t = MountTable::from_mounts(parse_mountinfo(line));
        let point = PathBuf::from(OsString::from_vec(b"/media/caf\xe9".to_vec()));
        assert!(t.is_mount_point(&point));
    }

    #[test]
    fn sees_bind_mounts_that_share_a_device() {
        let t = table();
        let bound = t.at(Path::new("/home/u/bound")).unwrap();
        let home = t.at(Path::new("/home")).unwrap();
        assert_eq!(
            bound.device, home.device,
            "same st_dev, still another mount"
        );
        assert_eq!(bound.root, Path::new("/@home/u/data"));
        assert_eq!(
            t.action_for(Path::new("/home/u/bound")),
            DirAction::Stub(MountKind::Local)
        );
    }

    #[test]
    fn classifies_file_systems() {
        for kernel in [
            "proc", "sysfs", "cgroup2", "bpf", "efivarfs", "autofs", "fusectl",
        ] {
            assert_eq!(classify(kernel), MountClass::Kernel, "{kernel}");
        }
        for remote in [
            "nfs",
            "nfs4",
            "cifs",
            "9p",
            "fuse.sshfs",
            "fuse.gvfsd-fuse",
            "fuse.rclone",
        ] {
            assert_eq!(classify(remote), MountClass::Remote, "{remote}");
        }
        assert_eq!(
            classify("fuseblk"),
            MountClass::Local,
            "ntfs-3g is a local disk"
        );
        assert_eq!(classify("squashfs"), MountClass::DoubleCounted);
        assert_eq!(classify("overlay"), MountClass::DoubleCounted);
        assert_eq!(classify("tmpfs"), MountClass::Memory);
        assert_eq!(classify("ext4"), MountClass::Local);
        assert_eq!(classify("btrfs"), MountClass::Local);
    }

    #[test]
    fn applies_the_scan_policy() {
        let t = table();
        let act = |p: &str| t.action_for(Path::new(p));
        assert_eq!(act("/proc"), DirAction::Skip);
        assert_eq!(act("/sys"), DirAction::Skip);
        assert_eq!(act("/dev"), DirAction::Skip);
        assert_eq!(act("/run"), DirAction::Skip);
        assert_eq!(act("/snap/core/123"), DirAction::Skip);
        assert_eq!(act("/home/u/merged"), DirAction::Skip);
        assert_eq!(act("/net"), DirAction::Skip, "autofs with nothing mounted");
        assert_eq!(act("/home/u/remote"), DirAction::Stub(MountKind::Remote));
        assert_eq!(act("/home/u/nas"), DirAction::Stub(MountKind::Remote));
        assert_eq!(act("/home/u/ntfs"), DirAction::Stub(MountKind::Local));
        assert_eq!(act("/home"), DirAction::Stub(MountKind::Local));
        assert_eq!(
            act("/auto"),
            DirAction::Stub(MountKind::Local),
            "topmost entry wins"
        );
        assert_eq!(act("/tmp"), DirAction::Stub(MountKind::Memory));
        assert_eq!(act("/home/u/Documents"), DirAction::Enter);
    }

    #[test]
    fn decides_which_roots_a_walk_may_start_from() {
        let t = table();
        let ok = |p: &str| t.root_allowed(Path::new(p));
        assert!(ok("/"));
        assert!(ok("/home/u"));
        assert!(ok("/home/u/remote"), "the user asked for it");
        assert!(ok("/home/u/merged"), "the user asked for it");
        assert!(ok("/run/user/1000/gvfs"), "a FUSE mount the user picked");
        assert!(!ok("/proc"));
        assert!(!ok("/proc/1/fd"));
        assert!(!ok("/sys/kernel"));
        assert!(!ok("/dev/shm"));
        assert!(!ok("/run"));
        assert!(!ok("/run/user/1000"));
        assert!(!ok("/net"));
        let media = MountTable::from_mounts(parse_mountinfo(
            b"1 0 8:1 / / rw - ext4 /dev/sda1 rw\n2 1 0:2 / /run rw - tmpfs tmpfs rw\n3 2 8:17 / /run/media/u/USB rw - exfat /dev/sdb1 rw",
        ));
        assert!(media.root_allowed(Path::new("/run/media/u/USB")));
        assert!(media.root_allowed(Path::new("/run/media/u/USB/photos")));
        assert!(!media.root_allowed(Path::new("/run/media/u")));
    }

    #[test]
    fn finds_the_mount_holding_a_path() {
        let t = table();
        let holding = |p: &str| t.holding(Path::new(p)).map(|m| m.id);
        assert_eq!(holding("/home/u/My Disk/film.mkv"), Some(31));
        assert_eq!(holding("/home/u/Documents"), Some(29));
        assert_eq!(holding("/etc/passwd"), Some(28));
    }

    /// The real file on this machine must parse. `/` is always mounted.
    #[test]
    fn reads_the_live_table() {
        let t = MountTable::read_current().unwrap();
        assert!(t.is_mount_point(Path::new("/")));
        assert!(
            t.at(Path::new("/proc"))
                .is_some_and(|m| m.fs_type == "proc")
        );
    }
}
