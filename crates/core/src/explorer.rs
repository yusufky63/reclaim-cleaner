//! Space explorer: one parallel scan of a folder or drive into an arena tree that the UI reads
//! a few levels at a time.
//!
//! Folders are always nodes. Files of [`BIG_FILE`] or more are their own nodes; smaller files
//! are folded into one "small files" node per folder, which keeps a full drive at a few hundred
//! thousand nodes instead of millions.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use parking_lot::Mutex;
use rayon::prelude::*;
use serde::Serialize;

use crate::categories::{Category, classify_dir, classify_file, is_project_marker};
use crate::fsmeta::{is_cloud_only, is_link, mtime, now_unix};
use crate::job::Job;
use crate::paths::key;
use crate::rules::Risk;

pub const BIG_FILE: u64 = 4 * 1024 * 1024;
const NO_PARENT: u32 = u32::MAX;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    Dir,
    File,
    /// All files under [`BIG_FILE`] directly inside one folder.
    Small,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    pub parent: u32,
    pub kind: NodeKind,
    pub bytes: u64,
    pub files: u64,
    pub mtime: i64,
    pub category: Category,
    pub denied: bool,
    pub children: Vec<u32>,
    pub reclaim_safe: u64,
    pub reclaim_review: u64,
    pub marked: Option<Risk>,
    pub removed: bool,
}

struct RawDir {
    name: String,
    bytes: u64,
    files: u64,
    mtime: i64,
    denied: bool,
    is_project: bool,
    big: Vec<(String, u64, i64)>,
    small_bytes: u64,
    small_files: u64,
    small_mtime: i64,
    /// Online-only cloud files below this folder: listed, but not stored on the drive.
    cloud_bytes: u64,
    cloud_files: u64,
    dirs: Vec<RawDir>,
}

#[derive(Default)]
struct ExtAcc {
    map: Mutex<HashMap<String, (u64, u64)>>,
}

fn empty_raw(name: String) -> RawDir {
    RawDir {
        name,
        bytes: 0,
        files: 0,
        mtime: 0,
        denied: false,
        is_project: false,
        big: Vec::new(),
        small_bytes: 0,
        small_files: 0,
        small_mtime: 0,
        cloud_bytes: 0,
        cloud_files: 0,
        dirs: Vec::new(),
    }
}

fn add_child_totals(raw: &mut RawDir) {
    for d in &raw.dirs {
        raw.bytes += d.bytes;
        raw.files += d.files;
        raw.mtime = raw.mtime.max(d.mtime);
        raw.cloud_bytes += d.cloud_bytes;
        raw.cloud_files += d.cloud_files;
    }
}

fn scan_raw(path: &Path, name: String, job: &Job, ext: &ExtAcc) -> RawDir {
    let mut raw = empty_raw(name);
    if job.is_cancelled() {
        return raw;
    }
    let rd = match fs::read_dir(path) {
        Ok(rd) => rd,
        Err(_) => {
            raw.denied = true;
            job.add_error();
            return raw;
        }
    };
    let mut subdirs: Vec<(PathBuf, String)> = Vec::new();
    let mut local_ext: HashMap<String, (u64, u64)> = HashMap::new();
    for entry in rd.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if is_link(&ft) {
            continue;
        }
        let fname = entry.file_name().to_string_lossy().into_owned();
        if ft.is_dir() {
            subdirs.push((entry.path(), fname));
            continue;
        }
        let Ok(md) = entry.metadata() else { continue };
        if is_cloud_only(&md) {
            raw.cloud_bytes += md.len();
            raw.cloud_files += 1;
            continue;
        }
        if is_project_marker(&fname) {
            raw.is_project = true;
        }
        let len = md.len();
        let t = mtime(&md);
        raw.files += 1;
        raw.bytes += len;
        raw.mtime = raw.mtime.max(t);
        let e = fname
            .rsplit_once('.')
            .map(|(_, e)| e.to_lowercase())
            .filter(|e| e.len() <= 12 && !e.contains(' '))
            .unwrap_or_default();
        let slot = local_ext.entry(e).or_default();
        slot.0 += len;
        slot.1 += 1;
        if len >= BIG_FILE {
            raw.big.push((fname, len, t));
        } else {
            raw.small_bytes += len;
            raw.small_files += 1;
            raw.small_mtime = raw.small_mtime.max(t);
        }
    }
    job.add_files(raw.files, raw.bytes);
    job.add_dir();
    if !local_ext.is_empty() {
        let mut global = ext.map.lock();
        for (k, (b, c)) in local_ext {
            let slot = global.entry(k).or_default();
            slot.0 += b;
            slot.1 += c;
        }
    }
    if job
        .dirs
        .load(std::sync::atomic::Ordering::Relaxed)
        .is_multiple_of(64)
    {
        job.set_current(path);
    }
    raw.dirs = subdirs
        .into_par_iter()
        .map(|(p, n)| scan_raw(&p, n, job, ext))
        .collect();
    add_child_totals(&mut raw);
    raw
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtStat {
    pub ext: String,
    pub bytes: u64,
    pub files: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanSummary {
    pub root: String,
    pub bytes: u64,
    pub files: u64,
    pub dirs: u64,
    pub denied: u64,
    pub millis: u64,
    pub scanned_at: i64,
    pub cancelled: bool,
    /// Online-only cloud files that were skipped: their full size and count.
    pub cloud_bytes: u64,
    pub cloud_files: u64,
    /// Names of the folders scanned inside `root` when only a selection was scanned (several
    /// folders picked together in Explorer); empty for a scan of the whole root.
    pub selection: Vec<String>,
}

pub struct Tree {
    pub root_path: PathBuf,
    pub nodes: Vec<Node>,
    pub extensions: Vec<ExtStat>,
    pub summary: ScanSummary,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeView {
    pub id: u32,
    pub name: String,
    pub path: String,
    pub kind: NodeKind,
    pub category: Category,
    pub bytes: u64,
    pub files: u64,
    pub mtime: i64,
    pub denied: bool,
    pub reclaim_safe: u64,
    pub reclaim_review: u64,
    pub marked: Option<Risk>,
    pub child_count: u32,
    pub children: Vec<NodeView>,
    /// Bytes and count of children cut off by the `limit`.
    pub rest_bytes: u64,
    pub rest_count: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileView {
    pub id: u32,
    pub name: String,
    pub path: String,
    pub bytes: u64,
    pub mtime: i64,
    pub category: Category,
}

impl Tree {
    /// Scans `root` (a folder or drive root like `C:\`).
    pub fn scan(root: &Path, job: &Job) -> Tree {
        Self::scan_inner(root, None, job)
    }

    /// Scans only the named folders inside `parent`, for several folders picked together in
    /// Explorer. The tree is rooted at `parent`, so paths and lookups work as usual; files
    /// directly in `parent` and its other folders are left out.
    pub fn scan_selection(parent: &Path, names: &[String], job: &Job) -> Tree {
        Self::scan_inner(parent, Some(names), job)
    }

    fn scan_inner(root: &Path, selection: Option<&[String]>, job: &Job) -> Tree {
        let started = Instant::now();
        job.set_stage("scan");
        let ext = ExtAcc::default();
        let root_name = root.to_string_lossy().trim_end_matches('\\').to_string();
        let raw = match selection {
            None => scan_raw(root, root_name, job, &ext),
            Some(names) => {
                let mut raw = empty_raw(root_name);
                raw.dirs = names
                    .par_iter()
                    .map(|n| scan_raw(&root.join(n), n.clone(), job, &ext))
                    .collect();
                add_child_totals(&mut raw);
                raw
            }
        };
        let (cloud_bytes, cloud_files) = (raw.cloud_bytes, raw.cloud_files);
        let depth0 = root.components().count().saturating_sub(1);
        let mut nodes: Vec<Node> = Vec::with_capacity(
            job.dirs.load(std::sync::atomic::Ordering::Relaxed) as usize * 2 + 16,
        );
        let root_cat = classify_dir(
            raw.name.rsplit('\\').next().unwrap_or(""),
            Category::Other,
            depth0,
            raw.is_project,
        );
        flatten(raw, NO_PARENT, root_cat, depth0, &mut nodes);
        let mut extensions: Vec<ExtStat> = ext
            .map
            .into_inner()
            .into_iter()
            .map(|(ext, (bytes, files))| ExtStat { ext, bytes, files })
            .collect();
        extensions.sort_by_key(|x| std::cmp::Reverse(x.bytes));
        let root_node = &nodes[0];
        let summary = ScanSummary {
            root: root.to_string_lossy().into_owned(),
            bytes: root_node.bytes,
            files: root_node.files,
            dirs: job.dirs.load(std::sync::atomic::Ordering::Relaxed),
            denied: nodes.iter().filter(|n| n.denied).count() as u64,
            millis: started.elapsed().as_millis() as u64,
            scanned_at: now_unix(),
            cancelled: job.is_cancelled(),
            cloud_bytes,
            cloud_files,
            selection: selection.map(<[String]>::to_vec).unwrap_or_default(),
        };
        Tree {
            root_path: root.to_path_buf(),
            nodes,
            extensions,
            summary,
        }
    }

    pub fn path_of(&self, id: u32) -> PathBuf {
        let mut parts = Vec::new();
        let mut cur = id;
        while cur != NO_PARENT && cur != 0 {
            let n = &self.nodes[cur as usize];
            parts.push(n.name.as_str());
            cur = n.parent;
        }
        let mut p = self.root_path.clone();
        for part in parts.iter().rev() {
            p.push(part);
        }
        p
    }

    pub fn view(&self, id: u32, depth: u32, limit: usize) -> Option<NodeView> {
        let node = self.nodes.get(id as usize)?;
        if node.removed {
            return None;
        }
        let live: Vec<u32> = node
            .children
            .iter()
            .copied()
            .filter(|c| !self.nodes[*c as usize].removed)
            .collect();
        let mut children = Vec::new();
        let (mut rest_bytes, mut rest_count) = (0u64, 0u32);
        if depth > 0 {
            for (i, c) in live.iter().enumerate() {
                if i < limit {
                    let child_limit = if depth > 1 {
                        (limit / 2).max(10)
                    } else {
                        limit
                    };
                    if let Some(v) = self.view(*c, depth - 1, child_limit) {
                        children.push(v);
                    }
                } else {
                    rest_bytes += self.nodes[*c as usize].bytes;
                    rest_count += 1;
                }
            }
        }
        Some(NodeView {
            id,
            name: if node.kind == NodeKind::Small {
                String::new()
            } else {
                node.name.clone()
            },
            path: self.path_of(id).to_string_lossy().into_owned(),
            kind: node.kind,
            category: node.category,
            bytes: node.bytes,
            files: node.files,
            mtime: node.mtime,
            denied: node.denied,
            reclaim_safe: node.reclaim_safe,
            reclaim_review: node.reclaim_review,
            marked: node.marked,
            child_count: live.len() as u32,
            children,
            rest_bytes,
            rest_count,
        })
    }

    /// Ids from the root down to `id`, for breadcrumbs.
    pub fn ancestors(&self, id: u32) -> Vec<(u32, String)> {
        let mut out = Vec::new();
        let mut cur = id;
        while cur != NO_PARENT {
            let Some(n) = self.nodes.get(cur as usize) else {
                break;
            };
            out.push((cur, n.name.clone()));
            cur = n.parent;
        }
        out.reverse();
        out
    }

    /// Folders the scan could not read, shallowest first.
    pub fn denied_paths(&self, limit: usize) -> Vec<String> {
        let depth = |mut id: u32| {
            let mut d = 0usize;
            while id != NO_PARENT && id != 0 {
                id = self.nodes[id as usize].parent;
                d += 1;
            }
            d
        };
        let mut ids: Vec<(usize, u32)> = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.denied && !n.removed)
            .map(|(i, _)| (depth(i as u32), i as u32))
            .collect();
        ids.sort_unstable();
        ids.into_iter()
            .take(limit)
            .map(|(_, id)| self.path_of(id).to_string_lossy().into_owned())
            .collect()
    }

    pub fn large_files(&self, min_bytes: u64, limit: usize) -> Vec<FileView> {
        let mut files: Vec<(u32, &Node)> = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.kind == NodeKind::File && !n.removed && n.bytes >= min_bytes)
            .map(|(i, n)| (i as u32, n))
            .collect();
        files.sort_by_key(|f| std::cmp::Reverse(f.1.bytes));
        files
            .into_iter()
            .take(limit)
            .map(|(id, n)| FileView {
                id,
                name: n.name.clone(),
                path: self.path_of(id).to_string_lossy().into_owned(),
                bytes: n.bytes,
                mtime: n.mtime,
                category: n.category,
            })
            .collect()
    }

    /// Finds a node by absolute path (case-insensitive).
    pub fn find(&self, path: &Path) -> Option<u32> {
        let root_key = key(&self.root_path);
        let target = key(path);
        if target == root_key {
            return Some(0);
        }
        let rest = if root_key.ends_with('\\') {
            target.strip_prefix(&root_key)?
        } else {
            target.strip_prefix(&root_key)?.strip_prefix('\\')?
        };
        let mut cur = 0u32;
        for seg in rest.split('\\').filter(|s| !s.is_empty()) {
            let node = &self.nodes[cur as usize];
            cur = *node.children.iter().find(|c| {
                let n = &self.nodes[**c as usize];
                n.kind != NodeKind::Small && !n.removed && n.name.to_lowercase() == seg
            })?;
        }
        Some(cur)
    }

    /// Marks reclaimable folders from the latest junk and artifact scans, so the "Cleanup" view
    /// can paint them. Each entry is an absolute folder path, its risk and its reclaimable bytes.
    pub fn mark(&mut self, entries: &[(PathBuf, Risk, u64)]) {
        for n in &mut self.nodes {
            n.reclaim_safe = 0;
            n.reclaim_review = 0;
            n.marked = None;
        }
        for (path, risk, bytes) in entries {
            let Some(id) = self.find(path) else { continue };
            if self.nodes[id as usize].marked.is_some() {
                continue;
            }
            // Never count more than the node holds; a rule may measure files the tree folded.
            let amount = (*bytes).min(self.nodes[id as usize].bytes);
            self.nodes[id as usize].marked = Some(*risk);
            let mut cur = id;
            while cur != NO_PARENT {
                let n = &mut self.nodes[cur as usize];
                match risk {
                    Risk::Safe => n.reclaim_safe += amount,
                    _ => n.reclaim_review += amount,
                }
                cur = n.parent;
            }
        }
    }

    /// Drops a deleted path from the tree and subtracts its size from every ancestor.
    pub fn remove(&mut self, path: &Path) -> bool {
        let Some(id) = self.find(path) else {
            return false;
        };
        if id == 0 {
            return false;
        }
        let (bytes, files) = (self.nodes[id as usize].bytes, self.nodes[id as usize].files);
        let (rs, rr) = (
            self.nodes[id as usize].reclaim_safe,
            self.nodes[id as usize].reclaim_review,
        );
        self.nodes[id as usize].removed = true;
        let mut cur = self.nodes[id as usize].parent;
        while cur != NO_PARENT {
            let n = &mut self.nodes[cur as usize];
            n.bytes = n.bytes.saturating_sub(bytes);
            n.files = n.files.saturating_sub(files);
            n.reclaim_safe = n.reclaim_safe.saturating_sub(rs);
            n.reclaim_review = n.reclaim_review.saturating_sub(rr);
            cur = n.parent;
        }
        self.summary.bytes = self.nodes[0].bytes;
        self.summary.files = self.nodes[0].files;
        true
    }
}

fn flatten(
    raw: RawDir,
    parent: u32,
    category: Category,
    depth: usize,
    nodes: &mut Vec<Node>,
) -> u32 {
    let id = nodes.len() as u32;
    nodes.push(Node {
        name: raw.name,
        parent,
        kind: NodeKind::Dir,
        bytes: raw.bytes,
        files: raw.files,
        mtime: raw.mtime,
        category,
        denied: raw.denied,
        children: Vec::new(),
        reclaim_safe: 0,
        reclaim_review: 0,
        marked: None,
        removed: false,
    });
    let mut kids: Vec<(u64, u32)> = Vec::new();
    for d in raw.dirs {
        let cat = classify_dir(&d.name, category, depth + 1, d.is_project);
        let bytes = d.bytes;
        let cid = flatten(d, id, cat, depth + 1, nodes);
        kids.push((bytes, cid));
    }
    for (name, bytes, t) in raw.big {
        let cid = nodes.len() as u32;
        let cat = classify_file(&name, category, depth + 1);
        nodes.push(leaf(name, id, NodeKind::File, bytes, 1, t, cat));
        kids.push((bytes, cid));
    }
    if raw.small_files > 0 {
        let cid = nodes.len() as u32;
        nodes.push(leaf(
            String::from("*"),
            id,
            NodeKind::Small,
            raw.small_bytes,
            raw.small_files,
            raw.small_mtime,
            category,
        ));
        kids.push((raw.small_bytes, cid));
    }
    kids.sort_by_key(|k| std::cmp::Reverse(k.0));
    nodes[id as usize].children = kids.into_iter().map(|(_, c)| c).collect();
    id
}

fn leaf(
    name: String,
    parent: u32,
    kind: NodeKind,
    bytes: u64,
    files: u64,
    mtime: i64,
    category: Category,
) -> Node {
    Node {
        name,
        parent,
        kind,
        bytes,
        files,
        mtime,
        category,
        denied: false,
        children: Vec::new(),
        reclaim_safe: 0,
        reclaim_review: 0,
        marked: None,
        removed: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(path: &Path, bytes: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::File::create(path)
            .unwrap()
            .write_all(&vec![1u8; bytes])
            .unwrap();
    }

    #[test]
    fn builds_sorted_tree_with_big_and_small_files() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join("proj/package.json"), 10);
        write(&tmp.path().join("proj/node_modules/a.js"), 100);
        write(&tmp.path().join("media/movie.mkv"), BIG_FILE as usize + 5);
        write(&tmp.path().join("media/note.txt"), 3);
        let job = Job::new();
        let mut tree = Tree::scan(tmp.path(), &job);
        assert_eq!(tree.summary.files, 4);
        let root = tree.view(0, 2, 50).unwrap();
        assert_eq!(root.children[0].name, "media");
        assert_eq!(root.children[0].children[0].kind, NodeKind::File);
        let proj = tree.find(&tmp.path().join("proj")).unwrap();
        assert_eq!(tree.nodes[proj as usize].category, Category::Projects);
        let movie = tree.find(&tmp.path().join("media/movie.mkv")).unwrap();
        assert_eq!(tree.nodes[movie as usize].category, Category::Media);
        assert_eq!(tree.large_files(BIG_FILE, 10).len(), 1);

        let nm = tmp.path().join("proj/node_modules");
        tree.mark(&[(nm.clone(), Risk::Safe, 100)]);
        assert_eq!(tree.nodes[0].reclaim_safe, 100);
        assert!(tree.remove(&nm));
        assert_eq!(tree.summary.files, 3);
        assert_eq!(tree.nodes[0].reclaim_safe, 0);
        assert!(tree.find(&nm).is_none());
    }

    #[test]
    fn scans_a_selection_rooted_at_the_parent() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join("a/one.bin"), 10);
        write(&tmp.path().join("b/two.bin"), 20);
        write(&tmp.path().join("c/three.bin"), 40);
        write(&tmp.path().join("loose.bin"), 80);
        let tree = Tree::scan_selection(tmp.path(), &["a".into(), "c".into()], &Job::new());
        assert_eq!(tree.summary.bytes, 50);
        assert_eq!(
            tree.summary.selection,
            vec!["a".to_string(), "c".to_string()]
        );
        let c = tree.find(&tmp.path().join("c")).unwrap();
        assert_eq!(tree.path_of(c), tmp.path().join("c"));
        assert!(tree.find(&tmp.path().join("b")).is_none());
        assert!(tree.denied_paths(5).is_empty());
        assert_eq!(tree.summary.cloud_files, 0);
    }
}
