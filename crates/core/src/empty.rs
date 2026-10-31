//! Empty folder finder. Reports the topmost empty folder of each empty branch, so a chain of
//! nested empty folders becomes one entry.

use std::fs;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::Serialize;

use crate::fsmeta::{is_link, mtime};
use crate::job::Job;
use crate::paths::{Guard, Origin};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmptyFolder {
    pub path: String,
    pub mtime: i64,
    /// Empty folders nested inside, removed together with this one.
    pub nested: u32,
}

const SKIP: &[&str] = &[
    ".git",
    "node_modules",
    "$recycle.bin",
    "system volume information",
    "appdata",
];

/// Returns (is_empty, nested_count, found) for `dir`.
type Visit = (bool, u32, Vec<EmptyFolder>);

fn visit(dir: &Path, job: &Job) -> Visit {
    if job.is_cancelled() {
        return (false, 0, Vec::new());
    }
    let Ok(rd) = fs::read_dir(dir) else {
        job.add_error();
        return (false, 0, Vec::new());
    };
    job.add_dir();
    let mut has_files = false;
    let mut subdirs = Vec::new();
    for e in rd.flatten() {
        let Ok(ft) = e.file_type() else {
            has_files = true;
            continue;
        };
        if is_link(&ft) || !ft.is_dir() {
            has_files = true;
            continue;
        }
        let name = e.file_name().to_string_lossy().to_lowercase();
        if SKIP.contains(&name.as_str()) {
            has_files = true;
            continue;
        }
        subdirs.push(e.path());
    }
    let results: Vec<(PathBuf, Visit)> = subdirs
        .into_par_iter()
        .map(|p| {
            let r = visit(&p, job);
            (p, r)
        })
        .collect();
    let all_empty = !has_files && results.iter().all(|(_, (e, _, _))| *e);
    if all_empty {
        let nested = results.iter().map(|(_, (_, n, _))| n + 1).sum();
        return (true, nested, Vec::new());
    }
    let mut found = Vec::new();
    for (p, (empty, nested, inner)) in results {
        if empty {
            let t = fs::metadata(&p).map(|m| mtime(&m)).unwrap_or(0);
            found.push(EmptyFolder {
                path: p.to_string_lossy().into_owned(),
                mtime: t,
                nested,
            });
        } else {
            found.extend(inner);
        }
    }
    (false, 0, found)
}

pub fn find_empty(roots: &[PathBuf], guard: &Guard, job: &Job) -> Vec<EmptyFolder> {
    job.set_stage("empty");
    let mut out: Vec<EmptyFolder> = roots.par_iter().flat_map(|r| visit(r, job).2).collect();
    out.retain(|f| guard.check(Path::new(&f.path), Origin::User).is_ok());
    out.sort_by_key(|a| a.path.to_lowercase());
    out.dedup_by(|a, b| a.path.eq_ignore_ascii_case(&b.path));
    out.truncate(5000);
    out
}

// Rule expansion and the guard only know Windows paths so far.
#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn reports_topmost_empty_folders() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join(r"a\b\c")).unwrap();
        fs::create_dir_all(tmp.path().join(r"d\e")).unwrap();
        fs::write(tmp.path().join(r"d\file.txt"), b"x").unwrap();
        fs::create_dir_all(tmp.path().join(r"f\.git")).unwrap();
        let found = find_empty(
            &[tmp.path().to_path_buf()],
            &Guard::new(&[], None),
            &Job::new(),
        );
        let names: Vec<String> = found
            .iter()
            .map(|f| {
                Path::new(&f.path)
                    .strip_prefix(tmp.path())
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(names, vec!["a".to_string(), r"d\e".to_string()]);
        assert_eq!(found[0].nested, 2);
    }
}
