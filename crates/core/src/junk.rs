//! Junk scan: measures every cleanup target without deleting anything.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::Serialize;

use crate::fsmeta::{is_cloud_only, is_link, mtime, now_unix};
use crate::job::Job;
use crate::paths::{expand_pattern, wildcard_match};
use crate::rules::{Mode, TargetRule};
use crate::system::recycle_bin_info;
use crate::walk::{Measure, measure_dir_except, measure_path};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundPath {
    pub path: String,
    pub bytes: u64,
    pub files: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetResult {
    pub id: String,
    pub bytes: u64,
    pub files: u64,
    pub found: Vec<FoundPath>,
    /// Processes from `closeApps` that are running now.
    pub running: Vec<String>,
    /// Some paths could not be read (usually needs admin).
    pub partial: bool,
    /// The rule's official command exists on this PC.
    pub command_available: bool,
}

/// Unix-seconds cutoff for a rule's `minAgeHours`.
pub fn cutoff_for(rule: &TargetRule) -> Option<i64> {
    rule.min_age_hours.map(|h| now_unix() - i64::from(h) * 3600)
}

/// Files directly inside `dir` whose names match any of `include`.
pub fn matching_files(dir: &Path, include: &[String]) -> Vec<(PathBuf, u64, i64)> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(dir) else {
        return out;
    };
    for entry in rd.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_file() || is_link(&ft) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if !include.iter().any(|g| wildcard_match(g, &name)) {
            continue;
        }
        let Ok(md) = entry.metadata() else { continue };
        if is_cloud_only(&md) {
            continue;
        }
        out.push((entry.path(), md.len(), mtime(&md)));
    }
    out
}

fn command_tool(rule: &TargetRule) -> Option<&str> {
    let cmd = rule.command.as_deref()?;
    if cmd.starts_with("api:") {
        return None;
    }
    let first = cmd.split_whitespace().next()?;
    Some(first.trim_matches('"'))
}

/// Measures one rule.
pub fn scan_target(rule: &TargetRule, running: &HashSet<String>, job: &Job) -> TargetResult {
    let cutoff = cutoff_for(rule);
    let mut found = Vec::new();
    let mut partial = false;
    if rule
        .command
        .as_deref()
        .is_some_and(|c| c.starts_with("api:SHEmptyRecycleBin"))
    {
        let info = recycle_bin_info();
        if info.bytes > 0 {
            found.push(FoundPath {
                path: String::from("Recycle Bin"),
                bytes: info.bytes,
                files: info.items,
            });
        }
    } else {
        for pattern in &rule.paths {
            let Ok(paths) = expand_pattern(pattern) else {
                continue;
            };
            for p in paths {
                if job.is_cancelled() {
                    break;
                }
                // Rules are measured in parallel, so each path gets its own counters; reading
                // the shared error count would blame one rule for another's unreadable folder.
                let local = Job::new();
                let m = match rule.mode {
                    Mode::Files => matching_files(&p, &rule.include)
                        .into_iter()
                        .filter(|(_, _, t)| cutoff.is_none_or(|c| *t <= c))
                        .fold(Measure::default(), |acc, (_, len, t)| {
                            acc.merge(Measure {
                                bytes: len,
                                files: 1,
                                dirs: 0,
                                newest: t,
                            })
                        }),
                    Mode::Contents => measure_dir_except(&p, cutoff, &rule.exclude, &local),
                    Mode::Folder | Mode::Report => measure_path(&p, cutoff, &local),
                };
                let errors = local.errors.load(std::sync::atomic::Ordering::Relaxed);
                if errors > 0 {
                    partial = true;
                }
                job.add_files(m.files, m.bytes);
                job.errors
                    .fetch_add(errors, std::sync::atomic::Ordering::Relaxed);
                if m.bytes > 0 || m.files > 0 {
                    found.push(FoundPath {
                        path: p.to_string_lossy().into_owned(),
                        bytes: m.bytes,
                        files: m.files,
                    });
                }
            }
        }
    }
    let running_now: Vec<String> = rule
        .close_apps
        .iter()
        .filter(|a| running.contains(&a.to_lowercase()))
        .cloned()
        .collect();
    let command_available = command_tool(rule).is_some_and(|t| crate::system::which(t).is_some());
    TargetResult {
        id: rule.id.clone(),
        bytes: found.iter().map(|f| f.bytes).sum(),
        files: found.iter().map(|f| f.files).sum(),
        found,
        running: running_now,
        partial,
        command_available,
    }
}

/// Measures all rules in parallel. Results keep the input order.
pub fn scan_targets(rules: &[TargetRule], job: &Job) -> Vec<TargetResult> {
    job.set_stage("junk");
    job.set_total(rules.len() as u64);
    let running = crate::system::running_processes();
    rules
        .par_iter()
        .map(|r| {
            let res = scan_target(r, &running, job);
            job.add_done(1);
            res
        })
        .collect()
}

// Rule expansion and the guard only know Windows paths so far.
#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::rules::{Group, Risk};

    fn rule(paths: Vec<String>, mode: Mode, include: Vec<String>) -> TargetRule {
        TargetRule {
            id: "t".into(),
            group: Group::Apps,
            name: "t".into(),
            name_tr: "t".into(),
            description: "d".into(),
            description_tr: "d".into(),
            paths,
            mode,
            include,
            exclude: vec![],
            min_age_hours: None,
            risk: Risk::Safe,
            admin: false,
            close_apps: vec!["definitely-not-running-xyz.exe".into()],
            command: None,
            verified: true,
            source: None,
            user: false,
        }
    }

    #[test]
    fn measures_contents_and_files_modes() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("Cache");
        fs::create_dir_all(cache.join("sub")).unwrap();
        fs::write(cache.join("a.bin"), [0u8; 10]).unwrap();
        fs::write(cache.join(r"sub\b.bin"), [0u8; 20]).unwrap();
        fs::write(tmp.path().join("x.dmp"), [0u8; 5]).unwrap();
        fs::write(tmp.path().join("keep.txt"), [0u8; 7]).unwrap();
        let job = Job::new();
        let running = HashSet::new();
        let r = scan_target(
            &rule(
                vec![cache.to_string_lossy().into_owned()],
                Mode::Contents,
                vec![],
            ),
            &running,
            &job,
        );
        // Absolute paths without a variable are fine here: expand_pattern only needs a drive.
        assert_eq!(r.bytes, 30);
        assert_eq!(r.files, 2);
        let r = scan_target(
            &rule(
                vec![tmp.path().to_string_lossy().into_owned()],
                Mode::Files,
                vec!["*.dmp".into()],
            ),
            &running,
            &job,
        );
        assert_eq!(r.bytes, 5);
        assert!(r.running.is_empty());
    }
}
