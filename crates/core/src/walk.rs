//! Parallel directory measuring.
//!
//! jwalk is archived, so this is a small rayon walker over `std::fs::read_dir`. On Windows the
//! metadata of a `DirEntry` comes from the directory listing itself, so sizing a file costs no
//! extra system call.

use std::fs;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::Serialize;

use crate::fsmeta::{is_cloud_only, is_link, mtime};
use crate::job::Job;
#[cfg(target_os = "linux")]
use crate::mounts::MountTable;

/// How a walk treats a folder it found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirAction {
    Enter,
    /// Not entered and not shown.
    Skip,
    /// Not entered, but shown as an empty marked entry.
    Stub(MountKind),
}

/// What kind of mount a stub stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MountKind {
    /// Another local file system.
    Local,
    /// A network or FUSE mount; reading it downloads data.
    Remote,
    /// tmpfs: memory rather than disk.
    Memory,
}

/// What one walk knows about the machine, read when it starts: on Linux the mount table, and the
/// hard links it has already counted. Elsewhere every folder is entered and sizes come straight
/// from the metadata.
pub struct WalkContext {
    #[cfg(target_os = "linux")]
    mounts: MountTable,
    /// The walk's root and its real location, when a link makes them differ. The mount table
    /// only knows real paths.
    #[cfg(target_os = "linux")]
    link_root: Option<(PathBuf, PathBuf)>,
    #[cfg(target_os = "linux")]
    counted: parking_lot::Mutex<std::collections::HashSet<(u64, u64)>>,
}

impl WalkContext {
    #[cfg(target_os = "linux")]
    pub fn new(root: &Path) -> WalkContext {
        // Without a table the fixed paths are still skipped, and a measure is all that is at stake.
        Self::with_mounts(root, MountTable::read_current().unwrap_or_default())
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn with_mounts(root: &Path, mounts: MountTable) -> WalkContext {
        let link_root = fs::canonicalize(root)
            .ok()
            .filter(|real| real != root)
            .map(|real| (root.to_path_buf(), real));
        WalkContext {
            mounts,
            link_root,
            counted: Default::default(),
        }
    }

    #[cfg(not(target_os = "linux"))]
    pub fn new(_root: &Path) -> WalkContext {
        WalkContext {}
    }

    #[cfg(target_os = "linux")]
    fn real<'a>(&self, path: &'a Path) -> std::borrow::Cow<'a, Path> {
        match &self.link_root {
            Some((root, real)) => match path.strip_prefix(root) {
                Ok(rest) => real.join(rest).into(),
                Err(_) => path.into(),
            },
            None => path.into(),
        }
    }

    /// May a walk start at `path`, which the user or a rule named?
    #[cfg(target_os = "linux")]
    pub fn may_start(&self, path: &Path) -> bool {
        self.mounts.root_allowed(&self.real(path))
    }

    #[cfg(not(target_os = "linux"))]
    pub fn may_start(&self, _path: &Path) -> bool {
        true
    }

    /// What to do with a folder found during the walk.
    #[cfg(target_os = "linux")]
    pub fn action(&self, dir: &Path) -> DirAction {
        self.mounts.action_for(&self.real(dir))
    }

    #[cfg(not(target_os = "linux"))]
    pub fn action(&self, _dir: &Path) -> DirAction {
        DirAction::Enter
    }

    /// The bytes to count for a file, or `None` for another name of a file already counted.
    #[cfg(target_os = "linux")]
    pub fn count(&self, md: &fs::Metadata) -> Option<u64> {
        use std::os::unix::fs::MetadataExt;
        if md.nlink() > 1 && !self.counted.lock().insert((md.dev(), md.ino())) {
            return None;
        }
        Some(crate::fsmeta::disk_size(md))
    }

    #[cfg(not(target_os = "linux"))]
    pub fn count(&self, md: &fs::Metadata) -> Option<u64> {
        Some(crate::fsmeta::disk_size(md))
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Measure {
    pub bytes: u64,
    pub files: u64,
    pub dirs: u64,
    /// Newest modification time seen (Unix seconds), including files filtered out by age.
    pub newest: i64,
}

impl Measure {
    pub fn merge(mut self, other: Measure) -> Measure {
        self.bytes += other.bytes;
        self.files += other.files;
        self.dirs += other.dirs;
        self.newest = self.newest.max(other.newest);
        self
    }
}

/// Size of everything below `dir` (not counting `dir` itself). Files newer than `cutoff`
/// (Unix seconds) are skipped when a cutoff is given. Links are not followed.
pub fn measure_dir(dir: &Path, cutoff: Option<i64>, job: &Job) -> Measure {
    measure_dir_except(dir, cutoff, &[], job)
}

/// Like [`measure_dir`], but entries whose name matches one of `exclude` (globs, any depth)
/// are skipped, matching what a cleanup with the same rule would keep.
pub fn measure_dir_except(
    dir: &Path,
    cutoff: Option<i64>,
    exclude: &[String],
    job: &Job,
) -> Measure {
    let ctx = WalkContext::new(dir);
    if !ctx.may_start(dir) {
        return Measure::default();
    }
    measure_inner(dir, cutoff, exclude, job, &ctx)
}

fn measure_inner(
    dir: &Path,
    cutoff: Option<i64>,
    exclude: &[String],
    job: &Job,
    ctx: &WalkContext,
) -> Measure {
    if job.is_cancelled() {
        return Measure::default();
    }
    let Ok(rd) = fs::read_dir(dir) else {
        job.add_error();
        return Measure::default();
    };
    let mut m = Measure::default();
    let mut subdirs: Vec<PathBuf> = Vec::new();
    for entry in rd.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if is_link(&ft) {
            continue;
        }
        if !exclude.is_empty() && name_excluded(&entry.file_name().to_string_lossy(), exclude) {
            continue;
        }
        if ft.is_dir() {
            let path = entry.path();
            if ctx.action(&path) == DirAction::Enter {
                subdirs.push(path);
                m.dirs += 1;
            }
            continue;
        }
        let Ok(md) = entry.metadata() else { continue };
        if is_cloud_only(&md) {
            continue;
        }
        let t = mtime(&md);
        m.newest = m.newest.max(t);
        if cutoff.is_some_and(|c| t > c) {
            continue;
        }
        let Some(bytes) = ctx.count(&md) else {
            continue;
        };
        m.bytes += bytes;
        m.files += 1;
    }
    job.add_files(m.files, m.bytes);
    job.add_dir();
    let sub = subdirs
        .par_iter()
        .map(|p| measure_inner(p, cutoff, exclude, job, ctx))
        .reduce(Measure::default, Measure::merge);
    m.merge(sub)
}

pub fn name_excluded(name: &str, exclude: &[String]) -> bool {
    exclude
        .iter()
        .any(|g| crate::paths::wildcard_match(g, name))
}

/// Measures a path that may be a file or a folder.
pub fn measure_path(path: &Path, cutoff: Option<i64>, job: &Job) -> Measure {
    match path.symlink_metadata() {
        Ok(md) if is_link(&md.file_type()) => Measure::default(),
        Ok(md) if md.is_dir() => measure_dir(path, cutoff, job),
        Ok(md) => {
            if is_cloud_only(&md) {
                return Measure::default();
            }
            let t = mtime(&md);
            if cutoff.is_some_and(|c| t > c) {
                return Measure {
                    newest: t,
                    ..Default::default()
                };
            }
            let bytes = crate::fsmeta::disk_size(&md);
            job.add_files(1, bytes);
            Measure {
                bytes,
                files: 1,
                dirs: 0,
                newest: t,
            }
        }
        Err(_) => Measure::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(path: &Path, bytes: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut f = fs::File::create(path).unwrap();
        f.write_all(&vec![7u8; bytes]).unwrap();
    }

    fn size_on_disk(path: &Path) -> u64 {
        crate::fsmeta::disk_size(&fs::symlink_metadata(path).unwrap())
    }

    #[test]
    fn measures_nested_folders() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join("a.bin"), 100);
        write(&tmp.path().join("x/b.bin"), 50);
        write(&tmp.path().join("x/y/c.bin"), 25);
        let job = Job::new();
        let m = measure_dir(tmp.path(), None, &job);
        let expected: u64 = ["a.bin", "x/b.bin", "x/y/c.bin"]
            .iter()
            .map(|f| size_on_disk(&tmp.path().join(f)))
            .sum();
        assert_eq!(m.bytes, expected);
        assert_eq!(m.files, 3);
        assert_eq!(m.dirs, 2);
    }

    #[test]
    fn cutoff_skips_new_files() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join("new.bin"), 10);
        let job = Job::new();
        let m = measure_dir(tmp.path(), Some(crate::fsmeta::now_unix() - 3600), &job);
        assert_eq!(m.bytes, 0);
        assert_eq!(m.files, 0);
        assert!(m.newest > 0);
    }

    #[cfg(windows)]
    #[test]
    fn does_not_follow_junctions() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tmp.path().join("outside");
        write(&outside.join("secret.bin"), 1000);
        let inside = tmp.path().join("inside");
        fs::create_dir_all(&inside).unwrap();
        let link = inside.join("link");
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&outside)
            .output()
            .unwrap();
        assert!(status.status.success(), "mklink failed");
        let job = Job::new();
        let m = measure_dir(&inside, None, &job);
        assert_eq!(m.bytes, 0);
    }

    #[cfg(unix)]
    #[test]
    fn does_not_follow_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tmp.path().join("outside");
        write(&outside.join("secret.bin"), 1000);
        let inside = tmp.path().join("inside");
        fs::create_dir_all(&inside).unwrap();
        std::os::unix::fs::symlink(&outside, inside.join("link")).unwrap();
        let job = Job::new();
        let m = measure_dir(&inside, None, &job);
        assert_eq!(m.bytes, 0);
    }

    #[cfg(target_os = "linux")]
    mod linux {
        use super::*;
        use crate::mounts::parse_mountinfo;
        use std::os::unix::fs::MetadataExt;

        /// Pretends each `(folder, file system)` below `root` is a mount.
        fn table(mounts: &[(&Path, &str)]) -> MountTable {
            let text: String = mounts
                .iter()
                .enumerate()
                .map(|(i, (p, fs))| {
                    format!("{} 1 0:{i} / {} rw - {fs} src rw\n", i + 2, p.display())
                })
                .collect();
            MountTable::from_mounts(parse_mountinfo(text.as_bytes()))
        }

        fn measure(dir: &Path, mounts: MountTable) -> Measure {
            let ctx = WalkContext::with_mounts(dir, mounts);
            measure_inner(dir, None, &[], &Job::new(), &ctx)
        }

        #[test]
        fn counts_allocated_blocks_not_the_apparent_size() {
            let tmp = tempfile::tempdir().unwrap();
            let sparse = tmp.path().join("disk.img");
            let f = fs::File::create(&sparse).unwrap();
            f.set_len(1 << 30).unwrap();
            drop(f);
            let md = fs::metadata(&sparse).unwrap();
            assert_eq!(md.len(), 1 << 30);
            assert_eq!(crate::fsmeta::disk_size(&md), md.blocks() * 512);
            let m = measure_dir(tmp.path(), None, &Job::new());
            assert!(
                m.bytes < 1 << 20,
                "a sparse file costs almost nothing: {}",
                m.bytes
            );
            assert_eq!(m.files, 1);
            assert_eq!(measure_path(&sparse, None, &Job::new()).bytes, m.bytes);
        }

        #[test]
        fn counts_a_hard_linked_file_once() {
            let tmp = tempfile::tempdir().unwrap();
            let a = tmp.path().join("a");
            write(&a.join("one.bin"), 8192);
            write(&tmp.path().join("other.bin"), 8192);
            fs::hard_link(a.join("one.bin"), a.join("two.bin")).unwrap();
            fs::create_dir(tmp.path().join("b")).unwrap();
            fs::hard_link(a.join("one.bin"), tmp.path().join("b/three.bin")).unwrap();
            let m = measure_dir(tmp.path(), None, &Job::new());
            assert_eq!(m.files, 2);
            assert_eq!(
                m.bytes,
                size_on_disk(&a.join("one.bin")) + size_on_disk(&tmp.path().join("other.bin"))
            );
            // Each measure starts over, so a later one still sees the file.
            assert_eq!(measure_dir(&a, None, &Job::new()).files, 1);
        }

        #[test]
        fn stays_on_one_mount() {
            let tmp = tempfile::tempdir().unwrap();
            for name in [
                "plain", "nas", "sshfs", "usb", "snap", "docker", "ram", "kernel",
            ] {
                write(&tmp.path().join(name).join("f.bin"), 4096);
            }
            let m = |name: &str| tmp.path().join(name);
            let (nas, ssh, usb, snap, docker, ram, kernel) = (
                m("nas"),
                m("sshfs"),
                m("usb"),
                m("snap"),
                m("docker"),
                m("ram"),
                m("kernel"),
            );
            let t = table(&[
                (&nas, "nfs4"),
                (&ssh, "fuse.sshfs"),
                (&usb, "ext4"),
                (&snap, "squashfs"),
                (&docker, "overlay"),
                (&ram, "tmpfs"),
                (&kernel, "sysfs"),
            ]);
            let measured = measure(tmp.path(), t);
            assert_eq!(measured.files, 1, "only the plain folder is entered");
            assert_eq!(measured.dirs, 1);
            assert_eq!(measured.bytes, size_on_disk(&m("plain/f.bin")));
        }

        #[test]
        fn a_chosen_root_may_be_a_mount_but_not_a_kernel_one() {
            let tmp = tempfile::tempdir().unwrap();
            let root = tmp.path().join("nas");
            write(&root.join("f.bin"), 4096);
            let ctx = WalkContext::with_mounts(&root, table(&[(&root, "nfs4")]));
            assert!(ctx.may_start(&root));
            let ctx = WalkContext::with_mounts(&root, table(&[(&root, "proc")]));
            assert!(!ctx.may_start(&root));
            assert!(!WalkContext::new(Path::new("/proc")).may_start(Path::new("/proc/self")));
            assert_eq!(
                measure_dir(Path::new("/proc/self"), None, &Job::new()).files,
                0
            );
            assert_eq!(measure_dir(Path::new("/sys"), None, &Job::new()).files, 0);
        }

        #[test]
        fn follows_a_linked_root_to_the_real_mount_table() {
            let tmp = tempfile::tempdir().unwrap();
            let real = tmp.path().join("real");
            write(&real.join("nas/f.bin"), 4096);
            write(&real.join("g.bin"), 4096);
            let link = tmp.path().join("link");
            std::os::unix::fs::symlink(&real, &link).unwrap();
            let nas = fs::canonicalize(real.join("nas")).unwrap();
            let measured = measure(&link, table(&[(&nas, "nfs4")]));
            assert_eq!(measured.files, 1);
        }
    }
}
