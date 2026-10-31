//! Structured views of folders that grow messy: local AI model stores (Hugging Face, LM Studio,
//! Ollama, ComfyUI), the data AI coding assistants keep (Claude Code, Codex, Gemini CLI), and
//! the Downloads folder. Everything here only reads; deletes go through `clean::clean_paths` or
//! the tool's own command.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use xxhash_rust::xxh3::Xxh3;

use crate::fsmeta::{is_cloud_only, is_link, mtime};
use crate::job::Job;
use crate::paths::env_path;
use crate::rules::Risk;
use crate::walk::measure_dir;

const MODEL_EXTS: &[&str] = &[
    "gguf",
    "safetensors",
    "ckpt",
    "pt",
    "pth",
    "bin",
    "onnx",
    "ggml",
    "pb",
    "h5",
    "tflite",
    "mlmodel",
    "sft",
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiModel {
    pub id: String,
    pub name: String,
    /// `huggingface`, `lmstudio`, `ollama` or `comfyui`.
    pub source: String,
    /// Model type inside the source: `model`, `dataset`, `checkpoints`, `loras`...
    pub kind: String,
    pub formats: Vec<String>,
    pub bytes: u64,
    pub files: u64,
    pub mtime: i64,
    pub path: String,
    /// Folders or files to delete to remove this model (empty when only `command` is safe).
    pub remove: Vec<String>,
    /// Tool command that removes it properly (Ollama shares layers between models).
    pub command: Option<String>,
    /// Ids of models in other tools holding the same weights.
    pub same_as: Vec<String>,
}

fn home() -> PathBuf {
    env_path("USERPROFILE").unwrap_or_else(|| PathBuf::from(r"C:\Users\Default"))
}

fn ext_of(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, e)| e.to_lowercase())
        .unwrap_or_default()
}

/// Weight files below `dir`: (file name, size, path).
fn weight_files(dir: &Path, depth: usize, out: &mut Vec<(String, u64, PathBuf)>) {
    if depth > 6 {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        if is_link(&ft) {
            continue;
        }
        if ft.is_dir() {
            weight_files(&e.path(), depth + 1, out);
        } else if let Ok(md) = e.metadata() {
            let name = e.file_name().to_string_lossy().into_owned();
            if MODEL_EXTS.contains(&ext_of(&name).as_str())
                && md.len() > 1024 * 1024
                && !is_cloud_only(&md)
            {
                out.push((name, md.len(), e.path()));
            }
        }
    }
}

fn model_entry(
    id: String,
    name: String,
    source: &str,
    kind: &str,
    dir: &Path,
    remove: Vec<String>,
    job: &Job,
) -> AiModel {
    let m = measure_dir(dir, None, job);
    let mut weights = Vec::new();
    weight_files(dir, 0, &mut weights);
    let mut formats: Vec<String> = weights.iter().map(|(n, _, _)| ext_of(n)).collect();
    formats.sort();
    formats.dedup();
    AiModel {
        id,
        name,
        source: source.into(),
        kind: kind.into(),
        formats,
        bytes: m.bytes,
        files: m.files,
        mtime: m.newest,
        path: dir.to_string_lossy().into_owned(),
        remove,
        command: None,
        same_as: Vec::new(),
    }
}

/// Hugging Face hub cache: `models--org--name`, `datasets--...`, `spaces--...`.
fn huggingface(job: &Job) -> Vec<AiModel> {
    let hub = env_path("HF_HUB_CACHE")
        .or_else(|| env_path("HF_HOME").map(|h| h.join("hub")))
        .unwrap_or_else(|| home().join(r".cache\huggingface\hub"));
    let Ok(rd) = fs::read_dir(&hub) else {
        return Vec::new();
    };
    let dirs: Vec<(String, PathBuf)> = rd
        .flatten()
        .filter(|e| {
            e.file_type()
                .map(|t| t.is_dir() && !is_link(&t))
                .unwrap_or(false)
        })
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .collect();
    dirs.par_iter()
        .filter_map(|(name, path)| {
            let (kind, rest) = name.split_once("--")?;
            let kind = match kind {
                "models" => "model",
                "datasets" => "dataset",
                "spaces" => "space",
                _ => return None,
            };
            let display = rest.replace("--", "/");
            Some(model_entry(
                format!("hf:{name}"),
                display,
                "huggingface",
                kind,
                path,
                vec![path.to_string_lossy().into_owned()],
                job,
            ))
        })
        .collect()
}

/// LM Studio: `models/<publisher>/<repo>/files`, in the current and the legacy location.
fn lmstudio(job: &Job) -> Vec<AiModel> {
    let roots = [
        home().join(r".lmstudio\models"),
        home().join(r".cache\lm-studio\models"),
    ];
    let mut repos = Vec::new();
    for root in roots {
        let Ok(publishers) = fs::read_dir(&root) else {
            continue;
        };
        for p in publishers.flatten() {
            if !p
                .file_type()
                .map(|t| t.is_dir() && !is_link(&t))
                .unwrap_or(false)
            {
                continue;
            }
            let Ok(models) = fs::read_dir(p.path()) else {
                continue;
            };
            for m in models.flatten() {
                if m.file_type()
                    .map(|t| t.is_dir() && !is_link(&t))
                    .unwrap_or(false)
                {
                    repos.push((
                        p.file_name().to_string_lossy().into_owned(),
                        m.file_name().to_string_lossy().into_owned(),
                        m.path(),
                    ));
                }
            }
        }
    }
    repos
        .par_iter()
        .map(|(publisher, repo, path)| {
            model_entry(
                format!("lms:{publisher}/{repo}"),
                format!("{publisher}/{repo}"),
                "lmstudio",
                "model",
                path,
                vec![path.to_string_lossy().into_owned()],
                job,
            )
        })
        .collect()
}

#[derive(Deserialize)]
struct OllamaLayer {
    digest: String,
    size: u64,
}

#[derive(Deserialize)]
struct OllamaManifest {
    #[serde(default)]
    layers: Vec<OllamaLayer>,
    config: Option<OllamaLayer>,
}

/// Ollama: manifests name the layers; blobs are shared between tags, so removal goes through
/// `ollama rm` instead of deleting files.
fn ollama() -> Vec<AiModel> {
    let models = env_path("OLLAMA_MODELS").unwrap_or_else(|| home().join(r".ollama\models"));
    let base = models.join("manifests");
    let mut out = Vec::new();
    let mut stack = vec![(base.clone(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() && depth < 4 {
                stack.push((e.path(), depth + 1));
                continue;
            }
            if !ft.is_file() {
                continue;
            }
            let path = e.path();
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(manifest) = serde_json::from_str::<OllamaManifest>(&text) else {
                continue;
            };
            // manifests/<host>/<namespace>/<model>/<tag>
            let rel: Vec<String> = path
                .strip_prefix(&base)
                .ok()
                .map(|r| {
                    r.components()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            if rel.len() != 4 {
                continue;
            }
            let (host, ns, model, tag) = (&rel[0], &rel[1], &rel[2], &rel[3]);
            let mut name = if ns == "library" {
                format!("{model}:{tag}")
            } else {
                format!("{ns}/{model}:{tag}")
            };
            if host != "registry.ollama.ai" {
                name = format!("{host}/{name}");
            }
            let bytes = manifest.layers.iter().map(|l| l.size).sum::<u64>()
                + manifest.config.as_ref().map(|c| c.size).unwrap_or(0);
            let digest = manifest
                .layers
                .iter()
                .max_by_key(|l| l.size)
                .map(|l| l.digest.replace(':', "-"));
            let blob = digest
                .map(|d| models.join("blobs").join(d))
                .filter(|p| p.is_file());
            out.push(AiModel {
                id: format!("ollama:{name}"),
                name: name.clone(),
                source: "ollama".into(),
                kind: "model".into(),
                formats: vec!["gguf".into()],
                bytes,
                files: manifest.layers.len() as u64,
                mtime: e.metadata().map(|m| mtime(&m)).unwrap_or(0),
                path: blob
                    .unwrap_or_else(|| path.clone())
                    .to_string_lossy()
                    .into_owned(),
                remove: Vec::new(),
                command: crate::system::which("ollama").map(|_| format!("ollama rm {name}")),
                same_as: Vec::new(),
            });
        }
    }
    out
}

/// ComfyUI installs: the desktop app's base path plus common folders.
fn comfy_roots(extra: &[PathBuf]) -> Vec<PathBuf> {
    let h = home();
    let mut candidates = vec![
        h.join(r"Documents\ComfyUI"),
        h.join("ComfyUI"),
        h.join(r"Desktop\ComfyUI"),
        h.join(r"ComfyUI_windows_portable\ComfyUI"),
    ];
    if let Some(appdata) = env_path("APPDATA")
        && let Ok(text) = fs::read_to_string(appdata.join(r"ComfyUI\config.json"))
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(&text)
        && let Some(base) = v.get("basePath").and_then(|b| b.as_str())
    {
        candidates.push(PathBuf::from(base));
    }
    for root in extra {
        if let Ok(rd) = fs::read_dir(root) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_lowercase();
                if n.contains("comfy") {
                    candidates.push(e.path());
                    candidates.push(e.path().join("ComfyUI"));
                }
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    candidates
        .into_iter()
        .filter(|p| p.join("models").is_dir())
        .filter(|p| seen.insert(p.to_string_lossy().to_lowercase()))
        .collect()
}

fn comfyui(extra: &[PathBuf]) -> Vec<AiModel> {
    let mut out = Vec::new();
    for root in comfy_roots(extra) {
        let Ok(kinds) = fs::read_dir(root.join("models")) else {
            continue;
        };
        for k in kinds.flatten() {
            if !k
                .file_type()
                .map(|t| t.is_dir() && !is_link(&t))
                .unwrap_or(false)
            {
                continue;
            }
            let kind = k.file_name().to_string_lossy().into_owned();
            let mut files = Vec::new();
            weight_files(&k.path(), 0, &mut files);
            for (name, size, path) in files {
                let t = fs::metadata(&path).map(|m| mtime(&m)).unwrap_or(0);
                out.push(AiModel {
                    id: format!("comfy:{}", path.to_string_lossy()),
                    name,
                    source: "comfyui".into(),
                    kind: kind.clone(),
                    formats: vec![ext_of(&path.to_string_lossy())],
                    bytes: size,
                    files: 1,
                    mtime: t,
                    path: path.to_string_lossy().into_owned(),
                    remove: vec![path.to_string_lossy().into_owned()],
                    command: None,
                    same_as: Vec::new(),
                });
            }
        }
    }
    out
}

/// First and last 64 KB of a file, to confirm two equally sized weights are the same.
fn edge_hash(path: &Path, size: u64) -> Option<u64> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = fs::File::open(path).ok()?;
    let mut h = Xxh3::new();
    let mut buf = vec![0u8; 64 * 1024];
    let n = f.read(&mut buf).ok()?;
    h.update(&buf[..n]);
    if size > 128 * 1024 {
        f.seek(SeekFrom::End(-(64 * 1024))).ok()?;
        let n = f.read(&mut buf).ok()?;
        h.update(&buf[..n]);
    }
    h.update(&size.to_le_bytes());
    Some(h.digest())
}

/// Marks models in different tools that hold the same weight files (same name, size and
/// edges), the classic "downloaded the same GGUF for LM Studio and from Hugging Face".
fn link_duplicates(models: &mut [AiModel]) {
    let mut by_file: HashMap<(String, u64), Vec<(usize, PathBuf)>> = HashMap::new();
    for (i, m) in models.iter().enumerate() {
        let mut files = Vec::new();
        let p = PathBuf::from(&m.path);
        if p.is_dir() {
            weight_files(&p, 0, &mut files);
        } else if let Ok(md) = fs::metadata(&p) {
            files.push((
                p.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                md.len(),
                p.clone(),
            ));
        }
        for (name, size, path) in files {
            if size >= 64 * 1024 * 1024 {
                by_file
                    .entry((name.to_lowercase(), size))
                    .or_default()
                    .push((i, path));
            }
        }
    }
    for ((_, size), entries) in by_file {
        let owners: std::collections::HashSet<usize> = entries.iter().map(|(i, _)| *i).collect();
        if owners.len() < 2 {
            continue;
        }
        let hashes: Vec<(usize, Option<u64>)> = entries
            .iter()
            .map(|(i, p)| (*i, edge_hash(p, size)))
            .collect();
        for (a, ha) in &hashes {
            for (b, hb) in &hashes {
                if a != b && ha.is_some() && ha == hb && models[*a].source != models[*b].source {
                    let other = models[*b].id.clone();
                    if !models[*a].same_as.contains(&other) {
                        models[*a].same_as.push(other);
                    }
                }
            }
        }
    }
}

pub fn ai_models(extra_roots: &[PathBuf], job: &Job) -> Vec<AiModel> {
    job.set_stage("models");
    let mut all = Vec::new();
    all.extend(huggingface(job));
    all.extend(lmstudio(job));
    all.extend(ollama());
    all.extend(comfyui(extra_roots));
    // Entries that hold only metadata (a model downloaded through another tool) are noise.
    all.retain(|m| m.bytes >= 1024 * 1024);
    link_duplicates(&mut all);
    all.sort_by_key(|m| std::cmp::Reverse(m.bytes));
    all
}

/// Removes an Ollama model with `ollama rm`, which keeps layers other models still use. The id
/// must match a model listed right now, so no arbitrary argument reaches the command.
pub fn ollama_remove(id: &str) -> crate::Result<()> {
    let model = ollama()
        .into_iter()
        .find(|m| m.id == id)
        .ok_or_else(|| crate::Error::Other("model not found".into()))?;
    let exe = crate::system::which("ollama")
        .ok_or_else(|| crate::Error::Other("ollama is not installed or not on PATH".into()))?;
    let mut cmd = std::process::Command::new(exe);
    cmd.args(["rm", &model.name]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console flashes up.
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd
        .output()
        .map_err(|e| crate::Error::Other(e.to_string()))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(crate::Error::Other(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ))
    }
}

// ---------------------------------------------------------------- assistant data

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataItem {
    pub name: String,
    pub path: String,
    pub bytes: u64,
    pub files: u64,
    pub mtime: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantData {
    /// Stable id, also the translation key: `claude-projects`, `codex-sessions`...
    pub id: String,
    pub tool: String,
    pub path: String,
    pub bytes: u64,
    pub files: u64,
    pub mtime: i64,
    pub risk: Risk,
    /// Per-project or per-month breakdown when the folder has one.
    pub items: Vec<DataItem>,
}

struct DataSpec {
    id: &'static str,
    tool: &'static str,
    rel: &'static str,
    risk: Risk,
    /// Break the folder down by its direct children.
    split: bool,
}

/// Only these subfolders are ever listed. Credentials, settings, skills and plugins in the same
/// tool folders are never touched.
const SPECS: &[DataSpec] = &[
    DataSpec {
        id: "claude-projects",
        tool: "Claude Code",
        rel: r".claude\projects",
        risk: Risk::Review,
        split: true,
    },
    DataSpec {
        id: "claude-file-history",
        tool: "Claude Code",
        rel: r".claude\file-history",
        risk: Risk::Review,
        split: true,
    },
    DataSpec {
        id: "claude-shell-snapshots",
        tool: "Claude Code",
        rel: r".claude\shell-snapshots",
        risk: Risk::Safe,
        split: false,
    },
    DataSpec {
        id: "claude-todos",
        tool: "Claude Code",
        rel: r".claude\todos",
        risk: Risk::Safe,
        split: false,
    },
    DataSpec {
        id: "claude-cache",
        tool: "Claude Code",
        rel: r".claude\cache",
        risk: Risk::Safe,
        split: false,
    },
    DataSpec {
        id: "claude-statsig",
        tool: "Claude Code",
        rel: r".claude\statsig",
        risk: Risk::Safe,
        split: false,
    },
    DataSpec {
        id: "claude-debug",
        tool: "Claude Code",
        rel: r".claude\debug",
        risk: Risk::Safe,
        split: false,
    },
    DataSpec {
        id: "claude-backups",
        tool: "Claude Code",
        rel: r".claude\backups",
        risk: Risk::Review,
        split: false,
    },
    DataSpec {
        id: "codex-sessions",
        tool: "Codex",
        rel: r".codex\sessions",
        risk: Risk::Review,
        split: true,
    },
    DataSpec {
        id: "codex-archived",
        tool: "Codex",
        rel: r".codex\archived_sessions",
        risk: Risk::Review,
        split: false,
    },
    DataSpec {
        id: "codex-log",
        tool: "Codex",
        rel: r".codex\log",
        risk: Risk::Safe,
        split: false,
    },
    DataSpec {
        id: "gemini-tmp",
        tool: "Gemini CLI",
        rel: r".gemini\tmp",
        risk: Risk::Review,
        split: true,
    },
    DataSpec {
        id: "gemini-history",
        tool: "Gemini CLI",
        rel: r".gemini\history",
        risk: Risk::Review,
        split: false,
    },
];

pub fn assistant_data(job: &Job) -> Vec<AssistantData> {
    job.set_stage("assistants");
    let h = home();
    let mut out: Vec<AssistantData> = SPECS
        .par_iter()
        .filter_map(|spec| {
            let dir = h.join(spec.rel);
            if !dir.is_dir()
                || dir
                    .symlink_metadata()
                    .map(|m| is_link(&m.file_type()))
                    .unwrap_or(true)
            {
                return None;
            }
            let m = measure_dir(&dir, None, job);
            let mut items = Vec::new();
            if spec.split
                && let Ok(rd) = fs::read_dir(&dir)
            {
                for e in rd.flatten() {
                    let Ok(ft) = e.file_type() else { continue };
                    if is_link(&ft) {
                        continue;
                    }
                    let (bytes, files, t) = if ft.is_dir() {
                        let mm = measure_dir(&e.path(), None, job);
                        (mm.bytes, mm.files, mm.newest)
                    } else {
                        let md = e.metadata().ok();
                        (
                            md.as_ref().map(|m| m.len()).unwrap_or(0),
                            1,
                            md.as_ref().map(mtime).unwrap_or(0),
                        )
                    };
                    if bytes > 0 {
                        items.push(DataItem {
                            name: e.file_name().to_string_lossy().into_owned(),
                            path: e.path().to_string_lossy().into_owned(),
                            bytes,
                            files,
                            mtime: t,
                        });
                    }
                }
                items.sort_by_key(|i| std::cmp::Reverse(i.bytes));
            }
            (m.bytes > 0).then(|| AssistantData {
                id: spec.id.into(),
                tool: spec.tool.into(),
                path: dir.to_string_lossy().into_owned(),
                bytes: m.bytes,
                files: m.files,
                mtime: m.newest,
                risk: spec.risk,
                items,
            })
        })
        .collect();
    // Codex leaves one temp copy of its state file per crash or update; old ones are junk.
    let codex = h.join(".codex");
    if let Ok(rd) = fs::read_dir(&codex) {
        let cutoff = crate::fsmeta::now_unix() - 86_400;
        let items: Vec<DataItem> = rd
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("..codex-global-state.json.")
            })
            .filter_map(|e| {
                let md = e.metadata().ok()?;
                (md.is_file() && mtime(&md) < cutoff).then(|| DataItem {
                    name: e.file_name().to_string_lossy().into_owned(),
                    path: e.path().to_string_lossy().into_owned(),
                    bytes: md.len(),
                    files: 1,
                    mtime: mtime(&md),
                })
            })
            .collect();
        if !items.is_empty() {
            out.push(AssistantData {
                id: "codex-temp-state".into(),
                tool: "Codex".into(),
                path: codex.to_string_lossy().into_owned(),
                bytes: items.iter().map(|i| i.bytes).sum(),
                files: items.len() as u64,
                mtime: items.iter().map(|i| i.mtime).max().unwrap_or(0),
                risk: Risk::Safe,
                items,
            });
        }
    }
    out.sort_by_key(|d| std::cmp::Reverse(d.bytes));
    out
}

// ---------------------------------------------------------------- downloads

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadFile {
    pub path: String,
    pub name: String,
    /// Folder inside Downloads ("" for the top level).
    pub folder: String,
    pub ext: String,
    pub bytes: u64,
    pub mtime: i64,
    /// Looks like a browser re-download (`name (1).ext`) of a file with the same size.
    pub copy: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadsReport {
    pub root: String,
    pub bytes: u64,
    pub files: Vec<DownloadFile>,
    pub truncated: bool,
}

const DOWNLOAD_LIMIT: usize = 20_000;

/// `report (1).pdf` -> `report.pdf`
fn original_name(name: &str) -> Option<String> {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s, Some(e)),
        None => (name, None),
    };
    let open = stem.rfind(" (")?;
    let inner = stem[open + 2..].strip_suffix(')')?;
    if inner.is_empty() || !inner.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let base = &stem[..open];
    Some(match ext {
        Some(e) => format!("{base}.{e}"),
        None => base.to_string(),
    })
}

pub fn downloads(root: &Path, job: &Job) -> DownloadsReport {
    job.set_stage("downloads");
    let mut files = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut truncated = false;
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if is_link(&ft) {
                continue;
            }
            if ft.is_dir() {
                if depth < 4 {
                    stack.push((e.path(), depth + 1));
                }
                continue;
            }
            let Ok(md) = e.metadata() else { continue };
            if is_cloud_only(&md) {
                continue;
            }
            if files.len() >= DOWNLOAD_LIMIT {
                truncated = true;
                break;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            if name.eq_ignore_ascii_case("desktop.ini") {
                continue;
            }
            let folder = dir
                .strip_prefix(root)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            files.push(DownloadFile {
                path: e.path().to_string_lossy().into_owned(),
                ext: ext_of(&name),
                name,
                folder,
                bytes: md.len(),
                mtime: mtime(&md),
                copy: false,
            });
            job.add_files(1, md.len());
        }
    }
    // Flag "name (1).ext" files whose original sits in the same folder with the same size.
    let index: HashMap<(String, String), u64> = files
        .iter()
        .map(|f| ((f.folder.to_lowercase(), f.name.to_lowercase()), f.bytes))
        .collect();
    for f in &mut files {
        if let Some(orig) = original_name(&f.name) {
            f.copy = index.get(&(f.folder.to_lowercase(), orig.to_lowercase())) == Some(&f.bytes);
        }
    }
    files.sort_by_key(|f| std::cmp::Reverse(f.bytes));
    DownloadsReport {
        root: root.to_string_lossy().into_owned(),
        bytes: files.iter().map(|f| f.bytes).sum(),
        files,
        truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_redownload_names() {
        assert_eq!(
            original_name("report (1).pdf").as_deref(),
            Some("report.pdf")
        );
        assert_eq!(
            original_name("setup (12).exe").as_deref(),
            Some("setup.exe")
        );
        assert_eq!(original_name("notes (draft).txt"), None);
        assert_eq!(original_name("plain.txt"), None);
        assert_eq!(original_name("archive (2)").as_deref(), Some("archive"));
    }

    #[test]
    fn flags_copies_in_downloads() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a.pdf"), [1u8; 10]).unwrap();
        fs::write(tmp.path().join("a (1).pdf"), [1u8; 10]).unwrap();
        fs::write(tmp.path().join("b (1).pdf"), [1u8; 10]).unwrap();
        fs::create_dir_all(tmp.path().join("sub")).unwrap();
        fs::write(tmp.path().join("sub/c.zip"), [1u8; 5]).unwrap();
        let r = downloads(tmp.path(), &Job::new());
        assert_eq!(r.files.len(), 4);
        assert!(r.files.iter().find(|f| f.name == "a (1).pdf").unwrap().copy);
        assert!(!r.files.iter().find(|f| f.name == "b (1).pdf").unwrap().copy);
        assert_eq!(
            r.files.iter().find(|f| f.name == "c.zip").unwrap().folder,
            "sub"
        );
    }

    #[test]
    fn links_identical_weights_across_tools() {
        let tmp = tempfile::tempdir().unwrap();
        let data: Vec<u8> = (0..70 * 1024 * 1024u32).map(|i| (i % 253) as u8).collect();
        let a = tmp.path().join("hf/snap");
        let b = tmp.path().join("lms/repo");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("Model-Q4.gguf"), &data).unwrap();
        fs::write(b.join("Model-Q4.gguf"), &data).unwrap();
        let job = Job::new();
        let mut models = vec![
            model_entry(
                "hf:x".into(),
                "x".into(),
                "huggingface",
                "model",
                &a,
                vec![],
                &job,
            ),
            model_entry(
                "lms:x".into(),
                "x".into(),
                "lmstudio",
                "model",
                &b,
                vec![],
                &job,
            ),
        ];
        link_duplicates(&mut models);
        assert_eq!(models[0].same_as, vec!["lms:x".to_string()]);
        assert_eq!(models[1].same_as, vec!["hf:x".to_string()]);
        assert_eq!(models[0].formats, vec!["gguf".to_string()]);
    }
}
