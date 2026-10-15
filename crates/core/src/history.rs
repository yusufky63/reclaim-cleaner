//! Cleanup history as JSON Lines: one object per cleanup, appended, never rewritten.

use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::clean::{CleanReport, DeleteMethod};
use crate::{Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    /// RFC 3339 timestamp.
    pub time: String,
    /// `junk`, `projects`, `large`, `duplicates`, `empty`, `explorer`, `installers`,
    /// `recycle-bin`, `tool`.
    pub kind: String,
    pub method: DeleteMethod,
    pub freed: u64,
    pub recycled: u64,
    pub files: u64,
    pub failed: u64,
    /// Rule ids or (up to 50) paths.
    pub items: Vec<String>,
    #[serde(default)]
    pub elevated: bool,
}

impl HistoryEntry {
    pub fn from_report(
        kind: &str,
        method: DeleteMethod,
        report: &CleanReport,
        elevated: bool,
    ) -> Self {
        HistoryEntry {
            time: chrono::Local::now().to_rfc3339(),
            kind: kind.to_string(),
            method,
            freed: report.freed,
            recycled: report.recycled,
            files: report.files,
            failed: report.failed,
            items: report.items.iter().take(50).cloned().collect(),
            elevated,
        }
    }
}

fn file(dir: &Path) -> std::path::PathBuf {
    dir.join("history.jsonl")
}

pub fn append(dir: &Path, entry: &HistoryEntry) -> Result<()> {
    std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    let path = file(dir);
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| Error::io(&path, e))?;
    let mut line = serde_json::to_vec(entry)?;
    line.push(b'\n');
    f.write_all(&line).map_err(|e| Error::io(&path, e))
}

/// Newest first. Lines that fail to parse are skipped rather than hiding the rest.
pub fn read(dir: &Path, limit: usize) -> Vec<HistoryEntry> {
    let Ok(f) = std::fs::File::open(file(dir)) else {
        return Vec::new();
    };
    let mut entries: Vec<HistoryEntry> = BufReader::new(f)
        .lines()
        .map_while(|l| l.ok())
        .filter_map(|l| serde_json::from_str(&l).ok())
        .collect();
    entries.reverse();
    entries.truncate(limit);
    entries
}

pub fn clear(dir: &Path) -> Result<()> {
    let path = file(dir);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::io(path, e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_and_reads_newest_first() {
        let tmp = tempfile::tempdir().unwrap();
        let mut r = CleanReport {
            freed: 10,
            ..Default::default()
        };
        append(
            tmp.path(),
            &HistoryEntry::from_report("junk", DeleteMethod::Permanent, &r, false),
        )
        .unwrap();
        r.freed = 20;
        append(
            tmp.path(),
            &HistoryEntry::from_report("large", DeleteMethod::Recycle, &r, false),
        )
        .unwrap();
        let entries = read(tmp.path(), 10);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].kind, "large");
        clear(tmp.path()).unwrap();
        assert!(read(tmp.path(), 10).is_empty());
    }
}
