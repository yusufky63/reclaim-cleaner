//! Developer artifact finder: build output and dependency folders inside project folders
//! (`node_modules`, `target`, `.next`, `__pycache__`, ...), detected by marker files the way
//! kondo and npkill do.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use rayon::prelude::*;
use serde::Serialize;

use crate::fsmeta::{is_link, mtime};
use crate::job::Job;
use crate::paths::{env_path, wildcard_match};
use crate::rules::{ArtifactRule, Risk};
use crate::walk::measure_dir;

const MAX_DEPTH: usize = 14;

/// Folders never searched for projects: they hold installed software or OS data, and
/// `node_modules` inside an installed app belongs to that app.
const SKIP_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "appdata",
    "windows",
    "program files",
    "program files (x86)",
    "programdata",
    "$recycle.bin",
    "system volume information",
    ".cache",
    ".vscode",
    ".cursor",
    "scoop",
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactHit {
    pub path: String,
    pub rule_id: String,
    pub dir_name: String,
    pub risk: Risk,
    pub bytes: u64,
    pub files: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub path: String,
    pub name: String,
    /// Latest activity outside the artifact folders (Unix seconds).
    pub last_activity: i64,
    pub has_git: bool,
    pub artifacts: Vec<ArtifactHit>,
    pub bytes: u64,
}

struct DirListing {
    files: HashSet<String>,
    dirs: Vec<(String, PathBuf)>,
}

fn list(dir: &Path) -> Option<DirListing> {
    let rd = fs::read_dir(dir).ok()?;
    let mut files = HashSet::new();
    let mut dirs = Vec::new();
    for e in rd.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        if is_link(&ft) {
            continue;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        if ft.is_dir() {
            dirs.push((name, e.path()));
        } else {
            files.insert(name.to_lowercase());
        }
    }
    Some(DirListing { files, dirs })
}

fn marker_present(dir: &Path, listing: &DirListing, marker: &str) -> bool {
    if marker.contains('/') || marker.contains('\\') {
        return dir.join(marker.replace('/', "\\")).exists();
    }
    if marker.contains('*') || marker.contains('?') {
        return listing.files.iter().any(|f| wildcard_match(marker, f));
    }
    listing.files.contains(&marker.to_lowercase())
}

/// The rule that owns folder `name` inside `parent`, if any. Rules are tried in file order,
/// which lists specific owners (Gradle, Flutter) before generic ones (JS `build`).
fn match_rule<'a>(
    parent: &Path,
    listing: &DirListing,
    name: &str,
    path: &Path,
    rules: &'a [ArtifactRule],
) -> Option<&'a ArtifactRule> {
    rules.iter().find(|r| {
        wildcard_match(&r.dir_name, name)
            && (r.markers.is_empty()
                || r.markers.iter().any(|m| marker_present(parent, listing, m)))
            && r.contains.iter().all(|c| path.join(c).exists())
    })
}

fn activity(dir: &Path, listing: &DirListing, artifact_names: &HashSet<String>) -> i64 {
    let mut latest = 0i64;
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_lowercase();
            if artifact_names.contains(&name) {
                continue;
            }
            if let Ok(md) = e.metadata() {
                latest = latest.max(mtime(&md));
            }
        }
    }
    // The git index changes on every commit and checkout, a good proxy for "worked on".
    for p in [r".git\index", r".git\HEAD", r".git\FETCH_HEAD"] {
        if let Ok(md) = fs::metadata(dir.join(p)) {
            latest = latest.max(mtime(&md));
        }
    }
    let _ = listing;
    latest
}

fn walk(
    dir: &Path,
    depth: usize,
    rules: &[ArtifactRule],
    job: &Job,
    out: &Mutex<BTreeMap<String, Project>>,
) {
    if job.is_cancelled() || depth > MAX_DEPTH {
        return;
    }
    let Some(listing) = list(dir) else {
        job.add_error();
        return;
    };
    job.add_dir();
    if job
        .dirs
        .load(std::sync::atomic::Ordering::Relaxed)
        .is_multiple_of(32)
    {
        job.set_current(dir);
    }
    let mut hits = Vec::new();
    let mut descend = Vec::new();
    for (name, path) in &listing.dirs {
        let lower = name.to_lowercase();
        if let Some(rule) = match_rule(dir, &listing, name, path, rules) {
            hits.push((rule, name.clone(), path.clone()));
            continue;
        }
        if SKIP_DIRS.contains(&lower.as_str()) {
            continue;
        }
        descend.push(path.clone());
    }
    if !hits.is_empty() {
        let names: HashSet<String> = hits.iter().map(|(_, n, _)| n.to_lowercase()).collect();
        let measured: Vec<ArtifactHit> = hits
            .par_iter()
            .map(|(rule, name, path)| {
                let m = measure_dir(path, None, job);
                ArtifactHit {
                    path: path.to_string_lossy().into_owned(),
                    rule_id: rule.id.clone(),
                    dir_name: name.clone(),
                    risk: rule.risk,
                    bytes: m.bytes,
                    files: m.files,
                }
            })
            .filter(|h| h.bytes > 0)
            .collect();
        if !measured.is_empty() {
            let project = Project {
                path: dir.to_string_lossy().into_owned(),
                name: dir
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                last_activity: activity(dir, &listing, &names),
                has_git: dir.join(".git").exists(),
                bytes: measured.iter().map(|h| h.bytes).sum(),
                artifacts: measured,
            };
            out.lock().insert(project.path.to_lowercase(), project);
        }
    }
    descend
        .par_iter()
        .for_each(|p| walk(p, depth + 1, rules, job, out));
}

/// Finds projects with artifacts below `roots`. Results are sorted by reclaimable size.
pub fn scan_projects(roots: &[PathBuf], rules: &[ArtifactRule], job: &Job) -> Vec<Project> {
    job.set_stage("projects");
    let out = Mutex::new(BTreeMap::new());
    roots.par_iter().for_each(|r| walk(r, 0, rules, job, &out));
    let mut projects: Vec<Project> = out.into_inner().into_values().collect();
    projects.sort_by_key(|x| std::cmp::Reverse(x.bytes));
    projects
}

/// Folders where code usually lives. Only existing ones are returned.
pub fn suggested_roots() -> Vec<String> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(home) = env_path("USERPROFILE") {
        for sub in [
            r"source\repos",
            "source",
            "repos",
            "Projects",
            "projects",
            "dev",
            "Dev",
            "code",
            "Code",
            "git",
            "GitHub",
            "workspace",
            "Developer",
            r"Documents\GitHub",
            r"Documents\Projects",
        ] {
            out.push(home.join(sub));
        }
    }
    for d in crate::system::drives().iter().filter(|d| !d.removable) {
        for sub in [
            "Projects",
            "Projeler",
            "Projelerim",
            "dev",
            "src",
            "code",
            "repos",
            "git",
            "workspace",
            "Work",
        ] {
            out.push(PathBuf::from(&d.root).join(sub));
        }
    }
    let mut seen = HashSet::new();
    out.into_iter()
        .filter(|p| p.is_dir())
        .map(|p| p.to_string_lossy().into_owned())
        .filter(|p| seen.insert(p.to_lowercase()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::builtin_artifacts;

    #[test]
    fn finds_artifacts_by_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let web = tmp.path().join("web");
        fs::create_dir_all(web.join(r"node_modules\pkg")).unwrap();
        fs::create_dir_all(web.join(".next")).unwrap();
        fs::write(web.join("package.json"), b"{}").unwrap();
        fs::write(web.join(r"node_modules\pkg\index.js"), [0u8; 100]).unwrap();
        fs::write(web.join(r".next\build.js"), [0u8; 50]).unwrap();
        // A "target" folder without Cargo.toml is not an artifact.
        let docs = tmp.path().join("docs");
        fs::create_dir_all(docs.join("target")).unwrap();
        fs::write(docs.join(r"target\plan.txt"), [0u8; 10]).unwrap();
        let rust = tmp.path().join("tool");
        fs::create_dir_all(rust.join(r"target\debug")).unwrap();
        fs::write(rust.join("Cargo.toml"), b"[package]").unwrap();
        fs::write(rust.join(r"target\debug\tool.exe"), [0u8; 30]).unwrap();
        // A Gradle "build" is owned by the Gradle rule, not the JS one.
        let app = tmp.path().join("android");
        fs::create_dir_all(app.join("build")).unwrap();
        fs::write(app.join("build.gradle"), b"").unwrap();
        fs::write(app.join("package.json"), b"{}").unwrap();
        fs::write(app.join(r"build\out.apk"), [0u8; 7]).unwrap();

        let job = Job::new();
        let projects = scan_projects(&[tmp.path().to_path_buf()], builtin_artifacts(), &job);
        assert_eq!(projects.len(), 3);
        let web_p = projects.iter().find(|p| p.name == "web").unwrap();
        assert_eq!(web_p.bytes, 150);
        assert_eq!(web_p.artifacts.len(), 2);
        assert!(projects.iter().all(|p| p.name != "docs"));
        let android = projects.iter().find(|p| p.name == "android").unwrap();
        assert_eq!(android.artifacts[0].rule_id, "gradle-build");
    }
}
