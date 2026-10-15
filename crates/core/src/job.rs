use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use parking_lot::Mutex;
use serde::Serialize;

/// Shared state of one long-running operation: a cancel flag plus progress counters that worker
/// threads bump without locking. The UI reads snapshots on a timer instead of receiving an event
/// per file.
#[derive(Default)]
pub struct Job {
    cancelled: AtomicBool,
    pub files: AtomicU64,
    pub dirs: AtomicU64,
    pub bytes: AtomicU64,
    pub errors: AtomicU64,
    pub done: AtomicU64,
    pub total: AtomicU64,
    stage: Mutex<String>,
    current: Mutex<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProgressSnapshot {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub errors: u64,
    pub done: u64,
    pub total: u64,
    pub stage: String,
    pub current: String,
}

impl Job {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    pub fn set_stage(&self, stage: &str) {
        *self.stage.lock() = stage.to_string();
    }

    /// Updating the current path takes a lock, so callers do it per directory, not per file.
    pub fn set_current(&self, path: &std::path::Path) {
        if let Some(mut guard) = self.current.try_lock() {
            guard.clear();
            guard.push_str(&path.to_string_lossy());
        }
    }

    pub fn add_files(&self, files: u64, bytes: u64) {
        if files > 0 {
            self.files.fetch_add(files, Ordering::Relaxed);
        }
        if bytes > 0 {
            self.bytes.fetch_add(bytes, Ordering::Relaxed);
        }
    }

    pub fn add_dir(&self) {
        self.dirs.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add_error(&self) {
        self.errors.fetch_add(1, Ordering::Relaxed);
    }

    pub fn set_total(&self, total: u64) {
        self.total.store(total, Ordering::Relaxed);
        self.done.store(0, Ordering::Relaxed);
    }

    pub fn add_done(&self, n: u64) {
        self.done.fetch_add(n, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> ProgressSnapshot {
        ProgressSnapshot {
            files: self.files.load(Ordering::Relaxed),
            dirs: self.dirs.load(Ordering::Relaxed),
            bytes: self.bytes.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
            done: self.done.load(Ordering::Relaxed),
            total: self.total.load(Ordering::Relaxed),
            stage: self.stage.lock().clone(),
            current: self.current.lock().clone(),
        }
    }
}
