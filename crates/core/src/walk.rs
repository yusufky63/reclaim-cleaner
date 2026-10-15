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
        if ft.is_dir() {
            subdirs.push(entry.path());
            m.dirs += 1;
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
        m.bytes += md.len();
        m.files += 1;
    }
    job.add_files(m.files, m.bytes);
    job.add_dir();
    let sub = subdirs
        .par_iter()
        .map(|p| measure_dir(p, cutoff, job))
        .reduce(Measure::default, Measure::merge);
    m.merge(sub)
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
            job.add_files(1, md.len());
            Measure {
                bytes: md.len(),
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

    #[test]
    fn measures_nested_folders() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join("a.bin"), 100);
        write(&tmp.path().join(r"x\b.bin"), 50);
        write(&tmp.path().join(r"x\y\c.bin"), 25);
        let job = Job::new();
        let m = measure_dir(tmp.path(), None, &job);
        assert_eq!(m.bytes, 175);
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
}
